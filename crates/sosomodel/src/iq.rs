//! Dequantización de los formatos GGML que faltaban para correr los GGUF de Unsloth tal cual (IQ2_XS, IQ2_S, IQ3_XXS, IQ3_S, IQ4_XS) y los K-quants
//! Q3_K y Q5_K. Portada línea a línea de `ggml-quants.c` (llama.cpp, MIT) conservando el **orden de las operaciones en coma flotante**, para que el
//! resultado coincida bit a bit con libggml (ver `tests/iq_oraculo.rs` y `tools/iq-oraculo`).
//!
//! Cada función descuantiza **un** super-bloque de 256 elementos: `bloque` tiene exactamente `*_BYTES` bytes y `y` 256 posiciones.

use crate::dequant::f16_to_f32;
use crate::iq_tablas::{IQ2S_GRID, IQ2XS_GRID, IQ3S_GRID, IQ3XXS_GRID, KMASK_IQ2XS, KSIGNS_IQ2XS, KVALUES_IQ4NL};

/// Elementos por super-bloque.
pub const QK_K: usize = 256;

pub const Q3_K_BYTES: usize = 110;
pub const Q5_K_BYTES: usize = 176;
pub const IQ2_XS_BYTES: usize = 74;
pub const IQ2_S_BYTES: usize = 82;
pub const IQ3_XXS_BYTES: usize = 98;
pub const IQ3_S_BYTES: usize = 110;
pub const IQ4_XS_BYTES: usize = 136;

/// Identificadores de tipo de GGML (los que lleva cada tensor en un GGUF).
pub const GGML_Q3_K: u32 = 11;
pub const GGML_Q5_K: u32 = 13;
pub const GGML_IQ2_XS: u32 = 17;
pub const GGML_IQ3_XXS: u32 = 18;
pub const GGML_IQ3_S: u32 = 21;
pub const GGML_IQ2_S: u32 = 22;
pub const GGML_IQ4_XS: u32 = 23;

/// Bytes de un super-bloque de ese tipo, si es uno de los de este módulo.
pub fn bytes_de_bloque(ggml: u32) -> Option<usize> {
    Some(match ggml {
        GGML_Q3_K => Q3_K_BYTES,
        GGML_Q5_K => Q5_K_BYTES,
        GGML_IQ2_XS => IQ2_XS_BYTES,
        GGML_IQ2_S => IQ2_S_BYTES,
        GGML_IQ3_XXS => IQ3_XXS_BYTES,
        GGML_IQ3_S => IQ3_S_BYTES,
        GGML_IQ4_XS => IQ4_XS_BYTES,
        _ => return None,
    })
}

/// Descuantiza un super-bloque de `ggml`. `false` si el tipo no es de este módulo.
pub fn dequant_bloque(ggml: u32, bloque: &[u8], y: &mut [f32]) -> bool {
    match ggml {
        GGML_Q3_K => dequant_q3_k(bloque, y),
        GGML_Q5_K => dequant_q5_k(bloque, y),
        GGML_IQ2_XS => dequant_iq2_xs(bloque, y),
        GGML_IQ2_S => dequant_iq2_s(bloque, y),
        GGML_IQ3_XXS => dequant_iq3_xxs(bloque, y),
        GGML_IQ3_S => dequant_iq3_s(bloque, y),
        GGML_IQ4_XS => dequant_iq4_xs(bloque, y),
        _ => return false,
    }
    true
}

fn semi(b: &[u8], i: usize) -> f32 {
    f16_to_f32(u16::from_le_bytes([b[i], b[i + 1]]))
}

#[inline]
fn signo(signs: u8, j: usize) -> f32 {
    if signs & KMASK_IQ2XS[j] != 0 { -1.0 } else { 1.0 }
}

/// `block_iq4_xs`: d(2) + scales_h(2) + scales_l(4) + qs(128).
pub fn dequant_iq4_xs(b: &[u8], y: &mut [f32]) {
    debug_assert!(b.len() >= IQ4_XS_BYTES && y.len() >= QK_K);
    let d = semi(b, 0);
    let scales_h = u16::from_le_bytes([b[2], b[3]]);
    let scales_l = &b[4..8];
    let qs = &b[8..8 + 128];
    for ib in 0..QK_K / 32 {
        let ls = (((scales_l[ib / 2] >> (4 * (ib % 2))) & 0xf) as i32) | ((((scales_h >> (2 * ib)) & 3) as i32) << 4);
        let dl = d * (ls - 32) as f32;
        for j in 0..16 {
            y[ib * 32 + j] = dl * KVALUES_IQ4NL[(qs[ib * 16 + j] & 0xf) as usize] as f32;
            y[ib * 32 + j + 16] = dl * KVALUES_IQ4NL[(qs[ib * 16 + j] >> 4) as usize] as f32;
        }
    }
}

/// `block_iq3_xxs`: d(2) + qs(96) (64 índices de rejilla y 32 bytes de escalas y signos).
pub fn dequant_iq3_xxs(b: &[u8], y: &mut [f32]) {
    debug_assert!(b.len() >= IQ3_XXS_BYTES && y.len() >= QK_K);
    let d = semi(b, 0);
    let qs = &b[2..2 + 96];
    let scales_and_signs = &qs[QK_K / 4..];
    let mut o = 0;
    for ib32 in 0..QK_K / 32 {
        let aux32 = u32::from_le_bytes([scales_and_signs[4 * ib32], scales_and_signs[4 * ib32 + 1], scales_and_signs[4 * ib32 + 2], scales_and_signs[4 * ib32 + 3]]);
        let db = d * (0.5f32 + (aux32 >> 28) as f32) * 0.5f32;
        for l in 0..4 {
            let signs = KSIGNS_IQ2XS[((aux32 >> (7 * l)) & 127) as usize];
            let grid1 = IQ3XXS_GRID[qs[ib32 * 8 + 2 * l] as usize].to_le_bytes();
            let grid2 = IQ3XXS_GRID[qs[ib32 * 8 + 2 * l + 1] as usize].to_le_bytes();
            for j in 0..4 {
                y[o + j] = db * grid1[j] as f32 * signo(signs, j);
                y[o + j + 4] = db * grid2[j] as f32 * signo(signs, j + 4);
            }
            o += 8;
        }
    }
}

/// `block_iq3_s`: d(2) + qs(64) + qh(8) + signs(32) + scales(4).
pub fn dequant_iq3_s(b: &[u8], y: &mut [f32]) {
    debug_assert!(b.len() >= IQ3_S_BYTES && y.len() >= QK_K);
    let d = semi(b, 0);
    let qs = &b[2..66];
    let qh = &b[66..74];
    let signs = &b[74..106];
    let scales = &b[106..110];
    let (mut qo, mut ho, mut so, mut o) = (0usize, 0usize, 0usize, 0usize);
    for ib32 in (0..QK_K / 32).step_by(2) {
        let db1 = d * (1 + 2 * (scales[ib32 / 2] & 0xf) as i32) as f32;
        let db2 = d * (1 + 2 * (scales[ib32 / 2] >> 4) as i32) as f32;
        for l in 0..4 {
            let i1 = qs[qo + 2 * l] as usize | (((qh[ho] as usize) << (8 - 2 * l)) & 256);
            let i2 = qs[qo + 2 * l + 1] as usize | (((qh[ho] as usize) << (7 - 2 * l)) & 256);
            let (g1, g2) = (IQ3S_GRID[i1].to_le_bytes(), IQ3S_GRID[i2].to_le_bytes());
            for j in 0..4 {
                y[o + j] = db1 * g1[j] as f32 * signo(signs[so + l], j);
                y[o + j + 4] = db1 * g2[j] as f32 * signo(signs[so + l], j + 4);
            }
            o += 8;
        }
        qo += 8;
        so += 4;
        for l in 0..4 {
            let i1 = qs[qo + 2 * l] as usize | (((qh[ho + 1] as usize) << (8 - 2 * l)) & 256);
            let i2 = qs[qo + 2 * l + 1] as usize | (((qh[ho + 1] as usize) << (7 - 2 * l)) & 256);
            let (g1, g2) = (IQ3S_GRID[i1].to_le_bytes(), IQ3S_GRID[i2].to_le_bytes());
            for j in 0..4 {
                y[o + j] = db2 * g1[j] as f32 * signo(signs[so + l], j);
                y[o + j + 4] = db2 * g2[j] as f32 * signo(signs[so + l], j + 4);
            }
            o += 8;
        }
        ho += 2;
        qo += 8;
        so += 4;
    }
}

/// `block_iq2_xs`: d(2) + qs(32 × u16) + scales(8).
pub fn dequant_iq2_xs(b: &[u8], y: &mut [f32]) {
    debug_assert!(b.len() >= IQ2_XS_BYTES && y.len() >= QK_K);
    let d = semi(b, 0);
    let scales = &b[66..74];
    let mut o = 0;
    for ib32 in 0..QK_K / 32 {
        let db = [d * (0.5f32 + (scales[ib32] & 0xf) as f32) * 0.25f32, d * (0.5f32 + (scales[ib32] >> 4) as f32) * 0.25f32];
        for l in 0..4 {
            let i = 2 + 2 * (4 * ib32 + l);
            let q = u16::from_le_bytes([b[i], b[i + 1]]);
            let grid = IQ2XS_GRID[(q & 511) as usize].to_le_bytes();
            let signs = KSIGNS_IQ2XS[(q >> 9) as usize];
            for j in 0..8 {
                y[o + j] = db[l / 2] * grid[j] as f32 * signo(signs, j);
            }
            o += 8;
        }
    }
}

/// `block_iq2_s`: d(2) + qs(64: 32 de rejilla y 32 de signos) + qh(8) + scales(8).
pub fn dequant_iq2_s(b: &[u8], y: &mut [f32]) {
    debug_assert!(b.len() >= IQ2_S_BYTES && y.len() >= QK_K);
    let d = semi(b, 0);
    let qs = &b[2..66];
    let qh = &b[66..74];
    let scales = &b[74..82];
    let (mut qo, mut so, mut o) = (0usize, QK_K / 8, 0usize);
    for ib32 in 0..QK_K / 32 {
        let db = [d * (0.5f32 + (scales[ib32] & 0xf) as f32) * 0.25f32, d * (0.5f32 + (scales[ib32] >> 4) as f32) * 0.25f32];
        for l in 0..4 {
            let dl = db[l / 2];
            let grid = IQ2S_GRID[qs[qo + l] as usize | (((qh[ib32] as usize) << (8 - 2 * l)) & 0x300)].to_le_bytes();
            for j in 0..8 {
                y[o + j] = dl * grid[j] as f32 * signo(qs[so + l], j);
            }
            o += 8;
        }
        qo += 4;
        so += 4;
    }
}

/// `block_q3_K`: hmask(32) + qs(64) + scales(12) + d(2).
pub fn dequant_q3_k(b: &[u8], y: &mut [f32]) {
    debug_assert!(b.len() >= Q3_K_BYTES && y.len() >= QK_K);
    const KMASK1: u32 = 0x0303_0303;
    const KMASK2: u32 = 0x0f0f_0f0f;
    let d_all = semi(b, 108);
    let hm = &b[0..32];
    let q = &b[32..96];
    let mut aux = [0u32; 4];
    for i in 0..3 {
        aux[i] = u32::from_le_bytes([b[96 + 4 * i], b[97 + 4 * i], b[98 + 4 * i], b[99 + 4 * i]]);
    }
    let tmp = aux[2];
    aux[2] = ((aux[0] >> 4) & KMASK2) | (((tmp >> 4) & KMASK1) << 4);
    aux[3] = ((aux[1] >> 4) & KMASK2) | (((tmp >> 6) & KMASK1) << 4);
    aux[0] = (aux[0] & KMASK2) | ((tmp & KMASK1) << 4);
    aux[1] = (aux[1] & KMASK2) | (((tmp >> 2) & KMASK1) << 4);
    let mut scales = [0i8; 16];
    for i in 0..4 {
        for (k, v) in aux[i].to_le_bytes().iter().enumerate() {
            scales[4 * i + k] = *v as i8;
        }
    }
    let (mut m, mut is, mut o, mut qo) = (1u8, 0usize, 0usize, 0usize);
    for _n in (0..QK_K).step_by(128) {
        let mut shift = 0;
        for _j in 0..4 {
            let dl = d_all * (scales[is] as i32 - 32) as f32;
            is += 1;
            for l in 0..16 {
                y[o] = dl * (((q[qo + l] >> shift) & 3) as i8 as i32 - if hm[l] & m != 0 { 0 } else { 4 }) as f32;
                o += 1;
            }
            let dl = d_all * (scales[is] as i32 - 32) as f32;
            is += 1;
            for l in 0..16 {
                y[o] = dl * (((q[qo + l + 16] >> shift) & 3) as i8 as i32 - if hm[l + 16] & m != 0 { 0 } else { 4 }) as f32;
                o += 1;
            }
            shift += 2;
            m <<= 1;
        }
        qo += 32;
    }
}

/// `get_scale_min_k4` de ggml: escala y mínimo de 6 bits del sub-bloque `j` (Q4_K y Q5_K).
fn escala_y_minimo_k4(j: usize, q: &[u8]) -> (u8, u8) {
    if j < 4 {
        (q[j] & 63, q[j + 4] & 63)
    } else {
        ((q[j + 4] & 0xF) | ((q[j - 4] >> 6) << 4), (q[j + 4] >> 4) | ((q[j] >> 6) << 4))
    }
}

/// `block_q5_K`: d(2) + dmin(2) + scales(12) + qh(32) + qs(128).
pub fn dequant_q5_k(b: &[u8], y: &mut [f32]) {
    debug_assert!(b.len() >= Q5_K_BYTES && y.len() >= QK_K);
    let d = semi(b, 0);
    let min = semi(b, 2);
    let scales = &b[4..16];
    let qh = &b[16..48];
    let qs = &b[48..176];
    let (mut is, mut u1, mut u2, mut o, mut qo) = (0usize, 1u8, 2u8, 0usize, 0usize);
    for _j in (0..QK_K).step_by(64) {
        let (sc, m) = escala_y_minimo_k4(is, scales);
        let (d1, m1) = (d * sc as f32, min * m as f32);
        let (sc, m) = escala_y_minimo_k4(is + 1, scales);
        let (d2, m2) = (d * sc as f32, min * m as f32);
        for l in 0..32 {
            y[o] = d1 * ((qs[qo + l] & 0xF) as i32 + if qh[l] & u1 != 0 { 16 } else { 0 }) as f32 - m1;
            o += 1;
        }
        for l in 0..32 {
            y[o] = d2 * ((qs[qo + l] >> 4) as i32 + if qh[l] & u2 != 0 { 16 } else { 0 }) as f32 - m2;
            o += 1;
        }
        qo += 32;
        is += 2;
        u1 <<= 2;
        u2 = u2.wrapping_shl(2);
    }
}
