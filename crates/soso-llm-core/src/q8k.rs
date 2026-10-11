//! Cuantización Q8_K (intermedia GGML) y producto Q4_K × Q8_K.
//!
//! Misma semántica que `quantize_row_q8_K_ref` / `ggml_vec_dot_q4_K_q8_K` de llama.cpp.

use sosomodel::layout::{Q4_K_BLOCK_BYTES, Q4_K_BLOCK_ELEMS};

/// Elementos por bloque Q8_K (igual que Q4_K).
pub const Q8_K_BLOCK_ELEMS: usize = Q4_K_BLOCK_ELEMS;
/// `d` f32 + 256 × i8 + 16 × i16 bsums.
pub const Q8_K_BLOCK_BYTES: usize = 4 + Q8_K_BLOCK_ELEMS + 16 * 2;

#[inline]
fn nearest_i32(x: f32) -> i32 {
    if x >= 0.0 {
        libm::floorf(x + 0.5) as i32
    } else {
        libm::ceilf(x - 0.5) as i32
    }
}

/// Cuantiza `x` (múltiplo de 256) a bloques Q8_K consecutivos en `out`.
pub fn quantize_row_q8_k(x: &[f32], out: &mut [u8]) {
    assert_eq!(x.len() % Q8_K_BLOCK_ELEMS, 0);
    assert_eq!(out.len(), (x.len() / Q8_K_BLOCK_ELEMS) * Q8_K_BLOCK_BYTES);
    let mut xi = 0usize;
    let mut oi = 0usize;
    while xi < x.len() {
        let block = &x[xi..xi + Q8_K_BLOCK_ELEMS];
        let ob = &mut out[oi..oi + Q8_K_BLOCK_BYTES];
        let mut amax = 0.0f32;
        let mut max = 0.0f32;
        for &v in block {
            let ax = libm::fabsf(v);
            if ax > amax {
                amax = ax;
                max = v;
            }
        }
        if amax == 0.0 {
            ob[..4].copy_from_slice(&0f32.to_le_bytes());
            ob[4..4 + Q8_K_BLOCK_ELEMS].fill(0);
            for s in ob[4 + Q8_K_BLOCK_ELEMS..].chunks_exact_mut(2) {
                s.copy_from_slice(&0i16.to_le_bytes());
            }
            xi += Q8_K_BLOCK_ELEMS;
            oi += Q8_K_BLOCK_BYTES;
            continue;
        }
        let iscale = -127.0f32 / max;
        {
            let qs = &mut ob[4..4 + Q8_K_BLOCK_ELEMS];
            for (q, &v) in qs.iter_mut().zip(block) {
                let mut vi = nearest_i32(iscale * v);
                if vi > 127 {
                    vi = 127;
                }
                *q = vi as i8 as u8;
            }
        }
        for g in 0..16 {
            let mut sum = 0i32;
            for ii in 0..16 {
                sum += ob[4 + g * 16 + ii] as i8 as i32;
            }
            let off = 4 + Q8_K_BLOCK_ELEMS + g * 2;
            ob[off..off + 2].copy_from_slice(&(sum as i16).to_le_bytes());
        }
        let d = 1.0f32 / iscale;
        ob[..4].copy_from_slice(&d.to_le_bytes());
        xi += Q8_K_BLOCK_ELEMS;
        oi += Q8_K_BLOCK_BYTES;
    }
}

/// Bytes necesarios para `n` vectores de `cols` elementos en Q8_K.
pub fn q8k_buffer_len(n: usize, cols: usize) -> usize {
    assert_eq!(cols % Q8_K_BLOCK_ELEMS, 0);
    n * (cols / Q8_K_BLOCK_ELEMS) * Q8_K_BLOCK_BYTES
}

fn unpack_q4_k_scales(scales12: &[u8; 12]) -> ([u8; 8], [u8; 8]) {
    const KMASK1: u32 = 0x3f3f3f3f;
    const KMASK2: u32 = 0x0f0f0f0f;
    const KMASK3: u32 = 0x03030303;
    let mut utmp = [0u32; 4];
    utmp[0] = u32::from_le_bytes([scales12[0], scales12[1], scales12[2], scales12[3]]);
    utmp[1] = u32::from_le_bytes([scales12[4], scales12[5], scales12[6], scales12[7]]);
    utmp[2] = u32::from_le_bytes([scales12[8], scales12[9], scales12[10], scales12[11]]);
    utmp[3] = ((utmp[2] >> 4) & KMASK2) | (((utmp[1] >> 6) & KMASK3) << 4);
    let uaux = utmp[1] & KMASK1;
    utmp[1] = (utmp[2] & KMASK2) | (((utmp[0] >> 6) & KMASK3) << 4);
    utmp[2] = uaux;
    utmp[0] &= KMASK1;
    let mut sc = [0u8; 8];
    sc[..4].copy_from_slice(&utmp[0].to_le_bytes());
    sc[4..8].copy_from_slice(&utmp[1].to_le_bytes());
    let mut mn = [0u8; 8];
    mn[..4].copy_from_slice(&utmp[2].to_le_bytes());
    mn[4..8].copy_from_slice(&utmp[3].to_le_bytes());
    (sc, mn)
}

/// Producto punto de un superbloque Q4_K con uno Q8_K (referencia entera, estilo ggml).
pub fn dot_q4_k_q8_k_block(q4: &[u8], q8: &[u8]) -> f32 {
    assert!(q4.len() >= Q4_K_BLOCK_BYTES && q8.len() >= Q8_K_BLOCK_BYTES);
    use crate::f16::f16_to_f32;
    let d_w = f16_to_f32(u16::from_le_bytes([q4[0], q4[1]]));
    let dmin_w = f16_to_f32(u16::from_le_bytes([q4[2], q4[3]]));
    let mut scales12 = [0u8; 12];
    scales12.copy_from_slice(&q4[4..16]);
    let (scales, mins) = unpack_q4_k_scales(&scales12);

    let d_y = f32::from_le_bytes([q8[0], q8[1], q8[2], q8[3]]);
    let d = d_w * d_y;
    let dmin = -d_y * dmin_w;

    let q4qs = &q4[16..144];
    let q8qs = &q8[4..4 + Q8_K_BLOCK_ELEMS];
    let bsums = &q8[4 + Q8_K_BLOCK_ELEMS..4 + Q8_K_BLOCK_ELEMS + 32];

    let mut aux8 = [0i8; Q8_K_BLOCK_ELEMS];
    let mut a = 0usize;
    let mut q4p = 0usize;
    for _ in 0..Q8_K_BLOCK_ELEMS / 64 {
        for l in 0..32 {
            aux8[a + l] = (q4qs[q4p + l] & 0xF) as i8;
        }
        a += 32;
        for l in 0..32 {
            aux8[a + l] = (q4qs[q4p + l] >> 4) as i8;
        }
        a += 32;
        q4p += 32;
    }

    let mut sumi = 0i32;
    for j in 0..Q8_K_BLOCK_ELEMS / 16 {
        let bs = i16::from_le_bytes([bsums[j * 2], bsums[j * 2 + 1]]) as i32;
        sumi += bs * mins[j / 2] as i32;
    }

    let mut aux32 = [0i32; 8];
    let mut q8p = 0usize;
    a = 0;
    let mut is = 0usize;
    for _ in 0..Q8_K_BLOCK_ELEMS / 32 {
        let scale = scales[is] as i32;
        is += 1;
        for _chunk in 0..4 {
            for l in 0..8 {
                aux32[l] += scale * (q8qs[q8p + l] as i8 as i32) * (aux8[a + l] as i32);
            }
            q8p += 8;
            a += 8;
        }
    }

    let mut sumf = -(dmin * sumi as f32);
    for l in 0..8 {
        sumf += d * aux32[l] as f32;
    }
    sumf
}

const AVX2: bool = cfg!(all(
    target_arch = "x86_64",
    target_feature = "avx2",
    target_feature = "fma"
));

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::{Q4_K_BLOCK_BYTES, Q8_K_BLOCK_BYTES, Q8_K_BLOCK_ELEMS, unpack_q4_k_scales};
    use core::arch::x86_64::*;

    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn dot_q4_k_q8_k_block_avx2(q4: &[u8], q8: &[u8]) -> f32 {
        use crate::f16::f16_to_f32;
        unsafe {
            assert!(q4.len() >= Q4_K_BLOCK_BYTES && q8.len() >= Q8_K_BLOCK_BYTES);
            let d = f32::from_le_bytes([q8[0], q8[1], q8[2], q8[3]])
                * f16_to_f32(u16::from_le_bytes([q4[0], q4[1]]));
            let dmin = -f32::from_le_bytes([q8[0], q8[1], q8[2], q8[3]])
                * f16_to_f32(u16::from_le_bytes([q4[2], q4[3]]));

            let mut scales12 = [0u8; 12];
            scales12.copy_from_slice(&q4[4..16]);
            let mut utmp = [0u32; 4];
            utmp[0] = u32::from_le_bytes([scales12[0], scales12[1], scales12[2], scales12[3]]);
            utmp[1] = u32::from_le_bytes([scales12[4], scales12[5], scales12[6], scales12[7]]);
            utmp[2] = u32::from_le_bytes([scales12[8], scales12[9], scales12[10], scales12[11]]);
            const KMASK1: u32 = 0x3f3f3f3f;
            const KMASK2: u32 = 0x0f0f0f0f;
            const KMASK3: u32 = 0x03030303;
            utmp[3] = ((utmp[2] >> 4) & KMASK2) | (((utmp[1] >> 6) & KMASK3) << 4);
            let uaux = utmp[1] & KMASK1;
            utmp[1] = (utmp[2] & KMASK2) | (((utmp[0] >> 6) & KMASK3) << 4);
            utmp[2] = uaux;
            utmp[0] &= KMASK1;

            let m4 = _mm256_set1_epi8(0xF);
            let q4p_base = q4.as_ptr();
            let q8_base = q8.as_ptr();

            let mins_and_scales = _mm256_cvtepu8_epi16(_mm_set_epi32(
                utmp[3] as i32, utmp[2] as i32, utmp[1] as i32, utmp[0] as i32,
            ));
            let q8sums = _mm256_loadu_si256(q8_base.add(Q8_K_BLOCK_ELEMS) as *const __m256i);
            let q8s = _mm_hadd_epi16(
                _mm256_extracti128_si256(q8sums, 0),
                _mm256_extracti128_si256(q8sums, 1),
            );
            let prod = _mm_madd_epi16(_mm256_extracti128_si256(mins_and_scales, 1), q8s);
            let mut acc_m = _mm_mul_ps(_mm_set1_ps(dmin), _mm_cvtepi32_ps(prod));

            let sc128 = _mm256_extracti128_si256(mins_and_scales, 0);
            let scales_v = _mm256_set_m128i(sc128, sc128);

            let mut sumi = _mm256_setzero_si256();
            let mut q4p = q4p_base.add(16);
            let mut q8p = q8_base;

            for j in 0..Q8_K_BLOCK_ELEMS / 64 {
                let scale_l = _mm256_shuffle_epi8(scales_v, get_scale_shuffle_k4(2 * j as i32));
                let scale_h = _mm256_shuffle_epi8(scales_v, get_scale_shuffle_k4(2 * j as i32 + 1));

                let q4bits = _mm256_loadu_si256(q4p as *const __m256i);
                q4p = q4p.add(32);
                let q4l = _mm256_and_si256(q4bits, m4);
                let q4h = _mm256_and_si256(_mm256_srli_epi16(q4bits, 4), m4);

                let q8l = _mm256_loadu_si256(q8p as *const __m256i);
                q8p = q8p.add(32);
                let mut p16l = _mm256_maddubs_epi16(q4l, q8l);
                p16l = _mm256_madd_epi16(scale_l, p16l);

                let q8h = _mm256_loadu_si256(q8p as *const __m256i);
                q8p = q8p.add(32);
                let mut p16h = _mm256_maddubs_epi16(q4h, q8h);
                p16h = _mm256_madd_epi16(scale_h, p16h);

                sumi = _mm256_add_epi32(sumi, _mm256_add_epi32(p16l, p16h));
            }

            let vd = _mm256_set1_ps(d);
            let mut acc = _mm256_mul_ps(vd, _mm256_cvtepi32_ps(sumi));

            acc_m = _mm_add_ps(acc_m, _mm_movehl_ps(acc_m, acc_m));
            acc_m = _mm_add_ss(acc_m, _mm_movehdup_ps(acc_m));

            acc_m128_hsum(acc) + _mm_cvtss_f32(acc_m)
        }
    }

    #[inline]
    unsafe fn acc_m128_hsum(acc: __m256) -> f32 {
        unsafe {
            let t = _mm_add_ps(_mm256_castps256_ps128(acc), _mm256_extractf128_ps(acc, 1));
            let s = _mm_hadd_ps(t, t);
            _mm_cvtss_f32(_mm_hadd_ps(s, s))
        }
    }

    #[inline]
    unsafe fn get_scale_shuffle_k4(i: i32) -> __m256i {
        unsafe {
            // Misma tabla que `get_scale_shuffle_k4` en ggml-cpu/arch/x86/quants.c.
            static K_SHUFFLE: [u8; 256] = [
                0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1,
                2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3,
                4, 5, 4, 5, 4, 5, 4, 5, 4, 5, 4, 5, 4, 5, 4, 5, 4, 5, 4, 5, 4, 5, 4, 5, 4, 5, 4, 5, 4, 5, 4, 5,
                6, 7, 6, 7, 6, 7, 6, 7, 6, 7, 6, 7, 6, 7, 6, 7, 6, 7, 6, 7, 6, 7, 6, 7, 6, 7, 6, 7, 6, 7, 6, 7,
                8, 9, 8, 9, 8, 9, 8, 9, 8, 9, 8, 9, 8, 9, 8, 9, 8, 9, 8, 9, 8, 9, 8, 9, 8, 9, 8, 9, 8, 9, 8, 9,
                10, 11, 10, 11, 10, 11, 10, 11, 10, 11, 10, 11, 10, 11, 10, 11, 10, 11, 10, 11, 10, 11, 10, 11,
                10, 11, 10, 11, 10, 11, 10, 11, 12, 13, 12, 13, 12, 13, 12, 13, 12, 13, 12, 13, 12, 13, 12, 13,
                12, 13, 12, 13, 12, 13, 12, 13, 12, 13, 12, 13, 12, 13, 12, 13, 14, 15, 14, 15, 14, 15, 14, 15,
                14, 15, 14, 15, 14, 15, 14, 15, 14, 15, 14, 15, 14, 15, 14, 15, 14, 15, 14, 15, 14, 15, 14, 15,
            ];
            _mm256_loadu_si256(K_SHUFFLE.as_ptr().add(i as usize * 32) as *const __m256i)
        }
    }
}

/// Producto de superbloque: AVX2 si el binario lo trae, si no la referencia escalar.
pub fn dot_q4_k_q8_k_block_dispatch(q4: &[u8], q8: &[u8]) -> f32 {
    if AVX2 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            return avx2::dot_q4_k_q8_k_block_avx2(q4, q8);
        }
    }
    dot_q4_k_q8_k_block(q4, q8)
}

/// Matvec Q4_K con activaciones ya cuantizadas a Q8_K (`q8k` = `n` filas seguidas).
pub fn matvec_q4_k_q8k_prequantized(
    bytes: &[u8],
    rows: usize,
    cols: usize,
    q8k: &[u8],
    n: usize,
    outs: &mut [f32],
    stride: usize,
) -> Result<(), ()> {
    use sosomodel::layout::{Q4_K_BLOCK_BYTES, Q4_K_BLOCK_ELEMS};
    if cols % Q8_K_BLOCK_ELEMS != 0 {
        return Err(());
    }
    let blocks = cols / Q8_K_BLOCK_ELEMS;
    let row_bytes = blocks * Q4_K_BLOCK_BYTES;
    let q8_row_bytes = blocks * Q8_K_BLOCK_BYTES;
    if bytes.len() != rows * row_bytes
        || q8k.len() != n * q8_row_bytes
        || (n > 0 && (n - 1) * stride + rows > outs.len())
    {
        return Err(());
    }
    for r in 0..rows {
        let row = &bytes[r * row_bytes..(r + 1) * row_bytes];
        for j in 0..n {
            let mut acc = 0.0f32;
            let q8_row = &q8k[j * q8_row_bytes..(j + 1) * q8_row_bytes];
            for b in 0..blocks {
                let q4_blk = &row[b * Q4_K_BLOCK_BYTES..(b + 1) * Q4_K_BLOCK_BYTES];
                let q8_blk = &q8_row[b * Q8_K_BLOCK_BYTES..(b + 1) * Q8_K_BLOCK_BYTES];
                acc += dot_q4_k_q8_k_block_dispatch(q4_blk, q8_blk);
            }
            outs[j * stride + r] = acc;
        }
    }
    Ok(())
}

/// Cuantiza `xs` (`n`×`cols`) en `q8k_buf` y ejecuta el matvec entero.
pub fn matvec_q4_k_q8k_batch_strided(
    bytes: &[u8],
    rows: usize,
    cols: usize,
    xs: &[f32],
    n: usize,
    outs: &mut [f32],
    stride: usize,
    q8k_buf: &mut [u8],
) -> Result<(), ()> {
    let need = q8k_buffer_len(n, cols);
    if xs.len() != n * cols || q8k_buf.len() < need {
        return Err(());
    }
    for j in 0..n {
        quantize_row_q8_k(
            &xs[j * cols..(j + 1) * cols],
            &mut q8k_buf[j * (need / n)..(j + 1) * (need / n)],
        );
    }
    matvec_q4_k_q8k_prequantized(bytes, rows, cols, &q8k_buf[..need], n, outs, stride)
}
