//! Carga de tensores desde shards mapeados vía index.som.

use crate::layer::{TensorSource, TensorView};
use crate::quant::{dequant_mxfp4, dequant_mxfp4_range, dequant_q4_k, dequant_q4_k_range, dequant_q8_0, dequant_q8_0_range};
use crate::stage::{PrefetchSink, SyncStager};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use core::sync::atomic::{AtomicU32, Ordering};
use sosomodel::index::{verify_shard, TensorEntry, TensorIndex};
use sosomodel::layout::{DTYPE_F32, DTYPE_MXFP4, DTYPE_Q4_K, DTYPE_Q8_0};

/// Región de fichero mapeada en memoria.
#[derive(Clone, Debug)]
pub struct MappedShard {
    pub addr: u64,
    pub len: usize,
}

/// Abstracción sobre open/stat/mmap/munmap (libsoso en userspace, memoria en tests).
pub trait FileMapper {
    fn map_file(&mut self, path: &str) -> Result<MappedShard, ()>;
    fn unmap_file(&mut self, shard: &MappedShard);
}

struct CachedShard {
    mapped: MappedShard,
    /// Desplazamiento del payload dentro del fichero (tras la cabecera .som).
    payload_off: usize,
    payload_len: usize,
    /// Prefetch a stride 2 MiB ya ejecutado sobre este shard.
    touched: bool,
}

/// Spinlock cooperativo para acceso concurrente al caché de shards (staging ∥ compute).
struct CacheLock(AtomicU32);

impl CacheLock {
    const fn new() -> Self {
        Self(AtomicU32::new(0))
    }

    fn lock(&self) {
        while self
            .0
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    fn unlock(&self) {
        self.0.store(0, Ordering::Release);
    }
}

/// TensorSource que lee tensores desde shards según index.som, con cache de
/// mmaps. El CRC del shard se verifica una sola vez, al mapearlo.
pub struct MmapTensorSource<M: FileMapper> {
    pub shards_base: String,
    pub index: TensorIndex,
    pub mapper: M,
    cache: BTreeMap<String, CachedShard>,
    cache_lock: CacheLock,
    /// Double-buffer AirLLM: kick/wait solapan I/O con cómputo.
    pub stager: SyncStager,
    /// Si true, kick/wait usan el stager; si false, prefetch síncrono clásico.
    pub async_staging: bool,
}

impl<M: FileMapper> MmapTensorSource<M> {
    pub fn new(shards_base: String, index: TensorIndex, mapper: M) -> Self {
        Self {
            shards_base,
            index,
            mapper,
            cache: BTreeMap::new(),
            cache_lock: CacheLock::new(),
            stager: SyncStager::new(),
            async_staging: true,
        }
    }

    pub fn with_sync_prefetch(mut self) -> Self {
        self.async_staging = false;
        self
    }

    fn shard_path(&self, shard: &str) -> String {
        format!("{}/{}", self.shards_base, shard)
    }

    fn ensure_mapped(&mut self, shard_name: &str) -> Result<(u64, usize, usize), ()> {
        self.cache_lock.lock();
        if let Some(cached) = self.cache.get(shard_name) {
            let out = (
                cached.mapped.addr,
                cached.mapped.len,
                cached.payload_off,
            );
            self.cache_lock.unlock();
            return Ok(out);
        }
        self.cache_lock.unlock();

        let path = self.shard_path(shard_name);
        let mapped = self.mapper.map_file(&path)?;
        let ptr = mapped.addr as *const u8;
        let data = unsafe { core::slice::from_raw_parts(ptr, mapped.len) };
        let payload = verify_shard(data).map_err(|_| ())?;
        let payload_off = payload.as_ptr() as usize - data.as_ptr() as usize;
        let entry = CachedShard {
            payload_off,
            payload_len: payload.len(),
            mapped,
            touched: false,
        };

        self.cache_lock.lock();
        if let Some(cached) = self.cache.get(shard_name) {
            let out = (
                cached.mapped.addr,
                cached.mapped.len,
                cached.payload_off,
            );
            self.cache_lock.unlock();
            self.mapper.unmap_file(&entry.mapped);
            return Ok(out);
        }
        self.cache.insert(String::from(shard_name), entry);
        let cached = self.cache.get(shard_name).ok_or(())?;
        let out = (
            cached.mapped.addr,
            cached.mapped.len,
            cached.payload_off,
        );
        self.cache_lock.unlock();
        Ok(out)
    }

    fn cached_payload_len(&mut self, shard_name: &str) -> Result<usize, ()> {
        self.cache_lock.lock();
        let out = self
            .cache
            .get(shard_name)
            .map(|c| c.payload_len)
            .ok_or(());
        self.cache_lock.unlock();
        out
    }

    /// Bytes `[off, off+len)` del payload de un tensor, sin re-verificar CRC.
    fn tensor_bytes(&mut self, entry: &TensorEntry, off: usize, len: usize) -> Result<&[u8], ()> {
        let end = off.checked_add(len).ok_or(())?;
        if end > entry.byte_len as usize {
            return Err(());
        }
        let base = usize::try_from(entry.offset).map_err(|_| ())?;
        let shard = entry.shard.clone();
        let (addr, map_len, payload_off) = self.ensure_mapped(&shard)?;
        let payload_len = self.cached_payload_len(&shard)?;
        let start = base.checked_add(off).ok_or(())?;
        if start + len > payload_len {
            return Err(());
        }
        // SAFETY: el mmap vive en `cache` hasta `release_shards_except`.
        let data = unsafe { core::slice::from_raw_parts(addr as *const u8, map_len) };
        Ok(&data[payload_off + start..payload_off + start + len])
    }

    pub fn drop_cache(&mut self) {
        self.cache_lock.lock();
        let keys: alloc::vec::Vec<String> = self.cache.keys().cloned().collect();
        self.cache_lock.unlock();
        for key in keys {
            self.cache_lock.lock();
            let entry = self.cache.remove(&key);
            self.cache_lock.unlock();
            if let Some(entry) = entry {
                self.mapper.unmap_file(&entry.mapped);
            }
        }
    }

    /// Mapea shards y toca páginas a stride de 2 MiB (page-fault adelantado
    /// alineado con el camino huge del kernel), no solo el primer byte.
    pub fn prefetch_shards_impl(&mut self, shards: &[String]) {
        const STRIDE: usize = 2 * 1024 * 1024;
        for name in shards {
            self.cache_lock.lock();
            if self.cache.get(name).is_some_and(|c| c.touched) {
                self.cache_lock.unlock();
                continue;
            }
            self.cache_lock.unlock();

            if let Ok((addr, len, _)) = self.ensure_mapped(name) {
                if len == 0 {
                    continue;
                }
                let ptr = addr as *const u8;
                let mut off = 0usize;
                while off < len {
                    let _ = unsafe { core::ptr::read_volatile(ptr.add(off)) };
                    off = off.saturating_add(STRIDE);
                    if off == 0 {
                        break;
                    }
                }
                self.cache_lock.lock();
                if let Some(c) = self.cache.get_mut(name) {
                    c.touched = true;
                }
                self.cache_lock.unlock();
            }
        }
    }

    /// Desmapea todo lo que no esté en `keep` (working set de capas).
    pub fn release_shards_except_impl(&mut self, keep: &[String]) {
        self.cache_lock.lock();
        let keys: alloc::vec::Vec<String> = self.cache.keys().cloned().collect();
        self.cache_lock.unlock();
        for key in keys {
            if keep.iter().any(|k| k == &key) {
                continue;
            }
            self.cache_lock.lock();
            let entry = self.cache.remove(&key);
            self.cache_lock.unlock();
            if let Some(entry) = entry {
                self.mapper.unmap_file(&entry.mapped);
            }
        }
    }
}

impl<M: FileMapper> PrefetchSink for MmapTensorSource<M> {
    fn prefetch_shards_sync(&mut self, shards: &[String]) {
        self.prefetch_shards_impl(shards);
    }
}

impl<M: FileMapper> Drop for MmapTensorSource<M> {
    fn drop(&mut self) {
        self.drop_cache();
    }
}

fn bytes_to_f32(bytes: &[u8], out: &mut [f32]) -> Result<(), ()> {
    for (o, chunk) in out.iter_mut().zip(bytes.chunks_exact(4)) {
        *o = f32::from_le_bytes(chunk.try_into().map_err(|_| ())?);
    }
    Ok(())
}

impl<M: FileMapper> TensorSource for MmapTensorSource<M> {
    fn load_f32(&mut self, name: &str, out: &mut [f32]) -> Result<(), ()> {
        let entry = self.index.find(name).ok_or(())?.clone();
        if entry.elems() != out.len() {
            return Err(());
        }
        match entry.dtype {
            DTYPE_F32 => {
                let bytes = self.tensor_bytes(&entry, 0, out.len() * 4)?;
                bytes_to_f32(bytes, out)
            }
            DTYPE_Q8_0 => {
                let bytes = self.tensor_bytes(&entry, 0, entry.byte_len as usize)?;
                dequant_q8_0(bytes, out)
            }
            DTYPE_Q4_K => {
                let bytes = self.tensor_bytes(&entry, 0, entry.byte_len as usize)?;
                dequant_q4_k(bytes, out)
            }
            DTYPE_MXFP4 => {
                let bytes = self.tensor_bytes(&entry, 0, entry.byte_len as usize)?;
                dequant_mxfp4(bytes, out)
            }
            _ => Err(()),
        }
    }

    fn load_f32_range(&mut self, name: &str, elem_off: usize, out: &mut [f32]) -> Result<(), ()> {
        let entry = self.index.find(name).ok_or(())?.clone();
        let end = elem_off.checked_add(out.len()).ok_or(())?;
        if end > entry.elems() {
            return Err(());
        }
        match entry.dtype {
            DTYPE_F32 => {
                let bytes = self.tensor_bytes(&entry, elem_off * 4, out.len() * 4)?;
                bytes_to_f32(bytes, out)
            }
            DTYPE_Q8_0 => {
                let bytes = self.tensor_bytes(&entry, 0, entry.byte_len as usize)?;
                dequant_q8_0_range(bytes, elem_off, out)
            }
            DTYPE_Q4_K => {
                let bytes = self.tensor_bytes(&entry, 0, entry.byte_len as usize)?;
                dequant_q4_k_range(bytes, elem_off, out)
            }
            DTYPE_MXFP4 => {
                let bytes = self.tensor_bytes(&entry, 0, entry.byte_len as usize)?;
                dequant_mxfp4_range(bytes, elem_off, out)
            }
            _ => Err(()),
        }
    }

    fn tensor_view(&mut self, name: &str) -> Result<TensorView<'_>, ()> {
        let entry = self.index.find(name).ok_or(())?.clone();
        let elems = entry.elems();
        let dtype = entry.dtype;
        let bytes = self.tensor_bytes(&entry, 0, entry.byte_len as usize)?;
        Ok(TensorView {
            bytes,
            dtype,
            elems,
        })
    }

    fn prefetch_shards(&mut self, shards: &[String]) {
        self.prefetch_shards_impl(shards);
    }

    fn kick_prefetch_shards(&mut self, shards: &[String]) {
        if self.async_staging {
            self.stager.kick_layer(shards);
        } else {
            self.prefetch_shards_impl(shards);
        }
    }

    fn wait_prefetch(&mut self) {
        if self.async_staging {
            let shards = self.stager.drain_layer_shards();
            if !shards.is_empty() {
                self.prefetch_shards_impl(&shards);
            }
        }
    }

    fn kick_moe_prefetch(&mut self, shards: &[String]) {
        if self.async_staging {
            self.stager.kick_moe(shards);
        } else if !shards.is_empty() {
            self.prefetch_shards_impl(shards);
        }
    }

    fn wait_moe_prefetch(&mut self) {
        if self.async_staging {
            let shards = self.stager.drain_moe_shards();
            if !shards.is_empty() {
                self.prefetch_shards_impl(&shards);
            }
        }
    }

    fn release_shards_except(&mut self, keep: &[String]) {
        self.release_shards_except_impl(keep);
    }

    fn prefetch_embed_row(&mut self, token: u32, hidden: usize) {
        let Some(entry) = self.index.find("embed") else {
            return;
        };
        let entry = entry.clone();
        let elem_off = token as usize * hidden;
        if elem_off >= entry.elems() {
            return;
        }
        let byte_off = match entry.dtype {
            DTYPE_F32 => elem_off * 4,
            _ => 0, // cuantizado: tocar inicio del shard basta
        };
        let need = if entry.dtype == DTYPE_F32 {
            (hidden * 4).min(entry.byte_len as usize - byte_off)
        } else {
            64.min(entry.byte_len as usize)
        };
        if let Ok(bytes) = self.tensor_bytes(&entry, byte_off, need.max(1).min(4096)) {
            if !bytes.is_empty() {
                let _ = unsafe { core::ptr::read_volatile(bytes.as_ptr()) };
            }
        }
    }
}

#[cfg(feature = "std")]
pub mod host {
    use super::*;
    use std::collections::BTreeMap as StdMap;

    /// FileMapper en memoria para tests host.
    pub struct MemFileMapper {
        pub files: StdMap<String, Vec<u8>>,
    }

    impl MemFileMapper {
        pub fn new() -> Self {
            Self {
                files: StdMap::new(),
            }
        }
    }

    impl FileMapper for MemFileMapper {
        fn map_file(&mut self, path: &str) -> Result<MappedShard, ()> {
            let data = self.files.get(path).ok_or(())?;
            let boxed = data.clone().into_boxed_slice();
            let len = boxed.len();
            let ptr = Box::into_raw(boxed) as *mut u8;
            Ok(MappedShard {
                addr: ptr as u64,
                len,
            })
        }

        fn unmap_file(&mut self, shard: &MappedShard) {
            unsafe {
                let _ = Box::from_raw(core::slice::from_raw_parts_mut(
                    shard.addr as *mut u8,
                    shard.len,
                ));
            }
        }
    }

    /// TensorSource con prefetch en hilo std (solape real I/O ∥ compute en host).
    pub struct ThreadStagedSource<M: FileMapper + Send + 'static> {
        inner: MmapTensorSource<M>,
        worker: HostStagingWorker,
    }

    fn prefetch_erased<M: FileMapper>(ptr: *mut (), shards: &[String]) {
        // SAFETY: el hilo principal espera en `wait` antes de `release_shards_except`.
        unsafe {
            (*(ptr as *mut MmapTensorSource<M>)).prefetch_shards_sync(shards);
        }
    }

    struct HostStagingWorker {
        tx: std::sync::mpsc::Sender<Vec<String>>,
        done: std::sync::Arc<std::sync::atomic::AtomicU32>,
        source_ptr: std::sync::Arc<std::sync::atomic::AtomicPtr<()>>,
        _handle: std::thread::JoinHandle<()>,
    }

    impl HostStagingWorker {
        fn new(prefetch: unsafe fn(*mut (), &[String])) -> Self {
            let (tx, rx) = std::sync::mpsc::channel::<Vec<String>>();
            let done = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(1));
            let done_w = done.clone();
            let source_ptr = std::sync::Arc::new(std::sync::atomic::AtomicPtr::new(core::ptr::null_mut()));
            let source_w = source_ptr.clone();
            let handle = std::thread::spawn(move || {
                while let Ok(shards) = rx.recv() {
                    let ptr: *mut () = source_w.load(std::sync::atomic::Ordering::Acquire);
                    if !ptr.is_null() && !shards.is_empty() {
                        unsafe {
                            prefetch(ptr, &shards);
                        }
                    }
                    done_w.store(1, std::sync::atomic::Ordering::Release);
                }
            });
            Self {
                tx,
                done,
                source_ptr,
                _handle: handle,
            }
        }

        fn attach<M: FileMapper>(&self, source: *mut MmapTensorSource<M>) {
            self.source_ptr.store(source as *mut _, std::sync::atomic::Ordering::Release);
        }

        fn kick<M: FileMapper>(&self, source: *mut MmapTensorSource<M>, shards: &[String]) {
            if shards.is_empty() {
                return;
            }
            self.wait();
            self.attach(source);
            self.done.store(0, std::sync::atomic::Ordering::Release);
            let _ = self.tx.send(shards.to_vec());
        }

        fn wait(&self) {
            for _ in 0..256 {
                if self.done.load(std::sync::atomic::Ordering::Acquire) != 0 {
                    return;
                }
                std::hint::spin_loop();
            }
            while self.done.load(std::sync::atomic::Ordering::Acquire) == 0 {
                std::thread::yield_now();
            }
        }
    }

    impl<M: FileMapper + Send + 'static> ThreadStagedSource<M> {
        pub fn new(source: MmapTensorSource<M>) -> Self {
            Self {
                inner: source,
                worker: HostStagingWorker::new(prefetch_erased::<M>),
            }
        }
    }

    impl<M: FileMapper + Send + 'static> crate::layer::TensorSource for ThreadStagedSource<M> {
        fn load_f32(&mut self, name: &str, out: &mut [f32]) -> Result<(), ()> {
            self.inner.load_f32(name, out)
        }

        fn load_f32_range(
            &mut self,
            name: &str,
            elem_off: usize,
            out: &mut [f32],
        ) -> Result<(), ()> {
            self.inner.load_f32_range(name, elem_off, out)
        }

        fn tensor_view(&mut self, name: &str) -> Result<crate::layer::TensorView<'_>, ()> {
            self.inner.tensor_view(name)
        }

        fn prefetch_shards(&mut self, shards: &[String]) {
            self.inner.prefetch_shards(shards);
        }

        fn kick_prefetch_shards(&mut self, shards: &[String]) {
            let p = &mut self.inner as *mut _;
            self.worker.kick(p, shards);
        }

        fn wait_prefetch(&mut self) {
            self.worker.wait();
        }

        fn kick_moe_prefetch(&mut self, shards: &[String]) {
            let p = &mut self.inner as *mut _;
            self.worker.kick(p, shards);
        }

        fn wait_moe_prefetch(&mut self) {
            self.worker.wait();
        }

        fn release_shards_except(&mut self, keep: &[String]) {
            self.worker.wait();
            self.inner.release_shards_except(keep);
        }

        fn prefetch_embed_row(&mut self, token: u32, hidden: usize) {
            self.inner.prefetch_embed_row(token, hidden);
        }
    }
}

#[cfg(test)]
#[cfg(feature = "std")]
mod tests {
    use super::host::MemFileMapper;
    use super::*;
    use sosomodel::index::{make_f32_entry, pack_shard, TensorIndex};

    fn source_with(
        name: &str,
        shard: &str,
        shape: &[u32],
        values: &[f32],
    ) -> MmapTensorSource<MemFileMapper> {
        let raw: Vec<u8> = values.iter().flat_map(|f| f.to_le_bytes()).collect();
        let packed = pack_shard(&raw);
        let mut mapper = MemFileMapper::new();
        mapper
            .files
            .insert(format!("/models/tiny/shards/{shard}"), packed);
        let index = TensorIndex {
            entries: vec![make_f32_entry(0, name, shard, 0, shape)],
        };
        MmapTensorSource::new(String::from("/models/tiny/shards"), index, mapper)
    }

    #[test]
    fn load_f32_from_indexed_shard() {
        let mut src = source_with(
            "L00.ffn_up",
            "L00.ffn_up.tensor",
            &[2, 2],
            &[0.0, 1.0, 2.0, 3.0],
        );
        let mut out = [0.0f32; 4];
        src.load_f32("L00.ffn_up", &mut out).unwrap();
        assert!((out[0] - 0.0).abs() < 1e-6);
        assert!((out[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn load_f32_range_lee_una_fila() {
        let mut src = source_with("embed", "embed.tensor", &[4, 2], &[
            0.0, 1.0, 10.0, 11.0, 20.0, 21.0, 30.0, 31.0,
        ]);
        let mut row = [0.0f32; 2];
        src.load_f32_range("embed", 4, &mut row).unwrap();
        assert!((row[0] - 20.0).abs() < 1e-6);
        assert!((row[1] - 21.0).abs() < 1e-6);
        // fuera de rango
        assert!(src.load_f32_range("embed", 7, &mut row).is_err());
    }

    #[test]
    fn rechaza_longitud_incorrecta() {
        let mut src = source_with("w", "w.tensor", &[2], &[1.0, 2.0]);
        let mut out = [0.0f32; 3];
        assert!(src.load_f32("w", &mut out).is_err());
    }

    #[test]
    fn load_f32_desde_trunk_empaquetado() {
        let raw_a: Vec<u8> = [1.0f32, 2.0f32].iter().flat_map(|f| f.to_le_bytes()).collect();
        let raw_b: Vec<u8> = [3.0f32, 4.0f32, 5.0f32, 6.0f32]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let mut blob = raw_a.clone();
        blob.extend_from_slice(&raw_b);
        let packed = pack_shard(&blob);
        let shard = "L00.trunk.tensor";
        let mut mapper = MemFileMapper::new();
        mapper
            .files
            .insert(format!("/models/tiny/shards/{shard}"), packed);
        let index = TensorIndex {
            entries: vec![
                make_f32_entry(0, "L00.a", shard, 0, &[2]),
                make_f32_entry(1, "L00.b", shard, 8, &[2, 2]),
            ],
        };
        let mut src = MmapTensorSource::new(String::from("/models/tiny/shards"), index, mapper);
        let mut out_a = [0.0f32; 2];
        src.load_f32("L00.a", &mut out_a).unwrap();
        assert!((out_a[0] - 1.0).abs() < 1e-6);
        let mut out_b = [0.0f32; 4];
        src.load_f32("L00.b", &mut out_b).unwrap();
        assert!((out_b[0] - 3.0).abs() < 1e-6);
        assert!((out_b[3] - 6.0).abs() < 1e-6);
    }

    #[test]
    fn prefetch_repetido_no_falla() {
        let mut src = source_with("w", "w.tensor", &[2], &[1.0, 2.0]);
        let shards = vec![String::from("w.tensor")];
        src.prefetch_shards_impl(&shards);
        src.prefetch_shards_impl(&shards);
        let mut out = [0.0f32; 2];
        src.load_f32("w", &mut out).unwrap();
        assert!((out[0] - 1.0).abs() < 1e-6);
    }
}
