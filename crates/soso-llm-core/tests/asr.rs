//! Test ASR con modelo sintético.

use soso_llm_core::asr::{AsrProfile, AsrRuntime};
use soso_llm_core::source::{FileMapper, MappedShard, MmapTensorSource};
use soso_llm_core::tokenizer::Tokenizer;
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;
use std::path::PathBuf;

struct HostMapper;

impl FileMapper for HostMapper {
    fn map_file(&mut self, path: &str) -> Result<MappedShard, ()> {
        let data = std::fs::read(path).map_err(|_| ())?;
        let len = data.len();
        let ptr = Box::leak(data.into_boxed_slice()).as_ptr();
        Ok(MappedShard {
            addr: ptr as u64,
            len,
        })
    }

    fn unmap_file(&mut self, _shard: &MappedShard) {}
}

#[test]
fn tiny_asr_encode_decode_smoke() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tiny-asr-model");
    if !root.join("manifest.som").exists() {
        eprintln!("skip: ejecuta mkmodel-soso --asr primero");
        return;
    }
    let manifest = Manifest::parse(&std::fs::read(root.join("manifest.som")).unwrap()).unwrap();
    let index = TensorIndex::parse(&std::fs::read(root.join("index.som")).unwrap()).unwrap();
    let source = MmapTensorSource::new(
        format!("{}/shards", root.display()),
        index.clone(),
        HostMapper,
    )
    .with_sync_prefetch();
    let mut rt = AsrRuntime::new(manifest, index, source, Tokenizer::byte_level()).unwrap();
    let n_frames = 10usize;
    let mel = vec![0.1f32; 80 * n_frames];
    let enc = rt.encode(&mel, n_frames).expect("encode");
    assert!(!enc.is_empty());
    let enc_seq = enc.len() / rt.manifest.audio.n_audio_state as usize;
    rt.profile = Some(AsrProfile::new(true));
    let _text = rt.decode_greedy(&enc, enc_seq, &[50]).expect("decode");
    if let Some(ref prof) = rt.profile {
        assert!(!prof.tokens.is_empty(), "perfil ASR vacío");
        for t in &prof.tokens {
            assert!(
                t.matvec_calls <= 30,
                "demasiados matvec por token: {} (seq={})",
                t.matvec_calls,
                t.seq
            );
        }
    }
}

#[test]
fn whisper_tiny_wav_reference() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/whisper-tiny-model");
    let wav = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rootfs/etc/voz-prueba.wav");
    if !root.join("manifest.som").exists() {
        eprintln!("skip: cargo xtask fetch-whisper");
        return;
    }
    if !wav.exists() {
        eprintln!("skip: falta rootfs/etc/voz-prueba.wav");
        return;
    }
    let manifest = Manifest::parse(&std::fs::read(root.join("manifest.som")).unwrap()).unwrap();
    let index = TensorIndex::parse(&std::fs::read(root.join("index.som")).unwrap()).unwrap();
    let tokenizer = match std::fs::read(root.join("tokenizer.som")) {
        Ok(d) => Tokenizer::parse(&d).expect("tokenizer"),
        Err(_) => Tokenizer::byte_level(),
    };
    let source = MmapTensorSource::new(
        format!("{}/shards", root.display()),
        index.clone(),
        HostMapper,
    )
    .with_sync_prefetch();
    let mut rt = AsrRuntime::new(manifest, index, source, tokenizer).unwrap();
    let data = std::fs::read(wav).unwrap();
    let (rate, pcm) = soso_audio::wav::parse_pcm16_mono(&data).unwrap();
    let samples = soso_audio::wav::resample_to_16k(rate, &pcm);
    let cap = soso_audio::whisper_mel_frame_count(samples.len()).max(1);
    let mut mel = vec![0.0f32; soso_audio::N_MELS * cap];
    let n_frames = soso_audio::log_mel_spectrogram(&samples, &mut mel);
    rt.profile = Some(AsrProfile::new(true));
    let text = rt.transcribe(&mel, n_frames, 3).expect("transcribe");
    assert!(!text.trim().is_empty(), "transcripción vacía");
    let lower = text.to_lowercase();
    assert!(
        lower.contains("música") || lower.contains("musica"),
        "transcripción inesperada: {text:?}"
    );
    if let Some(ref prof) = rt.profile {
        for t in &prof.tokens {
            assert!(
                t.matvec_calls <= 30,
                "decode incremental: {} matvec en seq={}",
                t.matvec_calls,
                t.seq
            );
        }
    }
}
