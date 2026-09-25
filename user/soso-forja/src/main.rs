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
/// Lo que `sync` deja escrito para que `build` pueda comprobar el recibo.
///
/// Va a disco porque `sync` y `build` son **dos invocaciones distintas**: sin
/// esto, `build` no tendría contra qué comparar y sólo podría confiar en que
/// el servidor le cuenta la verdad sobre sus propias fuentes.
const ESTADO_SYNC: &str = "/var/forja-cache/ultimo-sync.txt";
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

/// Lo que el cliente sabe de su último `sync`.
struct EstadoSync {
    build_id: String,
    fuentes_sha256: String,
}

fn guardar_estado_sync(build_id: &str, fuentes_sha256: &str) -> Result<(), i64> {
    let _ = sys::mkdir(CACHE);
    let txt = format!("build-id={build_id}\nsource-manifest-sha256={fuentes_sha256}\n");
    escribir(ESTADO_SYNC, txt.as_bytes())
}

fn leer_estado_sync() -> Option<EstadoSync> {
    let datos = leer_fichero(ESTADO_SYNC).ok()?;
    let txt = core::str::from_utf8(&datos).ok()?;
    let build_id = campo(txt, "build-id=")?;
    let fuentes = campo(txt, "source-manifest-sha256=")?;
    if build_id.is_empty() || fuentes.is_empty() {
        return None;
    }
    Some(EstadoSync {
        build_id: String::from(build_id),
        fuentes_sha256: String::from(fuentes),
    })
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
    // El sha256 se toma **de los mismos bytes** que van en la petición, antes
    // de añadir el separador: es lo que el servidor recibe como manifiesto, y
    // hashear otra cosa haría fallar la comprobación por la razón equivocada.
    let fuentes_sha256 = hex_sha256(&body);
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
            let Some(build_id) = campo(txt, "build-id=") else {
                println!("forja: sync: el servidor no devolvió build-id");
                return 1;
            };
            // Si el servidor dice haber recibido otras fuentes, se para aquí:
            // más adelante el recibo cuadraría consigo mismo y el desajuste
            // pasaría desapercibido.
            match campo(txt, "source-manifest-sha256=") {
                Some(h) if h == fuentes_sha256 => {}
                Some(h) => {
                    println!(
                        "forja: sync: el servidor resumió otras fuentes ({} … frente a {} …)",
                        &h[..h.len().min(16)],
                        &fuentes_sha256[..fuentes_sha256.len().min(16)]
                    );
                    return 1;
                }
                None => {
                    println!("forja: sync: el servidor no devolvió source-manifest-sha256");
                    return 1;
                }
            }
            if guardar_estado_sync(build_id, &fuentes_sha256).is_err() {
                println!("forja: sync: no pude guardar {ESTADO_SYNC}");
                return 1;
            }
            println!("forja: sync OK ({} ficheros) {txt}", manifest.len());
            0
        }
        Err(e) => {
            println!("forja: sync: {e}");
            1
        }
    }
}

/// Un campo del recibo, o `None` si no está.
fn campo<'a>(recibo: &'a str, clave: &str) -> Option<&'a str> {
    recibo
        .lines()
        .find_map(|l| l.strip_prefix(clave).map(str::trim))
}

/// El sha256 que el recibo declara para un artefacto.
///
/// Las líneas son `artefacto=<nombre> sha256=<hex> bytes=<n>`; se busca por
/// nombre y no por posición, para que añadir artefactos más adelante no
/// desplace lo que se comprueba.
fn sha_de_artefacto<'a>(recibo: &'a str, nombre: &str) -> Option<&'a str> {
    for l in recibo.lines() {
        let Some(resto) = l.strip_prefix("artefacto=") else {
            continue;
        };
        let mut campos = resto.split_whitespace();
        if campos.next() != Some(nombre) {
            continue;
        }
        for c in campos {
            if let Some(h) = c.strip_prefix("sha256=") {
                return Some(h);
            }
        }
    }
    None
}

/// Comprueba que el recibo corresponde a **esta** petición y a lo descargado.
///
/// Es el núcleo de [T36](../../../docs/self-improvement/T36-forja-trazabilidad.md):
/// antes, `build` escribía el staging con lo que devolviera el servidor sin
/// mirar nada. Un pack de un build anterior, o de otras fuentes, entraba igual
/// — y desde el staging se aplica.
///
/// Se comprueba en este orden a propósito: primero que el recibo sea legible y
/// de una versión conocida, después que hable de nuestras fuentes, y sólo
/// entonces que los bytes descargados sean los que declara. Al revés, un
/// recibo de otro build podría "validar" unos bytes coherentes consigo mismos.
fn verificar_recibo(
    recibo: &[u8],
    esperado: &EstadoSync,
    pack: &[u8],
    kernel: &[u8],
) -> Result<(), String> {
    let Ok(txt) = core::str::from_utf8(recibo) else {
        return Err(String::from("el recibo no es texto"));
    };
    // La 2 añade `comando=` y `herramienta=`, que **describen** el build sin
    // acreditar nada: el cliente no tiene contra qué compararlos. Por eso la 1
    // sigue valiendo — lo que este cliente verifica no cambió — y rechazar una
    // versión conocida sólo por ser más vieja dejaría fuera a un servidor que
    // dice exactamente lo mismo de lo que sí se comprueba.
    match campo(txt, "forja-recibo=") {
        Some("1") | Some("2") => {}
        Some(v) => return Err(format!("recibo de versión {v}, no la 1 ni la 2")),
        None => {
            return Err(String::from(
                "el servidor no emite recibo (¿versión antigua?)",
            ));
        }
    }
    let Some(id) = campo(txt, "build-id=") else {
        return Err(String::from("el recibo no trae build-id"));
    };
    if id != esperado.build_id {
        return Err(format!(
            "build-id {id} pero el sync dio {}",
            esperado.build_id
        ));
    }
    let Some(fuentes) = campo(txt, "source-manifest-sha256=") else {
        return Err(String::from("el recibo no trae source-manifest-sha256"));
    };
    if fuentes != esperado.fuentes_sha256 {
        return Err(format!(
            "el recibo habla de otras fuentes ({} … frente a {} …)",
            &fuentes[..fuentes.len().min(16)],
            &esperado.fuentes_sha256[..esperado.fuentes_sha256.len().min(16)]
        ));
    }
    for (nombre, datos) in [("rootfs.pack", pack), ("kernel-x86_64", kernel)] {
        let Some(want) = sha_de_artefacto(txt, nombre) else {
            return Err(format!("el recibo no declara {nombre}"));
        };
        let visto = hex_sha256(datos);
        if visto != want {
            return Err(format!("{nombre} no coincide con el recibo"));
        }
    }
    Ok(())
}

fn cmd_build(ip: [u8; 4], port: u16, token: Option<&str>) -> u8 {
    let esperado = match leer_estado_sync() {
        Some(e) => e,
        None => {
            println!("forja: no hay un sync previo en esta máquina; corre `soso-forja sync`");
            return 1;
        }
    };
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
    // **Antes de tocar el staging.** Lo que se escribe ahí se aplica después,
    // así que un artefacto que no cuadra no debe llegar a existir en disco.
    if let Err(por_que) = verificar_recibo(&manifest, &esperado, &pack, &kernel) {
        println!("forja: recibo rechazado: {por_que}");
        println!("forja: no se escribió nada en {STAGING}");
        return 1;
    }
    let _ = sys::mkdir(STAGING);
    if escribir(&format!("{STAGING}/rootfs.pack"), &pack).is_err()
        || escribir(&format!("{STAGING}/manifest.txt"), &manifest).is_err()
        || escribir(&format!("{STAGING}/kernel-x86_64"), &kernel).is_err()
    {
        println!("forja: no se pudo escribir staging");
        return 1;
    }
    println!("forja: recibo OK (build-id={})", esperado.build_id);
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
        "#![no_std]\n#![no_main]\n\nextern crate alloc;\n\nuse soso_std::println;\n\nlibsoso::entry!(main);\n\nfn main(_a: &[alloc::string::String]) -> u8 {{\n    libsoso::heap_init();\n    soso_std::init();\n    println!(\"{msg}\");\n    0\n}}\n"
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

fn main(args: &[String]) -> u8 {
    let mut ip = DEFAULT_IP;
    let mut port = DEFAULT_PORT;
    let mut cmd = "help";
    let mut msg = "hola-astra-B3-guest";
    let mut token: Option<&str> = None;
    let mut it = args.iter().map(|s| s.as_str());
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
