//! `uname` — versión y resumen del equipo.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use coreutils::util::leer_fichero;
use libsoso::{abi, println, sys};

libsoso::entry!(main);

fn main(args: &[String]) -> u8 {
    for a in args {
        if a != "-a" && a != "--all" {
            println!("uso: uname");
            return 2;
        }
    }
    mostrar();
    0
}

fn mostrar() {
    match release() {
        Some((version, build, fecha)) => {
            println!("soso {version}");
            if !build.is_empty() {
                println!("build: {build}");
            }
            if !fecha.is_empty() {
                println!("fecha: {fecha}");
            }
        }
        None => println!("soso (sin /etc/soso-release)"),
    }
    match kernel() {
        Some(k) => println!("kernel: {k}"),
        None => println!("kernel: desconocido"),
    }
    println!("arquitectura: x86_64");
    let n = sys::ncpu();
    if n > 0 {
        println!("procesadores: {n}");
    } else {
        println!("procesadores: desconocido");
    }
    let ms = sys::uptime_ms();
    if ms >= 0 {
        println!("encendido: {}", texto_encendido(ms as u64));
    } else {
        println!("encendido: desconocido");
    }
    println!("memoria: {}", texto_memoria());
    println!("disco: {}", texto_disco());
    println!("arranque: {}", texto_arranque());
    println!("red: {}", texto_red());
}

/// `version`, `build` y `fecha` de `/etc/soso-release`.
fn release() -> Option<(String, String, String)> {
    let bytes = leer_fichero("/etc/soso-release").ok()?;
    let text = core::str::from_utf8(&bytes).ok()?;
    let mut version = String::new();
    let mut build = String::new();
    let mut fecha = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("version=") {
            version = v.trim().into();
        } else if let Some(v) = line.strip_prefix("build=") {
            build = v.trim().into();
        } else if let Some(v) = line.strip_prefix("fecha=") {
            fecha = v.trim().into();
        }
    }
    if version.is_empty() {
        None
    } else {
        Some((version, build, fecha))
    }
}

fn kernel() -> Option<String> {
    let mut buf = [0u8; 128];
    let n = sys::version(&mut buf);
    if n <= 0 {
        return None;
    }
    let n = (n as usize).min(buf.len());
    let s = core::str::from_utf8(&buf[..n]).ok()?.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.into())
    }
}

fn texto_encendido(ms: u64) -> String {
    let total = ms / 1000;
    let s = total % 60;
    let min = (total / 60) % 60;
    let h = total / 3600;
    if h > 0 {
        if min == 0 {
            format!("{h} h")
        } else {
            format!("{h} h {min} min")
        }
    } else if min > 0 {
        if s == 0 {
            format!("{min} min")
        } else {
            format!("{min} min {s} s")
        }
    } else {
        format!("{s} s")
    }
}

fn texto_bytes(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB {
        format!("{} MiB", bytes / MIB)
    } else if bytes >= 1024 {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

fn texto_memoria() -> String {
    let mut info = abi::MemInfo::default();
    if sys::meminfo(&mut info) < 0 {
        return "no disponible".into();
    }
    let total = info.total_frames.saturating_mul(4096);
    let libres = info.free_frames.saturating_mul(4096);
    let rec = info.reclaimable_frames.saturating_mul(4096);
    if rec > 0 {
        format!(
            "{} libres de {} ({} recuperables)",
            texto_bytes(libres),
            texto_bytes(total),
            texto_bytes(rec)
        )
    } else {
        format!("{} libres de {}", texto_bytes(libres), texto_bytes(total))
    }
}

fn texto_disco() -> String {
    let mut fs = abi::FsInfo::default();
    if sys::fsinfo(&mut fs) < 0 || fs.block_size == 0 {
        return "no disponible".into();
    }
    let total = fs.total_blocks.saturating_mul(fs.block_size);
    let libres = fs.free_blocks.saturating_mul(fs.block_size);
    format!("{} libres de {}", texto_bytes(libres), texto_bytes(total))
}

fn texto_arranque() -> &'static str {
    let mut discos = [abi::DiskInfo::default(); 8];
    let n = sys::disk_list(&mut discos);
    if n <= 0 {
        return "desconocido";
    }
    discos[..n as usize]
        .iter()
        .find(|d| d.flags & abi::DISK_FLAG_BOOT != 0)
        .map(|d| match d.kind {
            abi::DISK_KIND_USB => "USB live (pendrive)",
            abi::DISK_KIND_NVME => "disco instalado",
            abi::DISK_KIND_VIRTIO => "virtio (prueba QEMU)",
            _ => "desconocido",
        })
        .unwrap_or("desconocido")
}

fn texto_red() -> String {
    let mut info = abi::NetInfo::default();
    if sys::netinfo(&mut info) < 0 || info.flags & abi::NET_FLAG_PRESENT == 0 {
        return "sin adaptador".into();
    }
    let medio = match info.backend {
        abi::NET_BACKEND_WIFI => "wifi",
        _ => "ethernet",
    };
    if info.flags & abi::NET_FLAG_CONFIGURED == 0 {
        return format!("sin dirección ({medio})");
    }
    format!(
        "{}.{}.{}.{}/{} {medio}",
        info.addr[0], info.addr[1], info.addr[2], info.addr[3], info.prefix_len
    )
}
