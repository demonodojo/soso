//! Enumeración PCI genérica y detección de dispositivos.

use crate::mm;
use crate::println;

const ECAM_BASE: u64 = 0xB000_0000;
const ECAM_BUS0_SIZE: u64 = 256 * 4096;

#[derive(Clone, Copy, Debug)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub bar0: u64,
    pub bar0_size: u64,
}

fn ecam_offset(bus: u8, dev: u8, func: u8, off: u8) -> u64 {
    ECAM_BASE + ((bus as u64) << 20 | (dev as u64) << 15 | (func as u64) << 12 | off as u64)
}

fn read32(bus: u8, dev: u8, func: u8, off: u8) -> u32 {
    let addr = ecam_offset(bus, dev, func, off);
    mm::ensure_mmio_mapped(addr, 4);
    unsafe { core::ptr::read_volatile(mm::phys_to_virt(addr).as_ptr()) }
}

fn read16(bus: u8, dev: u8, func: u8, off: u8) -> u16 {
    (read32(bus, dev, func, off & !3) >> ((off & 2) * 8)) as u16
}

fn bar_size(bus: u8, dev: u8, func: u8, bar_off: u8) -> u64 {
    let old = read32(bus, dev, func, bar_off);
    unsafe {
        core::ptr::write_volatile(
            mm::phys_to_virt(ecam_offset(bus, dev, func, bar_off)).as_mut_ptr::<u32>(),
            0xffff_ffff,
        );
    }
    let mask = read32(bus, dev, func, bar_off);
    unsafe {
        core::ptr::write_volatile(
            mm::phys_to_virt(ecam_offset(bus, dev, func, bar_off)).as_mut_ptr(),
            old,
        );
    }
    if old & 1 != 0 {
        return 0;
    }
    let size_mask = mask & !0xf;
    if size_mask == 0 {
        return 0;
    }
    (!size_mask as u64) + 1
}

pub fn enumerate() -> alloc::vec::Vec<PciDevice> {
    mm::ensure_mmio_mapped(ECAM_BASE, ECAM_BUS0_SIZE);
    let mut out = alloc::vec::Vec::new();
    for dev in 0..32u8 {
        let vendor = read16(0, dev, 0, 0);
        if vendor == 0xffff {
            continue;
        }
        let class_rev = read32(0, dev, 0, 0x08);
        let bar0 = read32(0, dev, 0, 0x10) as u64 & !0xf;
        out.push(PciDevice {
            bus: 0,
            device: dev,
            function: 0,
            vendor_id: vendor,
            device_id: read16(0, dev, 0, 2),
            class: ((class_rev >> 24) & 0xff) as u8,
            subclass: ((class_rev >> 16) & 0xff) as u8,
            bar0,
            bar0_size: bar_size(0, dev, 0, 0x10),
        });
    }
    out
}

pub fn init() {
    let devs = enumerate();
    println!("pci: {} dispositivos en bus 0", devs.len());
    for d in &devs {
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
