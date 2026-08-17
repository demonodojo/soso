//! Descuantización de los formatos on-disk: Q8_0, Q4_K (layout GGML exacto) y
//! MXFP4.
//!
//! **Por qué vive aquí y no en `soso-llm-core`.** Estos decodificadores hacen
//! falta en tres sitios: la inferencia en CPU (userspace), el despacho a la GPU
//! (userspace) y el **kernel**, que necesita descuantizar para que el dispositivo
//! software (`SOFTG`) pueda ejecutar el comando `MATVQ` y así el camino de pesos
//! cuantizados en la GPU tenga cobertura numérica sin silicio. El kernel no enlaza
//! `soso-llm-core` —arrastraría el runtime de LLM entero e invertiría la
//! dependencia— pero sí ve `sosomodel`, que además es de quien son las constantes
//! de bloque. La alternativa era duplicar ~70 líneas de desempaquetado de nibbles
//! y escalas de 6 bits en el kernel, o sea dos implementaciones que divergen y un
//! síntoma («el dispositivo software da otros tokens») que no señala a nadie.
//!
//! Aquí sólo van los DECODIFICADORES. Los cuantizadores se quedan en
//! `soso-llm-core::quant`: usan `libm::roundf` y `sosomodel` no depende de `libm`.

use crate::layout::{
    MXFP4_BLOCK_BYTES, MXFP4_BLOCK_ELEMS, Q4_K_BLOCK_BYTES, Q4_K_BLOCK_ELEMS, Q8_0_BLOCK_BYTES,
    Q8_0_BLOCK_ELEMS,
};

const POW2_NEG14: f32 = 6.103_515_6e-5; // 2^-14

/// f16 (IEEE 754 binary16) → f32, sin hardware F16C.
pub fn f16_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) & 1) as u32;
    let exp = ((bits >> 10) & 0x1f) as u32;
    let frac = (bits & 0x3ff) as u32;
    if exp == 0 {
        if frac == 0 {
            return f32::from_bits(sign << 31);
        }
        let v = (frac as f32) / 1024.0 * POW2_NEG14;
        return if sign != 0 { -v } else { v };
    }
    if exp == 31 {
        return if frac == 0 {
            f32::from_bits((sign << 31) | 0x7f80_0000)
        } else {
            f32::NAN
        };
    }
    f32::from_bits((sign << 31) | ((exp + 112) << 23) | (frac << 13))
}

/// Bytes por fila de una matriz row-major con `cols` columnas en `dtype`, o
/// `None` si el dtype no es de bloques o `cols` no es múltiplo del bloque.
///
/// Está aquí y no repartido por los llamantes porque es la aritmética que decide
/// qué trozo de VRAM lee un kernel de la GPU: equivocarla no da error, da la fila
/// del tensor de al lado y un vector perfectamente creíble.
pub fn row_bytes(dtype: u8, cols: usize) -> Option<usize> {
    let (elems, bytes) = match dtype {
        crate::layout::DTYPE_Q8_0 => (Q8_0_BLOCK_ELEMS, Q8_0_BLOCK_BYTES),
        crate::layout::DTYPE_Q4_K => (Q4_K_BLOCK_ELEMS, Q4_K_BLOCK_BYTES),
        crate::layout::DTYPE_MXFP4 => (MXFP4_BLOCK_ELEMS, MXFP4_BLOCK_BYTES),
        _ => return None,
    };
    if cols == 0 || cols % elems != 0 {
        return None;
    }
    Some((cols / elems) * bytes)
}

// ---- Q8_0: bloques de 32, escala f32 + 32 i8 ----

/// Descuantiza `out.len()` elementos a partir del elemento `elem_off`.
/// `bytes` es el tensor Q8_0 completo (múltiplo de bloque).
pub fn dequant_q8_0_range(bytes: &[u8], elem_off: usize, out: &mut [f32]) -> Result<(), ()> {
    if bytes.len() % Q8_0_BLOCK_BYTES != 0 || out.is_empty() {
        return Err(());
    }
    let total = (bytes.len() / Q8_0_BLOCK_BYTES) * Q8_0_BLOCK_ELEMS;
    let end = elem_off.checked_add(out.len()).ok_or(())?;
    if end > total {
        return Err(());
    }
    let first = elem_off / Q8_0_BLOCK_ELEMS;
    let last = (end - 1) / Q8_0_BLOCK_ELEMS;
    for b in first..=last {
        let chunk = &bytes[b * Q8_0_BLOCK_BYTES..(b + 1) * Q8_0_BLOCK_BYTES];
        let scale = f32::from_le_bytes(chunk[..4].try_into().map_err(|_| ())?);
        let base = b * Q8_0_BLOCK_ELEMS;
        for (j, &q) in chunk[4..].iter().enumerate() {
            let e = base + j;
            if e >= elem_off && e < end {
                out[e - elem_off] = (q as i8) as f32 * scale;
            }
        }
    }
    Ok(())
}

/// Descuantiza el tensor completo.
pub fn dequant_q8_0(bytes: &[u8], out: &mut [f32]) -> Result<(), ()> {
    dequant_q8_0_range(bytes, 0, out)
}

// ---- Q4_K (layout GGML) ----
//
// Superbloque de 256 elementos = 144 bytes:
//   d: f16 | dmin: f16 | scales[12] (8 escalas + 8 mins de 6 bits) | qs[128]
// 8 sub-bloques de 32; el elemento e del par n usa el nibble bajo (primeros
// 32) o alto (siguientes 32) de qs. valor = d·sc·q − dmin·m.

/// Escala y min de 6 bits del sub-bloque `j` (0..8) — `get_scale_min_k4`.
pub fn q4k_scale_min(scales: &[u8], j: usize) -> (u8, u8) {
    if j < 4 {
        (scales[j] & 63, scales[j + 4] & 63)
    } else {
        (
            (scales[j + 4] & 0x0F) | ((scales[j - 4] >> 6) << 4),
            (scales[j + 4] >> 4) | ((scales[j] >> 6) << 4),
        )
    }
}

/// Producto punto del sub-bloque `j` (0..8, 32 elementos) de un superbloque Q4_K
/// por los 32 valores de `x` que le tocan (`x` apunta ya a `x[j*32]`).
///
/// Es el **gemelo Rust** de `q4k_dot_sub` de `lxdde/ports/nouveau/q4k_decode.h`, que
/// es lo que compila nvcc dentro del kernel SASS. Existe para poder atar las dos
/// implementaciones con una tabla de valores dorados
/// (`crates/sosomodel/tests/dequant_golden.rs` ↔ `check_q4k_decode` del hostcheck):
/// sin eso, una divergencia sólo se ve como «la GPU saca otros tokens».
///
/// Acumula `Σq·x` y `Σx` y aplica `d·sc` / `dmin·m` una vez, en vez de reconstruir
/// cada peso. Devuelve 0.0 si el bloque no mide 144 B o `j >= 8`.
pub fn q4k_dot_sub(blk: &[u8], j: usize, x: &[f32]) -> f32 {
    if blk.len() < Q4_K_BLOCK_BYTES || j >= 8 || x.len() < 32 {
        return 0.0;
    }
    let d = f16_to_f32(u16::from_le_bytes([blk[0], blk[1]]));
    let dmin = f16_to_f32(u16::from_le_bytes([blk[2], blk[3]]));
    let (sc, m) = q4k_scale_min(&blk[4..16], j);
    let qs = &blk[16..144];
    let pair = j >> 1;
    let alto = j & 1 == 1;
    let mut qx = 0.0f32;
    let mut sx = 0.0f32;
    for l in 0..32 {
        let q = qs[pair * 32 + l];
        let v = if alto { q >> 4 } else { q & 0x0F } as f32;
        qx += v * x[l];
        sx += x[l];
    }
    (d * sc as f32) * qx - (dmin * m as f32) * sx
}

/// Producto punto de un superbloque Q4_K entero, por sub-bloques. Gemelo de
/// `q4k_dot_block` del lado C.
pub fn q4k_dot_ref(blk: &[u8], x: &[f32]) -> f32 {
    let mut acc = 0.0f32;
    for j in 0..8 {
        acc += q4k_dot_sub(blk, j, &x[j * 32..]);
    }
    acc
}

/// Descuantiza `out.len()` elementos Q4_K a partir del elemento `elem_off`.
pub fn dequant_q4_k_range(bytes: &[u8], elem_off: usize, out: &mut [f32]) -> Result<(), ()> {
    if bytes.len() % Q4_K_BLOCK_BYTES != 0 || out.is_empty() {
        return Err(());
    }
    let total = (bytes.len() / Q4_K_BLOCK_BYTES) * Q4_K_BLOCK_ELEMS;
    let end = elem_off.checked_add(out.len()).ok_or(())?;
    if end > total {
        return Err(());
    }
    let first = elem_off / Q4_K_BLOCK_ELEMS;
    let last = (end - 1) / Q4_K_BLOCK_ELEMS;
    for b in first..=last {
        let blk = &bytes[b * Q4_K_BLOCK_BYTES..(b + 1) * Q4_K_BLOCK_BYTES];
        let d = f16_to_f32(u16::from_le_bytes([blk[0], blk[1]]));
        let dmin = f16_to_f32(u16::from_le_bytes([blk[2], blk[3]]));
        let scales = &blk[4..16];
        let qs = &blk[16..144];
        for pair in 0..4 {
            let (sc1, m1) = q4k_scale_min(scales, 2 * pair);
            let (sc2, m2) = q4k_scale_min(scales, 2 * pair + 1);
            let (d1, min1) = (d * sc1 as f32, dmin * m1 as f32);
            let (d2, min2) = (d * sc2 as f32, dmin * m2 as f32);
            let base = b * Q4_K_BLOCK_ELEMS + pair * 64;
            for l in 0..32 {
                let q = qs[pair * 32 + l];
                let e1 = base + l;
                let e2 = base + 32 + l;
                if e1 >= elem_off && e1 < end {
                    out[e1 - elem_off] = d1 * (q & 0x0F) as f32 - min1;
                }
                if e2 >= elem_off && e2 < end {
                    out[e2 - elem_off] = d2 * (q >> 4) as f32 - min2;
                }
            }
        }
    }
    Ok(())
}

/// Descuantiza el tensor Q4_K completo.
pub fn dequant_q4_k(bytes: &[u8], out: &mut [f32]) -> Result<(), ()> {
    dequant_q4_k_range(bytes, 0, out)
}

// ---- MXFP4: bloques de 32, escala f32 + nibbles ----

pub fn dequant_mxfp4(bytes: &[u8], out: &mut [f32]) -> Result<(), ()> {
    dequant_mxfp4_range(bytes, 0, out)
}

pub fn dequant_mxfp4_range(bytes: &[u8], elem_off: usize, out: &mut [f32]) -> Result<(), ()> {
    if bytes.len() % MXFP4_BLOCK_BYTES != 0 || out.is_empty() {
        return Err(());
    }
    let total = (bytes.len() / MXFP4_BLOCK_BYTES) * MXFP4_BLOCK_ELEMS;
    let end = elem_off.checked_add(out.len()).ok_or(())?;
    if end > total {
        return Err(());
    }
    let first = elem_off / MXFP4_BLOCK_ELEMS;
    let last = (end - 1) / MXFP4_BLOCK_ELEMS;
    for b in first..=last {
        let chunk = &bytes[b * MXFP4_BLOCK_BYTES..(b + 1) * MXFP4_BLOCK_BYTES];
        let scale = f32::from_le_bytes(chunk[..4].try_into().map_err(|_| ())?);
        let base = b * MXFP4_BLOCK_ELEMS;
        for j in 0..MXFP4_BLOCK_ELEMS {
            let e = base + j;
            if e >= elem_off && e < end {
                let byte = chunk[4 + j / 2];
                let n = if j % 2 == 0 { byte & 0x0f } else { byte >> 4 };
                let signed = if n & 0x8 != 0 {
                    (n as i8).wrapping_sub(16)
                } else {
                    n as i8
                };
                out[e - elem_off] = signed as f32 * scale;
            }
        }
    }
    Ok(())
}

/// Producto punto de UNA fila cuantizada por `x`, descuantizando por bloques con
/// un scratch de `cols` elementos.
///
/// Es lo que ejecuta el dispositivo software del kernel para `MATVQ`, y por tanto
/// la referencia contra la que se compara el kernel SASS. Devuelve `None` si el
/// dtype no es de bloques, `cols` no es múltiplo del bloque, o la fila no cabe.
pub fn matvec_row(
    dtype: u8,
    fila: &[u8],
    cols: usize,
    x: &[f32],
    scratch: &mut [f32],
) -> Option<f32> {
    if x.len() != cols || scratch.len() < cols || row_bytes(dtype, cols)? != fila.len() {
        return None;
    }
    let deq = match dtype {
        crate::layout::DTYPE_Q8_0 => dequant_q8_0(fila, &mut scratch[..cols]),
        crate::layout::DTYPE_Q4_K => dequant_q4_k(fila, &mut scratch[..cols]),
        crate::layout::DTYPE_MXFP4 => dequant_mxfp4(fila, &mut scratch[..cols]),
        _ => return None,
    };
    deq.ok()?;
    let mut sum = 0.0f32;
    for c in 0..cols {
        sum += scratch[c] * x[c];
    }
    Some(sum)
}
