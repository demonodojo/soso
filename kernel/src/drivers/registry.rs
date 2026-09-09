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
            devices: &[0x7f70, 0x51f0, 0x54f0, 0x2723],
        },
        lxdde_port: Some("iwlwifi"),
    },
    DriverInfo {
        name: "lx-ath11k",
        feature: "lxdde",
        rule: MatchRule::PciIds {
            vendor: 0x17cb,
            // WCN6855 (Steam Deck OLED, `17cb:1103`), y sus hermanos que
            // comparten transporte MHI/QMI en `ath11k/pci.c:27`.
            devices: &[0x1103, 0x1101, 0x1104],
        },
        lxdde_port: Some("ath11k"),
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
            devices: &[
                0x10d3, 0x100e, 0x10f5, 0x10a4, 0x15fc, 0x15f8, 0x15b8, 0x15b7, 0x15d8,
                0x0d4f, 0x15e3,
            ],
        },
        lxdde_port: None,
    },
    DriverInfo {
        name: "rtl8169",
        feature: "drv-rtl8169",
        rule: MatchRule::PciIds {
            vendor: 0x10ec,
            devices: &[0x8168, 0x8161, 0x8162, 0x8167, 0x8136],
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
    DriverInfo {
        name: "hda",
        feature: "drv-hda",
        rule: MatchRule::PciClass {
            class: 0x04,
            subclass: Some(0x03),
            prog_if: None,
        },
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
        "drv-rtl8169" => cfg!(feature = "drv-rtl8169"),
        "drv-nvme" => cfg!(feature = "drv-nvme"),
        "drv-usb" => cfg!(feature = "drv-usb"),
        "drv-gpu-nvidia" => cfg!(feature = "drv-gpu-nvidia"),
        "drv-live-disk" => cfg!(feature = "drv-live-disk"),
        "drv-hda" => cfg!(feature = "drv-hda"),
        "lxdde" => cfg!(feature = "lxdde"),
        _ => false,
    }
}

/// Device ID PCI → id de dispositivo virtio (1 = net, 2 = blk).
///
/// Los IDs transitional NO son `0x1000 + id`: están asignados a mano y no
/// siguen el orden de los modernos (0x1002 es balloon = 5, 0x1003 consola = 3).
/// Con la resta de más se etiquetaba el virtio-blk (1af4:1001) como
/// «virtio-net» y la NIC de verdad (1af4:1000) se quedaba sin driver en el
/// informe; la columna de clase PCI del hwscan lo destapó (2026-08-31).
fn virtio_subtype(device_id: u16) -> Option<u8> {
    // Modern: 0x1040 + id de dispositivo virtio.
    if (0x1040..=0x104f).contains(&device_id) {
        return Some((device_id - 0x1040) as u8);
    }
    // Transitional: tabla explícita (virtio 1.x, 4.1.2 «PCI Device Discovery»).
    match device_id {
        0x1000 => Some(1),  // net
        0x1001 => Some(2),  // blk
        0x1002 => Some(5),  // balloon
        0x1003 => Some(3),  // consola
        0x1004 => Some(8),  // scsi
        0x1005 => Some(4),  // entropía
        0x1009 => Some(9),  // 9p
        _ => None,
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

/// Nombre corto de la clase PCI, para leer el informe sin tabla delante.
fn clase_texto(dev: &PciDevice) -> &'static str {
    match (dev.class, dev.subclass) {
        (0x01, _) => "almacenamiento",
        (0x02, 0x00) => "ethernet",
        (0x02, 0x80) => "red-otra",
        (0x03, _) => "gráfica",
        (0x04, _) => "multimedia",
        (0x06, _) => "puente",
        (0x0c, 0x03) => "usb",
        (0x0c, _) => "bus-serie",
        _ => "otro",
    }
}

/// Chips que sabemos identificar aunque no tengamos (todavía) driver.
///
/// En una máquina sin puerto serie el informe hwscan del USB es la única
/// descripción del hardware, y un `1002:163f` a secas no dice nada. Los IDs de
/// la Steam Deck salen de `lxdde/linux/drivers/net/wireless/ath/ath11k/pci.c`
/// y de la enumeración PCI de la propia máquina.
fn chip_conocido(vid: u16, did: u16) -> Option<&'static str> {
    Some(match (vid, did) {
        (0x1002, 0x163f) => "AMD Van Gogh/Sephiroth (APU Steam Deck)",
        (0x1002, 0x1435) => "AMD Van Gogh ACP (audio I2S)",
        (0x17cb, 0x1103) => "Qualcomm WCN6855 / QCNFA765 (WiFi 6E)",
        (0x17cb, 0x1101) => "Qualcomm QCA6390 (WiFi)",
        (0x17cb, 0x1104) => "Qualcomm QCN9074 (WiFi)",
        (0x1022, 0x163a) => "AMD xHCI (FCH Van Gogh)",
        (0x1022, 0x7906) => "AMD SD/eMMC (FCH)",
        _ => return None,
    })
}

/// ¿Es un controlador de red (clase 0x02) sin driver que lo reclame?
fn red_sin_driver(dev: &PciDevice) -> bool {
    dev.class == 0x02 && driver_for(dev).is_none()
}

/// Genera líneas parseables, **una por dispositivo PCI**, con o sin driver:
///   `drv: <bdf> <vid>:<did> <driver> <compilado|ausente> clase <cc>:<ss>:<pi> <texto>`
///   `drv: <bdf> <vid>:<did> sin-driver desconocido clase <cc>:<ss>:<pi> <texto>`
///
/// Los cuatro primeros campos no se tocan: `xtask::drivers::parse_hwscan_line`
/// lee `driver` y `estado` por posición. Lo que se añadió (2026-08-31) es el
/// resto del bus: un hardware que no encaja con ninguna regla no aparecía en el
/// informe, así que la NIC que no arrancaba era justo la que no se veía.
pub fn hwscan_lines() -> Vec<String> {
    let devs = crate::drivers::pci::devices();
    let mut out = Vec::new();
    for dev in devs {
        let (nombre, estado) = match driver_for(dev) {
            Some(info) if compiled(info) => (info.name, "compilado"),
            Some(info) => (info.name, "ausente"),
            None => ("sin-driver", "desconocido"),
        };
        let mut line = String::new();
        let _ = write!(
            line,
            "drv: {} {:04x}:{:04x} {} {} clase {:02x}:{:02x}:{:02x} {}",
            format_bdf(dev),
            dev.vendor_id,
            dev.device_id,
            nombre,
            estado,
            dev.class,
            dev.subclass,
            dev.prog_if,
            clase_texto(dev)
        );
        // Campo extra al final: el parser de xtask lee por posición los
        // cuatro primeros, así que ampliar por la derecha es seguro.
        if let Some(chip) = chip_conocido(dev.vendor_id, dev.device_id) {
            let _ = write!(line, " [{chip}]");
        }
        out.push(line);
    }
    out.extend(usb_lines());
    out
}

/// Líneas del inventario USB (vacío sin driver USB compilado).
///
/// Van con las del hwscan porque comparten destino: consola y `SOSODRV.TXT`.
fn usb_lines() -> Vec<String> {
    #[cfg(feature = "drv-usb")]
    {
        crate::drivers::usb_storage::inventory_lines()
    }
    #[cfg(not(feature = "drv-usb"))]
    {
        Vec::new()
    }
}

/// Controladores de red presentes que ningún driver reclama.
pub fn nics_sin_driver() -> Vec<String> {
    let devs = crate::drivers::pci::devices();
    let mut out = Vec::new();
    for dev in devs.iter().filter(|d| red_sin_driver(d)) {
        let mut line = String::new();
        let _ = write!(
            line,
            "{} {:04x}:{:04x} ({})",
            format_bdf(dev),
            dev.vendor_id,
            dev.device_id,
            clase_texto(dev)
        );
        out.push(line);
    }
    out
}

pub fn hwscan_entries() -> Vec<ScanEntry> {
    let devs = crate::drivers::pci::devices();
    let mut out = Vec::new();
    for dev in devs {
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
    let lines = hwscan_lines();
    println!(
        "hwscan: {} dispositivos PCI, {} con driver conocido",
        lines.len(),
        hwscan_entries().len()
    );
    for line in lines {
        println!("{line}");
    }
    if missing_drivers() {
        println!("hwscan: hay driver conocido sin compilar (líneas «ausente»)");
    }
    for nic in nics_sin_driver() {
        println!("hwscan: RED SIN DRIVER {nic}");
    }
}

/// ¿Falta algún driver compilado para el hardware presente?
pub fn missing_drivers() -> bool {
    hwscan_entries().iter().any(|e| !e.compiled)
}
