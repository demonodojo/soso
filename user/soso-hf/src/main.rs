//! Descarga modelos GGUF desde Hugging Face Hub dentro de soso.

#![no_std]
#![no_main]

extern crate alloc;

mod guest_io;
mod hf;
mod net;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use gguf2som::{convert, SomOut};
use guest_io::{GuestFile, GuestSomOut};
use libsoso::{println, sys};
use soso_abi::O_RDONLY;

libsoso::entry!(main);

fn parse_args(args: &str) -> alloc::vec::Vec<alloc::string::String> {
    args.split_whitespace().map(alloc::string::String::from).collect()
}

fn main(args: &str) -> u8 {
    let args = parse_args(args);
    if args.len() < 2 {
        println!("uso: soso-hf pull <org/repo> [--file NAME.gguf] [--name NOMBRE]");
        return 2;
    }
    if args[0] != "pull" {
        println!("soso-hf: subcomando desconocido (solo pull)");
        return 2;
    }
    let repo = &args[1];
    let mut file: Option<String> = None;
    let mut name: Option<String> = None;
    let mut i = 2usize;
    while i < args.len() {
        match args[i].as_str() {
            "--file" => {
                i += 1;
                file = Some(args.get(i).cloned().unwrap_or_default());
            }
            "--name" => {
                i += 1;
                name = Some(args.get(i).cloned().unwrap_or_default());
            }
            other => {
                println!("soso-hf: opción desconocida {other}");
                return 2;
            }
        }
        i += 1;
    }
    if let Err(code) = pull(repo, file.as_deref(), name.as_deref()) {
        return code;
    }
    0
}

fn pull(repo: &str, file: Option<&str>, name: Option<&str>) -> Result<(), u8> {
    let slug = repo.trim().trim_end_matches('/');
    if !slug.contains('/') {
        println!("soso-hf: el repo debe ser org/nombre");
        return Err(2);
    }
    let token = read_hf_token();
    let entries = hf::fetch_tree(slug, token.as_deref()).map_err(|e| {
        println!("soso-hf: {e}");
        1
    })?;
    let chosen = hf::pick_gguf(&entries, file).map_err(|e| {
        println!("soso-hf: {e}");
        1
    })?;
    let model_name = name
        .map(String::from)
        .unwrap_or_else(|| hf::default_name(chosen.rsplit('/').next().unwrap_or(&chosen)));
    let base = format!("/var/models/{model_name}");
    let gguf_path = format!("/tmp/soso-hf-{model_name}.gguf");
    let _ = sys::mkdir("/var");
    let _ = sys::mkdir("/var/models");
    let _ = sys::mkdir(&base);

    let url = format!(
        "https://huggingface.co/{slug}/resolve/main/{}",
        chosen.trim_start_matches('/')
    );
    println!("soso-hf: descargando {url}…");
    net::download_url(&url, &gguf_path, token.as_deref()).map_err(|e| {
        println!("soso-hf: descarga falló ({e:?})");
        1
    })?;

    println!("soso-hf: convirtiendo a .som en {base}…");
    let mut src = GuestFile::open(&gguf_path).map_err(|_| {
        println!("soso-hf: no puedo abrir {gguf_path}");
        1
    })?;
    let mut out = GuestSomOut::new(&base);
    convert(&mut src, &mut out, Some(model_name.as_str())).map_err(|e| {
        println!("soso-hf: conversión falló: {e}");
        1
    })?;
    let _ = sys::unlink(&gguf_path);
    println!("soso-hf: listo — soso-llm run {model_name} --prompt hola");
    Ok(())
}

fn read_hf_token() -> Option<String> {
    let mut buf = [0u8; 256];
    let fd = sys::open("/etc/hf_token", O_RDONLY);
    if fd < 0 {
        return None;
    }
    let n = sys::read(fd as u64, &mut buf);
    let _ = sys::close(fd as u64);
    if n <= 0 {
        return None;
    }
    let s = core::str::from_utf8(&buf[..n as usize]).ok()?;
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}
