//! Gestor de memoria en tres niveles: disco → RAM → VRAM.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Disk,
    Ram,
    Vram,
}

pub struct TierSlot {
    pub name: String,
    pub tier: Tier,
    pub data: Vec<u8>,
    pub last_use: u64,
}

pub struct TierManager {
    pub ram_budget: usize,
    pub vram_budget: usize,
    pub ram_used: usize,
    pub vram_used: usize,
    pub clock: u64,
    slots: BTreeMap<String, TierSlot>,
    /// Cola de prefetch (capa N+1).
    pub prefetch_queue: Vec<String>,
}

impl TierManager {
    pub fn new(ram_budget: usize, vram_budget: usize) -> Self {
        Self {
            ram_budget,
            vram_budget,
            ram_used: 0,
            vram_used: 0,
            clock: 0,
            slots: BTreeMap::new(),
            prefetch_queue: Vec::new(),
        }
    }

    pub fn promote(&mut self, name: &str, data: Vec<u8>, target: Tier) -> Result<(), ()> {
        self.clock += 1;
        let size = data.len();
        match target {
            Tier::Ram => {
                if self.ram_used + size > self.ram_budget {
                    self.evict_lru(Tier::Ram, size)?;
                }
                self.ram_used += size;
            }
            Tier::Vram => {
                if self.vram_used + size > self.vram_budget {
                    self.evict_lru(Tier::Vram, size)?;
                }
                self.vram_used += size;
            }
            Tier::Disk => return Err(()),
        }
        if let Some(old) = self.slots.get(name) {
            match old.tier {
                Tier::Ram => self.ram_used = self.ram_used.saturating_sub(old.data.len()),
                Tier::Vram => self.vram_used = self.vram_used.saturating_sub(old.data.len()),
                Tier::Disk => {}
            }
        }
        self.slots.insert(
            String::from(name),
            TierSlot {
                name: String::from(name),
                tier: target,
                data,
                last_use: self.clock,
            },
        );
        Ok(())
    }

    fn evict_lru(&mut self, tier: Tier, need: usize) -> Result<(), ()> {
        loop {
            let victim = self
                .slots
                .iter()
                .filter(|(_, s)| s.tier == tier)
                .min_by_key(|(_, s)| s.last_use)
                .map(|(k, _)| k.clone());
            let Some(key) = victim else {
                return Err(());
            };
            if let Some(slot) = self.slots.remove(&key) {
                match tier {
                    Tier::Ram => self.ram_used = self.ram_used.saturating_sub(slot.data.len()),
                    Tier::Vram => self.vram_used = self.vram_used.saturating_sub(slot.data.len()),
                    Tier::Disk => {}
                }
            }
            let used = match tier {
                Tier::Ram => self.ram_used,
                Tier::Vram => self.vram_used,
                Tier::Disk => 0,
            };
            let budget = match tier {
                Tier::Ram => self.ram_budget,
                Tier::Vram => self.vram_budget,
                Tier::Disk => usize::MAX,
            };
            if used + need <= budget {
                return Ok(());
            }
        }
    }

    pub fn touch(&mut self, name: &str) {
        self.clock += 1;
        if let Some(s) = self.slots.get_mut(name) {
            s.last_use = self.clock;
        }
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.slots.get(name).map(|s| s.data.as_slice())
    }

    pub fn schedule_prefetch(&mut self, shards: &[String]) {
        self.prefetch_queue.clear();
        self.prefetch_queue.extend(shards.iter().cloned());
    }

    /// Consume la cola de prefetch y la lanza vía kick (TierManager → stager).
    pub fn kick_pending<S: crate::layer::TensorSource>(&mut self, source: &mut S) {
        if !self.prefetch_queue.is_empty() {
            source.kick_prefetch_shards(&self.prefetch_queue);
            self.prefetch_queue.clear();
        }
    }
}
