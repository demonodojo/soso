//! Convierte modelos whisper.cpp (.bin ggml) a layout .som.

mod emit;
mod ggml;
mod map;
mod tokens;

use ggml::WhisperBin;
use map::{map_tensor, MapMode};
use sosomodel::layout::{INDEX_FILE, MANIFEST_FILE, SHARDS_DIR};
use sosomodel::manifest::Manifest;
use soso_llm_core::tokenizer::VocabTokenizer;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

fn main() {
    let mut args = env::args().skip(1);
    let Some(input) = args.next() else {
        eprintln!("uso: convert-whisper <modelo.bin> [dir_salida] [--name whisper-tiny]");
        exit(1);
    };
    let mut out = PathBuf::from("target/whisper-tiny-model");
    let mut name = String::from("whisper-tiny");
    for a in args {
        if let Some(n) = a.strip_prefix("--name=") {
            name = n.to_string();
        } else if a == "--name" {
            continue;
        } else if !a.starts_with('-') {
            out = PathBuf::from(a);
        }
    }
    if !Path::new(&input).exists() {
        eprintln!("convert-whisper: no existe {input}");
        exit(1);
    }
    if let Err(e) = convert(&input, &out, &name) {
        eprintln!("convert-whisper: {e}");
        exit(1);
    }
    let mb = fs::metadata(out.join("index.som"))
        .map(|_| dir_size(&out) as f64 / (1024.0 * 1024.0))
        .unwrap_or(0.0);
    println!(
        "convert-whisper: {} → {} ({mb:.1} MiB)",
        input,
        out.display()
    );
    println!("SOSO_ASR_DIR={} cargo xtask mkfs", out.display());
}

fn dir_size(p: &Path) -> u64 {
    let mut n = 0u64;
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                n += dir_size(&path);
            } else if let Ok(m) = e.metadata() {
                n += m.len();
            }
        }
    }
    n
}

fn format_ne(ne: &[i32]) -> String {
    ne.iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join("×")
}

fn convert(input: &str, out: &Path, name: &str) -> Result<(), String> {
    let _ = fs::remove_dir_all(out);
    fs::create_dir_all(out.join(SHARDS_DIR)).map_err(|e| e.to_string())?;

    let mut bin = WhisperBin::open(Path::new(input))?;
    let hp = bin.hparams.clone();
    let tensor_list: Vec<_> = bin.tensors.clone();

    let mut emitter = emit::Emitter::new(out);
    for meta in &tensor_list {
        let Some(m) = map_tensor(&meta.name, &meta.ne) else {
            continue;
        };
        let reader = bin.tensor_data(meta)?;
        let emit = match m.mode {
            MapMode::PosEmbed => {
                let rows = m.shape[0];
                let cols = m.shape[1];
                emitter.emit_pos_or_embed(&m.som_name, rows, cols, reader, meta.ttype)
            }
            MapMode::Transpose2d => emitter.emit_f32_from_reader(
                &m.som_name,
                &m.shape,
                reader,
                meta.nbytes,
                meta.ttype,
                true,
            ),
            MapMode::Linear => emitter.emit_f32_from_reader(
                &m.som_name,
                &m.shape,
                reader,
                meta.nbytes,
                meta.ttype,
                false,
            ),
        };
        emit.map_err(|e| format!("{} ({}) → {}: {e}", meta.name, format_ne(&meta.ne), m.som_name))?;
    }

    let (index, total) = emitter.finish(out);

    let manifest = Manifest::whisper_asr(name, hp.n_vocab as u32);
    fs::write(out.join(MANIFEST_FILE), manifest.serialize()).map_err(|e| e.to_string())?;
    fs::write(out.join(INDEX_FILE), index.serialize()).map_err(|e| e.to_string())?;

    // Vocabulario: tokens GPT del bin + especiales Whisper 50257..n_vocab-1.
    let mut pieces = bin.vocab.clone();
    tokens::pad_whisper_special_tokens(&mut pieces, manifest.vocab_size as usize);
    let tok = VocabTokenizer::serialize(&pieces, tokens::SOT, tokens::EOT);
    fs::write(out.join("tokenizer.som"), tok).map_err(|e| e.to_string())?;

    eprintln!(
        "  hparams: audio ctx={} state={} heads={} layers={}, text ctx={} state={} heads={} layers={}, mels {}, vocab {}, ftype {}, {:.1} MiB tensores",
        hp.n_audio_ctx,
        hp.n_audio_state,
        hp.n_audio_head,
        hp.n_audio_layer,
        hp.n_text_ctx,
        hp.n_text_state,
        hp.n_text_head,
        hp.n_text_layer,
        hp.n_mels,
        hp.n_vocab,
        hp.ftype,
        total as f64 / (1024.0 * 1024.0)
    );
    Ok(())
}
