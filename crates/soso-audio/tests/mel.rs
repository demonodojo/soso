//! Tests host del frontend mel.

use soso_audio::{log_mel_spectrogram, N_FFT, N_MELS, SAMPLE_RATE};

fn sine_samples(freq: f32, secs: f32) -> Vec<f32> {
    let n = (secs * SAMPLE_RATE as f32) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            libm::sinf(2.0 * core::f32::consts::PI * freq * t)
        })
        .collect()
}

#[test]
fn log_mel_sine_440hz_shape() {
    let samples = sine_samples(440.0, 0.5);
    let n_frames = 1 + (samples.len() + N_FFT / 2 - N_FFT) / 160;
    let mut out = vec![0.0f32; N_MELS * n_frames];
    let nf = log_mel_spectrogram(&samples, &mut out);
    assert_eq!(nf, n_frames);
    assert!(out.iter().all(|v| v.is_finite()));
    // Debe haber energía en bins medios-altos para 440 Hz.
    let mid_energy: f32 = out[20..40].iter().sum();
    let low_energy: f32 = out[0..5].iter().sum();
    assert!(mid_energy > low_energy);
}

#[test]
fn log_mel_empty() {
    let mut out = [0.0f32; N_MELS];
    assert_eq!(log_mel_spectrogram(&[], &mut out), 0);
}

#[test]
fn wav_roundtrip() {
    use soso_audio::wav::{parse_pcm16_mono, resample_to_16k};
    let samples = sine_samples(1000.0, 0.1);
    let pcm: Vec<i16> = samples
        .iter()
        .map(|s| (s * 32767.0).clamp(-32768.0, 32767.0) as i16)
        .collect();
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    let data_sz = (pcm.len() * 2 + 36) as u32;
    wav.extend_from_slice(&data_sz.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    let byte_rate = SAMPLE_RATE * 2;
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(pcm.len() as u32 * 2).to_le_bytes());
    for s in &pcm {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    let (rate, parsed) = parse_pcm16_mono(&wav).unwrap();
    assert_eq!(rate, SAMPLE_RATE);
    assert_eq!(parsed.len(), pcm.len());
    let f32s = resample_to_16k(rate, &parsed);
    assert_eq!(f32s.len(), pcm.len());
}

#[test]
fn fft_dc() {
    use soso_audio::fft::rfft_power;
    let samples = vec![1.0f32; 400];
    let mut power = vec![0.0f32; 201];
    rfft_power(&samples, &mut power);
    assert!(power[0] > 100.0);
}
