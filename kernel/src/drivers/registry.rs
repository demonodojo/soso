//! Registro central de drivers: metadatos PCI, estado compilado y hwscan.

use crate::drivers::pci::PciDevice;
use crate::println;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

/// Regla de emparejamiento con un dispositivo PCI enumerado.
#[derive(Clone, Copy, Debug)]
pub enum MatchRule {
    /// class / subclass / prog_if opcionales (None = comodín).
    PciClass {
        class: u8,
        subclass: Option<u8>,
        prog_if: Option<u8>,
    },
    /// Lista de device IDs para un vendor concreto.
    PciIds {
        vendor: u16,
        devices: &'static [u16],
    },
    /// Vendor NVIDIA + class display (0x03).
    NvidiaGpu,
    /// Vendor Intel + class display (0x03). Reservada para un futuro driver.
    #[allow(dead_code)]
    IntelGpu,
    /// Virtio PCI (vendor 0x1af4): subtipo 1=net, 2=blk.
    Virtio { subtype: u8 },
}

/// Metadatos de un driver conocido. Siempre presentes; el código del driver
/// puede estar ausente según la feature de Cargo.
#[derive(Clone, Copy)]
pub struct DriverInfo {
    pub name: &'static str,
    pub feature: &'static str,
    pub rule: MatchRule,
    /// Puerto lxdde asociado, si aplica (metadato informativo).
    #[allow(dead_code)]
    pub lxdde_port: Option<&'static str>,
}

pub const DRIVERS: &[DriverInfo] = &[
    DriverInfo {
        name: "lx-e1000e",
        feature: "lxdde",
        rule: MatchRule::PciIds {
            vendor: 0x8086,
            devices: &[0x10d3, 0x100e, 0x10f5, 0x10a4],
        },
        lxdde_port: Some("e1000e"),
    },
    DriverInfo {
        name: "lx-iwlwifi",
        feature: "lxdde",
        rule: MatchRule::PciIds {
            vendor: 0x8086,
            devices: &[0x7f70, 0x51f0, 0x54f0],
        },
        lxdde_port: Some("iwlwifi"),
    },
    DriverInfo {
        name: "lx-nouveau",
        feature: "lxdde",
        rule: MatchRule::NvidiaGpu,
        lxdde_port: Some("nouveau"),
    },
    DriverInfo {
        name: "virtio-blk",
        feature: "drv-virtio-blk",
        rule: MatchRule::Virtio { subtype: 2 },
        lxdde_port: None,
    },
    DriverInfo {
        name: "virtio-net",
        feature: "drv-virtio-net",
        rule: MatchRule::Virtio { subtype: 1 },
        lxdde_port: None,
    },
    DriverInfo {
        name: "e1000e",
        feature: "drv-e1000e",
        rule: MatchRule::PciIds {
            vendor: 0x8086,
            devices: &[0x10d3, 0x100e, 0x10f5, 0x10a4],
        },
        lxdde_port: None,
    },
    DriverInfo {
        name: "nvme",
        feature: "drv-nvme",
        rule: MatchRule::PciClass {
            class: 0x01,
            subclass: Some(0x08),
            prog_if: None,
        },
        lxdde_port: None,
    },
    DriverInfo {
        name: "usb-xhci",
        feature: "drv-usb",
        rule: MatchRule::PciClass {
            class: 0x0c,
            subclass: Some(0x03),
            prog_if: Some(0x30),
        },
        lxdde_port: None,
    },
    DriverInfo {
        name: "gpu-nvidia",
        feature: "drv-gpu-nvidia",
        rule: MatchRule::NvidiaGpu,
        lxdde_port: None,
    },
];

const VIRTIO_VENDOR: u16 = 0x1af4;

/// ¿Está compilado el código de este driver?
pub fn compiled(info: &DriverInfo) -> bool {
    match info.feature {
        "drv-virtio-blk" => cfg!(feature = "drv-virtio-blk"),
        "drv-virtio-net" => cfg!(feature = "drv-virtio-net"),
        "drv-e1000e" => cfg!(feature = "drv-e1000e"),
        "drv-nvme" => cfg!(feature = "drv-nvme"),
        "drv-usb" => cfg!(feature = "drv-usb"),
        "drv-gpu-nvidia" => cfg!(feature = "drv-gpu-nvidia"),
        "drv-live-disk" => cfg!(feature = "drv-live-disk"),
        "lxdde" => cfg!(feature = "lxdde"),
        _ => false,
    }
}

fn virtio_subtype(device_id: u16) -> Option<u8> {
    // Transitional: 0x1000 + id; modern: 0x1040 + id.
    if device_id >= 0x1040 && device_id <= 0x104f {
        Some((device_id - 0x1040) as u8)
    } else if device_id >= 0x1000 && device_id <= 0x103f {
        Some((device_id - 0x1000) as u8)
    } else {
        None
    }
}

pub fn matches(dev: &PciDevice, rule: MatchRule) -> bool {
    match rule {
        MatchRule::PciClass {
            class,
            subclass,
            prog_if,
        } => {
            if dev.class != class {
                return false;
            }
            if let Some(sc) = subclass {
                if dev.subclass != sc {
                    return false;
                }
            }
            if let Some(pi) = prog_if {
                if dev.prog_if != pi {
                    return false;
                }
            }
            true
        }
        MatchRule::PciIds { vendor, devices } => {
            dev.vendor_id == vendor && devices.contains(&dev.device_id)
        }
        MatchRule::NvidiaGpu => dev.vendor_id == 0x10de && dev.class == 0x03,
        MatchRule::IntelGpu => dev.vendor_id == 0x8086 && dev.class == 0x03,
        MatchRule::Virtio { subtype } => {
            dev.vendor_id == VIRTIO_VENDOR
                && virtio_subtype(dev.device_id) == Some(subtype)
        }
    }
}

/// Primer driver cuyo match encaja con el dispositivo (orden de prioridad = DRIVERS).
pub fn driver_for(dev: &PciDevice) -> Option<&'static DriverInfo> {
    DRIVERS.iter().find(|d| matches(dev, d.rule))
}

/// Entrada del informe hwscan. Los campos identificativos son metadatos
/// para consumidores externos; hoy solo se lee `compiled`.
#[derive(Clone)]
#[allow(dead_code)]
pub struct ScanEntry {
    pub bdf: String,
    pub vid: u16,
    pub did: u16,
    pub driver: &'static str,
    pub compiled: bool,
}

fn format_bdf(dev: &PciDevice) -> String {
    let mut s = String::new();
    let _ = write!(s, "{:02x}:{:02x}.{}", dev.bus, dev.device, dev.function);
    s
}

/// Genera líneas parseables: `drv: <bdf> <vid>:<did> <driver> <compilado|ausente>`.
pub fn hwscan_lines() -> Vec<String> {
    let devs = crate::drivers::pci::enumerate();
    let mut out = Vec::new();
    for dev in &devs {
        // live-disk no es un match PCI real; lo omitimos del scan por dispositivo.
        let Some(info) = driver_for(dev) else {
            continue;
        };
        let mut line = String::new();
        let _ = write!(
            line,
            "drv: {} {:04x}:{:04x} {} {}",
            format_bdf(dev),
            dev.vendor_id,
            dev.device_id,
            info.name,
            if compiled(info) {
                "compilado"
            } else {
                "ausente"
            }
        );
        out.push(line);
    }
    out
}

pub fn hwscan_entries() -> Vec<ScanEntry> {
    let devs = crate::drivers::pci::enumerate();
    let mut out = Vec::new();
    for dev in &devs {
        let Some(info) = driver_for(dev) else {
            continue;
        };
        if info.name == "live-disk" {
            continue;
        }
        out.push(ScanEntry {
            bdf: format_bdf(dev),
            vid: dev.vendor_id,
            did: dev.device_id,
            driver: info.name,
            compiled: compiled(info),
        });
    }
    out
}

/// Imprime el informe por consola (serie / fb).
pub fn print_hwscan() {
    println!("hwscan: {} dispositivos con driver conocido", hwscan_entries().len());
    for line in hwscan_lines() {
        println!("{line}");
    }
}

/// ¿Falta algún driver compilado para el hardware presente?
pub fn missing_drivers() -> bool {
    hwscan_entries().iter().any(|e| !e.compiled)
}
