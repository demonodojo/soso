//! Atención de decode para Gemma 4: cualquier `head_dim`, escala arbitraria (Gemma usa **1.0**, no
//! `1/sqrt(d)`) y ventana deslizante opcional (`t_ini`).
//!
//! Es un camino **aparte** de `attn`/`attn_grupo` a propósito: aquellos kernels están especializados
//! (`head_dim` 64/128, escala fija) y generalizarlos arriesga a los modelos que ya funcionan. Éste prioriza
//! corrección y es suficientemente rápido en decode (cada fila de K/V se convierte de f16 una sola vez y se
//! reparte entre las `g` cabezas Q que la comparten); el prefill por lotes es otra cosa (ficha G7).
//!
//! Dos pasadas por cabeza, sin softmax en línea: puntuaciones de todas las posiciones → máximo y suma →
//! combinación de V. Gasta `g * n` floats de puntuaciones; para 24 k posiciones y 16 cabezas son 1,5 MiB.

use alloc::vec;
use alloc::vec::Vec;

/// f16 → f32 de una fila. Con AVX2 convierte de 8 en 8 los valores normales; los subnormales, infinitos y
/// NaN caen al camino escalar por vector (raros en un KV de inferencia).
pub fn f16_a_f32(src: &[u16], dst: &mut [f32]) {
    debug_assert!(dst.len() >= src.len());
    let mut i = 0usize;
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    unsafe {
        use core::arch::x86_64::*;
        while i + 8 <= src.len() {
            let h = _mm_loadu_si128(src.as_ptr().add(i) as *const __m128i);
            let w = _mm256_cvtepu16_epi32(h);
            let exp = _mm256_and_si256(_mm256_srli_epi32(w, 10), _mm256_set1_epi32(0x1f));
            let especial = _mm256_or_si256(
                _mm256_cmpeq_epi32(exp, _mm256_setzero_si256()),
                _mm256_cmpeq_epi32(exp, _mm256_set1_epi32(0x1f)),
            );
            if _mm256_movemask_epi8(especial) != 0 {
                for j in 0..8 {
                    dst[i + j] = crate::f16::f16_to_f32(src[i + j]);
                }
            } else {
                let sign = _mm256_slli_epi32(_mm256_srli_epi32(w, 15), 31);
                let mant = _mm256_slli_epi32(_mm256_and_si256(w, _mm256_set1_epi32(0x3ff)), 13);
                let fexp = _mm256_slli_epi32(_mm256_add_epi32(exp, _mm256_set1_epi32(112)), 23);
                let bits = _mm256_or_si256(sign, _mm256_or_si256(fexp, mant));
                _mm256_storeu_ps(dst.as_mut_ptr().add(i), _mm256_castsi256_ps(bits));
            }
            i += 8;
        }
    }
    while i < src.len() {
        dst[i] = crate::f16::f16_to_f32(src[i]);
        i += 1;
    }
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    // Cuatro acumuladores: deja que el compilador vectorice y fija el orden de las sumas.
    let mut s = [0.0f32; 4];
    let n = a.len().min(b.len());
    let mut i = 0;
    while i + 4 <= n {
        s[0] += a[i] * b[i];
        s[1] += a[i + 1] * b[i + 1];
        s[2] += a[i + 2] * b[i + 2];
        s[3] += a[i + 3] * b[i + 3];
        i += 4;
    }
    let mut t = (s[0] + s[1]) + (s[2] + s[3]);
    while i < n {
        t += a[i] * b[i];
        i += 1;
    }
    t
}

/// Atención de **un** token de consulta para `g` cabezas Q que comparten la cabeza KV `kv_head`.
///
/// - `q`: `g * head_dim` (las `g` cabezas seguidas); `out`: ídem.
/// - `k_f16`/`v_f16`: caché `[token][kv_dim]`; se atiende a las posiciones `t_ini..t_fin`.
/// - `scale`: factor de `Q·K` antes del softmax.
///
/// Con `t_ini >= t_fin` (nada que atender) la salida es 0.
pub fn attention_gqa_f16(
    q: &[f32],
    g: usize,
    k_f16: &[u16],
    v_f16: &[u16],
    head_dim: usize,
    kv_dim: usize,
    kv_head: usize,
    t_ini: usize,
    t_fin: usize,
    scale: f32,
    out: &mut [f32],
) {
    debug_assert!(q.len() >= g * head_dim && out.len() >= g * head_dim);
    out[..g * head_dim].fill(0.0);
    if g == 0 || head_dim == 0 || t_ini >= t_fin {
        return;
    }
    let n = t_fin - t_ini;
    let off = kv_head * head_dim;
    let mut kf = vec![0.0f32; head_dim];
    let mut puntos: Vec<f32> = vec![0.0; g * n];
    // Pasada 1: Q·K de todas las posiciones (cada fila de K se convierte una vez).
    for t in 0..n {
        let base = (t_ini + t) * kv_dim + off;
        f16_a_f32(&k_f16[base..base + head_dim], &mut kf);
        for j in 0..g {
            puntos[j * n + t] = scale * dot(&q[j * head_dim..(j + 1) * head_dim], &kf);
        }
    }
    // Softmax por cabeza.
    for j in 0..g {
        let p = &mut puntos[j * n..(j + 1) * n];
        let m = p.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let mut suma = 0.0f32;
        for x in p.iter_mut() {
            *x = libm::expf(*x - m);
            suma += *x;
        }
        let inv = if suma > 0.0 { 1.0 / suma } else { 0.0 };
        for x in p.iter_mut() {
            *x *= inv;
        }
    }
    // Pasada 2: combinación de V (cada fila de V se convierte una vez).
    let mut vf = vec![0.0f32; head_dim];
    for t in 0..n {
        let base = (t_ini + t) * kv_dim + off;
        f16_a_f32(&v_f16[base..base + head_dim], &mut vf);
        for j in 0..g {
            let p = puntos[j * n + t];
            let o = &mut out[j * head_dim..(j + 1) * head_dim];
            for d in 0..head_dim {
                o[d] += p * vf[d];
            }
        }
    }
}
