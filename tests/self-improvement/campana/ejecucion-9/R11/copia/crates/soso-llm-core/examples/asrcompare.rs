//! Compara mel, encoder y tokens soso vs whisper.cpp en un WAV.
//!
//!   cargo run --release -p soso-llm-core --features std --example asrcompare -- \
//!     target/whisper-tiny-model rootfs/etc/voz-prueba.wav [max_new_tokens] [lang_index]

use soso_audio::{log_mel_spectrogram, wav, whisper_mel_context_frames, whisper_mel_frame_count, N_MELS};
use soso_llm_core::asr::AsrRuntime;
use soso_llm_core::source::{FileMapper, MappedShard, MmapTensorSource};
use soso_llm_core::tokenizer::Tokenizer;
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;
use std::process::Command;

struct StdMapper;

impl FileMapper for StdMapper {
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

fn mel_stats(mel: &[f32], n_frames: usize) -> (f32, f32, f32) {
    let n = N_MELS * n_frames;
    if n == 0 || mel.len() < n {
        return (0.0, 0.0, 0.0);
    }
    let slice = &mel[..n];
    let min = slice.iter().copied().fold(f32::INFINITY, f32::min);
    let max = slice.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mean = slice.iter().sum::<f32>() / n as f32;
    (min, max, mean)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let model = args
        .next()
        .expect("uso: asrcompare <modelo-dir> <wav> [max_new_tokens] [lang_index]");
    let wav_path = args.next().expect("uso: asrcompare <modelo-dir> <wav> [max_new_tokens] [lang_index]");
    let max_new: Option<usize> = args.next().map(|s| s.parse().expect("max_new_tokens"));
    let lang: u32 = args.next().map(|s| s.parse().expect("lang_index")).unwrap_or(3);

    let base = model.trim_end_matches('/');
    let manifest = Manifest::parse(&std::fs::read(format!("{base}/manifest.som")).unwrap()).unwrap();
    let index = TensorIndex::parse(&std::fs::read(format!("{base}/index.som")).unwrap()).unwrap();
    let tokenizer = match std::fs::read(format!("{base}/tokenizer.som")) {
        Ok(d) => Tokenizer::parse(&d).expect("tokenizer.som inválido"),
        Err(_) => Tokenizer::byte_level(),
    };
    let source = MmapTensorSource::new(format!("{base}/shards"), index.clone(), StdMapper)
        .with_sync_prefetch();
    let mut rt = AsrRuntime::new(manifest.clone(), index, source, tokenizer).unwrap();

    let data = std::fs::read(&wav_path).expect("wav");
    let (rate, pcm) = wav::parse_pcm16_mono(&data).expect("parse wav");
    let samples = wav::resample_to_16k(rate, &pcm);
    let raw_frames = whisper_mel_frame_count(samples.len());
    let ctx_frames = whisper_mel_context_frames(manifest.audio.n_audio_ctx as usize);
    let cap = ctx_frames.max(raw_frames);
    let mut mel = vec![0.0f32; N_MELS * cap.max(1)];
    let n_frames = log_mel_spectrogram(&samples, &mut mel);
    let (mmin, mmax, mmean) = mel_stats(&mel, n_frames);
    println!("=== mel soso ===");
    println!("samples={} raw_frames={n_frames} ctx_target={ctx_frames}", samples.len());
    println!("stats min={mmin:.4} max={mmax:.4} mean={mmean:.4}");
    println!("mel[0..5]: {:?}", &mel[..5.min(mel.len())]);

    let enc = rt.encode(&mel, n_frames).expect("encode");
    let enc_seq = enc.len() / manifest.audio.n_audio_state as usize;
    let enc_norm: f32 = enc.iter().map(|x| x * x).sum::<f32>().sqrt();
    println!("=== encoder soso ===");
    println!("enc_seq={enc_seq} len={} l2={enc_norm:.4}", enc.len());
    println!("enc[0..8]: {:?}", &enc[..8.min(enc.len())]);

    let (text, all_tokens, new_tokens) = rt
        .transcribe_tokens(&mel, n_frames, lang, max_new)
        .expect("transcribe");

    println!("=== decode soso ===");
    println!("texto: {text:?}");
    println!("prompt+gen: {all_tokens:?}");
    println!("generados: {new_tokens:?}");
    print!("piezas: ");
    for &t in &all_tokens {
        let mut b = Vec::new();
        rt.tokenizer.token_bytes(t, &mut b);
        print!("{t}={:?} ", String::from_utf8_lossy(&b));
    }
    println!();
    let ref_enc = rt.tokenizer.encode_trozo("MÚSICA", false, true);
    println!("tokenizer ref MÚSICA (prefijo): {ref_enc:?}");

    let ggml = std::path::Path::new("target/ggml-tiny.bin");
    if !ggml.exists() {
        println!("(sin target/ggml-tiny.bin — omite whisper.cpp)");
        return;
    }
    let whisper_cli = std::env::var("WHISPER_CLI").unwrap_or_else(|_| {
        "/tmp/whisper.cpp/build/bin/whisper-cli".into()
    });
    let out = Command::new(&whisper_cli)
        .args([
            "-m",
            ggml.to_str().unwrap(),
            "-f",
            &wav_path,
            "-l",
            "es",
            "-nt",
            "-np",
        ])
        .output()
        .expect("whisper-cli");
    let ref_text = String::from_utf8_lossy(&out.stdout)
        .replace('\u{1b}', "")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect::<String>()
        .trim()
        .to_string();
    println!("=== whisper.cpp ===");
    println!("texto: {ref_text:?}");
    if !out.stderr.is_empty() {
        eprintln!(
            "whisper stderr: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
}
