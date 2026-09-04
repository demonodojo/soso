//! Runner host ASR: transcribe un WAV PCM16 mono con un modelo .som ASR.
//!
//!   cargo run --release -p soso-llm-core --features std --example asrrun -- \
//!     target/tiny-asr-model /ruta/audio.wav [lang_token]

use soso_audio::{log_mel_spectrogram, wav, whisper_mel_frame_count, N_MELS};
use soso_llm_core::asr::{AsrProfile, AsrRuntime};
use soso_llm_core::source::{FileMapper, MappedShard, MmapTensorSource};
use soso_llm_core::tokenizer::Tokenizer;
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;

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

fn main() {
    let mut args = std::env::args().skip(1);
    let model = args.next().expect("uso: asrrun <modelo-dir> <wav> [lang_token]");
    let wav_path = args.next().expect("uso: asrrun <modelo-dir> <wav> [lang_token]");
    let lang: u32 = args.next().map(|s| s.parse().expect("lang_token")).unwrap_or(3);

    let base = model.trim_end_matches('/');
    let manifest = Manifest::parse(&std::fs::read(format!("{base}/manifest.som")).unwrap()).unwrap();
    let index = TensorIndex::parse(&std::fs::read(format!("{base}/index.som")).unwrap()).unwrap();
    let tokenizer = match std::fs::read(format!("{base}/tokenizer.som")) {
        Ok(d) => Tokenizer::parse(&d).expect("tokenizer.som inválido"),
        Err(_) => Tokenizer::byte_level(),
    };
    let source = MmapTensorSource::new(format!("{base}/shards"), index.clone(), StdMapper)
        .with_sync_prefetch();
    let mut rt = AsrRuntime::new(manifest, index, source, tokenizer).unwrap();
    rt.profile = Some(AsrProfile::new(true));

    let data = std::fs::read(wav_path).expect("wav");
    let (rate, pcm) = wav::parse_pcm16_mono(&data).expect("parse wav");
    let samples = wav::resample_to_16k(rate, &pcm);
    let cap = whisper_mel_frame_count(samples.len()).max(1);
    let mut mel = vec![0.0f32; N_MELS * cap];
    let n_frames = log_mel_spectrogram(&samples, &mut mel);
    let text = rt.transcribe(&mel, n_frames, lang).expect("transcribe");
    if let Some(ref prof) = rt.profile {
        eprint!("{}", prof.format_phase_summary());
    }
    println!("{text}");
}
