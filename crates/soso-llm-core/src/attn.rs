//! Atención tiled: no materializa la matriz N×N completa.

use crate::gemm::{dot_f32, softmax_inplace};
use alloc::vec::Vec;

pub fn attention_step(
    q: &[f32],
    k: &[f32],
    v: &[f32],
    head_dim: usize,
    n_kv: usize,
    out: &mut [f32],
) {
    let mut scores = Vec::with_capacity(n_kv);
    scores.resize(n_kv, 0.0);
    for t in 0..n_kv {
        scores[t] = dot_f32(q, &k[t * head_dim..(t + 1) * head_dim]) / libm::sqrtf(head_dim as f32);
    }
    softmax_inplace(&mut scores);
    for d in 0..head_dim {
        let mut acc = 0.0f32;
        for t in 0..n_kv {
            acc += scores[t] * v[t * head_dim + d];
        }
        out[d] = acc;
    }
}
