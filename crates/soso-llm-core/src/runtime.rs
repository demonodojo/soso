//! Runtime de inferencia: bucle de generación de tokens.

use crate::gemm::matvec_f32;
use crate::layer::{LayerExecutor, TensorSource};
use crate::tier::{Tier, TierManager};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;

pub struct MemoryTensorSource {
    pub tensors: alloc::collections::BTreeMap<String, Vec<f32>>,
}

impl TensorSource for MemoryTensorSource {
    fn load_f32(&self, name: &str, out: &mut [f32]) -> Result<(), ()> {
        let data = self.tensors.get(name).ok_or(())?;
        if data.len() != out.len() {
            return Err(());
        }
        out.copy_from_slice(data);
        Ok(())
    }
}

pub struct Runtime {
    pub manifest: Manifest,
    pub index: TensorIndex,
    pub tiers: TierManager,
    pub hidden: Vec<f32>,
    pub backend: Backend,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Cpu,
    Gpu,
    Auto,
}

impl Runtime {
    pub fn new(manifest: Manifest, index: TensorIndex, ram_budget: usize, vram_budget: usize) -> Self {
        let h = manifest.hidden_dim as usize;
        Self {
            manifest,
            index,
            tiers: TierManager::new(ram_budget, vram_budget),
            hidden: vec![0.0f32; h],
            backend: Backend::Auto,
        }
    }

    pub fn embed_token(&mut self, token: u32, embed: &[f32]) {
        let h = self.manifest.hidden_dim as usize;
        let off = token as usize * h;
        if off + h <= embed.len() {
            self.hidden.copy_from_slice(&embed[off..off + h]);
        }
    }

    pub fn forward(&mut self, source: &MemoryTensorSource) -> Result<(), ()> {
        let h = self.manifest.hidden_dim as usize;
        let mut scratch = vec![0.0f32; self.manifest.ffn_dim as usize];
        let exec = LayerExecutor {
            manifest: &self.manifest,
            source,
        };
        for layer in 0..self.manifest.num_layers {
            if let Some(pf) = self.manifest.prefetch.get(layer as usize) {
                self.tiers.schedule_prefetch(&pf.shards);
            }
            exec.forward_layer(layer, &mut self.hidden, &mut scratch)?;
        }
        let mut logits = vec![0.0f32; self.manifest.vocab_size as usize];
        if let Some(emb) = source.tensors.get("embed") {
            matvec_f32(emb, self.manifest.vocab_size as usize, h, &self.hidden, &mut logits);
        }
        let _ = logits;
        Ok(())
    }

    pub fn load_shard_to_ram(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), ()> {
        self.tiers.promote(name, bytes, Tier::Ram)
    }
}
