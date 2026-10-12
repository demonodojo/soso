//! Descarga modelos GGUF desde Hugging Face Hub dentro de soso.

#![no_std]
#![no_main]

extern crate alloc;

mod guest_io;
mod hf;
mod hf_range;
mod net;

use alloc::format;
use alloc::string::{String, ToString};
use guest_io::{ScratchFile, ScratchSink, SomImportOut};
use gguf2som::convert;
use libsoso::{println, sys};
use soso_abi::O_RDONLY;

libsoso::entry!(main);

fn parse_args(args: &str) -> alloc::vec::Vec<alloc::string::String> {
    args.split_whitespace().map(alloc::string::String::from).collect()
}

fn main(args: &str) -> u8 {
    let args = parse_args(args);
    if args.is_empty() {
        print_usage();
        return 2;
    }
    match args[0].as_str() {
        "pull" => cmd_pull(&args[1..]),
        "search" => cmd_search(&args[1..]),
        "list" => cmd_list(&args[1..]),
        other => {
            println!("soso-hf: subcomando desconocido {other}");
            print_usage();
            2
        }
    }
}

fn print_usage() {
    println!("uso:");
    println!("  soso-hf search <consulta> [--limit N] [--all]");
    println!("  soso-hf list <org/repo>");
    println!("  soso-hf pull <org/repo> [--file NAME.gguf] [--name NOMBRE]");
}

fn cmd_search(args: &[String]) -> u8 {
    let query = match args.first() {
        Some(q) if !q.is_empty() => q.as_str(),
        _ => {
            println!("uso: soso-hf search <consulta> [--limit N] [--all]");
            return 2;
        }
    };
    let mut limit = 20u32;
    let mut gguf_only = true;
    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--limit" => {
                i += 1;
                limit = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(20);
            }
            "--all" => gguf_only = false,
            other => {
                println!("soso-hf: opción desconocida {other}");
                return 2;
            }
        }
        i += 1;
    }
    let token = read_hf_token();
    let hits = match hf::fetch_search(query, limit, token.as_deref()) {
        Ok(h) => h,
        Err(e) => {
            println!("soso-hf: {e}");
            return 1;
        }
    };
    let hits: alloc::vec::Vec<_> = if gguf_only {
        hits.into_iter().filter(|h| h.has_gguf_tag()).collect()
    } else {
        hits
    };
    if hits.is_empty() {
        println!("(sin resultados)");
        return 0;
    }
    for h in hits {
        println!("{}  {} descargas", h.id, h.downloads);
    }
    0
}

fn cmd_list(args: &[String]) -> u8 {
    let repo = match args.first() {
        Some(r) => r.as_str(),
        None => {
            println!("uso: soso-hf list <org/repo>");
            return 2;
        }
    };
    let slug = repo.trim().trim_end_matches('/');
    if !slug.contains('/') {
        println!("soso-hf: el repo debe ser org/nombre");
        return 2;
    }
    let token = read_hf_token();
    let entries = match hf::fetch_tree(slug, token.as_deref()) {
        Ok(e) => e,
        Err(e) => {
            println!("soso-hf: {e}");
            return 1;
        }
    };
    let ggufs: alloc::vec::Vec<_> = entries.iter().filter(|e| e.path.ends_with(".gguf")).collect();
    if ggufs.is_empty() {
        println!("soso-hf: {slug} no contiene ficheros .gguf");
        return 0;
    }
    for e in ggufs {
        println!("{}  ({} B)", e.path, e.size);
    }
    0
}

fn cmd_pull(args: &[String]) -> u8 {
    if args.is_empty() {
        println!("uso: soso-hf pull <org/repo> [--file NAME.gguf] [--name NOMBRE]");
        return 2;
    }
    let repo = &args[0];
    let mut file: Option<String> = None;
    let mut name: Option<String> = None;
    let mut i = 1usize;
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

fn abort_import() {
    let _ = sys::som_scratch_free();
    let _ = sys::som_abort();
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
    let gguf_size = entries
        .iter()
        .find(|e| e.path == chosen)
        .map(|e| e.size)
        .unwrap_or(0);
    if gguf_size == 0 {
        println!("soso-hf: tamaño desconocido para {chosen}");
        return Err(1);
    }
    let model_name = name
        .map(String::from)
        .unwrap_or_else(|| hf::default_name(chosen.rsplit('/').next().unwrap_or(&chosen)));

    if sys::som_begin(&model_name) < 0 {
        println!("soso-hf: som_begin falló (¿modelo duplicado o import en curso?)");
        return Err(1);
    }

    let start_lba = sys::som_scratch_alloc(gguf_size);
    if start_lba < 0 {
        println!("soso-hf: sin espacio en sosomfs (errno {start_lba})");
        abort_import();
        return Err(1);
    }

    let url = format!(
        "https://huggingface.co/{slug}/resolve/main/{}",
        chosen.trim_start_matches('/')
    );

    // Camino preferido: HTTP Range (sin materializar GGUF completo en scratch).
    if hf_range::convert_from_url(&url, token.as_deref(), &model_name, gguf_size).is_ok() {
        let _ = sys::som_scratch_free();
        if sys::som_commit() < 0 {
            println!("soso-hf: som_commit falló");
            abort_import();
            return Err(1);
        }
        println!("soso-hf: listo en /models/{model_name} — soso-llm run {model_name} --prompt hola");
        return Ok(());
    }

    println!("soso-hf: descargando {url}…");
    let sink = ScratchSink::new(start_lba as u64);
    if net::download_to_scratch(&url, token.as_deref(), sink).is_err() {
        println!("soso-hf: descarga falló");
        abort_import();
        return Err(1);
    }

    println!("soso-hf: convirtiendo a .som en /models/{model_name}…");
    let mut src = ScratchFile::new(start_lba as u64, gguf_size);
    let mut out = SomImportOut;
    if convert(&mut src, &mut out, Some(model_name.as_str())).is_err() {
        println!("soso-hf: conversión falló");
        abort_import();
        return Err(1);
    }

    let _ = sys::som_scratch_free();
    if sys::som_commit() < 0 {
        println!("soso-hf: som_commit falló");
        abort_import();
        return Err(1);
    }
    println!("soso-hf: listo en /models/{model_name} — soso-llm run {model_name} --prompt hola");
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
