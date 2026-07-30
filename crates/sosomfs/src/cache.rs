//! Caché de bloques con políticas NORMAL / STREAM / PIN y readahead.

use alloc::vec::Vec;
use block_dev::{Block, BlockError};
use crate::layout::{CACHE_PIN, CACHE_STREAM};
use crate::volume_set::VolumeSet;

const STREAM_WINDOW: usize = 32;

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
    /// Último start_lba prefetcheado: evita repetir el prefetch completo en
    /// cada lectura de 4 KiB (p. ej. la tormenta de page faults de mmap).
    last_prefetch: u64,
}

impl<V: VolumeSet> BlockCache<V> {
    pub fn new(vol: V, capacity: usize) -> Self {
        Self {
            vol,
            entries: Vec::new(),
            capacity: capacity.max(8),
            tick: 0,
            pin_count: 0,
            last_prefetch: u64::MAX,
        }
    }

    pub fn volume_mut(&mut self) -> &mut V {
        &mut self.vol
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
            return Ok(());
        }
        self.vol.read_lba(lba, buf)?;
        self.insert(lba, *buf, policy);
        Ok(())
    }

    fn insert(&mut self, lba: u64, data: Block, policy: u8) {
        self.tick = self.tick.wrapping_add(1);
        let cap = if policy == CACHE_STREAM {
            STREAM_WINDOW
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
        if self.entries.len() < self.capacity {
            self.entries.push(Entry { lba, data, age: self.tick, policy });
            if policy == CACHE_PIN {
                self.pin_count += 1;
            }
        }
    }

    pub fn prefetch(&mut self, start_lba: u64, bytes: u32, policy: u8) {
        if start_lba == self.last_prefetch {
            return;
        }
        self.last_prefetch = start_lba;
        let blocks = (bytes as u64 + 4095) / 4096;
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
