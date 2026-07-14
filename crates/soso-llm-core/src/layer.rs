//! Ejecutor capa a capa con carga de shards bajo demanda.

use crate::gemm::{matvec_f32, rmsnorm, silu};
use alloc::format;
use alloc::vec::Vec;
use sosomodel::manifest::Manifest;

pub trait TensorSource {
    fn load_f32(&self, name: &str, out: &mut [f32]) -> Result<(), ()>;
}

pub struct LayerExecutor<'a, S: TensorSource> {
    pub manifest: &'a Manifest,
    pub source: &'a S,
}

impl<'a, S: TensorSource> LayerExecutor<'a, S> {
    pub fn forward_layer(&self, layer: u32, hidden: &mut [f32], scratch: &mut [f32]) -> Result<(), ()> {
        let h = self.manifest.hidden_dim as usize;
        let ffn = self.manifest.ffn_dim as usize;
        let prefix = format!("L{layer:02}");
        let mut w = Vec::with_capacity(h * h.max(ffn));
        w.resize(h * ffn, 0.0);
        self.source.load_f32(&format!("{prefix}.ffn_up"), &mut w[..h * ffn])?;
        matvec_f32(&w[..h * ffn], h, ffn, hidden, scratch);
        for x in scratch.iter_mut().take(ffn) {
            *x = silu(*x);
        }
        self.source
            .load_f32(&format!("{prefix}.ffn_down"), &mut w[..ffn * h])?;
        matvec_f32(&w[..ffn * h], h, ffn, &scratch[..ffn], hidden);
        let mut norm_w = Vec::with_capacity(h);
        norm_w.resize(h, 0.0);
        self.source.load_f32("embed", &mut norm_w)?;
        rmsnorm(hidden, &norm_w[..h], 1e-5);
        Ok(())
    }
}
