//! FFT radix-2 in-place (compleja) y potencia espectral para señales reales.

/// FFT radix-2 in-place sobre pares (re, im). `n` debe ser potencia de 2.
pub fn fft_inplace(data: &mut [(f32, f32)]) {
    let n = data.len();
    if n <= 1 {
        return;
    }
    // Bit-reversal
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            data.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * core::f32::consts::PI / len as f32;
        let wlen_re = libm::cosf(ang);
        let wlen_im = libm::sinf(ang);
        let mut i = 0;
        while i + len <= n {
            let mut w_re = 1.0f32;
            let mut w_im = 0.0f32;
            for k in 0..(len / 2) {
                let u = data[i + k];
                let v_re = data[i + k + len / 2].0 * w_re - data[i + k + len / 2].1 * w_im;
                let v_im = data[i + k + len / 2].0 * w_im + data[i + k + len / 2].1 * w_re;
                data[i + k].0 = u.0 + v_re;
                data[i + k].1 = u.1 + v_im;
                data[i + k + len / 2].0 = u.0 - v_re;
                data[i + k + len / 2].1 = u.1 - v_im;
                let nw_re = w_re * wlen_re - w_im * wlen_im;
                w_im = w_re * wlen_im + w_im * wlen_re;
                w_re = nw_re;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// Potencia |X[k]|² para k=0..n/2 a partir de muestras reales ventaneadas.
pub fn rfft_power(samples: &[f32], power: &mut [f32]) {
    let n = samples.len();
    assert!(power.len() >= n / 2 + 1);
    let mut buf = alloc::vec![(0.0f32, 0.0f32); n];
    for (i, &s) in samples.iter().enumerate() {
        buf[i].0 = s;
    }
    fft_inplace(&mut buf);
    for k in 0..=n / 2 {
        let (re, im) = buf[k];
        power[k] = re * re + im * im;
    }
}
