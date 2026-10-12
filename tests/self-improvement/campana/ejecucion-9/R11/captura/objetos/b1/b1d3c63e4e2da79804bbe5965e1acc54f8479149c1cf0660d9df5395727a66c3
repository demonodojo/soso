//! Control de versiones mínimo sobre `/src/soso` (Hito 4).
//!
//! Sustituto de `git` hasta integrar **gix** con la toolchain nativa.
//! Comandos: `status`, `log` (lista de hashes por fichero).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libsoso::{abi, println, sys};
use soso_update_core::hex_sha256;

libsoso::entry!(main);

const SRC: &str = "/src/soso";

fn leer(path: &str) -> Result<Vec<u8>, i64> {
    let fd = sys::open(path, abi::O_RDONLY);
    if fd < 0 {
        return Err(fd);
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n < 0 {
            sys::close(fd as u64);
            return Err(n);
        }
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    sys::close(fd as u64);
    Ok(out)
}

fn listar(dir: &str, out: &mut Vec<(String, String)>) {
    let fd = sys::open(dir, abi::O_RDONLY);
    if fd < 0 {
        return;
    }
    let mut ents = [abi::Dirent::default(); 4];
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n <= 0 {
            break;
        }
        for d in &ents[..n as usize / abi::DIRENT_SIZE] {
            let name = core::str::from_utf8(d.name_bytes()).unwrap_or("");
            if name == "." || name == ".." {
                continue;
            }
            let ruta = if dir == "/" {
                format!("/{name}")
            } else {
                format!("{dir}/{name}")
            };
            if d.file_type == abi::FT_DIR {
                listar(&ruta, out);
            } else if let Ok(data) = leer(&ruta) {
                let rel = ruta.strip_prefix(SRC).unwrap_or(&ruta).trim_start_matches('/');
                out.push((String::from(rel), hex_sha256(&data)));
            }
        }
    }
    sys::close(fd as u64);
}

fn cmd_status() -> u8 {
    let mut files = Vec::new();
    listar(SRC, &mut files);
    if files.is_empty() {
        println!("soso-git: sin ficheros bajo {SRC}");
        return 1;
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    for (rel, h) in &files {
        println!(" {rel}\t{h}");
    }
    println!("soso-git: {} ficheros", files.len());
    0
}

fn cmd_log() -> u8 {
    cmd_status()
}

fn cmd_diff() -> u8 {
    let cache = "/var/forja-cache";
    let mut files = Vec::new();
    listar(SRC, &mut files);
    if files.is_empty() {
        println!("soso-git: sin ficheros bajo {SRC}");
        return 1;
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut cambios = 0usize;
    for (rel, h) in &files {
        let stamp = format!("{cache}/{rel}.sha256");
        let prev = leer(&stamp).ok();
        let prev_s = prev.as_ref().and_then(|v| core::str::from_utf8(v).ok());
        if prev_s != Some(h.as_str()) {
            println!(" M {rel}");
            cambios += 1;
        }
    }
    if cambios == 0 {
        println!("soso-git: sin cambios respecto a {cache}/");
    } else {
        println!("soso-git: {cambios} cambios");
    }
    0
}

fn ensure_parent(path: &str) {
    let Some(i) = path.rfind('/') else {
        return;
    };
    let mut cur = String::new();
    for comp in path[..i].split('/') {
        if comp.is_empty() {
            continue;
        }
        if cur.is_empty() {
            cur.push('/');
            cur.push_str(comp);
        } else {
            cur.push('/');
            cur.push_str(comp);
        }
        let _ = sys::mkdir(&cur);
    }
}

fn cmd_commit() -> u8 {
    let cache = "/var/forja-cache";
    let _ = sys::mkdir(cache);
    let mut files = Vec::new();
    listar(SRC, &mut files);
    if files.is_empty() {
        println!("soso-git: sin ficheros bajo {SRC}");
        return 1;
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    for (rel, h) in &files {
        let stamp = format!("{cache}/{rel}.sha256");
        ensure_parent(&stamp);
        if escribir(&stamp, h.as_bytes()).is_err() {
            println!("soso-git: no se pudo escribir {stamp}");
            return 1;
        }
    }
    println!("soso-git: commit → {cache}/ ({} ficheros)", files.len());
    0
}

fn main(args: &str) -> u8 {
    let mut cmd = "help";
    for tok in args.split_whitespace() {
        cmd = tok;
        break;
    }
    match cmd {
        "status" => cmd_status(),
        "log" => cmd_log(),
        "diff" => cmd_diff(),
        "commit" => cmd_commit(),
        _ => {
            println!("uso: soso-git status|log|diff|commit");
            2
        }
    }
}
