//! USB mass storage (BOT) sobre xHCI — lectura del stick live (placa real / QEMU USB).

use crate::drivers::dma;
use crate::drivers::pci;
use crate::mm;
use crate::println;
use spin::Mutex;
use xhci_nostd::{set_allocator, MassStorage, XhciController};

const SECTOR: usize = 512;
const CLASS_SERIAL_USB: u8 = 0x0c;
const SUBCLASS_USB2: u8 = 0x03;
const PROG_XHCI: u8 = 0x30;

struct UsbDisk {
    ctrl: XhciController,
    ms: MassStorage,
}

// SAFETY: acceso serializado con Mutex; punteros MMIO/DMA estables.
unsafe impl Send for UsbDisk {}

static DISK: Mutex<Option<UsbDisk>> = Mutex::new(None);
static LOG_INIT: spin::Once<()> = spin::Once::new();

struct SosoLog;

impl log::Log for SosoLog {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if record.level() <= log::Level::Info {
            println!("{}", record.args());
        }
    }

    fn flush(&self) {}
}

static SOSO_LOG: SosoLog = SosoLog;

fn init_log() {
    LOG_INIT.call_once(|| {
        let _ = log::set_logger(&SOSO_LOG);
        log::set_max_level(log::LevelFilter::Info);
    });
}

fn soso_dma_alloc(size: usize, _align: usize) -> (*mut u8, u64) {
    let pages = (size + dma::PAGE_SIZE - 1) / dma::PAGE_SIZE;
    let paddr = dma::alloc_pages(pages.max(1));
    let v = dma::virt(paddr);
    unsafe { core::ptr::write_bytes(v.as_ptr(), 0, pages * dma::PAGE_SIZE) };
    (v.as_ptr(), paddr as u64)
}

fn pci_enable(bus: u8, dev: u8, func: u8) {
    let cmd = pci::read16(bus, dev, func, 0x04);
    pci::write16(bus, dev, func, 0x04, cmd | 0x6);
}

pub fn init() {
    init_log();
    set_allocator(soso_dma_alloc);

    pci::init_ecam();
    let Some(xdev) = pci::enumerate().into_iter().find(|d| {
        d.class == CLASS_SERIAL_USB && d.subclass == SUBCLASS_USB2 && d.prog_if == PROG_XHCI
    }) else {
        return;
    };
    pci_enable(xdev.bus, xdev.device, xdev.function);

    let Some((bar, size)) = pci::bar_info(xdev.bus, xdev.device, xdev.function, 0) else {
        return;
    };
    mm::ensure_mmio_mapped(bar, size.min(256 * 1024).max(4096));
    let bar_va = mm::phys_to_virt(bar).as_u64() as usize;

    let mut ctrl = unsafe { XhciController::init(bar_va) };
    let Some(ms) = ctrl.probe_mass_storage() else {
        println!(
            "usb: xHCI {:04x}:{:04x} — sin mass storage",
            xdev.vendor_id, xdev.device_id
        );
        return;
    };

    println!(
        "usb: mass storage {:04x}:{:04x} — {} sectores ({} MiB)",
        xdev.vendor_id,
        xdev.device_id,
        ms.sectors,
        ms.sectors * SECTOR as u64 / (1024 * 1024)
    );
    *DISK.lock() = Some(UsbDisk { ctrl, ms });
}

pub fn present() -> bool {
    DISK.lock().is_some()
}

pub fn read_sector(lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), &'static str> {
    let mut guard = DISK.lock();
    let disk = guard.as_mut().ok_or("sin usb")?;
    if lba >= disk.ms.sectors {
        return Err("lba");
    }
    if disk
        .ctrl
        .read_sector10(&disk.ms, lba as u32, buf)
    {
        Ok(())
    } else {
        Err("usb read")
    }
}

pub fn write_sector(_lba: u64, _buf: &[u8; SECTOR]) -> Result<(), &'static str> {
    Err("usb ro")
}
