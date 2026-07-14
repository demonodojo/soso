//! GEMM CPU escalar (AVX2 en fases posteriores con target-feature).

pub fn matvec_f32(matrix: &[f32], rows: usize, cols: usize, x: &[f32], out: &mut [f32]) {
    assert_eq!(matrix.len(), rows * cols);
    assert_eq!(x.len(), cols);
    assert_eq!(out.len(), rows);
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
