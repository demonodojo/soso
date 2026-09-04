//! Procesado de audio estilo Whisper: FFT, banco mel y log-mel normalizado.
//! `no_std` + feature `std` para tests en host.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod fft;
pub mod mel;
pub mod wav;

/// Parámetros de frontend Whisper (tiny/base).
pub const SAMPLE_RATE: u32 = 16_000;
pub const N_FFT: usize = 400;
pub const HOP_LENGTH: usize = 160;
pub const N_MELS: usize = 80;
pub const FMAX: f32 = 8000.0;

/// Número de frames mel efectivos (estilo whisper.cpp `n_len_org`).
pub fn whisper_mel_frame_count(n_samples: usize) -> usize {
    if n_samples == 0 {
        return 0;
    }
    1 + (n_samples + N_FFT / 2 - N_FFT) / HOP_LENGTH
}

/// Frames de contexto mel del encoder (`2 * n_audio_ctx`).
pub fn whisper_mel_context_frames(n_audio_ctx: usize) -> usize {
    n_audio_ctx * 2
}

/// Calcula log-mel `[n_mels, n_frames]` en row-major (mel major, frame minor).
/// Incluye pad reflectivo al inicio como whisper.cpp.
pub fn log_mel_spectrogram(samples: &[f32], out: &mut [f32]) -> usize {
    if samples.is_empty() {
        return 0;
    }
    let half = N_FFT / 2;
    let n_reflect = half.min(samples.len().saturating_sub(1));
    let padded_len = samples.len() + half;
    let n_frames = whisper_mel_frame_count(samples.len());
    assert!(out.len() >= N_MELS * n_frames);
    let mut padded = alloc::vec![0.0f32; padded_len];
    if n_reflect > 0 {
        for i in 0..n_reflect {
            padded[half - n_reflect + i] = samples[n_reflect - i];
        }
    }
    padded[half..half + samples.len()].copy_from_slice(samples);
    let window = mel::hann_window(N_FFT);
    let filters = mel::mel_filters(N_MELS, N_FFT, SAMPLE_RATE, FMAX);
    let mut frame = alloc::vec![0.0f32; N_FFT];
    let mut power = alloc::vec![0.0f32; N_FFT / 2 + 1];
    let mut mel_frame = [0.0f32; N_MELS];
    let mut max_log = f32::NEG_INFINITY;
    for fi in 0..n_frames {
        let start = fi * HOP_LENGTH;
        frame.fill(0.0);
        let end = (start + N_FFT).min(padded_len);
        if start < end {
            let n = end - start;
            for i in 0..n {
                frame[i] = padded[start + i] * window[i];
            }
        }
        fft::rfft_power(&frame, &mut power);
        mel::apply_mel(&filters, &power, &mut mel_frame);
        for m in 0..N_MELS {
            let v = mel_frame[m].max(1e-10);
            let log = libm::log10f(v);
            out[m * n_frames + fi] = log;
            max_log = max_log.max(log);
        }
    }
    let floor = max_log - 8.0;
    for v in out[..N_MELS * n_frames].iter_mut() {
        *v = (*v).max(floor);
        *v = (*v + 4.0) / 4.0;
    }
    n_frames
}

/// Energía RMS de una ventana PCM16 (para VAD).
pub fn rms_pcm16(pcm: &[i16]) -> f32 {
    if pcm.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0f64;
    for s in pcm {
        let f = *s as f64 / 32768.0;
        sum += f * f;
    }
    libm::sqrtf((sum / pcm.len() as f64) as f32)
}

/// PCM16 mono → f32 [-1, 1].
pub fn pcm16_to_f32(pcm: &[i16], out: &mut [f32]) {
    let n = pcm.len().min(out.len());
    for i in 0..n {
        out[i] = pcm[i] as f32 / 32768.0;
    }
}
