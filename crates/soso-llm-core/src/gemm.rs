//! Kernels de matvec: escalares (referencia, cualquier target) y AVX2+FMA
//! (userspace de soso y hosts con `-C target-cpu=native`). El dispatch es en
//! compilación por `target_feature`; los tests host comparan ambos caminos
//! con detección en runtime.

/// ¿El binario está compilado con AVX2+FMA?
const AVX2: bool = cfg!(all(
    target_arch = "x86_64",
    target_feature = "avx2",
    target_feature = "fma"
));

pub fn matvec_f32(matrix: &[f32], rows: usize, cols: usize, x: &[f32], out: &mut [f32]) {
    assert_eq!(matrix.len(), rows * cols);
    assert_eq!(x.len(), cols);
    assert_eq!(out.len(), rows);
    if AVX2 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            return avx2::matvec_f32(matrix, rows, cols, x, out);
        }
    }
    matvec_f32_scalar(matrix, rows, cols, x, out)
}

pub fn matvec_f32_scalar(matrix: &[f32], rows: usize, cols: usize, x: &[f32], out: &mut [f32]) {
    for r in 0..rows {
        let mut acc = 0.0f32;
        let row = &matrix[r * cols..(r + 1) * cols];
        for c in 0..cols {
            acc += row[c] * x[c];
        }
        out[r] = acc;
    }
}

pub fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub fn softmax_inplace(x: &mut [f32]) {
    let max = x.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut sum = 0.0f32;
    for v in x.iter_mut() {
        *v = libm::expf(*v - max);
        sum += *v;
    }
    if sum > 0.0 {
        for v in x.iter_mut() {
            *v /= sum;
        }
    }
}

pub fn rmsnorm(x: &mut [f32], weight: &[f32], eps: f32) {
    let n = x.len() as f32;
    let var = x.iter().map(|v| v * v).sum::<f32>() / n;
    let scale = 1.0 / libm::sqrtf(var + eps);
    for (xi, w) in x.iter_mut().zip(weight) {
        *xi = *xi * scale * w;
    }
}

pub fn silu(x: f32) -> f32 {
    x / (1.0 + libm::expf(-x))
}

/// SwiGLU in-place: `up[i] = silu(gate[i]) * up[i]`.
pub fn swiglu_inplace(up: &mut [f32], gate: &[f32]) {
    let n = up.len().min(gate.len());
    for i in 0..n {
        up[i] = silu(gate[i]) * up[i];
    }
}

/// SiLU unario in-place (FFN sin gate).
pub fn silu_inplace(x: &mut [f32]) {
    for v in x.iter_mut() {
        *v = silu(*v);
    }
}

#[cfg(test)]
mod residual_tests {
    use super::*;

    #[test]
    fn add_f32_matches_scalar() {
        let a = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let b = [0.5f32; 9];
        let mut dst = [0.0f32; 9];
        add_f32(&a, &b, &mut dst);
        for i in 0..9 {
            assert!((dst[i] - (a[i] + b[i])).abs() < 1e-6);
        }
    }

    #[test]
    fn add_assign_matches() {
        let mut dst = [1.0f32, 2.0, 3.0];
        add_assign_f32(&mut dst, &[10.0, 20.0, 30.0]);
        assert_eq!(dst, [11.0, 22.0, 33.0]);
    }
}

/// `dst[i] = a[i] + b[i]` (residual de atención).
pub fn add_f32(a: &[f32], b: &[f32], dst: &mut [f32]) {
    let n = a.len().min(b.len()).min(dst.len());
    if AVX2 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            return avx2::add_f32(a, b, dst, n);
        }
    }
    for i in 0..n {
        dst[i] = a[i] + b[i];
    }
}

/// `dst[i] += src[i]` (residual de FFN).
pub fn add_assign_f32(dst: &mut [f32], src: &[f32]) {
    let n = dst.len().min(src.len());
    if AVX2 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            return avx2::add_assign_f32(dst, src, n);
        }
    }
    for i in 0..n {
        dst[i] += src[i];
    }
}

/// matvec fusionado sobre pesos Q8_0 on-disk (bloques de escala f32 + 32 i8):
/// descuantiza en registros dentro del bucle, sin buffer f32 intermedio.
/// `cols` debe ser múltiplo del bloque (32).
pub fn matvec_q8_0(bytes: &[u8], rows: usize, cols: usize, x: &[f32], out: &mut [f32]) -> Result<(), ()> {
    use sosomodel::layout::{Q8_0_BLOCK_BYTES, Q8_0_BLOCK_ELEMS};
    if cols % Q8_0_BLOCK_ELEMS != 0 || x.len() != cols || out.len() != rows {
        return Err(());
    }
    let row_bytes = (cols / Q8_0_BLOCK_ELEMS) * Q8_0_BLOCK_BYTES;
    if bytes.len() != rows * row_bytes {
        return Err(());
    }
    if AVX2 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            avx2::matvec_q8_0(bytes, rows, cols, x, out);
            return Ok(());
        }
    }
    matvec_q8_0_scalar(bytes, rows, cols, x, out);
    Ok(())
}

pub fn matvec_q8_0_scalar(bytes: &[u8], rows: usize, cols: usize, x: &[f32], out: &mut [f32]) {
    use sosomodel::layout::{Q8_0_BLOCK_BYTES, Q8_0_BLOCK_ELEMS};
    let row_bytes = (cols / Q8_0_BLOCK_ELEMS) * Q8_0_BLOCK_BYTES;
    for (r, o) in out.iter_mut().enumerate() {
        let row = &bytes[r * row_bytes..(r + 1) * row_bytes];
        let mut acc = 0.0f32;
        for (b, chunk) in row.chunks_exact(Q8_0_BLOCK_BYTES).enumerate() {
            let scale = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let xb = &x[b * Q8_0_BLOCK_ELEMS..(b + 1) * Q8_0_BLOCK_ELEMS];
            let mut s = 0.0f32;
            for (q, xv) in chunk[4..].iter().zip(xb) {
                s += (*q as i8) as f32 * xv;
            }
            acc += scale * s;
        }
        *o = acc;
    }
}

/// matvec fusionado sobre pesos Q4_K (superbloques GGML de 256 elems /
/// 144 B). Por sub-bloque: acc += d·sc·Σ(q·x) − dmin·m·Σx, descuantizando
/// en registros. `cols` debe ser múltiplo de 256.
pub fn matvec_q4_k(bytes: &[u8], rows: usize, cols: usize, x: &[f32], out: &mut [f32]) -> Result<(), ()> {
    use sosomodel::layout::{Q4_K_BLOCK_BYTES, Q4_K_BLOCK_ELEMS};
    if cols % Q4_K_BLOCK_ELEMS != 0 || x.len() != cols || out.len() != rows {
        return Err(());
    }
    let row_bytes = (cols / Q4_K_BLOCK_ELEMS) * Q4_K_BLOCK_BYTES;
    if bytes.len() != rows * row_bytes {
        return Err(());
    }
    if AVX2 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            avx2::matvec_q4_k(bytes, rows, cols, x, out);
            return Ok(());
        }
    }
    matvec_q4_k_scalar(bytes, rows, cols, x, out);
    Ok(())
}

pub fn matvec_q4_k_scalar(bytes: &[u8], rows: usize, cols: usize, x: &[f32], out: &mut [f32]) {
    use crate::f16::f16_to_f32;
    use crate::quant::q4k_scale_min;
    use sosomodel::layout::{Q4_K_BLOCK_BYTES, Q4_K_BLOCK_ELEMS};
    let row_bytes = (cols / Q4_K_BLOCK_ELEMS) * Q4_K_BLOCK_BYTES;
    for (r, o) in out.iter_mut().enumerate() {
        let row = &bytes[r * row_bytes..(r + 1) * row_bytes];
        let mut acc = 0.0f32;
        for (b, blk) in row.chunks_exact(Q4_K_BLOCK_BYTES).enumerate() {
            let d = f16_to_f32(u16::from_le_bytes([blk[0], blk[1]]));
            let dmin = f16_to_f32(u16::from_le_bytes([blk[2], blk[3]]));
            let scales = &blk[4..16];
            let qs = &blk[16..144];
            let xb = &x[b * Q4_K_BLOCK_ELEMS..(b + 1) * Q4_K_BLOCK_ELEMS];
            for pair in 0..4 {
                let (sc1, m1) = q4k_scale_min(scales, 2 * pair);
                let (sc2, m2) = q4k_scale_min(scales, 2 * pair + 1);
                let x1 = &xb[pair * 64..pair * 64 + 32];
                let x2 = &xb[pair * 64 + 32..pair * 64 + 64];
                let q = &qs[pair * 32..(pair + 1) * 32];
                let (mut s1, mut sx1, mut s2, mut sx2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
                for l in 0..32 {
                    s1 += (q[l] & 0x0F) as f32 * x1[l];
                    sx1 += x1[l];
                    s2 += (q[l] >> 4) as f32 * x2[l];
                    sx2 += x2[l];
                }
                acc += d * sc1 as f32 * s1 - dmin * m1 as f32 * sx1;
                acc += d * sc2 as f32 * s2 - dmin * m2 as f32 * sx2;
            }
        }
        *o = acc;
    }
}

/// Kernels AVX2+FMA. `# Safety`: requieren CPU con AVX2/FMA y longitudes ya
/// validadas por los wrappers públicos.
#[cfg(target_arch = "x86_64")]
pub mod avx2 {
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

    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn add_f32(a: &[f32], b: &[f32], dst: &mut [f32], n: usize) {
        unsafe {
            let mut i = 0usize;
            while i + 8 <= n {
                let va = _mm256_loadu_ps(a.as_ptr().add(i));
                let vb = _mm256_loadu_ps(b.as_ptr().add(i));
                _mm256_storeu_ps(dst.as_mut_ptr().add(i), _mm256_add_ps(va, vb));
                i += 8;
            }
            while i < n {
                dst[i] = a[i] + b[i];
                i += 1;
            }
        }
    }

    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn add_assign_f32(dst: &mut [f32], src: &[f32], n: usize) {
        unsafe {
            let mut i = 0usize;
            while i + 8 <= n {
                let vd = _mm256_loadu_ps(dst.as_ptr().add(i));
                let vs = _mm256_loadu_ps(src.as_ptr().add(i));
                _mm256_storeu_ps(dst.as_mut_ptr().add(i), _mm256_add_ps(vd, vs));
                i += 8;
            }
            while i < n {
                dst[i] += src[i];
                i += 1;
            }
        }
    }

    /// # Safety
    /// AVX2+FMA disponibles; `matrix.len()==rows*cols`, `x.len()==cols`,
    /// `out.len()==rows`.
    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn matvec_f32(matrix: &[f32], rows: usize, cols: usize, x: &[f32], out: &mut [f32]) {
        unsafe {
            for r in 0..rows {
                let row = matrix.as_ptr().add(r * cols);
                let mut acc0 = _mm256_setzero_ps();
                let mut acc1 = _mm256_setzero_ps();
                let mut c = 0usize;
                while c + 16 <= cols {
                    acc0 = _mm256_fmadd_ps(
                        _mm256_loadu_ps(row.add(c)),
                        _mm256_loadu_ps(x.as_ptr().add(c)),
                        acc0,
                    );
                    acc1 = _mm256_fmadd_ps(
                        _mm256_loadu_ps(row.add(c + 8)),
                        _mm256_loadu_ps(x.as_ptr().add(c + 8)),
                        acc1,
                    );
                    c += 16;
                }
                while c + 8 <= cols {
                    acc0 = _mm256_fmadd_ps(
                        _mm256_loadu_ps(row.add(c)),
                        _mm256_loadu_ps(x.as_ptr().add(c)),
                        acc0,
                    );
                    c += 8;
                }
                let mut s = hsum(_mm256_add_ps(acc0, acc1));
                while c < cols {
                    s += *row.add(c) * x[c];
                    c += 1;
                }
                out[r] = s;
            }
        }
    }

    /// # Safety
    /// Como `matvec_f32`; `cols % 32 == 0`, `bytes.len() == rows*cols/32*36`.
    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn matvec_q8_0(bytes: &[u8], rows: usize, cols: usize, x: &[f32], out: &mut [f32]) {
        use sosomodel::layout::{Q8_0_BLOCK_BYTES, Q8_0_BLOCK_ELEMS};
        unsafe {
            let blocks = cols / Q8_0_BLOCK_ELEMS;
            let row_bytes = blocks * Q8_0_BLOCK_BYTES;
            for r in 0..rows {
                let row = bytes.as_ptr().add(r * row_bytes);
                let mut acc = _mm256_setzero_ps();
                for b in 0..blocks {
                    let chunk = row.add(b * Q8_0_BLOCK_BYTES);
                    let scale = _mm256_set1_ps(f32::from_le_bytes([
                        *chunk,
                        *chunk.add(1),
                        *chunk.add(2),
                        *chunk.add(3),
                    ]));
                    let q = chunk.add(4);
                    let xb = x.as_ptr().add(b * Q8_0_BLOCK_ELEMS);
                    let mut blk = _mm256_setzero_ps();
                    for g in 0..4 {
                        let qi = _mm_loadl_epi64(q.add(g * 8) as *const __m128i);
                        let qf = _mm256_cvtepi32_ps(_mm256_cvtepi8_epi32(qi));
                        blk = _mm256_fmadd_ps(qf, _mm256_loadu_ps(xb.add(g * 8)), blk);
                    }
                    acc = _mm256_fmadd_ps(scale, blk, acc);
                }
                out[r] = hsum(acc);
            }
        }
    }

    /// # Safety
    /// Como `matvec_f32`; `cols % 256 == 0`, `bytes.len() == rows*cols/256*144`.
    /// Contribución por elemento: x·(d·sc·q − dmin·m), sin sumas horizontales
    /// intermedias.
    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn matvec_q4_k(bytes: &[u8], rows: usize, cols: usize, x: &[f32], out: &mut [f32]) {
        use crate::f16::f16_to_f32;
        use crate::quant::q4k_scale_min;
        use sosomodel::layout::{Q4_K_BLOCK_BYTES, Q4_K_BLOCK_ELEMS};
        unsafe {
            let blocks = cols / Q4_K_BLOCK_ELEMS;
            let row_bytes = blocks * Q4_K_BLOCK_BYTES;
            let mask = _mm256_set1_epi8(0x0F);
            for r in 0..rows {
                let row = bytes.as_ptr().add(r * row_bytes);
                let mut acc = _mm256_setzero_ps();
                for b in 0..blocks {
                    let blk = row.add(b * Q4_K_BLOCK_BYTES);
                    let d = f16_to_f32(u16::from_le_bytes([*blk, *blk.add(1)]));
                    let dmin = f16_to_f32(u16::from_le_bytes([*blk.add(2), *blk.add(3)]));
                    let scales = core::slice::from_raw_parts(blk.add(4), 12);
                    for pair in 0..4 {
                        let (sc1, m1) = q4k_scale_min(scales, 2 * pair);
                        let (sc2, m2) = q4k_scale_min(scales, 2 * pair + 1);
                        let dsc1 = _mm256_set1_ps(d * sc1 as f32);
                        let dm1 = _mm256_set1_ps(-dmin * m1 as f32);
                        let dsc2 = _mm256_set1_ps(d * sc2 as f32);
                        let dm2 = _mm256_set1_ps(-dmin * m2 as f32);
                        let qv = _mm256_loadu_si256(blk.add(16 + pair * 32) as *const __m256i);
                        let lo = _mm256_and_si256(qv, mask);
                        let hi = _mm256_and_si256(_mm256_srli_epi16(qv, 4), mask);
                        let xb = x.as_ptr().add(b * Q4_K_BLOCK_ELEMS + pair * 64);
                        // lo → x1[0..32], hi → x2[32..64]
                        for (v, base, dsc, dm) in
                            [(lo, 0usize, dsc1, dm1), (hi, 32usize, dsc2, dm2)]
                        {
                            let l0 = _mm256_castsi256_si128(v);
                            let l1 = _mm256_extracti128_si256(v, 1);
                            let quads = [
                                l0,
                                _mm_srli_si128(l0, 8),
                                l1,
                                _mm_srli_si128(l1, 8),
                            ];
                            for (g, qq) in quads.into_iter().enumerate() {
                                let qf = _mm256_cvtepi32_ps(_mm256_cvtepu8_epi32(qq));
                                let coef = _mm256_fmadd_ps(qf, dsc, dm);
                                let xv = _mm256_loadu_ps(xb.add(base + g * 8));
                                acc = _mm256_fmadd_ps(coef, xv, acc);
                            }
                        }
                    }
                }
                out[r] = hsum(acc);
            }
        }
    }
}

#[cfg(test)]
#[cfg(feature = "std")]
mod tests {
    use super::*;
    use crate::quant::{dequant_q4_k, quantize_q8_0};

    fn tiene_avx2() -> bool {
        std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma")
    }

    fn casi_iguales(a: &[f32], b: &[f32]) {
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            let tol = 1e-3 + x.abs() * 1e-4;
            assert!((x - y).abs() < tol, "fila {i}: {x} vs {y}");
        }
    }

    #[test]
    fn avx2_f32_coincide_con_escalar() {
        if !tiene_avx2() {
            return;
        }
        let (rows, cols) = (5, 100); // no múltiplo de 8: ejercita el tail
        let m: Vec<f32> = (0..rows * cols).map(|i| ((i * 31 % 97) as f32 - 48.0) * 0.03).collect();
        let x: Vec<f32> = (0..cols).map(|i| ((i * 17 % 89) as f32 - 44.0) * 0.02).collect();
        let mut a = vec![0.0f32; rows];
        let mut b = vec![0.0f32; rows];
        matvec_f32_scalar(&m, rows, cols, &x, &mut a);
        unsafe { avx2::matvec_f32(&m, rows, cols, &x, &mut b) };
        casi_iguales(&a, &b);
    }

    #[test]
    fn avx2_q8_coincide_con_escalar() {
        if !tiene_avx2() {
            return;
        }
        let (rows, cols) = (4, 96);
        let w: Vec<f32> = (0..rows * cols).map(|i| ((i * 13 % 61) as f32 - 30.0) * 0.05).collect();
        let bytes = quantize_q8_0(&w);
        let x: Vec<f32> = (0..cols).map(|i| ((i * 7 % 53) as f32 - 26.0) * 0.04).collect();
        let mut a = vec![0.0f32; rows];
        let mut b = vec![0.0f32; rows];
        matvec_q8_0_scalar(&bytes, rows, cols, &x, &mut a);
        unsafe { avx2::matvec_q8_0(&bytes, rows, cols, &x, &mut b) };
        casi_iguales(&a, &b);
    }

    #[test]
    fn avx2_q4k_coincide_con_escalar() {
        if !tiene_avx2() {
            return;
        }
        use crate::f16::f32_to_f16;
        // 2 filas × 512 cols de superbloques sintéticos variados
        let (rows, cols) = (2, 512);
        let blocks = rows * cols / 256;
        let mut bytes = Vec::new();
        for b in 0..blocks {
            bytes.extend_from_slice(&f32_to_f16(0.02 + b as f32 * 0.01).to_le_bytes());
            bytes.extend_from_slice(&f32_to_f16(0.005 * (b + 1) as f32).to_le_bytes());
            for j in 0..12 {
                bytes.push(((b * 37 + j * 11) % 251) as u8);
            }
            for j in 0..128 {
                bytes.push(((b * 3 + j * 7) % 256) as u8);
            }
        }
        let x: Vec<f32> = (0..cols).map(|i| ((i * 5 % 71) as f32 - 35.0) * 0.02).collect();
        let mut a = vec![0.0f32; rows];
        let mut b = vec![0.0f32; rows];
        matvec_q4_k_scalar(&bytes, rows, cols, &x, &mut a);
        unsafe { avx2::matvec_q4_k(&bytes, rows, cols, &x, &mut b) };
        casi_iguales(&a, &b);
        // y ambos contra dequant + matvec f32 de referencia
        let mut wq = vec![0.0f32; rows * cols];
        dequant_q4_k(&bytes, &mut wq).unwrap();
        let mut c = vec![0.0f32; rows];
        matvec_f32_scalar(&wq, rows, cols, &x, &mut c);
        casi_iguales(&a, &c);
    }
}

/// RoPE estilo llama sobre una cabeza (parejas intercaladas x[2i], x[2i+1]).
pub fn rope_inplace(x: &mut [f32], pos: usize, theta: f32) {
    let head_dim = x.len();
    let half = head_dim / 2;
    for i in 0..half {
        let freq = libm::powf(theta, -2.0 * (i as f32) / (head_dim as f32));
        let angle = pos as f32 * freq;
        let (sin, cos) = (libm::sinf(angle), libm::cosf(angle));
        let a = x[2 * i];
        let b = x[2 * i + 1];
        x[2 * i] = a * cos - b * sin;
        x[2 * i + 1] = a * sin + b * cos;
    }
}
