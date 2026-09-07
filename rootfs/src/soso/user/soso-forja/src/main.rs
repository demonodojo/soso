//! `soso-forja sync|build|install` — bucle de desarrollo remoto (Hito 0).

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
const STAGING: &str = "/var/actualiza-prueba";
const CACHE: &str = "/var/forja-cache";
const OUT: &str = "/var/forja-out";
const DEFAULT_IP: [u8; 4] = [10, 0, 2, 2];
const DEFAULT_PORT: u16 = 8740;

fn leer_fichero(path: &str) -> Result<Vec<u8>, i64> {
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

fn escribir(path: &str, data: &[u8]) -> Result<(), i64> {
    let fd = sys::open(path, abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC);
    if fd < 0 {
        return Err(fd);
    }
    if !data.is_empty() {
        sys::write_all(fd as u64, data)?;
    }
    sys::close(fd as u64);
    Ok(())
}

fn listar_dir(dir: &str, out: &mut Vec<(String, String)>) {
    let fd = sys::open(dir, abi::O_RDONLY);
    if fd < 0 {
        return;
    }
    let mut ents = [abi::Dirent::default(); 8];
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
                listar_dir(&ruta, out);
            } else if let Ok(data) = leer_fichero(&ruta) {
                let h = hex_sha256(&data);
                let rel = ruta.strip_prefix(SRC).unwrap_or(&ruta).trim_start_matches('/');
                out.push((String::from(rel), h));
            }
        }
    }
    sys::close(fd as u64);
}

fn http_post(ip: [u8; 4], port: u16, path: &str, body: &[u8]) -> Result<Vec<u8>, &'static str> {
    http_req(ip, port, "POST", path, body)
}

fn http_get(ip: [u8; 4], port: u16, path: &str) -> Result<Vec<u8>, &'static str> {
    http_req(ip, port, "GET", path, b"")
}

fn http_req(
    ip: [u8; 4],
    port: u16,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<Vec<u8>, &'static str> {
    let addr = sys::sock_addr(ip[0], ip[1], ip[2], ip[3], port);
    let fd = sys::tcp_connect(&addr, 30_000);
    if fd < 0 {
        return Err("connect");
    }
    let req = if method == "GET" {
        format!("GET {path} HTTP/1.1\r\nHost: forja\r\nConnection: close\r\n\r\n")
    } else {
        format!(
            "POST {path} HTTP/1.1\r\nHost: forja\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
    };
    sys::write_all(fd as u64, req.as_bytes()).map_err(|_| "write")?;
    if !body.is_empty() {
        sys::write_all(fd as u64, body).map_err(|_| "write")?;
    }
    let mut resp = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n <= 0 {
            break;
        }
        resp.extend_from_slice(&buf[..n as usize]);
    }
    sys::close(fd as u64);
    let sep = resp.windows(4).position(|w| w == b"\r\n\r\n").ok_or("parse")?;
    Ok(resp[sep + 4..].to_vec())
}

fn cmd_sync(ip: [u8; 4], port: u16) -> u8 {
    let _ = sys::mkdir(SRC);
    let mut manifest = Vec::new();
    listar_dir(SRC, &mut manifest);
    let mut body = String::new();
    for (rel, h) in &manifest {
        body.push_str(rel);
        body.push('\t');
        body.push_str(h);
        body.push('\n');
    }
    match http_post(ip, port, "/sync", body.as_bytes()) {
        Ok(_) => {
            println!("forja: sync OK ({} ficheros)", manifest.len());
            0
        }
        Err(e) => {
            println!("forja: sync: {e}");
            1
        }
    }
}

fn cmd_build(ip: [u8; 4], port: u16) -> u8 {
    let pack = match http_post(ip, port, "/build", b"") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            println!("forja: build falló");
            return 1;
        }
    };
    let manifest = match http_get(ip, port, "/manifest.txt") {
        Ok(m) if !m.is_empty() => m,
        _ => {
            println!("forja: sin manifest.txt");
            return 1;
        }
    };
    let kernel = match http_get(ip, port, "/kernel-x86_64") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            println!("forja: sin kernel-x86_64");
            return 1;
        }
    };
    let _ = sys::mkdir(STAGING);
    if escribir(&format!("{STAGING}/rootfs.pack"), &pack).is_err()
        || escribir(&format!("{STAGING}/manifest.txt"), &manifest).is_err()
        || escribir(&format!("{STAGING}/kernel-x86_64"), &kernel).is_err()
    {
        println!("forja: no se pudo escribir staging");
        return 1;
    }
    println!(
        "forja: build OK → {STAGING}/ (pack {} B, kernel {} B)",
        pack.len(),
        kernel.len()
    );
    0
}

/// Forja v1 local: plan incremental por hash de unidades.
fn cmd_local() -> u8 {
    let plan = format!("{SRC}/forja-unit-graph.txt");
    let data = match leer_fichero(&plan) {
        Ok(d) => d,
        Err(_) => {
            println!("forja: sin {plan}");
            return 1;
        }
    };
    let _ = sys::mkdir(CACHE);
    let texto = core::str::from_utf8(&data).unwrap_or("");
    let mut n = 0usize;
    let mut stale = 0usize;
    for linea in texto.lines() {
        let unit = linea.trim();
        if unit.is_empty() || unit.starts_with('#') {
            continue;
        }
        n += 1;
        let stamp = format!("{CACHE}/{unit}.sha256");
        let cur = hash_unit(unit);
        let prev = leer_fichero(&stamp).ok();
        if prev.as_deref() != Some(cur.as_bytes()) {
            stale += 1;
            println!("forja-local: rebuild → {unit}");
            if escribir(&stamp, cur.as_bytes()).is_err() {
                return 1;
            }
        } else {
            println!("forja-local: cache hit → {unit}");
        }
    }
    println!("forja-local: {n} unidades, {stale} stale");
    0
}

fn hash_unit(unit: &str) -> String {
    let base = format!("{SRC}/{unit}");
    let mut manifest = Vec::new();
    listar_dir(&base, &mut manifest);
    if manifest.is_empty() {
        let path = format!("{SRC}/{unit}");
        if let Ok(data) = leer_fichero(&path) {
            return hex_sha256(&data);
        }
        return String::from("vacío");
    }
    let mut body = String::new();
    manifest.sort_by(|a, b| a.0.cmp(&b.0));
    for (rel, h) in &manifest {
        body.push_str(rel);
        body.push('\t');
        body.push_str(h);
        body.push('\n');
    }
    hex_sha256(body.as_bytes())
}

/// Copia artefactos precompilados del host a staging (Hito 3/4).
fn cmd_build_local() -> u8 {
    if cmd_local() != 0 {
        return 1;
    }
    let _ = sys::mkdir(STAGING);
    let mut ok = true;
    for (src, dst) in [
        (format!("{OUT}/rootfs.pack"), format!("{STAGING}/rootfs.pack")),
        (format!("{OUT}/kernel-x86_64"), format!("{STAGING}/kernel-x86_64")),
        (format!("{OUT}/manifest.txt"), format!("{STAGING}/manifest.txt")),
    ] {
        if let Ok(data) = leer_fichero(&src) {
            if escribir(&dst, &data).is_err() {
                ok = false;
            } else {
                println!("forja: {dst} ← {} B", data.len());
            }
        } else {
            ok = false;
        }
    }
    if !ok {
        println!("forja: faltan artefactos en {OUT}/ (cargo xtask release && cargo xtask forja-out)");
        return 1;
    }
    0
}

fn cmd_install() -> u8 {
    let pid = sys::spawn("/bin/soso-update", "aplicar --local /var/actualiza-prueba");
    if pid < 0 {
        println!("forja: soso-update falló ({pid})");
        return 1;
    }
    let _ = sys::wait();
    println!("forja: reiniciando…");
    sys::halt();
    0
}

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut p = [0u8; 4];
    for (i, oct) in s.split('.').enumerate() {
        if i >= 4 {
            return None;
        }
        p[i] = oct.parse().ok()?;
    }
    Some(p)
}

fn main(args: &str) -> u8 {
    let mut ip = DEFAULT_IP;
    let mut port = DEFAULT_PORT;
    let mut cmd = "help";
    let mut it = args.split_whitespace();
    while let Some(tok) = it.next() {
        match tok {
            "--host" => {
                if let Some(h) = it.next() {
                    if let Some(p) = parse_ip(h) {
                        ip = p;
                    }
                }
            }
            "--port" => {
                if let Some(p) = it.next() {
                    port = p.parse().unwrap_or(DEFAULT_PORT);
                }
            }
            other => cmd = other,
        }
    }
    match cmd {
        "sync" => cmd_sync(ip, port),
        "build" => cmd_build(ip, port),
        "build-local" => cmd_build_local(),
        "install" => cmd_install(),
        "local" => cmd_local(),
        "all" => {
            if cmd_sync(ip, port) != 0 || cmd_build(ip, port) != 0 {
                return 1;
            }
            cmd_install()
        }
        "all-local" => {
            if cmd_build_local() != 0 {
                return 1;
            }
            cmd_install()
        }
        _ => {
            println!("uso: soso-forja sync|build|build-local|install|all|all-local|local [--host IP] [--port N]");
            2
        }
    }
}
