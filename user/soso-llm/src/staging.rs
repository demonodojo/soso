//! Hilo de staging para solapar prefetch de shards con cómputo (AirLLM).

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use libsoso::{sys, thread};
use soso_abi::{self as abi, O_RDONLY};
use soso_llm_core::layer::TensorSource;
use soso_llm_core::source::{FileMapper, MappedShard, MmapTensorSource};
use soso_llm_core::stage::PrefetchSink;

const MAX_SHARDS: usize = 64;
const MAX_NAME: usize = 64;

pub struct SyscallMapper;

impl FileMapper for SyscallMapper {
    fn map_file(&mut self, path: &str) -> Result<MappedShard, ()> {
        let fd = sys::open(path, O_RDONLY);
        if fd < 0 {
            return Err(());
        }
        let mut st = abi::Stat::default();
        if sys::stat(path, &mut st) < 0 {
            sys::close(fd as u64);
            return Err(());
        }
        let size = st.size as usize;
        let map = sys::mmap(0, size as u64, fd as u64, 0);
        sys::close(fd as u64);
        if map < 0 {
            return Err(());
        }
        let ptr = map as *const u8;
        let _ = unsafe { core::ptr::read_volatile(ptr) };
        Ok(MappedShard {
            addr: map as u64,
            len: size,
        })
    }

    fn unmap_file(&mut self, shard: &MappedShard) {
        let aligned = shard.len.next_multiple_of(4096);
        let _ = sys::munmap(shard.addr, aligned as u64);
    }
}

struct StageShared {
    generation: AtomicU32,
    n_shards: AtomicU32,
    shard_lens: [AtomicU32; MAX_SHARDS],
    shard_data: [[u8; MAX_NAME]; MAX_SHARDS],
    done: AtomicU32,
    shutdown: AtomicU32,
}

static mut STAGE: StageShared = StageShared {
    generation: AtomicU32::new(0),
    n_shards: AtomicU32::new(0),
    shard_lens: [const { AtomicU32::new(0) }; MAX_SHARDS],
    shard_data: [[0u8; MAX_NAME]; MAX_SHARDS],
    done: AtomicU32::new(1),
    shutdown: AtomicU32::new(0),
};

static mut SOURCE_PTR: usize = 0;

/// ¿Hay ya un hilo de staging en este proceso? Va aparte del `spawned` de cada
/// `StagingWorker` porque el hilo es único y sobrevive a la fuente que lo
/// arrancó.
static WORKER_VIVO: AtomicU32 = AtomicU32::new(0);

fn store_shards(shards: &[String]) {
    let shared = unsafe { &mut *(&raw mut STAGE) };
    let n = shards.len().min(MAX_SHARDS);
    shared.n_shards.store(n as u32, Ordering::Release);
    for (i, name) in shards.iter().take(n).enumerate() {
        let bytes = name.as_bytes();
        let len = bytes.len().min(MAX_NAME);
        shared.shard_data[i][..len].copy_from_slice(&bytes[..len]);
        shared.shard_lens[i].store(len as u32, Ordering::Release);
    }
}

fn load_shards(out: &mut Vec<String>) {
    let shared = unsafe { &*(&raw const STAGE) };
    let n = shared.n_shards.load(Ordering::Acquire) as usize;
    out.clear();
    for i in 0..n.min(MAX_SHARDS) {
        let len = shared.shard_lens[i].load(Ordering::Acquire) as usize;
        if len == 0 || len > MAX_NAME {
            continue;
        }
        if let Ok(s) = core::str::from_utf8(&shared.shard_data[i][..len]) {
            out.push(String::from(s));
        }
    }
}

extern "C" fn worker_entry(_arg: u64) -> ! {
    let shared = unsafe { &*(&raw const STAGE) };
    let mut last = 0u32;
    let mut buf = Vec::new();
    loop {
        // Dormir en el futex, NO girar.
        //
        // AVERÍA: esto era `while ... { spin_loop() }`. El banco corre con un
        // solo core, así que el planificador round-robin le daba una rodaja
        // entera de 20 ms al worker para girar en vacío mientras el hilo
        // principal tenía trabajo real. Medido con el modelo de 128 MiB:
        // matvec **1079 -> 3240 ms/capa** y 0,21 -> 0,07 tok/s, con el disco
        // idéntico (670 vs 806 ms). Un hilo de prefetch que se pasa el rato
        // esperando tiene que estar bloqueado, no listo para ejecutar.
        while shared.generation.load(Ordering::Acquire) == last {
            if shared.shutdown.load(Ordering::Acquire) != 0 {
                sys::exit(0);
            }
            sys::futex_wait(
                &shared.generation as *const AtomicU32 as *const u32,
                last,
            );
        }
        last = shared.generation.load(Ordering::Acquire);
        if shared.shutdown.load(Ordering::Acquire) != 0 {
            sys::exit(0);
        }
        load_shards(&mut buf);
        let ptr = unsafe { SOURCE_PTR as *mut MmapTensorSource<SyscallMapper> };
        if !ptr.is_null() && !buf.is_empty() {
            unsafe {
                (*ptr).prefetch_shards_sync(&buf);
            }
        }
        shared.done.store(1, Ordering::Release);
        sys::futex_wake(&shared.done as *const AtomicU32 as *const u32, 1);
    }
}

/// Arranca el worker de prefetch (idempotente).
pub struct StagingWorker {
    pub spawned: bool,
}

impl StagingWorker {
    pub fn new() -> Self {
        Self { spawned: false }
    }

    /// Publica la dirección ACTUAL de la fuente. Hay que rehacerlo en cada
    /// kick, no una sola vez al arrancar.
    ///
    /// AVERÍA: `enable_worker` guardaba `&mut self.inner` y justo después el
    /// `StagedSource` se movía dentro de `ModelBundle` (y el bundle, otra vez,
    /// al salir de `load_model`). El puntero quedaba apuntando a un hueco de
    /// pila muerto y el hilo de staging escribía ahí: corrupción a ciegas que
    /// salía como page fault en una dirección con pinta de cadena
    /// (`0x6e7474612e3032be` = «…20.attn», un nombre de shard usado como
    /// puntero). Un puntero crudo entregado a otro hilo no sobrevive a un move.
    fn publicar(&self, source: *mut MmapTensorSource<SyscallMapper>) {
        unsafe {
            SOURCE_PTR = source as usize;
        }
    }

    /// Engancha esta fuente al hilo de staging, arrancándolo la primera vez.
    ///
    /// El hilo y `STAGE` son **del proceso**, no de este `StagedSource`: sólo
    /// puede haber uno. Con `spawned` como campo de instancia, cargar un
    /// segundo modelo en el mismo proceso (el `:modelo` del REPL de `ask`)
    /// arrancaba otro worker sobre el mismo estado global y además reseteaba
    /// `generation` a 0, que el worker vivo veía como un kick nuevo: dos hilos
    /// prefetchando a la vez sobre el `BTreeMap` de la fuente, que no está
    /// sincronizado. Salía como «inferencia falló», y a veces como page fault
    /// en una dirección con pinta de cadena (`0x2f736c65646f6d2f` = «/models/»).
    /// Nadie pone `shutdown` a 1 nunca, así que el hilo dura lo que el proceso
    /// y reutilizarlo es lo correcto.
    pub fn attach(&mut self, source: *mut MmapTensorSource<SyscallMapper>) {
        self.publicar(source);
        if self.spawned {
            return;
        }
        if WORKER_VIVO.swap(1, Ordering::AcqRel) != 0 {
            // Ya hay hilo de este proceso: reutilizarlo tal cual, sin tocar
            // `generation` ni `done`.
            self.spawned = true;
            return;
        }
        let shared = unsafe { &mut *(&raw mut STAGE) };
        shared.shutdown.store(0, Ordering::Release);
        shared.generation.store(0, Ordering::Release);
        shared.done.store(1, Ordering::Release);
        if thread::spawn(worker_entry, 0).is_ok() {
            self.spawned = true;
        } else {
            WORKER_VIVO.store(0, Ordering::Release);
        }
    }

    fn kick(&self, source: *mut MmapTensorSource<SyscallMapper>, shards: &[String]) {
        if shards.is_empty() {
            return;
        }
        // Esperar al kick anterior antes de tocar `shard_data`: el worker lo
        // está leyendo, y pisarlo a media lectura le daba nombres de shard
        // partidos por la mitad.
        self.wait();
        self.publicar(source);
        let shared = unsafe { &*(&raw const STAGE) };
        store_shards(shards);
        shared.done.store(0, Ordering::Release);
        let _ = shared.generation.fetch_add(1, Ordering::AcqRel);
        sys::futex_wake(
            &shared.generation as *const AtomicU32 as *const u32,
            1,
        );
    }

    fn wait(&self) {
        let shared = unsafe { &*(&raw const STAGE) };
        // Giro corto para el caso común (el prefetch ya terminó): una syscall
        // cuesta más que unas cuantas vueltas. Si no, dormir — con un solo core,
        // girar aquí le roba al worker justo el CPU que necesita para acabar.
        for _ in 0..256 {
            if shared.done.load(Ordering::Acquire) != 0 {
                return;
            }
            core::hint::spin_loop();
        }
        while shared.done.load(Ordering::Acquire) == 0 {
            sys::futex_wait(&shared.done as *const AtomicU32 as *const u32, 0);
        }
    }
}

/// Envuelve `MmapTensorSource` con kick/wait en hilo de staging.
pub struct StagedSource {
    pub inner: MmapTensorSource<SyscallMapper>,
    worker: StagingWorker,
}

impl StagedSource {
    pub fn new(inner: MmapTensorSource<SyscallMapper>) -> Self {
        Self {
            inner,
            worker: StagingWorker::new(),
        }
    }

    pub fn enable_worker(&mut self) {
        self.worker.attach(&mut self.inner as *mut _);
        self.inner.async_staging = true;
    }
}

impl StagedSource {
    /// El worker y el hilo principal comparten `inner`, y `MmapTensorSource`
    /// no es sincronizado: su caché es un `BTreeMap` que ambos mutan al mapear
    /// o soltar shards. Dos hilos reescribiendo el mismo árbol lo dejan con
    /// punteros inventados.
    ///
    /// La regla es simple: **el hilo principal no toca `inner` mientras el
    /// worker esté dentro**. Esperar aquí no anula el solapamiento —lo que se
    /// solapa es el prefetch con el cómputo de la capa, que no pasa por la
    /// fuente—, y cuando no hay kick en vuelo `wait` no cuesta nada.
    fn sincroniza(&self) {
        if self.worker.spawned {
            self.worker.wait();
        }
    }
}

impl TensorSource for StagedSource {
    fn load_f32(&mut self, name: &str, out: &mut [f32]) -> Result<(), ()> {
        self.inner.load_f32(name, out)
    }

    fn load_f32_range(&mut self, name: &str, elem_off: usize, out: &mut [f32]) -> Result<(), ()> {
        self.inner.load_f32_range(name, elem_off, out)
    }

    fn tensor_view(&mut self, name: &str) -> Result<soso_llm_core::layer::TensorView<'_>, ()> {
        self.inner.tensor_view(name)
    }

    fn prefetch_shards(&mut self, shards: &[String]) {
        self.inner.prefetch_shards(shards);
    }

    fn kick_prefetch_shards(&mut self, shards: &[String]) {
        if self.worker.spawned {
            let p = &mut self.inner as *mut _;
            self.worker.kick(p, shards);
        } else {
            self.inner.kick_prefetch_shards(shards);
        }
    }

    fn wait_prefetch(&mut self) {
        if self.worker.spawned {
            self.worker.wait();
        } else {
            self.inner.wait_prefetch();
        }
    }

    fn kick_moe_prefetch(&mut self, shards: &[String]) {
        if self.worker.spawned {
            let p = &mut self.inner as *mut _;
            self.worker.kick(p, shards);
        } else {
            self.inner.kick_moe_prefetch(shards);
        }
    }

    fn wait_moe_prefetch(&mut self) {
        if self.worker.spawned {
            self.worker.wait();
        } else {
            self.inner.wait_moe_prefetch();
        }
    }

    fn release_shards_except(&mut self, keep: &[String]) {
        // El más peligroso de todos: desmapea shards que el worker puede estar
        // mapeando en ese mismo instante.
        self.sincroniza();
        self.inner.release_shards_except(keep);
    }

    fn prefetch_embed_row(&mut self, token: u32, hidden: usize) {
        self.inner.prefetch_embed_row(token, hidden);
    }
}
