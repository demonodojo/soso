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
use soso_update_core::hash::hex_sha256;
use soso_update_core::manifest::{self, Manifest};
use soso_update_core::mailbox::Mailbox;
use soso_update_core::pack::PackReader;
use soso_update_core::semver::{self, SemVer};

libsoso::entry!(main);

const SECTOR: usize = 512;
const CHUNK: usize = 64 * 1024;

fn main(args: &str) -> u8 {
    let parts: Vec<String> = args.split_whitespace().map(String::from).collect();
    let cmd = parts.first().map(|s| s.as_str()).unwrap_or("aplicar");
    match cmd {
        "estado" => cmd_estado(),
        "comprobar" => cmd_comprobar(&parts[1..]),
        "aplicar" => cmd_aplicar(&parts[1..]),
        "revertir" => cmd_revertir(),
        "help" | "--help" | "-h" => {
            print_usage();
            0
        }
        other if other.starts_with('-') || parts.is_empty() => cmd_aplicar(&parts),
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
    let mut cambios = 0u32;
    for f in &man.files {
        if file_needs_update(f) {
            cambios += 1;
            println!("  ~ {}", f.path);
        }
    }
    if cambios == 0 {
        println!("rootfs: sin cambios de fichero");
    }
    if man.kernel_size > 0 {
        println!("kernel: {} B (hash {})", man.kernel_size, &man.kernel_hash[..16]);
    }
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
    let pack = match load_pack(&opts, &man) {
        Ok(p) => p,
        Err(e) => {
            println!("soso-update: {e}");
            return 1;
        }
    };
    if hex_sha256(&pack) != man.pack_hash {
        println!("soso-update: hash de rootfs.pack no coincide");
        return 1;
    }
    let reader = PackReader::new(&pack);
    for f in &man.files {
        if !file_needs_update(f) {
            continue;
        }
        let Some(blob) = reader.read_entry(f) else {
            println!("soso-update: offset inválido para {}", f.path);
            return 1;
        };
        if hex_sha256(blob) != f.hash_hex {
            println!("soso-update: hash corrupto en {}", f.path);
            return 1;
        }
        let path = format!("/{}", f.path);
        if let Some(parent) = parent_dir(&path) {
            let _ = sys::mkdir(parent);
        }
        if !write_path(&path, blob) {
            println!("soso-update: no pude escribir {path}");
            return 1;
        }
        println!("  ok {}", f.path);
    }
    if !opts.sin_kernel {
        if let Err(e) = apply_kernel(&opts, &man) {
            println!("soso-update: aviso kernel: {e}");
        }
    }
    let release = format!(
        "version={}\nbuild={}\nfecha={}\n",
        man.version_raw, man.build, man.fecha
    );
    write_file("/etc/soso-release", &release);
    let _ = sys::unlink("/etc/actualiza.estado");
    println!("soso-update: listo — reinicia para arrancar soso {}", man.version_raw);
    0
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
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("version=") {
            version = v.into();
        } else if let Some(v) = line.strip_prefix("build=") {
            build = v.into();
        } else if let Some(v) = line.strip_prefix("fecha=") {
            fecha = v.into();
        }
    }
    if version.is_empty() {
        return None;
    }
    Some(ReleaseInfo {
        version,
        build,
        fecha,
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

fn load_pack(opts: &Opts, man: &Manifest) -> Result<Vec<u8>, &'static str> {
    if let Some(dir) = &opts.local {
        return read_file_bytes(&format!("{dir}/rootfs.pack"), man.pack_size as usize + 1)
            .ok_or("rootfs.pack local");
    }
    let url = format!("{}/rootfs.pack", read_config_url().trim_end_matches('/'));
    net::https_download_all(&url, None, man.pack_size)
}

fn apply_kernel(opts: &Opts, man: &Manifest) -> Result<(), &'static str> {
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
    let Some(data) = read_file_bytes(&path, entry.size as usize + 1) else {
        return true;
    };
    hex_sha256(&data) != entry.hash_hex
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
    let fd = sys::open(path, O_WRONLY);
    if fd < 0 {
        return false;
    }
    if sys::write_all(fd as u64, data).is_err() {
        let _ = sys::close(fd as u64);
        return false;
    }
    sys::close(fd as u64) >= 0
}

fn parent_dir(path: &str) -> Option<&str> {
    path.rfind('/').filter(|&i| i > 0).map(|i| &path[..i])
}
