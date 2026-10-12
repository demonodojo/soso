//! Abstracción GPU para el runtime LLM.

#![no_std]

extern crate alloc;

pub enum GpuBackend {
    None,
    Intel,
}

pub struct GpuContext {
    pub backend: GpuBackend,
    pub vram_budget: usize,
}

impl GpuContext {
    pub fn new(present: bool, vram: usize) -> Self {
        Self {
            backend: if present { GpuBackend::Intel } else { GpuBackend::None },
            vram_budget: vram,
        }
    }

    pub fn gemm_f32(&self, _m: usize, _n: usize, _k: usize, _a: &[f32], _b: &[f32], _c: &mut [f32]) -> bool {
        // Fase 6: kernels reales en EU de Intel.
        false
    }
}
