//! Atención de decodificación por **grupos GQA** con K/V en f16.
//!
//! Las `g` cabezas de consulta que comparten una cabeza KV se procesan juntas: cada fila de K/V se
//! convierte de f16 a f32 **una vez** por tile y se reutiliza para las `g` consultas (antes se
//! reconvertía y recargaba por cabeza). El producto Q·K va de 8 en 8 tokens con 8 acumuladores
//! independientes (sin cadena de latencia) y la exponencial es vectorial.
//!
//! **Invariante**: el resultado de cada consulta **no depende de `g`** ni del resto de consultas
//! del grupo: todas las sumas siguen un orden fijo. Así el prefill por bloques, la decodificación
//! token a token y cualquier reparto entre hilos dan lo mismo bit a bit.
//!
//! Sin AVX2+FMA (o con `head_dim` distinto de 64/128) se cae al camino escalar de `attn`.

use crate::attn::attention_decode_f16_tiled_scalar;

/// Tokens por tile. Con `head_dim` 128 los buffers K y V convertidos ocupan 2 × 16 KiB.
pub const GT: usize = 32;

const AVX2: bool = cfg!(all(
    target_arch = "x86_64",
    target_feature = "avx2",
    target_feature = "fma"
));

/// Consultas por pasada del kernel (`ntok * g` como mucho).
pub const MAX_Q: usize = 64;

/// Atención causal de `ntok` vectores de consulta consecutivos del mismo bloque, cada uno con `g`
/// cabezas que comparten la cabeza KV `kv_head`.
///
/// `qs`/`outs`: el token `i` empieza en `i * stride` y su cabeza `j`, en `i * stride + j * head_dim`.
/// El token `i` atiende a las posiciones `0..seq_primero + i`. K/V se convierten **una vez por tile**
/// para los `ntok * g` vectores. El resultado de cada consulta es el que daría ella sola.
pub fn attention_f16_tokens(
    qs: &[f32],
    outs: &mut [f32],
    stride: usize,
    ntok: usize,
    g: usize,
    seq_primero: usize,
    k_f16: &[u16],
    v_f16: &[u16],
    head_dim: usize,
    kv_dim: usize,
    kv_head: usize,
) {
    if ntok == 0 || g == 0 || head_dim == 0 {
        return;
    }
    assert!(qs.len() >= (ntok - 1) * stride + g * head_dim && outs.len() >= (ntok - 1) * stride + g * head_dim);
    unsafe {
        attention_f16_tokens_raw(
            qs.as_ptr(),
            outs.as_mut_ptr(),
            stride,
            ntok,
            g,
            seq_primero,
            k_f16,
            v_f16,
            head_dim,
            kv_dim,
            kv_head,
        );
    }
}

/// Como [`attention_f16_tokens`] con punteros crudos: varios hilos pueden atender a cabezas KV
/// distintas del mismo bloque, cuyas zonas de `qs`/`outs` se entrelazan por token sin solaparse;
/// con `&mut [f32]` esas porciones se solaparían como rangos.
///
/// # Safety
/// `qs` y `outs` válidos para `(ntok - 1) * stride + g * head_dim` elementos; nadie más escribe las
/// zonas que este kernel escribe (`g * head_dim` por token desde `i * stride`); K/V con
/// `seq_primero + ntok - 1` filas.
pub unsafe fn attention_f16_tokens_raw(
    qs: *const f32,
    outs: *mut f32,
    stride: usize,
    ntok: usize,
    g: usize,
    seq_primero: usize,
    k_f16: &[u16],
    v_f16: &[u16],
    head_dim: usize,
    kv_dim: usize,
    kv_head: usize,
) {
    if ntok == 0 || g == 0 || head_dim == 0 {
        return;
    }
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2", target_feature = "fma"))]
    if AVX2 && (head_dim == 64 || head_dim == 128) && g <= MAX_Q {
        // Tokens por llamada: que `ntok * g` quepa en el estado del kernel.
        let por_llamada = (MAX_Q / g).max(1);
        let mut i0 = 0usize;
        while i0 < ntok {
            let nt = (ntok - i0).min(por_llamada);
            unsafe {
                let q = qs.add(i0 * stride);
                let o = outs.add(i0 * stride);
                if head_dim == 128 {
                    avx::multi::<16>(q, o, stride, nt, g, seq_primero + i0, k_f16, v_f16, kv_dim, kv_head);
                } else {
                    avx::multi::<8>(q, o, stride, nt, g, seq_primero + i0, k_f16, v_f16, kv_dim, kv_head);
                }
            }
            i0 += nt;
        }
        return;
    }
    let _ = AVX2;
    for i in 0..ntok {
        for j in 0..g {
            let off = i * stride + j * head_dim;
            let (q, o) = unsafe {
                (
                    core::slice::from_raw_parts(qs.add(off), head_dim),
                    core::slice::from_raw_parts_mut(outs.add(off), head_dim),
                )
            };
            attention_decode_f16_tiled_scalar(q, k_f16, v_f16, head_dim, kv_dim, kv_head, seq_primero + i, o);
        }
    }
}

/// `qs`: `g` consultas seguidas (`g * head_dim`); `outs`: `g * head_dim` de salida.
/// Atiende a las posiciones `0..n_tokens` del KV (layout `[token][kv_dim]`).
pub fn attention_f16_group(
    qs: &[f32],
    g: usize,
    k_f16: &[u16],
    v_f16: &[u16],
    head_dim: usize,
    kv_dim: usize,
    kv_head: usize,
    n_tokens: usize,
    outs: &mut [f32],
) {
    debug_assert!(qs.len() >= g * head_dim && outs.len() >= g * head_dim);
    if n_tokens == 0 || head_dim == 0 || g == 0 {
        outs[..g * head_dim].fill(0.0);
        return;
    }
    attention_f16_tokens(qs, outs, g * head_dim, 1, g, n_tokens, k_f16, v_f16, head_dim, kv_dim, kv_head);
}

#[cfg(all(target_arch = "x86_64", target_feature = "avx2", target_feature = "fma"))]
mod avx {
    use super::{GT, MAX_Q};
    use crate::simd_math::avx::exp256;
    use core::arch::x86_64::*;

    /// f16 → f32 de 8 valores. Con F16C una instrucción; sin ella, la emulación (mismo resultado).
    #[inline]
    #[target_feature(enable = "avx2,fma")]
    unsafe fn cargar_f16x8(p: *const u16) -> __m256 {
        unsafe {
            #[cfg(target_feature = "f16c")]
            {
                _mm256_cvtph_ps(_mm_loadu_si128(p as *const __m128i))
            }
            #[cfg(not(target_feature = "f16c"))]
            {
                let mut tmp = [0.0f32; 8];
                for i in 0..8 {
                    tmp[i] = crate::f16::f16_to_f32(*p.add(i));
                }
                _mm256_loadu_ps(tmp.as_ptr())
            }
        }
    }

    /// Suma horizontal en orden fijo ((0+1)+(2+3))+((4+5)+(6+7)).
    #[inline]
    #[target_feature(enable = "avx2,fma")]
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

    /// Q·K de 8 tokens a la vez (`kbuf[(t0 + u) * hd ..]` es la fila convertida del token `u`): 8
    /// acumuladores independientes y una transposición con `hadd` que deja las 8 puntuaciones en un
    /// solo vector. Cada token suma sus trozos y sus 8 lanes en un orden fijo, sea cual sea `NCH`.
    #[inline]
    #[target_feature(enable = "avx2,fma")]
    unsafe fn dots8<const NCH: usize>(q: *const f32, kbuf: *const f32, hd: usize, t0: usize) -> __m256 {
        unsafe {
            let mut a = [_mm256_setzero_ps(); 8];
            for c in 0..NCH {
                let qv = _mm256_loadu_ps(q.add(c * 8));
                for u in 0..8 {
                    let kv = _mm256_loadu_ps(kbuf.add((t0 + u) * hd + c * 8));
                    a[u] = _mm256_fmadd_ps(qv, kv, a[u]);
                }
            }
            let t01 = _mm256_hadd_ps(a[0], a[1]);
            let t23 = _mm256_hadd_ps(a[2], a[3]);
            let t45 = _mm256_hadd_ps(a[4], a[5]);
            let t67 = _mm256_hadd_ps(a[6], a[7]);
            let u03 = _mm256_hadd_ps(t01, t23);
            let u47 = _mm256_hadd_ps(t45, t67);
            let s03 = _mm_add_ps(_mm256_castps256_ps128(u03), _mm256_extractf128_ps(u03, 1));
            let s47 = _mm_add_ps(_mm256_castps256_ps128(u47), _mm256_extractf128_ps(u47, 1));
            _mm256_insertf128_ps(_mm256_castps128_ps256(s03), s47, 1)
        }
    }

    /// Máscaras de las `r` primeras lanes (0..=8).
    static MASCARAS: [[u32; 8]; 9] = {
        let mut t = [[0u32; 8]; 9];
        let mut r = 0;
        while r < 9 {
            let mut i = 0;
            while i < r {
                t[r][i] = u32::MAX;
                i += 1;
            }
            r += 1;
        }
        t
    };

    /// Máximo horizontal de 8 lanes.
    #[inline]
    #[target_feature(enable = "avx2,fma")]
    unsafe fn hmax(v: __m256) -> f32 {
        unsafe {
            let m = _mm_max_ps(_mm256_castps256_ps128(v), _mm256_extractf128_ps(v, 1));
            let m = _mm_max_ps(m, _mm_movehl_ps(m, m));
            let m = _mm_max_ss(m, _mm_shuffle_ps(m, m, 1));
            _mm_cvtss_f32(m)
        }
    }

    /// # Safety
    /// AVX2+FMA; `ntok * g <= MAX_Q`; `qs`/`outs` con el token `i` en `i * stride`; el token `i`
    /// atiende a `seq_primero + i` filas válidas de K/V.
    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn multi<const NCH: usize>(
        qs: *const f32,
        outs: *mut f32,
        stride: usize,
        ntok: usize,
        g: usize,
        seq_primero: usize,
        k_f16: &[u16],
        v_f16: &[u16],
        kv_dim: usize,
        kv_head: usize,
    ) {
        unsafe {
            let hd = NCH * 8;
            let inv = 1.0 / libm::sqrtf(hd as f32);
            let nq = ntok * g;
            let mut kbuf = [0.0f32; GT * 128];
            let mut vbuf = [0.0f32; GT * 128];
            let mut m = [f32::NEG_INFINITY; MAX_Q];
            let mut l = [0.0f32; MAX_Q];
            let mut pr = [0.0f32; GT];
            let off_q = |idx: usize| (idx / g) * stride + (idx % g) * hd;
            for idx in 0..nq {
                core::ptr::write_bytes(outs.add(off_q(idx)), 0, hd);
            }
            let max_limite = seq_primero + ntok - 1;

            let mut t0 = 0usize;
            while t0 < max_limite {
                let tn_max = (max_limite - t0).min(GT);
                // K y V del tile a f32, una vez para todas las consultas.
                for i in 0..tn_max {
                    let off = (t0 + i) * kv_dim + kv_head * hd;
                    for c in 0..NCH {
                        _mm256_storeu_ps(kbuf.as_mut_ptr().add(i * hd + c * 8), cargar_f16x8(k_f16.as_ptr().add(off + c * 8)));
                        _mm256_storeu_ps(vbuf.as_mut_ptr().add(i * hd + c * 8), cargar_f16x8(v_f16.as_ptr().add(off + c * 8)));
                    }
                }
                // Relleno hasta múltiplo de 8 (las puntuaciones sobrantes no cuentan).
                for i in tn_max..tn_max.div_ceil(8) * 8 {
                    for d in 0..hd {
                        kbuf[i * hd + d] = 0.0;
                    }
                }
                for idx in 0..nq {
                    let limite = seq_primero + idx / g;
                    if limite <= t0 {
                        continue;
                    }
                    let tn = (limite - t0).min(GT);
                    let q = qs.add(off_q(idx));
                    let ng = tn.div_ceil(8);
                    let resto = tn - (ng - 1) * 8; // lanes válidas del último grupo (1..=8)
                    // 1) puntuaciones en registros; las lanes sobrantes del último grupo valen −inf
                    let invv = _mm256_set1_ps(inv);
                    let mut sv = [_mm256_setzero_ps(); GT / 8];
                    let mut vmax = _mm256_set1_ps(f32::NEG_INFINITY);
                    for gi in 0..ng {
                        let mut x = _mm256_mul_ps(dots8::<NCH>(q, kbuf.as_ptr(), hd, gi * 8), invv);
                        if gi == ng - 1 && resto < 8 {
                            let mk = _mm256_loadu_ps(MASCARAS[resto].as_ptr() as *const f32);
                            x = _mm256_blendv_ps(_mm256_set1_ps(f32::NEG_INFINITY), x, mk);
                        }
                        sv[gi] = x;
                        vmax = _mm256_max_ps(vmax, x);
                    }
                    let tile_max = hmax(vmax);
                    let m_new = if m[idx] > tile_max { m[idx] } else { tile_max };
                    let alpha = if m[idx] == f32::NEG_INFINITY { 0.0 } else { libm::expf(m[idx] - m_new) };
                    // 2) probabilidades del tile (exp vectorial; las lanes sobrantes valen 0)
                    let mv = _mm256_set1_ps(m_new);
                    let mut psum = _mm256_setzero_ps();
                    for gi in 0..ng {
                        let mut pgi = exp256(_mm256_sub_ps(sv[gi], mv));
                        if gi == ng - 1 && resto < 8 {
                            pgi = _mm256_and_ps(pgi, _mm256_loadu_ps(MASCARAS[resto].as_ptr() as *const f32));
                        }
                        _mm256_storeu_ps(pr.as_mut_ptr().add(gi * 8), pgi);
                        psum = _mm256_add_ps(psum, pgi);
                    }
                    let tile_sum = hsum(psum);
                    // 3) salida: o = o·alpha + Σ p·V; 8 acumuladores a la vez (caben en registros)
                    let o = outs.add(off_q(idx));
                    let av = _mm256_set1_ps(alpha);
                    for mitad in 0..NCH.div_ceil(8) {
                        let c0 = mitad * 8;
                        let nc = (NCH - c0).min(8);
                        let mut acc = [_mm256_setzero_ps(); 8];
                        for c in 0..nc {
                            acc[c] = _mm256_mul_ps(_mm256_loadu_ps(o.add((c0 + c) * 8)), av);
                        }
                        for i in 0..tn {
                            let pv = _mm256_set1_ps(pr[i]);
                            let fila = vbuf.as_ptr().add(i * hd + c0 * 8);
                            for c in 0..nc {
                                acc[c] = _mm256_fmadd_ps(pv, _mm256_loadu_ps(fila.add(c * 8)), acc[c]);
                            }
                        }
                        for c in 0..nc {
                            _mm256_storeu_ps(o.add((c0 + c) * 8), acc[c]);
                        }
                    }
                    l[idx] = l[idx] * alpha + tile_sum;
                    m[idx] = m_new;
                }
                t0 += tn_max;
            }
            for idx in 0..nq {
                if l[idx] > 0.0 {
                    let s = _mm256_set1_ps(1.0 / l[idx]);
                    let o = outs.add(off_q(idx));
                    for c in 0..NCH {
                        _mm256_storeu_ps(o.add(c * 8), _mm256_mul_ps(_mm256_loadu_ps(o.add(c * 8)), s));
                    }
                }
            }
        }
    }
}
