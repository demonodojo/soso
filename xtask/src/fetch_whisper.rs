//! Descarga ggml-tiny.bin (streaming a disco) y convierte a `.som`.
//!
//! Uso: `cargo xtask fetch-whisper [--convert-only]`
//!
//! La descarga usa `curl -C -` (reanudable); no carga el bin en RAM.

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

const URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin";

pub fn run(args: &[String]) {
    let root = super::project_root();
    let convert_only = args.iter().any(|a| a == "--convert-only");
    let bin = root.join("target/ggml-tiny.bin");
    let out = root.join("target/whisper-tiny-model");

    if !convert_only {
        if let Some(parent) = bin.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        println!("fetch-whisper: descargando {URL}");
        println!("fetch-whisper: destino {}", bin.display());
        let status = Command::new("curl")
            .args(["-fL", "-C", "-", URL, "-o"])
            .arg(&bin)
            .status()
            .unwrap_or_else(|e| {
                eprintln!("fetch-whisper: curl no disponible: {e}");
                exit(1);
            });
        if !status.success() {
            eprintln!("fetch-whisper: descarga falló");
            exit(status.code().unwrap_or(1));
        }
        let mb = bin
            .metadata()
            .map(|m| m.len() as f64 / (1024.0 * 1024.0))
            .unwrap_or(0.0);
        println!("fetch-whisper: {mb:.1} MiB en {}", bin.display());
    } else if !bin.exists() {
        eprintln!(
            "fetch-whisper: no existe {} (omite --convert-only o descarga antes)",
            bin.display()
        );
        exit(1);
    }

    println!("fetch-whisper: convirtiendo → {}", out.display());
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["run", "--release", "-p", "convert-whisper", "--"])
        .args([bin.to_str().unwrap(), out.to_str().unwrap()])
        .status()
        .expect("convert-whisper");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
    println!(
        "fetch-whisper: listo. Empaqueta con SOSO_ASR_DIR={} cargo xtask mkfs",
        out.display()
    );
}

/// Directorio ASR preferido: env → whisper-tiny convertido → sintético.
pub fn asr_model_dir(root: &Path) -> PathBuf {
    if let Some(p) = std::env::var_os("SOSO_ASR_DIR").map(PathBuf::from) {
        return p;
    }
    let whisper = root.join("target/whisper-tiny-model");
    if whisper.join("manifest.som").exists() {
        return whisper;
    }
    root.join("target/tiny-asr-model")
}
