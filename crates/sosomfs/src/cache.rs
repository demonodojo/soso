//! Caché de bloques con políticas NORMAL / STREAM / PIN y readahead.

use alloc::vec::Vec;
use block_dev::{Block, BlockError};
use crate::layout::{CACHE_NORMAL, CACHE_PIN, CACHE_STREAM};
use crate::volume_set::VolumeSet;

/// Suelo de la ventana de streaming, para cachés diminutas.
const STREAM_WINDOW_MIN: usize = 32;
/// Recuerdo de prefetch: cuántos destinos recientes se dan por hechos.
const PREFETCH_RECIENTES: usize = 16;

struct Entry {
    lba: u64,
    data: Block,
    age: u32,
    policy: u8,
}

pub struct BlockCache<V: VolumeSet> {
    vol: V,
    entries: Vec<Entry>,
    capacity: usize,
    tick: u32,
    pin_count: usize,
    /// Techo de entradas para los shards (`CACHE_STREAM`), para que un modelo
    /// grande no se coma la caché entera.
    ///
    /// AVERÍA (2026-08-02): esto era una constante de 32 bloques —128 KiB— y
    /// convertía el prefetch en un generador de trabajo inútil: `prefetch` leía
    /// el shard siguiente entero del disco y la ventana sólo podía quedarse con
    /// los 32 últimos bloques, así que lo tiraba y las faltas de página que
    /// venían detrás lo releían. Medido con el modelo `tiny` (577 bloques):
    /// **54 272 lecturas de 4 KiB a 177 us**, 94 bloques leídos por cada bloque
    /// del modelo, 9,2 s de disco. La ventana tiene que ser proporcional a la
    /// caché, no un número fijo más pequeño que un solo prefetch.
    stream_cap: usize,
    /// Destinos de prefetch ya servidos. Era **una sola casilla**, y eso sólo
    /// frena repeticiones consecutivas del mismo destino: en cuanto el acceso
    /// alterna entre dos shards (el runtime mapea la capa N y la N+1 a la vez)
    /// cada lectura de 4 KiB volvía a prefetchear desde cero.
    prefetch_hechos: [u64; PREFETCH_RECIENTES],
    prefetch_siguiente: usize,
    aciertos: u64,
    fallos: u64,
}

impl<V: VolumeSet> BlockCache<V> {
    pub fn new(vol: V, capacity: usize) -> Self {
        let capacity = capacity.max(8);
        Self {
            vol,
            entries: Vec::new(),
            capacity,
            tick: 0,
            pin_count: 0,
            stream_cap: (capacity / 2).max(STREAM_WINDOW_MIN).min(capacity),
            prefetch_hechos: [u64::MAX; PREFETCH_RECIENTES],
            prefetch_siguiente: 0,
            aciertos: 0,
            fallos: 0,
        }
    }

    pub fn volume_mut(&mut self) -> &mut V {
        &mut self.vol
    }

    /// (aciertos, fallos) desde el montaje. El kernel los publica por
    /// `SYS_IOSTAT`; sin esto, la amplificación de lectura sólo se puede medir
    /// reinstrumentando a mano, que es como se perdieron las cifras anteriores.
    pub fn estadisticas(&self) -> (u64, u64) {
        (self.aciertos, self.fallos)
    }

    fn touch(&mut self, idx: usize) {
        self.tick = self.tick.wrapping_add(1);
        self.entries[idx].age = self.tick;
    }

    fn evict_lru(&mut self, policy: u8) -> Option<usize> {
        let mut best: Option<usize> = None;
        let mut best_age = u32::MAX;
        for (i, e) in self.entries.iter().enumerate() {
            if e.policy == CACHE_PIN {
                continue;
            }
            if policy == CACHE_STREAM && e.policy != CACHE_STREAM {
                continue;
            }
            if e.age < best_age {
                best_age = e.age;
                best = Some(i);
            }
        }
        best
    }

    pub fn read_lba(&mut self, lba: u64, policy: u8, buf: &mut Block) -> Result<(), BlockError> {
        if let Some(idx) = self.entries.iter().position(|e| e.lba == lba) {
            buf.copy_from_slice(&self.entries[idx].data);
            self.touch(idx);
            self.aciertos += 1;
            return Ok(());
        }
        self.fallos += 1;
        self.vol.read_lba(lba, buf)?;
        self.insert(lba, *buf, policy);
        Ok(())
    }

    fn insert(&mut self, lba: u64, data: Block, policy: u8) {
        self.tick = self.tick.wrapping_add(1);
        let cap = if policy == CACHE_STREAM {
            self.stream_cap
        } else {
            self.capacity
        };
        let evictable = self.entries.iter().filter(|e| e.policy != CACHE_PIN).count();
        if evictable >= cap {
            if let Some(idx) = self.evict_lru(policy) {
                if self.entries[idx].policy == CACHE_PIN {
                    self.pin_count = self.pin_count.saturating_sub(1);
                }
                self.entries[idx] = Entry { lba, data, age: self.tick, policy };
                if policy == CACHE_PIN {
                    self.pin_count += 1;
                }
                return;
            }
        }
        // Crecer con `try_reserve`: la capacidad es un techo, no una promesa.
        //
        // AVERÍA (2026-08-02): `push` a secas duplica el buffer, y con capacidad
        // 2048 la última duplicación pide 2048 × 4112 B = **8,4 MiB contiguos**
        // de una vez. En la máquina de 48 MiB del test de reclaim eso es un
        // `memory allocation of 8421376 bytes failed` y el kernel entero se cae.
        // No se veía porque el techo fijo de 32 bloques impedía llegar; en cuanto
        // la ventana pasó a ser proporcional, saltó. Si no hay memoria, dejar de
        // crecer y reciclar entradas es una degradación correcta.
        if self.entries.len() < self.capacity
            && self.entries.try_reserve(1).is_ok()
        {
            self.entries.push(Entry { lba, data, age: self.tick, policy });
            if policy == CACHE_PIN {
                self.pin_count += 1;
            }
            return;
        }
        // Sin sitio para crecer: reutilizar la entrada más vieja que se pueda.
        if let Some(idx) = self.evict_lru(policy).or_else(|| self.evict_lru(CACHE_NORMAL)) {
            self.entries[idx] = Entry { lba, data, age: self.tick, policy };
            if policy == CACHE_PIN {
                self.pin_count += 1;
            }
        }
    }

    pub fn prefetch(&mut self, start_lba: u64, bytes: u32, policy: u8) {
        if self.prefetch_hechos.contains(&start_lba) {
            return;
        }
        self.prefetch_hechos[self.prefetch_siguiente] = start_lba;
        self.prefetch_siguiente = (self.prefetch_siguiente + 1) % PREFETCH_RECIENTES;
        // Nunca traer más de lo que la política puede retener: prefetchear por
        // encima del techo desaloja la cabeza del propio prefetch y garantiza
        // que las lecturas que vienen detrás fallen igualmente.
        let cap = if policy == CACHE_STREAM {
            self.stream_cap as u64
        } else {
            self.capacity as u64
        };
        let blocks = ((bytes as u64 + 4095) / 4096).min(cap);
        let end = start_lba.saturating_add(blocks).min(self.vol.total_blocks());
        let mut lba = start_lba;
        while lba < end {
            if self.entries.iter().any(|e| e.lba == lba) {
                lba += 1;
                continue;
            }
            let mut buf = [0u8; 4096];
            if self.vol.read_lba(lba, &mut buf).is_ok() {
                self.insert(lba, buf, policy);
            }
            lba += 1;
        }
    }
}
