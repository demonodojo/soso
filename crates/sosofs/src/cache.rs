//! Caché LRU de bloques de 4 KiB sobre un `BlockDevice`.

use alloc::vec::Vec;
use block_dev::{Block, BlockDevice, BlockError};

const DEFAULT_CAPACITY: usize = 256;

struct Entry {
    block: u64,
    data: Block,
    age: u32,
}

/// Envoltorio que cachea lecturas de bloques.
pub struct CachedBlockDevice<D: BlockDevice> {
    inner: D,
    entries: Vec<Entry>,
    capacity: usize,
    tick: u32,
    /// Si la última lectura fue un acierto. Lo consulta `read_node` para no
    /// recalcular el CRC de un bloque que ya venía comprobado (N-012).
    ultimo_acierto: bool,
}

impl<D: BlockDevice> CachedBlockDevice<D> {
    pub fn new(inner: D) -> Self {
        Self::with_capacity(inner, DEFAULT_CAPACITY)
    }

    pub fn with_capacity(inner: D, capacity: usize) -> Self {
        Self {
            inner,
            entries: Vec::with_capacity(capacity),
            capacity: capacity.max(4),
            tick: 0,
            ultimo_acierto: false,
        }
    }

    pub fn inner(&self) -> &D {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut D {
        &mut self.inner
    }

    pub fn into_inner(self) -> D {
        self.inner
    }

    fn touch(&mut self, idx: usize) {
        self.tick = self.tick.wrapping_add(1);
        self.entries[idx].age = self.tick;
    }

    fn evict_lru(&mut self) -> usize {
        let mut min_idx = 0;
        let mut min_age = self.entries[0].age;
        for (i, e) in self.entries.iter().enumerate().skip(1) {
            if e.age < min_age {
                min_age = e.age;
                min_idx = i;
            }
        }
        min_idx
    }

    pub fn invalidate(&mut self, block: u64) {
        self.entries.retain(|e| e.block != block);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl<D: BlockDevice> BlockDevice for CachedBlockDevice<D> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        if let Some(idx) = self.entries.iter().position(|e| e.block == block) {
            buf.copy_from_slice(&self.entries[idx].data);
            self.touch(idx);
            self.ultimo_acierto = true;
            return Ok(());
        }
        self.ultimo_acierto = false;
        self.inner.read_block(block, buf)?;
        self.tick = self.tick.wrapping_add(1);
        if self.entries.len() >= self.capacity {
            let idx = self.evict_lru();
            self.entries[idx] = Entry { block, data: *buf, age: self.tick };
        } else {
            self.entries.push(Entry { block, data: *buf, age: self.tick });
        }
        Ok(())
    }

    fn last_read_cached(&self) -> bool {
        self.ultimo_acierto
    }

    fn max_blocks_per_request(&self) -> usize {
        self.inner.max_blocks_per_request()
    }

    /// Lectura de un rango: va directa al dispositivo, en una petición, sin
    /// pasar por la caché de bloques. Un rango largo la barrería entera (256
    /// entradas) y cada bloque costaba un viaje al disco; el disco manda (las
    /// escrituras invalidan y escriben por debajo), así que lo leído es lo
    /// vigente.
    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.ultimo_acierto = false;
        self.inner.read_blocks(start, buf)
    }

    fn invalidate_block(&mut self, block: u64) {
        self.invalidate(block);
    }

    fn invalidate_all(&mut self) {
        self.clear();
    }

    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        self.invalidate(block);
        self.inner.write_block(block, buf)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
}
