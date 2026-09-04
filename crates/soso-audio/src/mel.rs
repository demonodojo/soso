//! Banco de filtros mel estilo Slaney (Whisper / librosa htk=False).

use alloc::vec;
use alloc::vec::Vec;

/// Ventana de Hann de longitud `n`.
pub fn hann_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            0.5 * (1.0 - libm::cosf(2.0 * core::f32::consts::PI * i as f32 / (n - 1) as f32))
        })
        .collect()
}

fn hz_to_mel_slaney(hz: f32) -> f32 {
    let f_min = 0.0f32;
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0f32;
    let min_log_mel = (min_log_hz - f_min) / f_sp;
    let logstep = libm::logf(6.4) / 27.0;
    if hz >= min_log_hz {
        min_log_mel + libm::logf(hz / min_log_hz) / logstep
    } else {
        (hz - f_min) / f_sp
    }
}

fn mel_to_hz_slaney(mel: f32) -> f32 {
    let f_min = 0.0f32;
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0f32;
    let min_log_mel = (min_log_hz - f_min) / f_sp;
    let logstep = libm::logf(6.4) / 27.0;
    if mel >= min_log_mel {
        min_log_hz * libm::expf(logstep * (mel - min_log_mel))
    } else {
        f_min + f_sp * mel
    }
}

/// Matriz `[n_mels, n_fft/2+1]` aplanada fila a fila.
pub fn mel_filters(n_mels: usize, n_fft: usize, sample_rate: u32, fmax: f32) -> Vec<f32> {
    let n_fft_bins = n_fft / 2 + 1;
    let fmin = 0.0f32;
    let mel_min = hz_to_mel_slaney(fmin);
    let mel_max = hz_to_mel_slaney(fmax);
    let mut mels = Vec::with_capacity(n_mels + 2);
    for i in 0..(n_mels + 2) {
        let mel = mel_min + (mel_max - mel_min) * i as f32 / (n_mels + 1) as f32;
        mels.push(mel_to_hz_slaney(mel));
    }
    let mut filters = vec![0.0f32; n_mels * n_fft_bins];
    for m in 0..n_mels {
        let f_left = mels[m];
        let f_center = mels[m + 1];
        let f_right = mels[m + 2];
        for k in 0..n_fft_bins {
            let freq = k as f32 * sample_rate as f32 / n_fft as f32;
            let mut w = 0.0f32;
            if freq >= f_left && freq <= f_center && f_center > f_left {
                w = (freq - f_left) / (f_center - f_left);
            } else if freq > f_center && freq <= f_right && f_right > f_center {
                w = (f_right - freq) / (f_right - f_center);
            }
            filters[m * n_fft_bins + k] = w;
        }
    }
    // Slaney: escala por ancho de banda del filtro.
    for m in 0..n_mels {
        let enorm = 2.0 / (mels[m + 2] - mels[m]);
        for k in 0..n_fft_bins {
            filters[m * n_fft_bins + k] *= enorm;
        }
    }
    filters
}

pub fn apply_mel(filters: &[f32], power: &[f32], out: &mut [f32]) {
    let n_mels = out.len();
    let n_bins = power.len();
    assert_eq!(filters.len(), n_mels * n_bins);
    for m in 0..n_mels {
        let row = &filters[m * n_bins..(m + 1) * n_bins];
        let mut acc = 0.0f32;
        for (f, p) in row.iter().zip(power) {
            acc += f * p;
        }
        out[m] = acc;
    }
}
