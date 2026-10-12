//! Parser mínimo de WAV PCM16 mono 16 kHz.

use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WavError {
    TooShort,
    BadRiff,
    BadFmt,
    Unsupported,
}

/// Lee PCM16 mono. Acepta cualquier sample rate (el caller puede remuestrear).
pub fn parse_pcm16_mono(data: &[u8]) -> Result<(u32, Vec<i16>), WavError> {
    if data.len() < 44 {
        return Err(WavError::TooShort);
    }
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(WavError::BadRiff);
    }
    let mut pos = 12usize;
    let mut sample_rate = 0u32;
    let mut bits = 0u16;
    let mut channels = 0u16;
    let mut pcm = Vec::new();
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let sz = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        pos += 8;
        if pos + sz > data.len() {
            break;
        }
        if id == b"fmt " && sz >= 16 {
            let audio_format = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
            channels = u16::from_le_bytes(data[pos + 2..pos + 4].try_into().unwrap());
            sample_rate = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap());
            bits = u16::from_le_bytes(data[pos + 14..pos + 16].try_into().unwrap());
            if audio_format != 1 {
                return Err(WavError::Unsupported);
            }
        } else if id == b"data" {
            if bits != 16 || channels != 1 {
                return Err(WavError::Unsupported);
            }
            let bytes = &data[pos..pos + sz];
            pcm.reserve(bytes.len() / 2);
            for chunk in bytes.chunks_exact(2) {
                pcm.push(i16::from_le_bytes([chunk[0], chunk[1]]));
            }
        }
        pos += sz + (sz & 1);
    }
    if pcm.is_empty() || sample_rate == 0 {
        return Err(WavError::BadFmt);
    }
    Ok((sample_rate, pcm))
}

/// Remuestreo lineal simple a 16 kHz (suficiente para tests).
pub fn resample_to_16k(input_rate: u32, samples: &[i16]) -> Vec<f32> {
    if input_rate == crate::SAMPLE_RATE {
        return samples.iter().map(|s| *s as f32 / 32768.0).collect();
    }
    let ratio = input_rate as f32 / crate::SAMPLE_RATE as f32;
    let out_len = libm::ceilf(samples.len() as f32 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f32 * ratio;
        let i0 = libm::floorf(src) as usize;
        let i1 = (i0 + 1).min(samples.len().saturating_sub(1));
        let t = src - i0 as f32;
        let s0 = samples.get(i0).copied().unwrap_or(0) as f32 / 32768.0;
        let s1 = samples.get(i1).copied().unwrap_or(0) as f32 / 32768.0;
        out.push(s0 * (1.0 - t) + s1 * t);
    }
    out
}
