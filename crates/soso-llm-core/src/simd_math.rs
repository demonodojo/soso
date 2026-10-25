//! Funciones matemáticas vectoriales (AVX2+FMA) compartidas por la atención y la FFN.
//!
//! Cada lane se calcula con las mismas operaciones que las demás: el resultado de un elemento no
//! depende de su posición en el vector ni de cuántos elementos viajan con él.

#[cfg(all(target_arch = "x86_64", target_feature = "avx2", target_feature = "fma"))]
pub(crate) mod avx {
    use core::arch::x86_64::*;

    /// exp(x) para x ≤ 88 (se satura en −87). Error relativo < 2e-7.
    #[inline]
    #[target_feature(enable = "avx2,fma")]
    pub(crate) unsafe fn exp256(x: __m256) -> __m256 {
        let x = _mm256_min_ps(_mm256_max_ps(x, _mm256_set1_ps(-87.0)), _mm256_set1_ps(88.0));
        let fx = _mm256_round_ps(
            _mm256_mul_ps(x, _mm256_set1_ps(core::f32::consts::LOG2_E)),
            _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC,
        );
        // r = x - fx·ln2 en dos pasos (ln2 alto y bajo) para no perder precisión.
        let r = _mm256_fmadd_ps(fx, _mm256_set1_ps(-0.693_145_75), x);
        let r = _mm256_fmadd_ps(fx, _mm256_set1_ps(-1.428_606_8e-6), r);
        // Horner de Taylor hasta grado 7 en |r| ≤ ln2/2.
        let mut p = _mm256_set1_ps(1.0 / 5040.0);
        p = _mm256_fmadd_ps(p, r, _mm256_set1_ps(1.0 / 720.0));
        p = _mm256_fmadd_ps(p, r, _mm256_set1_ps(1.0 / 120.0));
        p = _mm256_fmadd_ps(p, r, _mm256_set1_ps(1.0 / 24.0));
        p = _mm256_fmadd_ps(p, r, _mm256_set1_ps(1.0 / 6.0));
        p = _mm256_fmadd_ps(p, r, _mm256_set1_ps(0.5));
        p = _mm256_fmadd_ps(p, r, _mm256_set1_ps(1.0));
        p = _mm256_fmadd_ps(p, r, _mm256_set1_ps(1.0));
        // 2^fx por el exponente.
        let n = _mm256_cvtps_epi32(fx);
        let pot = _mm256_castsi256_ps(_mm256_slli_epi32(_mm256_add_epi32(n, _mm256_set1_epi32(127)), 23));
        _mm256_mul_ps(p, pot)
    }

    /// silu(x) = x / (1 + exp(−x)).
    #[inline]
    #[target_feature(enable = "avx2,fma")]
    pub(crate) unsafe fn silu256(x: __m256) -> __m256 {
        unsafe {
            let e = exp256(_mm256_sub_ps(_mm256_setzero_ps(), x));
            _mm256_div_ps(x, _mm256_add_ps(_mm256_set1_ps(1.0), e))
        }
    }

    /// `up[i] = silu(gate[i]) * up[i]`; la cola (< 8) usa la misma aritmética vectorial.
    #[target_feature(enable = "avx2,fma")]
    pub(crate) unsafe fn swiglu(up: &mut [f32], gate: &[f32]) {
        unsafe {
            let n = up.len().min(gate.len());
            let mut i = 0usize;
            while i + 8 <= n {
                let g = _mm256_loadu_ps(gate.as_ptr().add(i));
                let u = _mm256_loadu_ps(up.as_ptr().add(i));
                _mm256_storeu_ps(up.as_mut_ptr().add(i), _mm256_mul_ps(silu256(g), u));
                i += 8;
            }
            if i < n {
                let (mut g8, mut u8_) = ([0.0f32; 8], [0.0f32; 8]);
                let r = n - i;
                g8[..r].copy_from_slice(&gate[i..n]);
                u8_[..r].copy_from_slice(&up[i..n]);
                let o = _mm256_mul_ps(silu256(_mm256_loadu_ps(g8.as_ptr())), _mm256_loadu_ps(u8_.as_ptr()));
                let mut tmp = [0.0f32; 8];
                _mm256_storeu_ps(tmp.as_mut_ptr(), o);
                up[i..n].copy_from_slice(&tmp[..r]);
            }
        }
    }

    /// `x[i] = silu(x[i])`.
    #[target_feature(enable = "avx2,fma")]
    pub(crate) unsafe fn silu_slice(x: &mut [f32]) {
        unsafe {
            let n = x.len();
            let mut i = 0usize;
            while i + 8 <= n {
                let v = _mm256_loadu_ps(x.as_ptr().add(i));
                _mm256_storeu_ps(x.as_mut_ptr().add(i), silu256(v));
                i += 8;
            }
            if i < n {
                let mut t = [0.0f32; 8];
                let r = n - i;
                t[..r].copy_from_slice(&x[i..n]);
                let o = silu256(_mm256_loadu_ps(t.as_ptr()));
                _mm256_storeu_ps(t.as_mut_ptr(), o);
                x[i..n].copy_from_slice(&t[..r]);
            }
        }
    }
}
