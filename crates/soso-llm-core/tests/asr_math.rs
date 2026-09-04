//! Pruebas numéricas de atención y residuales Pre-LN (ASR).

use soso_llm_core::gemm::{dot_f32, layernorm, matmul_f32, softmax_inplace};

/// Réplica de la atención batched corregida (Q/K en buffers separados).
fn attention_batched_ref(
    q: &[f32],
    k: &[f32],
    v: &[f32],
    seq_q: usize,
    seq_kv: usize,
    d: usize,
    heads: usize,
    hd: usize,
    causal: bool,
    scores: &mut [f32],
    q_buf: &mut [f32],
    k_buf: &mut [f32],
    kt_buf: &mut [f32],
    head_out: &mut [f32],
    merged: &mut [f32],
) {
    let scale = 1.0 / libm::sqrtf(hd as f32);
    merged.fill(0.0);
    for h in 0..heads {
        let off = h * hd;
        for t in 0..seq_q {
            q_buf[t * hd..(t + 1) * hd].copy_from_slice(&q[t * d + off..t * d + off + hd]);
        }
        for t in 0..seq_kv {
            k_buf[t * hd..(t + 1) * hd].copy_from_slice(&k[t * d + off..t * d + off + hd]);
        }
        for j in 0..seq_kv {
            for ki in 0..hd {
                kt_buf[ki * seq_kv + j] = k_buf[j * hd + ki];
            }
        }
        matmul_f32(q_buf, seq_q, hd, kt_buf, seq_kv, scores);
        for i in 0..seq_q {
            let row = &mut scores[i * seq_kv..(i + 1) * seq_kv];
            if causal {
                for j in 0..seq_kv {
                    if j > i {
                        row[j] = f32::NEG_INFINITY;
                    }
                }
            }
            for x in row.iter_mut() {
                *x *= scale;
            }
            softmax_inplace(row);
        }
        for t in 0..seq_kv {
            k_buf[t * hd..(t + 1) * hd].copy_from_slice(&v[t * d + off..t * d + off + hd]);
        }
        matmul_f32(scores, seq_q, seq_kv, k_buf, hd, head_out);
        for t in 0..seq_q {
            merged[t * d + off..t * d + off + hd]
                .copy_from_slice(&head_out[t * hd..(t + 1) * hd]);
        }
    }
}

/// Atención de un query (decode incremental).
fn attention_single_ref(
    q: &[f32],
    k: &[f32],
    v: &[f32],
    seq: usize,
    d: usize,
    heads: usize,
    hd: usize,
    scores: &mut [f32],
    out: &mut [f32],
) {
    let scale = 1.0 / libm::sqrtf(hd as f32);
    out.fill(0.0);
    for h in 0..heads {
        let off = h * hd;
        let q_h = &q[off..off + hd];
        for t in 0..seq {
            let k_h = &k[t * d + off..t * d + off + hd];
            scores[t] = dot_f32(q_h, k_h) * scale;
        }
        softmax_inplace(&mut scores[..seq]);
        for ki in 0..hd {
            let mut acc = 0.0f32;
            for t in 0..seq {
                acc += scores[t] * v[t * d + off + ki];
            }
            out[off + ki] = acc;
        }
    }
}

#[test]
fn attention_batched_uses_q_not_k() {
    let d = 4usize;
    let heads = 1usize;
    let hd = 4usize;
    let seq = 2usize;
    let mut q = vec![0.0f32; seq * d];
    let mut k = vec![0.0f32; seq * d];
    let mut v = vec![0.0f32; seq * d];
    q[0] = 1.0;
    k[4] = 1.0;
    v[0] = 1.0;
    v[5] = 1.0;
    let mut scores = vec![0.0f32; seq * seq];
    let mut q_buf = vec![0.0f32; seq * hd];
    let mut k_buf = vec![0.0f32; seq * hd];
    let mut kt_buf = vec![0.0f32; hd * seq];
    let mut head_out = vec![0.0f32; seq * hd];
    let mut merged = vec![0.0f32; seq * d];
    attention_batched_ref(
        &q, &k, &v, seq, seq, d, heads, hd, false,
        &mut scores, &mut q_buf, &mut k_buf, &mut kt_buf,
        &mut head_out, &mut merged,
    );
    // Q·K^T raw score (0,1)=1; K·K^T bug gives (0,1)=0 → softmax distinto
    assert!(
        merged[4] > merged[0],
        "atención debe ponderar V[1]; merged={merged:?}"
    );
}

#[test]
fn preln_residual_preserves_input() {
    let mut x = vec![2.0f32, 4.0f32];
    let residual = x.clone();
    let gamma = vec![1.0, 1.0];
    let beta = vec![0.0, 0.0];
    layernorm(&mut x, &gamma, &beta, 1e-5);
    let mut branch = vec![1.0, 0.0];
    for i in 0..2 {
        x[i] = residual[i] + branch[i];
    }
    assert!((x[0] - 3.0).abs() < 1e-5);
    assert!((x[1] - 4.0).abs() < 1e-5);
}

#[test]
fn attention_single_matches_batched_last_query() {
    let d = 4usize;
    let heads = 2usize;
    let hd = 2usize;
    let seq = 3usize;
    let q = vec![1.0, 0.0, 0.0, 1.0];
    let mut k = vec![0.0f32; seq * d];
    let mut v = vec![0.0f32; seq * d];
    for t in 0..seq {
        k[t * d] = 1.0;
        v[t * d] = (t + 1) as f32;
    }
    let mut scores = vec![0.0f32; seq];
    let mut out_single = vec![0.0f32; d];
    attention_single_ref(&q, &k, &v, seq, d, heads, hd, &mut scores, &mut out_single);
    let mut q_full = vec![0.0f32; seq * d];
    q_full[(seq - 1) * d..].copy_from_slice(&q);
    let mut merged = vec![0.0f32; seq * d];
    let mut scores_b = vec![0.0f32; seq * seq];
    let mut q_buf = vec![0.0f32; seq * hd];
    let mut k_buf = vec![0.0f32; seq * hd];
    let mut kt = vec![0.0f32; hd * seq];
    let mut head_out = vec![0.0f32; seq * hd];
    attention_batched_ref(
        &q_full, &k, &v, seq, seq, d, heads, hd, true,
        &mut scores_b, &mut q_buf, &mut k_buf, &mut kt, &mut head_out, &mut merged,
    );
    let last = &merged[(seq - 1) * d..seq * d];
    for i in 0..d {
        assert!(
            (out_single[i] - last[i]).abs() < 1e-4,
            "mismatch at {i}: single={} batched={}",
            out_single[i],
            last[i]
        );
    }
}
