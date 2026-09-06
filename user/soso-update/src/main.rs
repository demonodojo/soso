//! Descarga e instala actualizaciones de soso desde GitHub Releases.

#![no_std]
#![no_main]

extern crate alloc;

mod net;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cmp::Ordering;

use libsoso::{println, sys};
use soso_abi::{O_RDONLY, O_WRONLY, UPD_WHICH_KERNEL, UPD_WHICH_MAILBOX};
use soso_update_core::hash::{hex_sha256, Hasher};
use soso_update_core::manifest::{self, FileEntry, Manifest};
use soso_update_core::mailbox::Mailbox;
use soso_update_core::plan::{self, Span};
use soso_update_core::semver::{self, SemVer};

libsoso::entry!(main);

const SECTOR: usize = 512;
const CHUNK: usize = 64 * 1024;
/// Buffer de lectura para hashear ficheros ya instalados.
const HASH_BUF: usize = 256 * 1024;

fn main(args: &str) -> u8 {
    let parts: Vec<String> = args.split_whitespace().map(String::from).collect();
    let cmd = parts.first().map(|s| s.as_str()).unwrap_or("aplicar");
    let rest: &[String] = parts.get(1..).unwrap_or(&[]);
    match cmd {
        "estado" => cmd_estado(),
        "comprobar" => cmd_comprobar(rest),
        "aplicar" => cmd_aplicar(rest),
        "revertir" => cmd_revertir(),
        "help" | "--help" | "-h" => {
            print_usage();
            0
        }
        other if other.starts_with('-') => cmd_aplicar(&parts),
        other => {
            println!("soso-update: subcomando desconocido {other}");
            print_usage();
            2
        }
    }
}

fn print_usage() {
    println!("uso:");
    println!("  soso-update estado");
    println!("  soso-update comprobar [--local DIR]");
    println!("  soso-update aplicar [--forzar] [--sin-kernel] [--local DIR]");
    println!("  soso-update revertir");
}

fn cmd_estado() -> u8 {
    let rel = read_release();
    if let Some(r) = &rel {
        println!("rootfs: {} ({})", r.version, r.build);
    } else {
        println!("rootfs: (sin /etc/soso-release)");
    }
    let mut kbuf = [0u8; 128];
    let n = sys::version(&mut kbuf);
    if n > 0 {
        println!(
            "kernel: {}",
            core::str::from_utf8(&kbuf[..n as usize]).unwrap_or("?")
        );
    }
    let mut mbuf = [0u8; 4096];
    let mn = sys::upd_read(UPD_WHICH_MAILBOX, 0, &mut mbuf);
    if mn > 0 {
        let mb = Mailbox::parse(core::str::from_utf8(&mbuf[..mn as usize]).unwrap_or(""));
        println!("buzón: {:?}", mb.cmd);
    } else if mn == -libsoso::abi::ENOTSUP {
        println!("buzón: no disponible (reflashea/reinstala para actualizar kernel)");
    }
    if file_exists("/etc/actualiza.estado") {
        if let Some(s) = read_file("/etc/actualiza.estado", 256) {
            println!("estado: {s}");
        }
    }
    0
}

fn cmd_comprobar(args: &[String]) -> u8 {
    let opts = parse_opts(args);
    let man = match load_manifest(&opts) {
        Ok(m) => m,
        Err(e) => {
            println!("soso-update: {e}");
            return 1;
        }
    };
    let actual = read_release()
        .map(|r| r.version_parsed())
        .unwrap_or(SemVer {
            major: 0,
            minor: 0,
            patch: 0,
        });
    println!("remoto: {} ({})", man.version_raw, man.build);
    println!(
        "local:  {}",
        read_release()
            .map(|r| r.version.clone())
            .unwrap_or_else(|| "?".into())
    );
    match semver::cmp(&man.version, &actual) {
        Ordering::Greater => println!("hay actualización disponible"),
        Ordering::Equal => println!("ya estás en la última versión"),
        Ordering::Less => println!("remoto es más antiguo que local"),
    }
    let pendientes: Vec<FileEntry> = man
        .files
        .iter()
        .filter(|f| file_needs_update(f))
        .cloned()
        .collect();
    for f in &pendientes {
        println!("  ~ {} ({})", f.path, humano(f.size));
    }
    let mut total = 0u64;
    if pendientes.is_empty() {
        println!("rootfs: sin cambios de fichero");
    } else {
        let spans = plan::plan_spans(&pendientes, plan::GAP_MAX, plan::SPAN_MAX);
        let bytes = plan::plan_bytes(&spans);
        total += bytes;
        println!(
            "rootfs: {} en {} {} (pack completo: {})",
            humano(bytes),
            spans.len(),
            if spans.len() == 1 { "petición" } else { "peticiones" },
            humano(man.pack_size)
        );
    }
    if man.kernel_size > 0 {
        if read_release().map(|r| r.kernel) == Some(man.kernel_hash.clone()) {
            println!("kernel: sin cambios");
        } else {
            total += man.kernel_size;
            println!("kernel: {} (hash {})", humano(man.kernel_size), &man.kernel_hash[..16]);
        }
    }
    println!("descarga total: {}", humano(total));
    0
}

fn cmd_aplicar(args: &[String]) -> u8 {
    let opts = parse_opts(args);
    let man = match load_manifest(&opts) {
        Ok(m) => m,
        Err(e) => {
            println!("soso-update: {e}");
            return 1;
        }
    };
    let actual = read_release()
        .map(|r| r.version_parsed())
        .unwrap_or(SemVer {
            major: 0,
            minor: 0,
            patch: 0,
        });
    if !opts.forzar && semver::cmp(&man.version, &actual) != Ordering::Greater {
        println!("soso-update: ya estás en {} (usa --forzar)", man.version_raw);
        return 0;
    }
    write_file(
        "/etc/actualiza.estado",
        &format!("APLICANDO {}\n", man.version_raw),
    );

    // Sólo los ficheros que de verdad cambian. El resto del pack ni se pide.
    let pendientes: Vec<FileEntry> = man
        .files
        .iter()
        .filter(|f| file_needs_update(f))
        .cloned()
        .collect();

    if pendientes.is_empty() {
        println!("rootfs: sin cambios");
    } else {
        let spans = plan::plan_spans(&pendientes, plan::GAP_MAX, plan::SPAN_MAX);
        let bytes = plan::plan_bytes(&spans);
        println!(
            "rootfs: {} de {} ficheros, {} de {} ({} {})",
            pendientes.len(),
            man.files.len(),
            humano(bytes),
            humano(man.pack_size),
            spans.len(),
            if spans.len() == 1 { "petición" } else { "peticiones" }
        );
        for span in &spans {
            if let Err(e) = apply_span(&opts, &pendientes, span) {
                println!("soso-update: {e}");
                return 1;
            }
        }
    }

    if !opts.sin_kernel {
        if let Err(e) = apply_kernel(&opts, &man) {
            println!("soso-update: aviso kernel: {e}");
        }
    }
    let release = format!(
        "version={}\nbuild={}\nfecha={}\nkernel={}\n",
        man.version_raw, man.build, man.fecha, man.kernel_hash
    );
    write_file("/etc/soso-release", &release);
    let _ = sys::unlink("/etc/actualiza.estado");
    println!("soso-update: listo — reinicia para arrancar soso {}", man.version_raw);
    0
}

/// Descarga un tramo del pack y escribe los ficheros que contiene.
fn apply_span(opts: &Opts, pendientes: &[FileEntry], span: &Span) -> Result<(), &'static str> {
    let blob = fetch_pack_span(opts, span.start, span.len())?;
    for &i in &span.files {
        let f = &pendientes[i];
        let rel = (f.offset - span.start) as usize;
        let data = blob
            .get(rel..rel + f.size as usize)
            .ok_or("tramo incompleto")?;
        if hex_sha256(data) != f.hash_hex {
            return Err("hash corrupto en fichero descargado");
        }
        let path = format!("/{}", f.path);
        if let Some(parent) = parent_dir(&path) {
            let _ = sys::mkdir(parent);
        }
        if let Err((fase, e)) = escribir(&path, data) {
            println!("soso-update: {path}: {fase} falló ({e})");
            return Err("no pude escribir el fichero");
        }
        println!("  ok {} ({})", f.path, humano(f.size));
    }
    Ok(())
}

fn humano(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{},{} MB", bytes / (1024 * 1024), (bytes % (1024 * 1024)) / 104858)
    } else if bytes >= 1024 {
        format!("{} kB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

fn cmd_revertir() -> u8 {
    let payload = Mailbox::format_revertir();
    let r = sys::upd_write(UPD_WHICH_MAILBOX, 0, &payload);
    if r < 0 {
        println!("soso-update: no pude escribir buzón ({r})");
        return 1;
    }
    println!("soso-update: REVERTIR registrado — reinicia para restaurar el kernel");
    0
}

struct Opts {
    local: Option<String>,
    forzar: bool,
    sin_kernel: bool,
}

fn parse_opts(args: &[String]) -> Opts {
    let mut opts = Opts {
        local: None,
        forzar: false,
        sin_kernel: false,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--local" => {
                i += 1;
                opts.local = args.get(i).cloned();
            }
            "--forzar" => opts.forzar = true,
            "--sin-kernel" => opts.sin_kernel = true,
            _ => {}
        }
        i += 1;
    }
    opts
}

struct ReleaseInfo {
    kernel: String,
    version: String,
    build: String,
    fecha: String,
}

impl ReleaseInfo {
    fn version_parsed(&self) -> SemVer {
        semver::parse(&self.version).unwrap_or(SemVer {
            major: 0,
            minor: 0,
            patch: 0,
        })
    }
}

fn read_release() -> Option<ReleaseInfo> {
    let text = read_file("/etc/soso-release", 512)?;
    let mut version = String::new();
    let mut build = String::new();
    let mut fecha = String::new();
    let mut kernel = String::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("version=") {
            version = v.into();
        } else if let Some(v) = line.strip_prefix("build=") {
            build = v.into();
        } else if let Some(v) = line.strip_prefix("fecha=") {
            fecha = v.into();
        } else if let Some(v) = line.strip_prefix("kernel=") {
            kernel = v.into();
        }
    }
    if version.is_empty() {
        return None;
    }
    Some(ReleaseInfo {
        version,
        build,
        fecha,
        kernel,
    })
}

fn read_config_url() -> String {
    let default = "https://github.com/demonodojo/soso/releases/latest/download".to_string();
    let Some(text) = read_file("/etc/actualiza.conf", 512) else {
        return default;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(u) = line.strip_prefix("url=") {
            return u.trim().into();
        }
    }
    default
}

fn load_manifest(opts: &Opts) -> Result<Manifest, &'static str> {
    let data = if let Some(dir) = &opts.local {
        read_file(&format!("{dir}/manifest.txt"), 256 * 1024).ok_or("manifest local")?
    } else {
        let url = format!("{}/manifest.txt", read_config_url().trim_end_matches('/'));
        let bytes = net::https_get_bytes(&url, None)?;
        String::from_utf8(bytes).map_err(|_| "manifest UTF-8")?
    };
    Manifest::parse(&data).map_err(|_| "manifest inválido")
}

/// Trae `[start, start+len)` del pack, de la copia local o por HTTP `Range`.
fn fetch_pack_span(opts: &Opts, start: u64, len: u64) -> Result<Vec<u8>, &'static str> {
    if let Some(dir) = &opts.local {
        return read_file_span(&format!("{dir}/rootfs.pack"), start, len)
            .ok_or("rootfs.pack local");
    }
    let url = format!("{}/rootfs.pack", read_config_url().trim_end_matches('/'));
    net::https_download_span(&url, None, start, len)
}

/// Lee un tramo de un fichero local sin cargarlo entero.
fn read_file_span(path: &str, start: u64, len: u64) -> Option<Vec<u8>> {
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return None;
    }
    if sys::seek(fd as u64, start as i64, soso_abi::SEEK_SET) < 0 {
        let _ = sys::close(fd as u64);
        return None;
    }
    let mut out = Vec::new();
    if out.try_reserve(len as usize).is_err() {
        let _ = sys::close(fd as u64);
        return None;
    }
    let mut buf = alloc::vec![0u8; HASH_BUF];
    while (out.len() as u64) < len {
        let falta = (len - out.len() as u64).min(HASH_BUF as u64) as usize;
        let n = sys::read(fd as u64, &mut buf[..falta]);
        if n <= 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    let _ = sys::close(fd as u64);
    if out.len() as u64 == len { Some(out) } else { None }
}

fn apply_kernel(opts: &Opts, man: &Manifest) -> Result<(), &'static str> {
    // El kernel es el otro bloque grande. Si el que ya está instalado tiene el
    // hash que pide el manifest, no hay nada que descargar ni que reflashear.
    if !man.kernel_hash.is_empty()
        && read_release().map(|r| r.kernel) == Some(man.kernel_hash.clone())
    {
        println!("kernel: sin cambios");
        return Ok(());
    }
    println!("kernel: {}", humano(man.kernel_size));
    let data = if let Some(dir) = &opts.local {
        read_file_bytes(
            &format!("{dir}/kernel-x86_64"),
            man.kernel_size as usize + 1,
        )
        .ok_or("kernel local")?
    } else {
        let url = format!(
            "{}/kernel-x86_64",
            read_config_url().trim_end_matches('/')
        );
        net::https_download_all(&url, None, man.kernel_size)?
    };
    if data.len() as u64 != man.kernel_size || hex_sha256(&data) != man.kernel_hash {
        return Err("kernel corrupto");
    }
    let mut off = 0u64;
    while off < man.kernel_size {
        let end = (off + CHUNK as u64).min(man.kernel_size);
        let chunk = &data[off as usize..end as usize];
        let pad = pad_sector(chunk);
        let r = sys::upd_write(UPD_WHICH_KERNEL, off, &pad);
        if r < 0 {
            if r == -libsoso::abi::ENOTSUP {
                return Err("reflashea/reinstala el live para habilitar actualización de kernel");
            }
            return Err("upd_write falló");
        }
        off = end;
    }
    let hash = hex_sha256(&data);
    let payload = Mailbox::format_kernel(man.kernel_size, &hash, &man.version_raw);
    let r = sys::upd_write(UPD_WHICH_MAILBOX, 0, &payload);
    if r < 0 {
        return Err("no pude escribir buzón KERNEL");
    }
    Ok(())
}

fn pad_sector(chunk: &[u8]) -> Vec<u8> {
    let mut v = chunk.to_vec();
    let rem = v.len() % SECTOR;
    if rem != 0 {
        v.resize(v.len() + (SECTOR - rem), 0);
    }
    v
}

fn file_needs_update(entry: &manifest::FileEntry) -> bool {
    let path = format!("/{}", entry.path);
    // El tamaño descarta la mayoría de los casos sin tocar el contenido: leer
    // los 63 MB de un firmware GSP sólo para descubrir que ha cambiado sería
    // tirar el disco a la basura.
    match file_size(&path) {
        None => true,
        Some(size) if size != entry.size => true,
        Some(_) => hash_file(&path).map(|h| h != entry.hash_hex).unwrap_or(true),
    }
}

fn file_size(path: &str) -> Option<u64> {
    let mut st = soso_abi::Stat {
        ino: 0,
        size: 0,
        mtime: 0,
        file_type: 0,
        _pad: [0u8; 7],
    };
    if sys::stat(path, &mut st) < 0 {
        return None;
    }
    Some(st.size)
}

/// SHA-256 de un fichero leyéndolo por trozos: el pico de RAM es el buffer, no
/// el fichero.
fn hash_file(path: &str) -> Option<String> {
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut h = Hasher::new();
    let mut buf = alloc::vec![0u8; HASH_BUF];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n < 0 {
            let _ = sys::close(fd as u64);
            return None;
        }
        if n == 0 {
            break;
        }
        h.update(&buf[..n as usize]);
    }
    let _ = sys::close(fd as u64);
    Some(h.finish_hex())
}

fn file_exists(path: &str) -> bool {
    let mut st = libsoso::abi::Stat::default();
    sys::stat(path, &mut st) >= 0
}

fn read_file(path: &str, max: usize) -> Option<String> {
    read_file_bytes(path, max).map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn read_file_bytes(path: &str, max: usize) -> Option<Vec<u8>> {
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n <= 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
        if out.len() >= max {
            break;
        }
    }
    let _ = sys::close(fd as u64);
    Some(out)
}

fn write_file(path: &str, content: &str) -> bool {
    write_path(path, content.as_bytes())
}

fn write_path(path: &str, data: &[u8]) -> bool {
    escribir(path, data).is_ok()
}

/// Escribe un fichero devolviendo el errno y la fase que falló: con
/// `Fd::WriteBuf` el contenido se materializa en el `close`, así que un fallo
/// de disco aparece ahí y no en el `write`.
fn escribir(path: &str, data: &[u8]) -> Result<(), (&'static str, i64)> {
    let fd = sys::open(path, O_WRONLY);
    if fd < 0 {
        return Err(("open", fd));
    }
    if let Err(e) = sys::write_all(fd as u64, data) {
        let _ = sys::close(fd as u64);
        return Err(("write", e));
    }
    let r = sys::close(fd as u64);
    if r < 0 { Err(("close", r)) } else { Ok(()) }
}

fn parent_dir(path: &str) -> Option<&str> {
    path.rfind('/').filter(|&i| i > 0).map(|i| &path[..i])
}
