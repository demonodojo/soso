//! USB mass storage (BOT) sobre xHCI — lectura del stick live (placa real).
//!
//! Estado: detecta xHCI y deja RUN; BOT/read pendiente (live en placa requiere esto).
//! Validación de imagen live: `SOSO_QEMU_LIVE=1` (virtio + GPT).

use crate::drivers::pci;
use crate::mm;
use crate::println;

const SECTOR: usize = 512;
const CLASS_SERIAL_USB: u8 = 0x0c;
const SUBCLASS_USB2: u8 = 0x03;
const PROG_XHCI: u8 = 0x30;

const USBCMD_RUN: u32 = 1 << 0;
const USBCMD_HCRST: u32 = 1 << 1;
const USBSTS_HCH: u32 = 1 << 12;

static mut XHCI_BASE: *mut u8 = core::ptr::null_mut();

pub fn init() {
    pci::init_ecam();
    let Some(dev) = pci::enumerate().into_iter().find(|d| {
        d.class == CLASS_SERIAL_USB && d.subclass == SUBCLASS_USB2 && d.prog_if == PROG_XHCI
    }) else {
        return;
    };
    let Some((bar, size)) = pci::bar_info(dev.bus, dev.device, dev.function, 0) else {
        return;
    };
    mm::ensure_mmio_mapped(bar, size.min(256 * 1024).max(4096));
    let base = mm::phys_to_virt(bar).as_mut_ptr::<u8>();
    unsafe {
        if xhci_try_init(base).is_ok() {
            XHCI_BASE = base;
            println!(
                "usb: xHCI {:04x}:{:04x} — storage BOT pendiente (L5c placa)",
                dev.vendor_id,
                dev.device_id
            );
        }
    }
}

pub fn present() -> bool {
    false // hasta implementar BOT
}

pub fn read_sector(_lba: u64, _buf: &mut [u8; SECTOR]) -> Result<(), &'static str> {
    Err("usb bot pendiente")
}

pub fn write_sector(_lba: u64, _buf: &[u8; SECTOR]) -> Result<(), &'static str> {
    Err("usb bot pendiente")
}

unsafe fn xhci_try_init(base: *mut u8) -> Result<(), ()> {
    let cap_len = core::ptr::read_volatile(base) as u32;
    let op = base.add(cap_len as usize);
    let usbcmd = op.add(0x00) as *mut u32;
    let usbsts = op.add(0x04) as *mut u32;
    usbcmd.write_volatile(usbcmd.read_volatile() | USBCMD_HCRST);
    for _ in 0..1_000_000 {
        if usbsts.read_volatile() & USBSTS_HCH != 0 {
            break;
        }
    }
    if usbsts.read_volatile() & USBSTS_HCH == 0 {
        return Err(());
    }
    usbcmd.write_volatile(USBCMD_RUN);
    for _ in 0..1_000_000 {
        if usbsts.read_volatile() & USBSTS_HCH == 0 {
            return Ok(());
        }
    }
    Err(())
}

#[allow(dead_code)]
unsafe fn xhci_base() -> Option<*mut u8> {
    if XHCI_BASE.is_null() {
        None
    } else {
        Some(XHCI_BASE)
    }
}
