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

fn http_post(
    ip: [u8; 4],
    port: u16,
    path: &str,
    body: &[u8],
    token: Option<&str>,
) -> Result<Vec<u8>, &'static str> {
    http_req(ip, port, "POST", path, body, token)
}

fn http_get(
    ip: [u8; 4],
    port: u16,
    path: &str,
    token: Option<&str>,
) -> Result<Vec<u8>, &'static str> {
    http_req(ip, port, "GET", path, b"", token)
}

fn auth_hdr(token: Option<&str>) -> String {
    match token {
        Some(t) if !t.is_empty() => format!("Authorization: Bearer {t}\r\n"),
        _ => String::new(),
    }
}

fn http_req(
    ip: [u8; 4],
    port: u16,
    method: &str,
    path: &str,
    body: &[u8],
    token: Option<&str>,
) -> Result<Vec<u8>, &'static str> {
    let addr = sys::sock_addr(ip[0], ip[1], ip[2], ip[3], port);
    let fd = sys::tcp_connect(&addr, 30_000);
    if fd < 0 {
        return Err("connect");
    }
    let auth = auth_hdr(token);
    let req = if method == "GET" {
        format!("GET {path} HTTP/1.1\r\nHost: forja\r\n{auth}Connection: close\r\n\r\n")
    } else {
        format!(
            "POST {path} HTTP/1.1\r\nHost: forja\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n",
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
    let headers = core::str::from_utf8(&resp[..sep]).map_err(|_| "parse")?;
    let status = headers
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err("http");
    }
    Ok(resp[sep + 4..].to_vec())
}

fn cmd_sync(ip: [u8; 4], port: u16, token: Option<&str>) -> u8 {
    let _ = sys::mkdir(SRC);
    let mut manifest = Vec::new();
    listar_dir(SRC, &mut manifest);
    let mut body: Vec<u8> = Vec::new();
    for (rel, h) in &manifest {
        body.extend_from_slice(rel.as_bytes());
        body.push(b'\t');
        body.extend_from_slice(h.as_bytes());
        body.push(b'\n');
    }
    body.extend_from_slice(b"\n---DATA---\n");
    for (rel, _) in &manifest {
        let path = format!("{SRC}/{rel}");
        let data = match leer_fichero(&path) {
            Ok(d) => d,
            Err(_) => {
                println!("forja: sync: no leí {path}");
                return 1;
            }
        };
        body.extend_from_slice(rel.as_bytes());
        body.push(b'\n');
        body.extend_from_slice(format!("{}", data.len()).as_bytes());
        body.push(b'\n');
        body.extend_from_slice(&data);
    }
    match http_post(ip, port, "/sync", &body, token) {
        Ok(resp) => {
            let txt = core::str::from_utf8(&resp).unwrap_or("");
            println!("forja: sync OK ({} ficheros) {txt}", manifest.len());
            0
        }
        Err(e) => {
            println!("forja: sync: {e}");
            1
        }
    }
}

fn cmd_build(ip: [u8; 4], port: u16, token: Option<&str>) -> u8 {
    let pack = match http_post(ip, port, "/build", b"", token) {
        Ok(p) if !p.is_empty() => p,
        _ => {
            println!("forja: build falló");
            return 1;
        }
    };
    let manifest = match http_get(ip, port, "/manifest.txt", token) {
        Ok(m) if !m.is_empty() => m,
        _ => {
            println!("forja: sin manifest.txt");
            return 1;
        }
    };
    let kernel = match http_get(ip, port, "/kernel-x86_64", token) {
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
    if let Ok(txt) = core::str::from_utf8(&manifest) {
        for line in txt.lines() {
            if line.starts_with("forja-id=") || line.starts_with("build-id=") {
                println!("forja: {line}");
            }
        }
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
            // Las unidades llevan barra (`user/coreutils`), así que el stamp
            // cae en un subdirectorio de CACHE que `mkdir(CACHE)` no crea; sin
            // esto `open(O_CREAT)` fallaba y `forja local` salía con 1 (lo
            // cazaba `init test`). Mismo helper que usa soso-git.
            ensure_parent(&stamp);
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

/// Crea los directorios intermedios de `path` (todos menos el último
/// componente). `mkdir` sobre uno que ya existe se ignora.
fn ensure_parent(path: &str) {
    let Some(i) = path.rfind('/') else {
        return;
    };
    let mut cur = String::new();
    for comp in path[..i].split('/') {
        if comp.is_empty() {
            continue;
        }
        cur.push('/');
        cur.push_str(comp);
        let _ = sys::mkdir(&cur);
    }
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

/// Reescribe `hola-std` en el guest (sosh interpreta `>` y no sirve `echo ->`).
fn cmd_write_hola(msg: &str) -> u8 {
    if msg.is_empty() || msg.len() > 80 || msg.bytes().any(|b| b < b' ' || b == b'"') {
        println!("forja: --msg inválido");
        return 1;
    }
    let src = format!(
        "#![no_std]\n#![no_main]\n\nextern crate alloc;\n\nuse soso_std::println;\n\nlibsoso::entry!(main);\n\nfn main(_a: &str) -> u8 {{\n    libsoso::heap_init();\n    soso_std::init();\n    println!(\"{msg}\");\n    0\n}}\n"
    );
    let path = "/src/soso/user/hola-std/src/main.rs";
    if escribir(path, src.as_bytes()).is_err() {
        println!("forja: no pude escribir {path}");
        return 1;
    }
    println!("forja: escrito {path}");
    0
}

fn pack_es_elf() -> bool {
    match leer_fichero(&format!("{STAGING}/rootfs.pack")) {
        Ok(p) => p.len() >= 4 && p[..4] == *b"\x7fELF",
        Err(_) => false,
    }
}

/// Demo B3: el pack de `hola-std` es el ELF; se instala sin OTA ni halt.
fn cmd_apply_bin() -> u8 {
    let pack = match leer_fichero(&format!("{STAGING}/rootfs.pack")) {
        Ok(p) => p,
        Err(_) => {
            println!("forja: sin {STAGING}/rootfs.pack");
            return 1;
        }
    };
    if pack.len() < 4 || pack[..4] != *b"\x7fELF" {
        println!("forja: el pack no es un ELF (usa install)");
        return 1;
    }
    if escribir("/bin/hola-std", &pack).is_err() {
        println!("forja: no pude escribir /bin/hola-std");
        return 1;
    }
    println!("forja: aplicado /bin/hola-std ({} B)", pack.len());
    0
}

fn cmd_apply_or_install() -> u8 {
    if pack_es_elf() {
        cmd_apply_bin()
    } else {
        cmd_install()
    }
}

fn cmd_install() -> u8 {
    let pid = sys::spawn("/bin/soso-update", "aplicar --local /var/actualiza-prueba");
    if pid < 0 {
        println!("forja: soso-update falló ({pid})");
        return 1;
    }
    match sys::wait() {
        Ok((_, 0)) => {
            println!("forja: reiniciando…");
            sys::halt();
            0
        }
        Ok((_, code)) => {
            println!("forja: soso-update salió con {code}");
            1
        }
        Err(e) => {
            println!("forja: wait soso-update ({e})");
            1
        }
    }
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
    let mut msg = "hola-astra-B3-guest";
    let mut token: Option<&str> = None;
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
            "--msg" => {
                if let Some(m) = it.next() {
                    msg = m;
                }
            }
            "--token" => {
                if let Some(t) = it.next() {
                    token = Some(t);
                }
            }
            other => cmd = other,
        }
    }
    match cmd {
        "write-hola" => cmd_write_hola(msg),
        "sync" => cmd_sync(ip, port, token),
        "build" => cmd_build(ip, port, token),
        "build-local" => cmd_build_local(),
        "apply" => cmd_apply_bin(),
        "install" => cmd_install(),
        "local" => cmd_local(),
        "all" => {
            if cmd_sync(ip, port, token) != 0 || cmd_build(ip, port, token) != 0 {
                return 1;
            }
            cmd_apply_or_install()
        }
        "all-local" => {
            if cmd_build_local() != 0 {
                return 1;
            }
            cmd_install()
        }
        _ => {
            println!("uso: soso-forja write-hola|sync|build|apply|build-local|install|all|all-local|local [--host IP] [--port N] [--msg TEXTO] [--token T]");
            2
        }
    }
}
