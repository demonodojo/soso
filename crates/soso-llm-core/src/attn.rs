//! Atención tiled estilo FlashAttention para decode (1 query):
//! online softmax por bloques de KV — no materializa scores N y mejora
//! localidad de caché sobre el KV en f16 (PagedAttention/Flash decode).
//!
//! Con AVX2+FMA (userspace soso) el dot Q·K y el axpy de V van por SIMD;
//! sin esas features se usa el camino escalar (tests host / referencia).

use crate::f16::f16_to_f32;
use crate::gemm::{dot_f32, rope_inplace, softmax_inplace};
use crate::layer::{matvec_view, TensorSource};
use alloc::vec::Vec;

/// Tamaño de tile sobre la secuencia KV (tokens). 64 encaja bien en L1
/// con head_dim típico ≤ 128 y f16.
pub const ATTN_TILE_TOKENS: usize = 64;

const AVX2: bool = cfg!(all(
    target_arch = "x86_64",
    target_feature = "avx2",
    target_feature = "fma"
));

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
    if AVX2 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            return avx2::attention_decode_f16_tiled(
                q, k_f16, v_f16, head_dim, kv_dim, kv_head, n_tokens, out,
            );
        }
    }
    attention_decode_f16_tiled_scalar(
        q, k_f16, v_f16, head_dim, kv_dim, kv_head, n_tokens, out,
    );
}

pub fn attention_decode_f16_tiled_scalar(
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
    let mut m = f32::NEG_INFINITY;
    let mut l = 0.0f32;
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

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::ATTN_TILE_TOKENS;
    use crate::f16::f16_to_f32;
    use core::arch::x86_64::*;

    #[inline]
    unsafe fn hsum(v: __m256) -> f32 {
        unsafe {
            let lo = _mm256_castps256_ps128(v);
            let hi = _mm256_extractf128_ps(v, 1);
            let s = _mm_add_ps(lo, hi);
            let s = _mm_add_ps(s, _mm_movehl_ps(s, s));
            let s = _mm_add_ss(s, _mm_shuffle_ps(s, s, 1));
            _mm_cvtss_f32(s)
        }
    }

    /// f16→f32 vectorial para normales finitos; si hay subnormal/inf/NaN
    /// cae al camino escalar por lane (raro en KV de inferencia).
    #[inline]
    unsafe fn load_f16x8(ptr: *const u16) -> __m256 {
        unsafe {
            let h = _mm_loadu_si128(ptr as *const __m128i);
            let w = _mm256_cvtepu16_epi32(h);
            // exp = (bits >> 10) & 0x1f
            let exp = _mm256_and_si256(_mm256_srli_epi32(w, 10), _mm256_set1_epi32(0x1f));
            let is_special = _mm256_or_si256(
                _mm256_cmpeq_epi32(exp, _mm256_setzero_si256()),
                _mm256_cmpeq_epi32(exp, _mm256_set1_epi32(0x1f)),
            );
            if _mm256_movemask_epi8(is_special) != 0 {
                let mut tmp = [0.0f32; 8];
                for i in 0..8 {
                    tmp[i] = f16_to_f32(*ptr.add(i));
                }
                return _mm256_loadu_ps(tmp.as_ptr());
            }
            // f32 = sign<<31 | (exp+112)<<23 | frac<<13
            let sign = _mm256_slli_epi32(_mm256_srli_epi32(w, 15), 31);
            let mant = _mm256_and_si256(w, _mm256_set1_epi32(0x3ff));
            let fexp = _mm256_slli_epi32(_mm256_add_epi32(exp, _mm256_set1_epi32(112)), 23);
            let fmant = _mm256_slli_epi32(mant, 13);
            let bits = _mm256_or_si256(sign, _mm256_or_si256(fexp, fmant));
            _mm256_castsi256_ps(bits)
        }
    }

    #[inline]
    unsafe fn dot_q_k_f16(q: &[f32], k: *const u16, head_dim: usize) -> f32 {
        unsafe {
            let mut acc = _mm256_setzero_ps();
            let mut d = 0usize;
            while d + 8 <= head_dim {
                let qv = _mm256_loadu_ps(q.as_ptr().add(d));
                let kv = load_f16x8(k.add(d));
                acc = _mm256_fmadd_ps(qv, kv, acc);
                d += 8;
            }
            let mut s = hsum(acc);
            while d < head_dim {
                s += q[d] * f16_to_f32(*k.add(d));
                d += 1;
            }
            s
        }
    }

    #[inline]
    unsafe fn axpy_v_f16(out: &mut [f32], scale: f32, v: *const u16, head_dim: usize) {
        unsafe {
            let sv = _mm256_set1_ps(scale);
            let mut d = 0usize;
            while d + 8 <= head_dim {
                let ov = _mm256_loadu_ps(out.as_ptr().add(d));
                let vv = load_f16x8(v.add(d));
                _mm256_storeu_ps(out.as_mut_ptr().add(d), _mm256_fmadd_ps(sv, vv, ov));
                d += 8;
            }
            while d < head_dim {
                out[d] += scale * f16_to_f32(*v.add(d));
                d += 1;
            }
        }
    }

    #[inline]
    unsafe fn scale_out(out: &mut [f32], alpha: f32, head_dim: usize) {
        unsafe {
            let a = _mm256_set1_ps(alpha);
            let mut d = 0usize;
            while d + 8 <= head_dim {
                let ov = _mm256_loadu_ps(out.as_ptr().add(d));
                _mm256_storeu_ps(out.as_mut_ptr().add(d), _mm256_mul_ps(ov, a));
                d += 8;
            }
            while d < head_dim {
                out[d] *= alpha;
                d += 1;
            }
        }
    }

    /// # Safety
    /// AVX2+FMA disponibles; slices con longitudes coherentes.
    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn attention_decode_f16_tiled(
        q: &[f32],
        k_f16: &[u16],
        v_f16: &[u16],
        head_dim: usize,
        kv_dim: usize,
        kv_head: usize,
        n_tokens: usize,
        out: &mut [f32],
    ) {
        unsafe {
            out.fill(0.0);
            if n_tokens == 0 || head_dim == 0 {
                return;
            }
            let inv = 1.0 / libm::sqrtf(head_dim as f32);
            let mut m = f32::NEG_INFINITY;
            let mut l = 0.0f32;
            let mut tile_scores = [0.0f32; ATTN_TILE_TOKENS];

            let mut t0 = 0usize;
            while t0 < n_tokens {
                let t1 = (t0 + ATTN_TILE_TOKENS).min(n_tokens);
                let mut tile_max = f32::NEG_INFINITY;
                for (i, t) in (t0..t1).enumerate() {
                    let k_off = t * kv_dim + kv_head * head_dim;
                    let s = dot_q_k_f16(q, k_f16.as_ptr().add(k_off), head_dim) * inv;
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
                scale_out(out, alpha, head_dim);
                l *= alpha;
                let mut tile_sum = 0.0f32;
                for (i, t) in (t0..t1).enumerate() {
                    let p = libm::expf(tile_scores[i] - m_new);
                    tile_sum += p;
                    let v_off = t * kv_dim + kv_head * head_dim;
                    axpy_v_f16(out, p, v_f16.as_ptr().add(v_off), head_dim);
                }
                l += tile_sum;
                m = m_new;
                t0 = t1;
            }
            if l > 0.0 {
                scale_out(out, 1.0 / l, head_dim);
            }
        }
    }
}

/// Decode MLA con cache latente: expande K/V on-the-fly vía `k_up`/`v_up`.
pub fn attention_decode_mla_latent<S: TensorSource>(
    q: &[f32],
    kv: &crate::kv::LayerKv,
    kv_rank: usize,
    qk_nope: usize,
    qk_rope: usize,
    v_dim: usize,
    head_dim: usize,
    kv_head: usize,
    n_tokens: usize,
    theta: f32,
    source: &mut S,
    k_up_name: &str,
    v_up_name: &str,
    kv_qk_dim: usize,
    kv_v_dim: usize,
    k_full: &mut [f32],
    v_full: &mut [f32],
    c_buf: &mut [f32],
    out: &mut [f32],
) -> Result<(), ()> {
    out.fill(0.0);
    if n_tokens == 0 || !kv.mla_latent {
        return Err(());
    }
    let qk_per = qk_nope + qk_rope;
    if qk_per == 0 || v_dim == 0 || head_dim == 0 {
        return Err(());
    }
    if k_full.len() < kv_qk_dim || v_full.len() < kv_v_dim {
        return Err(());
    }
    let inv = 1.0 / libm::sqrtf(qk_per as f32);
    let mut logits = alloc::vec![0.0f32; n_tokens];
    let k_off = kv_head * qk_per;
    let v_off = kv_head * v_dim;
    for t in 0..n_tokens {
        kv.load_latent_token(t, kv_rank, &mut c_buf[..kv_rank]);
        matvec_view(
            &source.tensor_view(k_up_name)?,
            kv_qk_dim,
            kv_rank,
            &c_buf[..kv_rank],
            k_full,
        )?;
        if qk_rope > 0 {
            rope_inplace(
                &mut k_full[k_off + qk_nope..k_off + qk_nope + qk_rope],
                t,
                theta,
            );
        }
        let q_len = qk_per.min(q.len());
        let mut dot = 0.0f32;
        for d in 0..q_len {
            dot += q[d] * k_full[k_off + d];
        }
        logits[t] = dot * inv;
    }
    let mut max_l = logits[0];
    for t in 1..n_tokens {
        if logits[t] > max_l {
            max_l = logits[t];
        }
    }
    let mut sum = 0.0f32;
    for t in 0..n_tokens {
        logits[t] = libm::expf(logits[t] - max_l);
        sum += logits[t];
    }
    if sum > 0.0 {
        for t in 0..n_tokens {
            logits[t] /= sum;
        }
    }
    for t in 0..n_tokens {
        kv.load_latent_token(t, kv_rank, &mut c_buf[..kv_rank]);
        matvec_view(
            &source.tensor_view(v_up_name)?,
            kv_v_dim,
            kv_rank,
            &c_buf[..kv_rank],
            v_full,
        )?;
        for d in 0..head_dim {
            if d < v_dim {
                out[d] += logits[t] * v_full[v_off + d];
            }
        }
    }
    Ok(())
}

/// Umbral Quest-lite: por encima, solo se atienden bloques top-k + sink + recent.
pub const SPARSE_TOKEN_THRESHOLD: usize = 256;
pub const SPARSE_BLOCK: usize = 32;
pub const SPARSE_TOP_BLOCKS: usize = 4;

/// Decode sobre `LayerKv` (f16 o int8 KIVI-lite). Online softmax tiled;
/// opcionalmente sparse por bloques (Quest). Devuelve scores softmax (para H2O)
/// en `mass_out` si se pasa.
pub fn attention_decode_kv(
    q: &[f32],
    kv: &crate::kv::LayerKv,
    head_dim: usize,
    kv_dim: usize,
    kv_head: usize,
    n_tokens: usize,
    out: &mut [f32],
    mut mass_out: Option<&mut [f32]>,
    sparse: bool,
) {
    out.fill(0.0);
    if n_tokens == 0 || head_dim == 0 {
        return;
    }
    let inv = 1.0 / libm::sqrtf(head_dim as f32);
    let mut tmp_k = alloc::vec![0.0f32; head_dim];
    let mut tmp_v = alloc::vec![0.0f32; head_dim];

    // Selección de tokens (dense o sparse).
    let mut token_list: alloc::vec::Vec<usize> = alloc::vec::Vec::with_capacity(n_tokens);
    if sparse && n_tokens > SPARSE_TOKEN_THRESHOLD {
        let n_blocks = (n_tokens + SPARSE_BLOCK - 1) / SPARSE_BLOCK;
        let mut block_score = alloc::vec![0.0f32; n_blocks];
        for b in 0..n_blocks {
            let t0 = b * SPARSE_BLOCK;
            let t1 = (t0 + SPARSE_BLOCK).min(n_tokens);
            let mut best = f32::NEG_INFINITY;
            // Sample del bloque (primer y último token) — proxy barato.
            for &t in &[t0, t1.saturating_sub(1)] {
                kv.load_k_head(t, kv_head, head_dim, kv_dim, &mut tmp_k);
                let mut dot = 0.0f32;
                for d in 0..head_dim {
                    dot += q[d] * tmp_k[d];
                }
                let s = libm::fabsf(dot * inv);
                if s > best {
                    best = s;
                }
            }
            block_score[b] = best;
        }
        let mut order: alloc::vec::Vec<usize> = (0..n_blocks).collect();
        order.sort_by(|&a, &b| {
            block_score[b]
                .partial_cmp(&block_score[a])
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        let mut keep = alloc::vec![false; n_blocks];
        keep[0] = true; // sink block
        keep[n_blocks - 1] = true; // recent
        for &b in order.iter().take(SPARSE_TOP_BLOCKS) {
            keep[b] = true;
        }
        for b in 0..n_blocks {
            if keep[b] {
                let t0 = b * SPARSE_BLOCK;
                let t1 = (t0 + SPARSE_BLOCK).min(n_tokens);
                for t in t0..t1 {
                    token_list.push(t);
                }
            }
        }
    } else {
        token_list.extend(0..n_tokens);
    }

    let mut m = f32::NEG_INFINITY;
    let mut scores = alloc::vec![0.0f32; token_list.len()];

    for (i, &t) in token_list.iter().enumerate() {
        kv.load_k_head(t, kv_head, head_dim, kv_dim, &mut tmp_k);
        let mut dot = 0.0f32;
        for d in 0..head_dim {
            dot += q[d] * tmp_k[d];
        }
        let s = dot * inv;
        scores[i] = s;
        if s > m {
            m = s;
        }
    }
    let mut sum = 0.0f32;
    for s in scores.iter_mut() {
        *s = libm::expf(*s - m);
        sum += *s;
    }
    let inv_sum = if sum > 0.0 { 1.0 / sum } else { 0.0 };
    if let Some(mass) = mass_out.as_mut() {
        let n = mass.len().min(n_tokens);
        for i in 0..n {
            mass[i] = 0.0;
        }
    }
    for (i, &t) in token_list.iter().enumerate() {
        let p = scores[i] * inv_sum;
        if let Some(mass) = mass_out.as_mut() {
            if t < mass.len() {
                mass[t] = p;
            }
        }
        kv.load_v_head(t, kv_head, head_dim, kv_dim, &mut tmp_v);
        for d in 0..head_dim {
            out[d] += p * tmp_v[d];
        }
    }
}

/// Prompt Lookup Decoding con n-gramo **adaptativo**: prueba del más largo
/// al más corto (7→2) y se queda con la primera coincidencia (más específica).
pub fn prompt_lookup_draft(haystack: &[u32], max_draft: usize) -> Vec<u32> {
    prompt_lookup_draft_adaptive(haystack, max_draft, 2, 7)
}

/// Igual que `prompt_lookup_draft` con rango de n-gramo configurable.
pub fn prompt_lookup_draft_adaptive(
    haystack: &[u32],
    max_draft: usize,
    min_n: usize,
    max_n: usize,
) -> Vec<u32> {
    prompt_lookup_draft_hinted(haystack, max_draft, min_n, max_n, max_n)
}

fn lookup_ngram(haystack: &[u32], n: usize, max_draft: usize) -> Option<Vec<u32>> {
    if haystack.len() < n + 1 || n == 0 || max_draft == 0 {
        return None;
    }
    let suffix = &haystack[haystack.len() - n..];
    let search_end = haystack.len() - n;
    let mut best_at = None;
    let mut i = 0usize;
    while i < search_end {
        if &haystack[i..i + n] == suffix {
            best_at = Some(i + n);
        }
        i += 1;
    }
    best_at.and_then(|start| {
        let end = (start + max_draft).min(haystack.len());
        if end > start {
            Some(haystack[start..end].to_vec())
        } else {
            None
        }
    })
}

/// PLD con **hint**: prueba primero `hint_n` (autotune del planner) y si falla
/// recorre max_n→min_n. Reduce falsos positivos tras fallos y acelera hits
/// cuando el hint es bueno.
pub fn prompt_lookup_draft_hinted(
    haystack: &[u32],
    max_draft: usize,
    min_n: usize,
    max_n: usize,
    hint_n: usize,
) -> Vec<u32> {
    if haystack.len() < min_n + 1 || max_draft == 0 || min_n == 0 {
        return Vec::new();
    }
    let max_n = max_n.min(haystack.len() - 1).min(16);
    let min_n = min_n.min(max_n);
    let hint = hint_n.clamp(min_n, max_n);
    if let Some(d) = lookup_ngram(haystack, hint, max_draft) {
        return d;
    }
    for n in (min_n..=max_n).rev() {
        if n == hint {
            continue;
        }
        if let Some(d) = lookup_ngram(haystack, n, max_draft) {
            return d;
        }
    }
    Vec::new()
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
        attention_decode_f16_tiled_scalar(&q, &k, &v, head_dim, kv_dim, 0, n, &mut tiled);
        for (a, b) in dense.iter().zip(tiled.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
        let mut tiled2 = vec![0.0f32; head_dim];
        attention_decode_f16_tiled(&q, &k, &v, head_dim, kv_dim, 0, n, &mut tiled2);
        for (a, b) in dense.iter().zip(tiled2.iter()) {
            assert!((a - b).abs() < 1e-3, "dispatch {a} vs {b}");
        }
    }

    #[test]
    fn layer_kv_decode_matches_dense() {
        use crate::kv::{KvDtype, LayerKv};
        let head_dim = 8;
        let kv_dim = 8;
        let n = 40;
        let q: Vec<f32> = (0..head_dim).map(|i| (i as f32) * 0.1).collect();
        let mut kv = LayerKv::with_capacity_dtype(n, kv_dim, KvDtype::F16);
        let mut k_f32 = Vec::new();
        let mut v_f32 = Vec::new();
        for t in 0..n {
            let mut k = vec![0.0f32; head_dim];
            let mut v = vec![0.0f32; head_dim];
            for d in 0..head_dim {
                k[d] = (t * head_dim + d) as f32 * 0.01;
                v[d] = (t + d) as f32 * 0.02;
            }
            k_f32.extend_from_slice(&k);
            v_f32.extend_from_slice(&v);
            kv.append(&k, &v);
        }
        let mut dense = vec![0.0f32; head_dim];
        attention_step(&q, &k_f32, &v_f32, head_dim, n, &mut dense);
        let mut out = vec![0.0f32; head_dim];
        let mut mass = vec![0.0f32; n];
        attention_decode_kv(&q, &kv, head_dim, kv_dim, 0, n, &mut out, Some(&mut mass), false);
        for (a, b) in dense.iter().zip(out.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
        let mass_sum: f32 = mass.iter().sum();
        assert!((mass_sum - 1.0).abs() < 1e-3, "softmax mass {mass_sum}");
    }

    #[test]
    fn prompt_lookup_finds_continuation() {
        let hist = [1u32, 2, 3, 4, 5, 2, 3, 4];
        let draft = prompt_lookup_draft(&hist, 2);
        assert_eq!(draft, vec![5, 2]);
        assert_eq!(prompt_lookup_draft(&hist, 1), vec![5]);
    }

    #[test]
    fn prompt_lookup_empty_without_match() {
        let hist = [1u32, 2, 3, 9, 8, 7];
        assert!(prompt_lookup_draft(&hist, 4).is_empty());
    }

    #[test]
    fn adaptive_prefers_longer_ngram() {
        // Con n=2 el sufijo [4,5] aparece al inicio → draft [4,5,6]
        // Con n=4 el sufijo [2,3,4,5] aparece una vez → draft [9]
        // Adaptativo debe elegir n=4 (más largo) → [9]
        let hist = [4u32, 5, 4, 5, 6, 2, 3, 4, 5, 9, 2, 3, 4, 5];
        let draft = prompt_lookup_draft_adaptive(&hist, 3, 2, 5);
        assert_eq!(draft[0], 9); // n=4 gana frente al falso positivo de n=2
        assert_eq!(prompt_lookup_draft_adaptive(&hist, 1, 2, 5), vec![9]);
    }

    #[test]
    fn hinted_tries_prefer_n_then_fallback() {
        // Solo n=2 tiene match → hint=4 falla y cae a n=2
        let hist = [1u32, 2, 9, 1, 2];
        assert_eq!(prompt_lookup_draft_hinted(&hist, 1, 2, 5, 4), vec![9]);
        assert_eq!(prompt_lookup_draft_hinted(&hist, 1, 2, 5, 2), vec![9]);
    }
}
