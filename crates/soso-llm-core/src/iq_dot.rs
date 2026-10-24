//! Producto de superbloque IQ3_XXS / IQ2_S / IQ4_XS × Q8_K (`ggml_vec_dot_*_q8_K` de llama.cpp).
//!
//! La referencia escalar y la AVX2 hacen la misma aritmética entera y la misma secuencia de operaciones en coma flotante
//! (un `d * bsum` por superbloque), así que dan **el mismo f32 bit a bit**; `tests/iq_dot.rs` lo comprueba contra la
//! descuantización de `sosomodel::iq` por un lado y contra el escalar por otro.

use crate::q8k::{Q8_K_BLOCK_BYTES, Q8_K_BLOCK_ELEMS};
use sosomodel::dequant::f16_to_f32;
use sosomodel::iq::{escala_y_minimo_k4, GGML_IQ2_S, GGML_IQ3_S, GGML_IQ3_XXS, GGML_IQ4_XS, GGML_Q5_K, IQ2_S_BYTES, IQ3_S_BYTES, IQ3_XXS_BYTES, IQ4_XS_BYTES, Q5_K_BYTES};
use sosomodel::iq_tablas::{IQ2S_GRID, IQ3S_GRID, IQ3XXS_GRID, KMASK_IQ2XS, KSIGNS_IQ2XS, KVALUES_IQ4NL};

/// Factor con el que ggml cierra cada fila de ese tipo (`0.25f * sumf`, …).
pub fn factor_fila(ggml: u32) -> f32 {
    match ggml {
        GGML_IQ3_XXS => 0.25,
        GGML_IQ2_S => 0.125,
        _ => 1.0,
    }
}

/// ¿Hay producto entero × Q8_K para este tipo GGML?
pub fn soporta(ggml: u32) -> bool {
    matches!(ggml, GGML_IQ3_XXS | GGML_IQ2_S | GGML_IQ4_XS | GGML_IQ3_S | GGML_Q5_K)
}

#[inline]
fn q8_d(q8: &[u8]) -> f32 {
    f32::from_le_bytes([q8[0], q8[1], q8[2], q8[3]])
}

#[inline]
fn s(sign: u8, j: usize) -> i32 {
    if sign & KMASK_IQ2XS[j] != 0 { -1 } else { 1 }
}

/// Suma entera del superbloque IQ3_XXS por Q8_K (sin el `d` ni el 0,25 finales) y su `d` de pesos.
fn bsum_iq3_xxs(w: &[u8], q8: &[u8]) -> i32 {
    let qs = &w[2..2 + 96];
    let gas = &qs[64..];
    let q8 = &q8[4..4 + Q8_K_BLOCK_ELEMS];
    let mut bsum = 0i32;
    for ib32 in 0..8 {
        let aux32 = u32::from_le_bytes([gas[4 * ib32], gas[4 * ib32 + 1], gas[4 * ib32 + 2], gas[4 * ib32 + 3]]);
        let ls = (2 * (aux32 >> 28) + 1) as i32;
        let mut sumi = 0i32;
        for l in 0..4 {
            let g1 = IQ3XXS_GRID[qs[ib32 * 8 + 2 * l] as usize].to_le_bytes();
            let g2 = IQ3XXS_GRID[qs[ib32 * 8 + 2 * l + 1] as usize].to_le_bytes();
            let sg = KSIGNS_IQ2XS[((aux32 >> (7 * l)) & 127) as usize];
            let q = &q8[ib32 * 32 + 8 * l..];
            for j in 0..4 {
                sumi += g1[j] as i32 * (q[j] as i8 as i32) * s(sg, j);
                sumi += g2[j] as i32 * (q[j + 4] as i8 as i32) * s(sg, j + 4);
            }
        }
        bsum += sumi * ls;
    }
    bsum
}

fn bsum_iq2_s(w: &[u8], q8: &[u8]) -> i32 {
    let qs = &w[2..66];
    let qh = &w[66..74];
    let scales = &w[74..82];
    let q8 = &q8[4..4 + Q8_K_BLOCK_ELEMS];
    let mut bsum = 0i32;
    for ib32 in 0..8 {
        let ls = [1 + 2 * (scales[ib32] & 0xf) as i32, 1 + 2 * (scales[ib32] >> 4) as i32];
        let mut sumi = [0i32; 2];
        for l in 0..4 {
            let grid = IQ2S_GRID[qs[ib32 * 4 + l] as usize | (((qh[ib32] as usize) << (8 - 2 * l)) & 0x300)].to_le_bytes();
            let sg = qs[32 + ib32 * 4 + l];
            let q = &q8[ib32 * 32 + 8 * l..];
            for j in 0..8 {
                sumi[l / 2] += q[j] as i8 as i32 * grid[j] as i32 * s(sg, j);
            }
        }
        bsum += ls[0] * sumi[0] + ls[1] * sumi[1];
    }
    bsum
}

fn sumf_iq4_xs(w: &[u8], q8: &[u8]) -> f32 {
    let d = f16_to_f32(u16::from_le_bytes([w[0], w[1]])) * q8_d(q8);
    let h = u16::from_le_bytes([w[2], w[3]]);
    let sl = &w[4..8];
    let qs = &w[8..136];
    let q8 = &q8[4..4 + Q8_K_BLOCK_ELEMS];
    let mut sumf = 0f32;
    for ib in 0..8 {
        let ls = ((sl[ib / 2] >> (4 * (ib % 2))) & 0xf) as i32 | ((((h >> (2 * ib)) & 3) as i32) << 4);
        let (mut a, mut b) = (0i32, 0i32);
        for j in 0..16 {
            a += q8[ib * 32 + j] as i8 as i32 * KVALUES_IQ4NL[(qs[ib * 16 + j] & 0xf) as usize] as i32;
            b += q8[ib * 32 + j + 16] as i8 as i32 * KVALUES_IQ4NL[(qs[ib * 16 + j] >> 4) as usize] as i32;
        }
        sumf += d * (ls - 32) as f32 * (a + b) as f32;
    }
    sumf
}

fn bsum_iq3_s(w: &[u8], q8: &[u8]) -> i32 {
    let qs = &w[2..66];
    let qh = &w[66..74];
    let signs = &w[74..106];
    let scales = &w[106..110];
    let q8 = &q8[4..4 + Q8_K_BLOCK_ELEMS];
    let mut bsum = 0i32;
    for ib in 0..8 {
        let ls = if ib % 2 == 0 { 1 + 2 * (scales[ib / 2] & 0xf) as i32 } else { 1 + 2 * (scales[ib / 2] >> 4) as i32 };
        let mut sumi = 0i32;
        for l in 0..4 {
            let i1 = qs[ib * 8 + 2 * l] as usize | (((qh[ib] as usize) << (8 - 2 * l)) & 256);
            let i2 = qs[ib * 8 + 2 * l + 1] as usize | (((qh[ib] as usize) << (7 - 2 * l)) & 256);
            let (g1, g2) = (IQ3S_GRID[i1].to_le_bytes(), IQ3S_GRID[i2].to_le_bytes());
            let sg = signs[ib * 4 + l];
            let q = &q8[ib * 32 + 8 * l..];
            for j in 0..4 {
                sumi += q[j] as i8 as i32 * g1[j] as i32 * s(sg, j);
                sumi += q[j + 4] as i8 as i32 * g2[j] as i32 * s(sg, j + 4);
            }
        }
        bsum += sumi * ls;
    }
    bsum
}

/// Q5_K × Q8_K: `(Σ escala·dot, Σ mínimo·bsums)` del superbloque.
fn sumas_q5_k(w: &[u8], q8: &[u8]) -> (i32, i32) {
    let scales = &w[4..16];
    let qh = &w[16..48];
    let qs = &w[48..176];
    let y = &q8[4..4 + Q8_K_BLOCK_ELEMS];
    let (mut bs, mut bm) = (0i32, 0i32);
    for j in 0..8 {
        let (sc, mn) = escala_y_minimo_k4(j, scales);
        let (k, alto) = (j / 2, j % 2 == 1);
        let mut dot = 0i32;
        for l in 0..32 {
            let q = qs[32 * k + l];
            let n = if alto { q >> 4 } else { q & 0xf };
            let bit = qh[l] & (1u8 << j) != 0;
            dot += (n as i32 + if bit { 16 } else { 0 }) * y[32 * j + l] as i8 as i32;
        }
        bs += sc as i32 * dot;
        let off = 4 + Q8_K_BLOCK_ELEMS + 4 * j;
        let b = |i: usize| i16::from_le_bytes([q8[off + i], q8[off + i + 1]]) as i32;
        bm += mn as i32 * (b(0) + b(2));
    }
    (bs, bm)
}

fn dot_q5_k(w: &[u8], q8: &[u8]) -> f32 {
    let d8 = q8_d(q8);
    let d = f16_to_f32(u16::from_le_bytes([w[0], w[1]])) * d8;
    let dmin = -(f16_to_f32(u16::from_le_bytes([w[2], w[3]])) * d8);
    let (bs, bm) = sumas_q5_k(w, q8);
    d * bs as f32 + dmin * bm as f32
}

/// Producto de un superbloque (referencia escalar). Devuelve la contribución **sin** el factor de fila.
pub fn dot_bloque_escalar(ggml: u32, w: &[u8], q8: &[u8]) -> f32 {
    match ggml {
        GGML_IQ3_XXS => f16_to_f32(u16::from_le_bytes([w[0], w[1]])) * q8_d(q8) * bsum_iq3_xxs(w, q8) as f32,
        GGML_IQ2_S => f16_to_f32(u16::from_le_bytes([w[0], w[1]])) * q8_d(q8) * bsum_iq2_s(w, q8) as f32,
        GGML_IQ3_S => f16_to_f32(u16::from_le_bytes([w[0], w[1]])) * q8_d(q8) * bsum_iq3_s(w, q8) as f32,
        GGML_Q5_K => dot_q5_k(w, q8),
        GGML_IQ4_XS => sumf_iq4_xs(w, q8),
        _ => 0.0,
    }
}

const AVX2: bool = cfg!(all(target_arch = "x86_64", target_feature = "avx2", target_feature = "fma"));

/// Producto de un superbloque: AVX2 si el binario lo trae, si no la referencia escalar.
pub fn dot_bloque(ggml: u32, w: &[u8], q8: &[u8]) -> f32 {
    if AVX2 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            return avx2::dot_bloque(ggml, w, q8);
        }
    }
    dot_bloque_escalar(ggml, w, q8)
}

/// `out[j*stride + r] = Σ_b dot(fila r, q8k[j])` por fila, ya con el factor de fila. `q8k` son `n` filas Q8_K seguidas.
pub fn matvec_q8k(ggml: u32, bytes: &[u8], rows: usize, cols: usize, q8k: &[u8], n: usize, outs: &mut [f32], stride: usize) -> Result<(), ()> {
    let bb = sosomodel::iq::bytes_de_bloque(ggml).ok_or(())?;
    if !soporta(ggml) || cols % Q8_K_BLOCK_ELEMS != 0 {
        return Err(());
    }
    let blocks = cols / Q8_K_BLOCK_ELEMS;
    let (row_bytes, q8_row) = (blocks * bb, blocks * Q8_K_BLOCK_BYTES);
    if bytes.len() != rows * row_bytes || q8k.len() != n * q8_row || (n > 0 && (n - 1) * stride + rows > outs.len()) {
        return Err(());
    }
    if AVX2 && n > 1 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            avx2::matvec_tokens(ggml, bytes, rows, cols, q8k, n, outs, stride);
            return Ok(());
        }
    }
    let f = factor_fila(ggml);
    for r in 0..rows {
        let row = &bytes[r * row_bytes..(r + 1) * row_bytes];
        for j in 0..n {
            let q = &q8k[j * q8_row..(j + 1) * q8_row];
            let mut acc = 0f32;
            for b in 0..blocks {
                acc += dot_bloque(ggml, &row[b * bb..(b + 1) * bb], &q[b * Q8_K_BLOCK_BYTES..(b + 1) * Q8_K_BLOCK_BYTES]);
            }
            outs[j * stride + r] = f * acc;
        }
    }
    Ok(())
}

// IQ3_XXS / IQ2_S: tabla "byte de signos → ±1 por byte" para `_mm256_sign_epi8`.
const fn signos_pm() -> [u64; 256] {
    let mut t = [0u64; 256];
    let mut m = 0;
    while m < 256 {
        let mut v = 0u64;
        let mut j = 0;
        while j < 8 {
            let b: u64 = if (m >> j) & 1 != 0 { 0xFF } else { 0x01 };
            v |= b << (8 * j);
            j += 1;
        }
        t[m] = v;
        m += 1;
    }
    t
}
#[cfg(target_arch = "x86_64")]
static SIGNOS_PM: [u64; 256] = signos_pm();

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::*;
    use core::arch::x86_64::*;

    #[inline]
    #[target_feature(enable = "avx2,fma")]
    unsafe fn suma_i32(v: __m256i) -> i32 {
        let s = _mm_add_epi32(_mm256_castsi256_si128(v), _mm256_extracti128_si256(v, 1));
        let s = _mm_add_epi32(s, _mm_shuffle_epi32(s, 0x4E));
        let s = _mm_add_epi32(s, _mm_shuffle_epi32(s, 0xB1));
        _mm_cvtsi128_si32(s)
    }

    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn dot_bloque(ggml: u32, w: &[u8], q8: &[u8]) -> f32 {
        assert!(q8.len() >= Q8_K_BLOCK_BYTES);
        unsafe {
            match ggml {
                GGML_IQ3_XXS => {
                    assert!(w.len() >= IQ3_XXS_BYTES);
                    f16_to_f32(u16::from_le_bytes([w[0], w[1]])) * q8_d(q8) * bsum_iq3_xxs(w, q8) as f32
                }
                GGML_IQ2_S => {
                    assert!(w.len() >= IQ2_S_BYTES);
                    f16_to_f32(u16::from_le_bytes([w[0], w[1]])) * q8_d(q8) * bsum_iq2_s(w, q8) as f32
                }
                GGML_IQ4_XS => {
                    assert!(w.len() >= IQ4_XS_BYTES);
                    sumf_iq4_xs(w, q8)
                }
                GGML_IQ3_S => {
                    assert!(w.len() >= IQ3_S_BYTES);
                    f16_to_f32(u16::from_le_bytes([w[0], w[1]])) * q8_d(q8) * bsum_iq3_s(w, q8) as f32
                }
                GGML_Q5_K => {
                    assert!(w.len() >= Q5_K_BYTES);
                    let d8 = q8_d(q8);
                    let d = f16_to_f32(u16::from_le_bytes([w[0], w[1]])) * d8;
                    let dmin = -(f16_to_f32(u16::from_le_bytes([w[2], w[3]])) * d8);
                    let (bs, bm) = sumas_q5_k(w, q8);
                    d * bs as f32 + dmin * bm as f32
                }
                _ => 0.0,
            }
        }
    }

    /// Descodifica un superbloque a 256 valores i8 y escalas por 16 elementos (la descodificación se paga una vez por
    /// bloque y se reparte entre los `n` tokens). Q5_K: valores 0..31 y mínimos por 32.
    fn descodificar(ggml: u32, w: &[u8], vals: &mut [i8; 256], sc: &mut [i32; 16]) {
        match ggml {
            GGML_IQ3_XXS => {
                let qs = &w[2..98];
                let gas = &qs[64..];
                for ib in 0..8 {
                    let aux = u32::from_le_bytes([gas[4 * ib], gas[4 * ib + 1], gas[4 * ib + 2], gas[4 * ib + 3]]);
                    let ls = (2 * (aux >> 28) + 1) as i32;
                    sc[2 * ib] = ls;
                    sc[2 * ib + 1] = ls;
                    for l in 0..4 {
                        let g1 = IQ3XXS_GRID[qs[ib * 8 + 2 * l] as usize].to_le_bytes();
                        let g2 = IQ3XXS_GRID[qs[ib * 8 + 2 * l + 1] as usize].to_le_bytes();
                        let sg = KSIGNS_IQ2XS[((aux >> (7 * l)) & 127) as usize];
                        for j in 0..4 {
                            vals[ib * 32 + 8 * l + j] = if sg & KMASK_IQ2XS[j] != 0 { -(g1[j] as i8) } else { g1[j] as i8 };
                            vals[ib * 32 + 8 * l + j + 4] = if sg & KMASK_IQ2XS[j + 4] != 0 { -(g2[j] as i8) } else { g2[j] as i8 };
                        }
                    }
                }
            }
            GGML_IQ2_S => {
                let (qs, qh, scales) = (&w[2..66], &w[66..74], &w[74..82]);
                for ib in 0..8 {
                    sc[2 * ib] = 1 + 2 * (scales[ib] & 0xf) as i32;
                    sc[2 * ib + 1] = 1 + 2 * (scales[ib] >> 4) as i32;
                    for l in 0..4 {
                        let g = IQ2S_GRID[qs[ib * 4 + l] as usize | (((qh[ib] as usize) << (8 - 2 * l)) & 0x300)].to_le_bytes();
                        let sg = qs[32 + ib * 4 + l];
                        for j in 0..8 {
                            vals[ib * 32 + 8 * l + j] = if sg & KMASK_IQ2XS[j] != 0 { -(g[j] as i8) } else { g[j] as i8 };
                        }
                    }
                }
            }
            GGML_IQ3_S => {
                let (qs, qh, signs, scales) = (&w[2..66], &w[66..74], &w[74..106], &w[106..110]);
                for ib in 0..8 {
                    let ls = if ib % 2 == 0 { 1 + 2 * (scales[ib / 2] & 0xf) as i32 } else { 1 + 2 * (scales[ib / 2] >> 4) as i32 };
                    sc[2 * ib] = ls;
                    sc[2 * ib + 1] = ls;
                    for l in 0..4 {
                        let i1 = qs[ib * 8 + 2 * l] as usize | (((qh[ib] as usize) << (8 - 2 * l)) & 256);
                        let i2 = qs[ib * 8 + 2 * l + 1] as usize | (((qh[ib] as usize) << (7 - 2 * l)) & 256);
                        let (g1, g2) = (IQ3S_GRID[i1].to_le_bytes(), IQ3S_GRID[i2].to_le_bytes());
                        let sg = signs[ib * 4 + l];
                        for j in 0..4 {
                            vals[ib * 32 + 8 * l + j] = if sg & KMASK_IQ2XS[j] != 0 { -(g1[j] as i8) } else { g1[j] as i8 };
                            vals[ib * 32 + 8 * l + j + 4] = if sg & KMASK_IQ2XS[j + 4] != 0 { -(g2[j] as i8) } else { g2[j] as i8 };
                        }
                    }
                }
            }
            GGML_IQ4_XS => {
                let h = u16::from_le_bytes([w[2], w[3]]);
                let (sl, qs) = (&w[4..8], &w[8..136]);
                for ib in 0..8 {
                    let ls = ((sl[ib / 2] >> (4 * (ib % 2))) & 0xf) as i32 | ((((h >> (2 * ib)) & 3) as i32) << 4);
                    sc[2 * ib] = ls - 32;
                    sc[2 * ib + 1] = ls - 32;
                    for j in 0..16 {
                        vals[ib * 32 + j] = KVALUES_IQ4NL[(qs[ib * 16 + j] & 0xf) as usize];
                        vals[ib * 32 + j + 16] = KVALUES_IQ4NL[(qs[ib * 16 + j] >> 4) as usize];
                    }
                }
            }
            GGML_Q5_K => {
                let (scales, qh, qs) = (&w[4..16], &w[16..48], &w[48..176]);
                for j in 0..8 {
                    let (s0, _) = escala_y_minimo_k4(j, scales);
                    sc[2 * j] = s0 as i32;
                    sc[2 * j + 1] = s0 as i32;
                    for l in 0..32 {
                        let q = qs[32 * (j / 2) + l];
                        let n = if j % 2 == 1 { q >> 4 } else { q & 0xf };
                        vals[j * 32 + l] = (n + if qh[l] & (1u8 << j) != 0 { 16 } else { 0 }) as i8;
                    }
                }
            }
            _ => {}
        }
    }

    /// Como `matvec_q8k` escalar pero con la descodificación de cada bloque compartida por los `n` tokens. El resultado de cada
    /// token es el mismo f32 que con un solo vector (mismos enteros, mismas operaciones en coma flotante y en el mismo orden).
    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn matvec_tokens(ggml: u32, bytes: &[u8], rows: usize, cols: usize, q8k: &[u8], n: usize, outs: &mut [f32], stride: usize) {
        let bb = sosomodel::iq::bytes_de_bloque(ggml).unwrap_or(1);
        let blocks = cols / Q8_K_BLOCK_ELEMS;
        let (row_bytes, q8_row) = (blocks * bb, blocks * Q8_K_BLOCK_BYTES);
        let f = factor_fila(ggml);
        let mut vals = [0i8; 256];
        let mut sc = [0i32; 16];
        let mut acc = alloc::vec![0f32; n];
        for r in 0..rows {
            let row = &bytes[r * row_bytes..(r + 1) * row_bytes];
            acc.iter_mut().for_each(|a| *a = 0.0);
            for b in 0..blocks {
                let w = &row[b * bb..(b + 1) * bb];
                descodificar(ggml, w, &mut vals, &mut sc);
                let d = f16_to_f32(u16::from_le_bytes([w[0], w[1]]));
                let dmin = if ggml == GGML_Q5_K { f16_to_f32(u16::from_le_bytes([w[2], w[3]])) } else { 0.0 };
                let mut scv = [_mm256_setzero_si256(); 8];
                for ib in 0..8 {
                    scv[ib] = _mm256_set_m128i(_mm_set1_epi16(sc[2 * ib + 1] as i16), _mm_set1_epi16(sc[2 * ib] as i16));
                }
                let mins: [i32; 8] = if ggml == GGML_Q5_K {
                    let mut m = [0i32; 8];
                    for j in 0..8 {
                        m[j] = escala_y_minimo_k4(j, &w[4..16]).1 as i32;
                    }
                    m
                } else {
                    [0; 8]
                };
                let wp = vals.as_ptr() as *const __m256i;
                for j in 0..n {
                    let q = &q8k[j * q8_row + b * Q8_K_BLOCK_BYTES..j * q8_row + (b + 1) * Q8_K_BLOCK_BYTES];
                    let d8 = q8_d(q);
                    let yp = q.as_ptr().add(4) as *const __m256i;
                    if ggml == GGML_IQ4_XS {
                        let dd = d * d8;
                        let mut sumf = 0f32;
                        for ib in 0..8 {
                            let wv = _mm256_loadu_si256(wp.add(ib));
                            let p = _mm256_maddubs_epi16(_mm256_sign_epi8(wv, wv), _mm256_sign_epi8(_mm256_loadu_si256(yp.add(ib)), wv));
                            let t = suma_i32(_mm256_madd_epi16(p, _mm256_set1_epi16(1)));
                            sumf += dd * sc[2 * ib] as f32 * t as f32;
                        }
                        acc[j] += sumf;
                    } else {
                        let mut a8 = _mm256_setzero_si256();
                        for ib in 0..8 {
                            let wv = _mm256_loadu_si256(wp.add(ib));
                            let p = _mm256_maddubs_epi16(_mm256_sign_epi8(wv, wv), _mm256_sign_epi8(_mm256_loadu_si256(yp.add(ib)), wv));
                            a8 = _mm256_add_epi32(a8, _mm256_madd_epi16(p, scv[ib]));
                        }
                        let bs = suma_i32(a8);
                        if ggml == GGML_Q5_K {
                            let mut bm = 0i32;
                            for jj in 0..8 {
                                let off = 4 + Q8_K_BLOCK_ELEMS + 4 * jj;
                                let bsum = |i: usize| i16::from_le_bytes([q[off + i], q[off + i + 1]]) as i32;
                                bm += mins[jj] * (bsum(0) + bsum(2));
                            }
                            acc[j] += d * d8 * bs as f32 + (-(dmin * d8)) * bm as f32;
                        } else {
                            acc[j] += d * d8 * bs as f32;
                        }
                    }
                }
            }
            for j in 0..n {
                outs[j * stride + r] = f * acc[j];
            }
        }
    }

    #[inline]
    #[target_feature(enable = "avx2,fma")]
    unsafe fn bsum_iq3_xxs(w: &[u8], q8: &[u8]) -> i32 {
        unsafe {
            let qs = w.as_ptr().add(2);
            let gas = qs.add(64);
            let q = q8.as_ptr().add(4) as *const __m256i;
            let mut acc = _mm256_setzero_si256();
            for ib32 in 0..8 {
                let aux32 = (gas.add(4 * ib32) as *const u32).read_unaligned();
                let ls = (2 * (aux32 >> 28) + 1) as i32;
                let i = qs.add(ib32 * 8);
                let g = _mm256_set_epi32(
                    IQ3XXS_GRID[*i.add(7) as usize] as i32,
                    IQ3XXS_GRID[*i.add(6) as usize] as i32,
                    IQ3XXS_GRID[*i.add(5) as usize] as i32,
                    IQ3XXS_GRID[*i.add(4) as usize] as i32,
                    IQ3XXS_GRID[*i.add(3) as usize] as i32,
                    IQ3XXS_GRID[*i.add(2) as usize] as i32,
                    IQ3XXS_GRID[*i.add(1) as usize] as i32,
                    IQ3XXS_GRID[*i as usize] as i32,
                );
                let sg = _mm256_set_epi64x(
                    SIGNOS_PM[KSIGNS_IQ2XS[((aux32 >> 21) & 127) as usize] as usize] as i64,
                    SIGNOS_PM[KSIGNS_IQ2XS[((aux32 >> 14) & 127) as usize] as usize] as i64,
                    SIGNOS_PM[KSIGNS_IQ2XS[((aux32 >> 7) & 127) as usize] as usize] as i64,
                    SIGNOS_PM[KSIGNS_IQ2XS[(aux32 & 127) as usize] as usize] as i64,
                );
                let y = _mm256_sign_epi8(_mm256_loadu_si256(q.add(ib32)), sg);
                let p = _mm256_maddubs_epi16(g, y);
                acc = _mm256_add_epi32(acc, _mm256_madd_epi16(p, _mm256_set1_epi16(ls as i16)));
            }
            suma_i32(acc)
        }
    }

    #[inline]
    #[target_feature(enable = "avx2,fma")]
    unsafe fn bsum_iq2_s(w: &[u8], q8: &[u8]) -> i32 {
        unsafe {
            let qs = w.as_ptr().add(2);
            let qh = w.as_ptr().add(66);
            let sc = w.as_ptr().add(74);
            let q = q8.as_ptr().add(4) as *const __m256i;
            let mut acc = _mm256_setzero_si256();
            for ib32 in 0..8 {
                let h = *qh.add(ib32) as usize;
                let i = qs.add(ib32 * 4);
                let gr = |l: usize| IQ2S_GRID[*i.add(l) as usize | ((h << (8 - 2 * l)) & 0x300)] as i64;
                let g = _mm256_set_epi64x(gr(3), gr(2), gr(1), gr(0));
                let sgp = qs.add(32 + ib32 * 4);
                let sg = _mm256_set_epi64x(
                    SIGNOS_PM[*sgp.add(3) as usize] as i64,
                    SIGNOS_PM[*sgp.add(2) as usize] as i64,
                    SIGNOS_PM[*sgp.add(1) as usize] as i64,
                    SIGNOS_PM[*sgp as usize] as i64,
                );
                let y = _mm256_sign_epi8(_mm256_loadu_si256(q.add(ib32)), sg);
                let p = _mm256_maddubs_epi16(g, y);
                let (l1, l2) = (1 + 2 * (*sc.add(ib32) & 0xf) as i16, 1 + 2 * (*sc.add(ib32) >> 4) as i16);
                let lsv = _mm256_set_m128i(_mm_set1_epi16(l2), _mm_set1_epi16(l1));
                acc = _mm256_add_epi32(acc, _mm256_madd_epi16(p, lsv));
            }
            suma_i32(acc)
        }
    }

    #[inline]
    #[target_feature(enable = "avx2,fma")]
    unsafe fn bsum_iq3_s(w: &[u8], q8: &[u8]) -> i32 {
        unsafe {
            let qs = w.as_ptr().add(2);
            let qh = w.as_ptr().add(66);
            let sg = w.as_ptr().add(74);
            let sc = w.as_ptr().add(106);
            let q = q8.as_ptr().add(4) as *const __m256i;
            let mut acc = _mm256_setzero_si256();
            for ib in 0..8 {
                let h = *qh.add(ib) as usize;
                let i = qs.add(ib * 8);
                let ix = |l: usize| {
                    (*i.add(2 * l) as usize | ((h << (8 - 2 * l)) & 256), *i.add(2 * l + 1) as usize | ((h << (7 - 2 * l)) & 256))
                };
                let (a0, a1) = ix(0);
                let (b0, b1) = ix(1);
                let (c0, c1) = ix(2);
                let (d0, d1) = ix(3);
                let g = _mm256_set_epi32(
                    IQ3S_GRID[d1] as i32, IQ3S_GRID[d0] as i32, IQ3S_GRID[c1] as i32, IQ3S_GRID[c0] as i32,
                    IQ3S_GRID[b1] as i32, IQ3S_GRID[b0] as i32, IQ3S_GRID[a1] as i32, IQ3S_GRID[a0] as i32,
                );
                let sp = sg.add(ib * 4);
                let sgn = _mm256_set_epi64x(
                    SIGNOS_PM[*sp.add(3) as usize] as i64,
                    SIGNOS_PM[*sp.add(2) as usize] as i64,
                    SIGNOS_PM[*sp.add(1) as usize] as i64,
                    SIGNOS_PM[*sp as usize] as i64,
                );
                let y = _mm256_sign_epi8(_mm256_loadu_si256(q.add(ib)), sgn);
                let p = _mm256_maddubs_epi16(g, y);
                let b = *sc.add(ib / 2);
                let ls = if ib % 2 == 0 { 1 + 2 * (b & 0xf) as i16 } else { 1 + 2 * (b >> 4) as i16 };
                acc = _mm256_add_epi32(acc, _mm256_madd_epi16(p, _mm256_set1_epi16(ls)));
            }
            suma_i32(acc)
        }
    }

    #[inline]
    #[target_feature(enable = "avx2,fma")]
    unsafe fn sumas_q5_k(w: &[u8], q8: &[u8]) -> (i32, i32) {
        unsafe {
            let scales = &w[4..16];
            let hbits = _mm256_loadu_si256(w.as_ptr().add(16) as *const __m256i);
            let qs = w.as_ptr().add(48);
            let q = q8.as_ptr().add(4) as *const __m256i;
            let m4 = _mm256_set1_epi8(0x0f);
            let m16 = _mm256_set1_epi8(0x10);
            let mut acc = _mm256_setzero_si256();
            let mut bm = 0i32;
            macro_rules! bit16 {
                (izq $n:literal) => { _mm256_and_si256(_mm256_slli_epi16(hbits, $n), m16) };
                (der $n:literal) => { _mm256_and_si256(_mm256_srli_epi16(hbits, $n), m16) };
            }
            for k in 0..4 {
                let bytes = _mm256_loadu_si256(qs.add(32 * k) as *const __m256i);
                let (t_lo, t_hi) = match k {
                    0 => (bit16!(izq 4), bit16!(izq 3)),
                    1 => (bit16!(izq 2), bit16!(izq 1)),
                    2 => (bit16!(der 0), bit16!(der 1)),
                    _ => (bit16!(der 2), bit16!(der 3)),
                };
                let lo = _mm256_or_si256(_mm256_and_si256(bytes, m4), t_lo);
                let hi = _mm256_or_si256(_mm256_and_si256(_mm256_srli_epi16(bytes, 4), m4), t_hi);
                let (s0, m0) = escala_y_minimo_k4(2 * k, scales);
                let (s1, m1) = escala_y_minimo_k4(2 * k + 1, scales);
                let p0 = _mm256_maddubs_epi16(lo, _mm256_loadu_si256(q.add(2 * k)));
                let p1 = _mm256_maddubs_epi16(hi, _mm256_loadu_si256(q.add(2 * k + 1)));
                acc = _mm256_add_epi32(acc, _mm256_madd_epi16(p0, _mm256_set1_epi16(s0 as i16)));
                acc = _mm256_add_epi32(acc, _mm256_madd_epi16(p1, _mm256_set1_epi16(s1 as i16)));
                for (j, mn) in [(2 * k, m0), (2 * k + 1, m1)] {
                    let off = 4 + Q8_K_BLOCK_ELEMS + 4 * j;
                    let b = |i: usize| i16::from_le_bytes([q8[off + i], q8[off + i + 1]]) as i32;
                    bm += mn as i32 * (b(0) + b(2));
                }
            }
            (suma_i32(acc), bm)
        }
    }

    #[inline]
    #[target_feature(enable = "avx2,fma")]
    unsafe fn sumf_iq4_xs(w: &[u8], q8: &[u8]) -> f32 {
        unsafe {
            let d = f16_to_f32(u16::from_le_bytes([w[0], w[1]])) * q8_d(q8);
            let h = u16::from_le_bytes([w[2], w[3]]);
            let sl = w.as_ptr().add(4);
            let qs = w.as_ptr().add(8);
            let q = q8.as_ptr().add(4) as *const __m256i;
            let tabla = _mm_loadu_si128(KVALUES_IQ4NL.as_ptr() as *const __m128i);
            let m4 = _mm_set1_epi8(0x0f);
            let mut sumf = 0f32;
            for ib in 0..8 {
                let ls = ((*sl.add(ib / 2) >> (4 * (ib % 2))) & 0xf) as i32 | ((((h >> (2 * ib)) & 3) as i32) << 4);
                let bits = _mm_loadu_si128(qs.add(ib * 16) as *const __m128i);
                let lo = _mm_and_si128(bits, m4);
                let hi = _mm_and_si128(_mm_srli_epi16(bits, 4), m4);
                let nib = _mm256_set_m128i(hi, lo);
                let w8 = _mm256_shuffle_epi8(_mm256_set_m128i(tabla, tabla), nib);
                let y = _mm256_loadu_si256(q.add(ib));
                let p = _mm256_maddubs_epi16(_mm256_sign_epi8(w8, w8), _mm256_sign_epi8(y, w8));
                let t = suma_i32(_mm256_madd_epi16(p, _mm256_set1_epi16(1)));
                sumf += d * (ls - 32) as f32 * t as f32;
            }
            sumf
        }
    }
}
