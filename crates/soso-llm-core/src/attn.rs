//! Atención tiled estilo FlashAttention para decode (1 query):
//! online softmax por bloques de KV — no materializa scores N y mejora
//! localidad de caché sobre el KV en f16 (PagedAttention/Flash decode).

use crate::f16::f16_to_f32;
use crate::gemm::{dot_f32, softmax_inplace};
use alloc::vec::Vec;

/// Tamaño de tile sobre la secuencia KV (tokens). 64 encaja bien en L1
/// con head_dim típico ≤ 128 y f16.
pub const ATTN_TILE_TOKENS: usize = 64;

/// Atención clásica (tests / prefill corto): materializa scores.
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
    let inv = 1.0 / libm::sqrtf(head_dim as f32);
    for t in 0..n_kv {
        scores[t] = dot_f32(q, &k[t * head_dim..(t + 1) * head_dim]) * inv;
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

/// Decode GQA: Q (f32) × K/V f16 layout `[token][kv_dim]`, una cabeza.
/// Online softmax por tiles (FlashAttention-2 decode path).
pub fn attention_decode_f16_tiled(
    q: &[f32],
    k_f16: &[u16],
    v_f16: &[u16],
    head_dim: usize,
    kv_dim: usize,
    kv_head: usize,
    n_tokens: usize,
    out: &mut [f32],
) {
    debug_assert_eq!(q.len(), head_dim);
    debug_assert_eq!(out.len(), head_dim);
    out.fill(0.0);
    if n_tokens == 0 || head_dim == 0 {
        return;
    }
    let inv = 1.0 / libm::sqrtf(head_dim as f32);
    let mut m = f32::NEG_INFINITY; // max log-sum-exp
    let mut l = 0.0f32; // sum de exps
    let mut tile_scores = [0.0f32; ATTN_TILE_TOKENS];

    let mut t0 = 0usize;
    while t0 < n_tokens {
        let t1 = (t0 + ATTN_TILE_TOKENS).min(n_tokens);
        let mut tile_max = f32::NEG_INFINITY;
        for (i, t) in (t0..t1).enumerate() {
            let k_off = t * kv_dim + kv_head * head_dim;
            let mut dot = 0.0f32;
            for d in 0..head_dim {
                dot += q[d] * f16_to_f32(k_f16[k_off + d]);
            }
            let s = dot * inv;
            tile_scores[i] = s;
            if s > tile_max {
                tile_max = s;
            }
        }
        let m_new = if m > tile_max { m } else { tile_max };
        let alpha = if m == f32::NEG_INFINITY {
            0.0
        } else {
            libm::expf(m - m_new)
        };
        for d in 0..head_dim {
            out[d] *= alpha;
        }
        l *= alpha;
        let mut tile_sum = 0.0f32;
        for (i, t) in (t0..t1).enumerate() {
            let p = libm::expf(tile_scores[i] - m_new);
            tile_sum += p;
            let v_off = t * kv_dim + kv_head * head_dim;
            for d in 0..head_dim {
                out[d] += p * f16_to_f32(v_f16[v_off + d]);
            }
        }
        l += tile_sum;
        m = m_new;
        t0 = t1;
    }
    if l > 0.0 {
        let inv_l = 1.0 / l;
        for d in 0..head_dim {
            out[d] *= inv_l;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::f16::f32_to_f16;

    #[test]
    fn tiled_matches_dense_f16() {
        let head_dim = 8;
        let kv_dim = 8;
        let n = 100;
        let q: Vec<f32> = (0..head_dim).map(|i| (i as f32) * 0.1).collect();
        let mut k = Vec::new();
        let mut v = Vec::new();
        let mut k_f32 = Vec::new();
        let mut v_f32 = Vec::new();
        for t in 0..n {
            for d in 0..head_dim {
                let kv = (t * head_dim + d) as f32 * 0.01;
                let vv = (t + d) as f32 * 0.02;
                k.push(f32_to_f16(kv));
                v.push(f32_to_f16(vv));
                k_f32.push(kv);
                v_f32.push(vv);
            }
        }
        let mut dense = vec![0.0f32; head_dim];
        attention_step(&q, &k_f32, &v_f32, head_dim, n, &mut dense);
        let mut tiled = vec![0.0f32; head_dim];
        attention_decode_f16_tiled(&q, &k, &v, head_dim, kv_dim, 0, n, &mut tiled);
        for (a, b) in dense.iter().zip(tiled.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
    }
}
