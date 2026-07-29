//! Carga de tensores desde shards mapeados vía index.som.

use crate::layer::{TensorSource, TensorView};
use crate::quant::{dequant_q4_k, dequant_q4_k_range, dequant_q8_0, dequant_q8_0_range};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use sosomodel::index::{verify_shard, TensorEntry, TensorIndex};
use sosomodel::layout::{DTYPE_F32, DTYPE_Q4_K, DTYPE_Q8_0};

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
}

/// TensorSource que lee tensores desde shards según index.som, con cache de
/// mmaps. El CRC del shard se verifica una sola vez, al mapearlo.
pub struct MmapTensorSource<M: FileMapper> {
    pub shards_base: String,
    pub index: TensorIndex,
    pub mapper: M,
    cache: BTreeMap<String, CachedShard>,
}

impl<M: FileMapper> MmapTensorSource<M> {
    pub fn new(shards_base: String, index: TensorIndex, mapper: M) -> Self {
        Self {
            shards_base,
            index,
            mapper,
            cache: BTreeMap::new(),
        }
    }

    fn shard_path(&self, shard: &str) -> String {
        format!("{}/{}", self.shards_base, shard)
    }

    fn ensure_mapped(&mut self, shard_name: &str) -> Result<&CachedShard, ()> {
        if !self.cache.contains_key(shard_name) {
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
            };
            self.cache.insert(String::from(shard_name), entry);
        }
        self.cache.get(shard_name).ok_or(())
    }

    /// Bytes `[off, off+len)` del payload de un tensor, sin re-verificar CRC.
    fn tensor_bytes(&mut self, entry: &TensorEntry, off: usize, len: usize) -> Result<&[u8], ()> {
        let end = off.checked_add(len).ok_or(())?;
        if end > entry.byte_len as usize {
            return Err(());
        }
        let base = usize::try_from(entry.offset).map_err(|_| ())?;
        let shard = entry.shard.clone();
        let cached = self.ensure_mapped(&shard)?;
        let start = base.checked_add(off).ok_or(())?;
        if start + len > cached.payload_len {
            return Err(());
        }
        let ptr = cached.mapped.addr as *const u8;
        let data = unsafe { core::slice::from_raw_parts(ptr, cached.mapped.len) };
        Ok(&data[cached.payload_off + start..cached.payload_off + start + len])
    }

    pub fn drop_cache(&mut self) {
        let keys: alloc::vec::Vec<String> = self.cache.keys().cloned().collect();
        for key in keys {
            if let Some(entry) = self.cache.remove(&key) {
                self.mapper.unmap_file(&entry.mapped);
            }
        }
    }

    /// Mapea shards y toca páginas a stride de 2 MiB (page-fault adelantado
    /// alineado con el camino huge del kernel), no solo el primer byte.
    pub fn prefetch_shards_impl(&mut self, shards: &[String]) {
        const STRIDE: usize = 2 * 1024 * 1024;
        for name in shards {
            if let Ok(cached) = self.ensure_mapped(name) {
                let ptr = cached.mapped.addr as *const u8;
                let len = cached.mapped.len;
                if len == 0 {
                    continue;
                }
                let mut off = 0usize;
                while off < len {
                    let _ = unsafe { core::ptr::read_volatile(ptr.add(off)) };
                    off = off.saturating_add(STRIDE);
                    if off == 0 {
                        break;
                    }
                }
            }
        }
    }

    /// Desmapea todo lo que no esté en `keep` (working set de capas).
    pub fn release_shards_except_impl(&mut self, keep: &[String]) {
        let keys: alloc::vec::Vec<String> = self.cache.keys().cloned().collect();
        for key in keys {
            if !keep.iter().any(|k| k == &key) {
                if let Some(entry) = self.cache.remove(&key) {
                    self.mapper.unmap_file(&entry.mapped);
                }
            }
        }
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

    fn release_shards_except(&mut self, keep: &[String]) {
        self.release_shards_except_impl(keep);
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
}
