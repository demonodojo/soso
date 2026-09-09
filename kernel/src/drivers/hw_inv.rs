//! Inventario de hardware en `/etc/soso-hw`: evita repetir el informe hwscan
//! en arranques nativos (NVMe) cuando el bus PCI no ha cambiado.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;
use core::sync::atomic::Ordering;

use crate::arch::smp;
use crate::drivers::pci;
use crate::vfs;

const PATH: &str = "/etc/soso-hw";
const MAGIC: &str = "soso-hw 1";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PciEntry {
    bus: u8,
    device: u8,
    function: u8,
    vid: u16,
    did: u16,
}

fn pci_entries() -> Vec<PciEntry> {
    let mut out: Vec<PciEntry> = pci::devices()
        .iter()
        .map(|d| PciEntry {
            bus: d.bus,
            device: d.device,
            function: d.function,
            vid: d.vendor_id,
            did: d.device_id,
        })
        .collect();
    out.sort();
    out
}

fn ncpu_actual() -> u32 {
    smp::CPUS_ONLINE.load(Ordering::Relaxed)
}

/// Texto del inventario actual (PCI + ncpu).
pub fn serializar() -> String {
    let ncpu = ncpu_actual();
    let entries = pci_entries();
    let mut out = String::new();
    let _ = write!(out, "{MAGIC}\nncpu {ncpu}\n");
    for e in entries {
        let _ = write!(
            out,
            "pci {:02x}:{:02x}.{} {:04x}:{:04x}\n",
            e.bus, e.device, e.function, e.vid, e.did
        );
    }
    out
}

#[derive(Default)]
struct Parsed {
    ncpu: Option<u32>,
    pci: Vec<PciEntry>,
    force: bool,
}

fn parse_bdf(s: &str) -> Option<(u8, u8, u8)> {
    let (bus_s, rest) = s.split_once(':')?;
    let (dev_s, func_s) = rest.split_once('.')?;
    let bus = u8::from_str_radix(bus_s, 16).ok()?;
    let device = u8::from_str_radix(dev_s, 16).ok()?;
    let function = u8::from_str_radix(func_s, 16).ok()?;
    Some((bus, device, function))
}

fn parse_vid_did(s: &str) -> Option<(u16, u16)> {
    let (v, d) = s.split_once(':')?;
    Some((u16::from_str_radix(v, 16).ok()?, u16::from_str_radix(d, 16).ok()?))
}

fn parse(text: &str) -> Option<Parsed> {
    let trimmed = text.trim();
    if trimmed == "force" {
        return Some(Parsed {
            force: true,
            ..Default::default()
        });
    }
    let mut p = Parsed::default();
    let mut saw_magic = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == MAGIC {
            saw_magic = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("ncpu ") {
            p.ncpu = rest.trim().parse().ok();
            continue;
        }
        if let Some(rest) = line.strip_prefix("pci ") {
            let mut parts = rest.split_whitespace();
            let bdf = parts.next()?;
            let ids = parts.next()?;
            let (bus, device, function) = parse_bdf(bdf)?;
            let (vid, did) = parse_vid_did(ids)?;
            p.pci.push(PciEntry {
                bus,
                device,
                function,
                vid,
                did,
            });
        }
    }
    if saw_magic {
        p.pci.sort();
        Some(p)
    } else {
        None
    }
}

fn coincide(text: &str) -> bool {
    let Some(parsed) = parse(text) else {
        return false;
    };
    if parsed.force {
        return false;
    }
    let Some(ncpu) = parsed.ncpu else {
        return false;
    };
    if ncpu != ncpu_actual() {
        return false;
    }
    pci_entries() == parsed.pci
}

fn leer() -> Option<String> {
    let ino = vfs::resolve(PATH).ok()?;
    let bytes = vfs::read_file(ino).ok()?;
    core::str::from_utf8(&bytes).ok().map(|s| s.to_string())
}

/// ¿Hay que imprimir hwscan y volcar SOSODRV? Live USB siempre; NVMe solo si
/// falta inventario, no coincide o el usuario pidió `force`.
#[cfg(feature = "drv-live-disk")]
pub fn debe_informar() -> bool {
    if !crate::drivers::live_disk::es_instalado() {
        return true;
    }
    let Some(text) = leer() else {
        return true;
    };
    if text.trim() == "force" {
        return true;
    }
    !coincide(&text)
}

#[cfg(not(feature = "drv-live-disk"))]
pub fn debe_informar() -> bool {
    true
}

/// Persiste el inventario actual en rootfs (live o instalado).
pub fn guardar() {
    if crate::fs::FS.get().is_none() {
        return;
    }
    let body = serializar();
    let mtime = crate::time::wall_secs();
    let Ok(etc) = vfs::resolve("/etc") else {
        crate::println!("hw-inv: sin /etc");
        return;
    };
    let _ = vfs::unlink(etc, "soso-hw");
    match vfs::create_file(etc, "soso-hw", body.as_bytes(), mtime) {
        Ok(_) => crate::println!("hw-inv: inventario en {PATH}"),
        Err(e) => crate::println!("hw-inv: no pude escribir {PATH} ({e:?})"),
    }
}
