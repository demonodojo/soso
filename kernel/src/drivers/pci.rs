//! Enumeración PCI genérica, ECAM dinámico (MCFG) y MSI-X.

use crate::arch::acpi;
use crate::arch::apic;
use crate::mm;
use crate::println;
use alloc::vec::Vec;
use spin::Once;

/// Fallback QEMU q35 si no hay MCFG.
const ECAM_FALLBACK: u64 = 0xB000_0000;

#[derive(Clone, Copy, Debug)]
pub struct EcamInfo {
    pub base: u64,
    pub bus_start: u8,
    pub bus_end: u8,
}

static ECAM: Once<EcamInfo> = Once::new();

/// Inicializa ECAM desde MCFG (o fallback). Llamar tras `acpi::init`.
pub fn init_ecam() {
    ECAM.call_once(|| {
        if let Some(m) = acpi::mcfg() {
            EcamInfo {
                base: m.base,
                bus_start: m.bus_start,
                bus_end: m.bus_end,
            }
        } else {
            println!("pci: ECAM fallback {:#x} buses 0-0", ECAM_FALLBACK);
            EcamInfo {
                base: ECAM_FALLBACK,
                bus_start: 0,
                bus_end: 0,
            }
        }
    });
    let e = ecam();
    // Cam::Ecam indexa desde bus 0 en `base`; mapear hasta bus_end inclusive.
    let size = (e.bus_end as u64 + 1) * 256 * 4096;
    mm::ensure_mmio_mapped(e.base, size);
}

pub fn ecam() -> EcamInfo {
    *ECAM.get().unwrap_or(&EcamInfo {
        base: ECAM_FALLBACK,
        bus_start: 0,
        bus_end: 0,
    })
}

/// Base física ECAM y tamaño (compat con virtio-drivers `MmioCam` + `Cam::Ecam`).
pub fn ecam_mmio() -> (u64, u64) {
    let e = ecam();
    let size = (e.bus_end as u64 + 1) * 256 * 4096;
    (e.base, size)
}

#[derive(Clone, Copy, Debug)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub bar0: u64,
    pub bar0_size: u64,
}

fn ecam_offset(bus: u8, dev: u8, func: u8, off: u8) -> u64 {
    let e = ecam();
    e.base
        + ((bus as u64) << 20 | (dev as u64) << 15 | (func as u64) << 12 | off as u64)
}

pub fn read32(bus: u8, dev: u8, func: u8, off: u8) -> u32 {
    let addr = ecam_offset(bus, dev, func, off);
    mm::ensure_mmio_mapped(addr, 4);
    unsafe { core::ptr::read_volatile(mm::phys_to_virt(addr).as_ptr()) }
}

pub fn write32(bus: u8, dev: u8, func: u8, off: u8, val: u32) {
    let addr = ecam_offset(bus, dev, func, off);
    mm::ensure_mmio_mapped(addr, 4);
    unsafe {
        core::ptr::write_volatile(mm::phys_to_virt(addr).as_mut_ptr(), val);
    }
}

pub fn read16(bus: u8, dev: u8, func: u8, off: u8) -> u16 {
    (read32(bus, dev, func, off & !3) >> ((off & 2) * 8)) as u16
}

pub fn write16(bus: u8, dev: u8, func: u8, off: u8, val: u16) {
    let aligned = off & !3;
    let shift = (off & 2) * 8;
    let old = read32(bus, dev, func, aligned);
    let mask = !(0xffffu32 << shift);
    write32(bus, dev, func, aligned, (old & mask) | ((val as u32) << shift));
}

pub fn read8(bus: u8, dev: u8, func: u8, off: u8) -> u8 {
    (read32(bus, dev, func, off & !3) >> ((off & 3) * 8)) as u8
}

/// Tamaño de un BAR por el método estándar (escribir unos y leer la máscara).
///
/// **Los dos dwords en un BAR de 64 bits.** Antes sólo se dimensionaba el bajo, y
/// para un BAR grande eso da cero: la apertura de FB de la GB205 son 16 GiB, o
/// sea que todos los bits de tamaño están por encima del 31 y el dword bajo se
/// lee entero a cero. `bar_info` lo interpretaba como "no hay BAR" y devolvía
/// None, así que la ventana de BAR1 no se podía ni intentar (silicio,
/// 2026-08-01). El comentario de `bar_info` ya decía "soporta 64-bit"; el tamaño
/// no lo soportaba.
fn bar_size(bus: u8, dev: u8, func: u8, bar_off: u8) -> u64 {
    let old_lo = read32(bus, dev, func, bar_off);
    if old_lo & 1 != 0 {
        return 0; // BAR de E/S
    }
    let is_64 = (old_lo >> 1) & 0b11 == 0b10;
    let old_hi = if is_64 {
        read32(bus, dev, func, bar_off + 4)
    } else {
        0
    };
    write32(bus, dev, func, bar_off, 0xffff_ffff);
    if is_64 {
        write32(bus, dev, func, bar_off + 4, 0xffff_ffff);
    }
    let lo = read32(bus, dev, func, bar_off);
    let hi = if is_64 {
        read32(bus, dev, func, bar_off + 4)
    } else {
        0
    };
    write32(bus, dev, func, bar_off, old_lo);
    if is_64 {
        write32(bus, dev, func, bar_off + 4, old_hi);
    }
    // La máscara se complementa con la ANCHURA del BAR: hacerlo a 64 bits en uno
    // de 32 metería los bits altos (que no son del campo) y el tamaño saldría
    // disparatado.
    if is_64 {
        let mask = ((hi as u64) << 32) | ((lo & !0xf) as u64);
        if mask == 0 {
            return 0;
        }
        (!mask).wrapping_add(1)
    } else {
        let mask = lo & !0xf;
        if mask == 0 {
            return 0;
        }
        (!mask as u64).wrapping_add(1)
    }
}

/// Lee BAR (memoria); soporta 64-bit. Devuelve (addr, size).
pub fn bar_info(bus: u8, dev: u8, func: u8, bar_index: u8) -> Option<(u64, u64)> {
    let off = 0x10 + bar_index * 4;
    let lo = read32(bus, dev, func, off);
    if lo & 1 != 0 {
        return None; // I/O BAR
    }
    let is_64 = (lo >> 1) & 0b11 == 0b10;
    let addr = if is_64 {
        let hi = read32(bus, dev, func, off + 4);
        ((hi as u64) << 32) | (lo as u64 & !0xf)
    } else {
        lo as u64 & !0xf
    };
    let size = bar_size(bus, dev, func, off);
    if addr == 0 || size == 0 {
        return None;
    }
    Some((addr, size))
}

pub fn enumerate() -> Vec<PciDevice> {
    init_ecam();
    let e = ecam();
    let mut out = Vec::new();
    for bus in e.bus_start..=e.bus_end {
        for dev in 0..32u8 {
            let vendor = read16(bus, dev, 0, 0);
            if vendor == 0xffff {
                continue;
            }
            let header = read8(bus, dev, 0, 0x0e);
            let max_func = if header & 0x80 != 0 { 8 } else { 1 };
            for func in 0..max_func {
                let vendor = read16(bus, dev, func, 0);
                if vendor == 0xffff {
                    continue;
                }
                let class_rev = read32(bus, dev, func, 0x08);
                let (bar0, bar0_size) = bar_info(bus, dev, func, 0).unwrap_or((0, 0));
                out.push(PciDevice {
                    bus,
                    device: dev,
                    function: func,
                    vendor_id: vendor,
                    device_id: read16(bus, dev, func, 2),
                    class: ((class_rev >> 24) & 0xff) as u8,
                    subclass: ((class_rev >> 16) & 0xff) as u8,
                    prog_if: ((class_rev >> 8) & 0xff) as u8,
                    bar0,
                    bar0_size,
                });
            }
        }
    }
    out
}

/// Foto del bus tomada en `init()`, antes de que ningún driver programe nada.
static SNAPSHOT: spin::Once<Vec<PciDevice>> = spin::Once::new();

/// Enumeración cacheada, para todo el que sólo quiera **mirar** el bus.
///
/// `enumerate()` mide cada BAR0 escribiéndole 0xffff_ffff y restaurándolo
/// después: con las colas de un dispositivo ya en marcha, ese instante deja el
/// BAR decodificando en una dirección falsa, y cualquier MMIO o DMA en vuelo
/// cae en el hueco. Hacerlo al arrancar los drivers es inocuo (nadie ha
/// programado nada todavía); repetirlo luego, no. El hwscan lee de aquí.
pub fn devices() -> &'static [PciDevice] {
    SNAPSHOT.call_once(enumerate)
}

pub fn init() {
    init_ecam();
    let devs = devices();
    println!("pci: {} dispositivos", devs.len());
    for d in devs {
        if d.class == 0x03 {
            println!(
                "pci: GPU {:04x}:{:04x} bar0={:#x} ({} KiB)",
                d.vendor_id,
                d.device_id,
                d.bar0,
                d.bar0_size / 1024
            );
        }
    }
}

// ---- MSI-X ----

const CAP_ID_MSIX: u8 = 0x11;
const PCI_STATUS_CAP_LIST: u16 = 1 << 4;
const PCI_CMD_INT_DISABLE: u16 = 1 << 10;

#[derive(Clone, Copy, Debug)]
pub struct MsixInfo {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    /// Offset de la capability MSI-X en config space.
    pub cap_off: u8,
    #[allow(dead_code)]
    pub table_bar: u8,
    #[allow(dead_code)]
    pub table_offset: u32,
    pub table_size: u16,
    pub table_phys: u64,
}

/// Localiza la capability MSI-X (0x11).
pub fn find_msix(bus: u8, dev: u8, func: u8) -> Option<MsixInfo> {
    let status = read16(bus, dev, func, 0x06);
    if status & PCI_STATUS_CAP_LIST == 0 {
        return None;
    }
    let mut off = read8(bus, dev, func, 0x34);
    for _ in 0..48 {
        if off < 0x40 || off == 0xff {
            break;
        }
        let id = read8(bus, dev, func, off);
        let next = read8(bus, dev, func, off + 1);
        if id == CAP_ID_MSIX {
            let msg_ctrl = read16(bus, dev, func, off + 2);
            let table_size = (msg_ctrl & 0x7ff) + 1;
            let table_bir = read32(bus, dev, func, off + 4);
            let table_bar = (table_bir & 0x7) as u8;
            let table_offset = table_bir & !0x7;
            let (bar_addr, _) = bar_info(bus, dev, func, table_bar)?;
            let table_phys = bar_addr + table_offset as u64;
            return Some(MsixInfo {
                bus,
                device: dev,
                function: func,
                cap_off: off,
                table_bar,
                table_offset,
                table_size,
                table_phys,
            });
        }
        if next == 0 || next == off {
            break;
        }
        off = next;
    }
    None
}

/// Programa la entrada MSI-X `entry` con `vector` hacia el LAPIC `dest_apic`,
/// la desenmascara y habilita MSI-X en el dispositivo.
pub fn msix_setup(
    info: &MsixInfo,
    entry: u16,
    vector: u8,
    dest_apic: u32,
) -> Result<(), &'static str> {
    if entry >= info.table_size {
        return Err("entrada MSI-X fuera de rango");
    }
    let entry_phys = info.table_phys + entry as u64 * 16;
    mm::ensure_mmio_mapped(entry_phys, 16);

    // Address: fee0_0000 | (apic_id << 12) — destination physical.
    let addr = 0xFEE0_0000u64 | ((dest_apic as u64 & 0xff) << 12);
    let data = vector as u32; // Fixed, edge

    unsafe {
        let p = mm::phys_to_virt(entry_phys).as_mut_ptr::<u32>();
        core::ptr::write_volatile(p, addr as u32);
        core::ptr::write_volatile(p.add(1), (addr >> 32) as u32);
        core::ptr::write_volatile(p.add(2), data);
        core::ptr::write_volatile(p.add(3), 0); // unmasked
    }

    // Enable MSI-X (bit 15), clear function mask (bit 14).
    let mut ctrl = read16(info.bus, info.device, info.function, info.cap_off + 2);
    ctrl |= 1 << 15;
    ctrl &= !(1 << 14);
    write16(
        info.bus,
        info.device,
        info.function,
        info.cap_off + 2,
        ctrl,
    );

    // Disable INTx.
    let mut cmd = read16(info.bus, info.device, info.function, 0x04);
    cmd |= PCI_CMD_INT_DISABLE;
    write16(info.bus, info.device, info.function, 0x04, cmd);

    let _ = apic::id(); // ensure LAPIC ready
    Ok(())
}

/// Destino APIC por defecto para MSI-X: la BSP.
pub fn msix_default_dest() -> u32 {
    apic::id()
}
