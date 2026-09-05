//! USB mass storage (BOT) sobre xHCI — lectura del stick live (placa real / QEMU USB).
//!
//! Si no hay mass storage, el controlador puede quedar activo para teclado HID.
//! Se conservan todos los xHCI (como Linux: un HCD por controlador).

use crate::arch::pit;
use crate::arch::tsc;
use crate::drivers::dma;
use crate::drivers::pci;
use crate::mm;
use crate::println;
use alloc::vec::Vec;
use spin::Mutex;
use xhci_nostd::{set_allocator, set_delay_us, MassStorage, XhciController};

const SECTOR: usize = 512;
const CLASS_SERIAL_USB: u8 = 0x0c;
const SUBCLASS_USB2: u8 = 0x03;
const PROG_XHCI: u8 = 0x30;
const AMD_VENDOR: u16 = 0x1022;
const AMD_RENOIR_XHCI: u16 = 0x1639;

struct UsbHost {
    ctrl: XhciController,
    ms: Option<MassStorage>,
}

// SAFETY: acceso serializado con Mutex; punteros MMIO/DMA estables.
unsafe impl Send for UsbHost {}

static HOSTS: Mutex<Vec<UsbHost>> = Mutex::new(Vec::new());
static RESCAN_DONE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
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
    // Mismo criterio que NVMe: el HC escribe el event ring por DMA. El mapeo
    // write-back del bootloader dejaba EINT=1 y el software veía el anillo vacío
    // (timeout + dump de puertos en bucle).
    let (paddr, v) = dma::alloc_zeroed_uc(pages.max(1));
    (v.as_ptr(), paddr as u64)
}

fn soso_delay_us(us: u32) {
    if tsc::ready() {
        tsc::spin_us(us as u64);
        return;
    }
    // Fallback burdo si el TSC aún no está calibrado.
    let ms = (us as u64).div_ceil(1000).max(1);
    let fin = pit::uptime_ms() + ms;
    while pit::uptime_ms() < fin {
        core::hint::spin_loop();
    }
}

/// MEM + Bus Master (como `pci_enable_device` mínimo de Linux).
fn pci_enable(bus: u8, dev: u8, func: u8) {
    let cmd = pci::read16(bus, dev, func, 0x04);
    let new = cmd | 0x6;
    pci::write16(bus, dev, func, 0x04, new);
    let after = pci::read16(bus, dev, func, 0x04);
    println!("usb: PCI {:02x}:{:02x}.{func} cmd {cmd:#06x}→{after:#06x}", bus, dev);
}

fn is_xhci(d: &pci::PciDevice) -> bool {
    d.class == CLASS_SERIAL_USB && d.subclass == SUBCLASS_USB2 && d.prog_if == PROG_XHCI
}

fn try_probe_ctrl(
    xdev: &pci::PciDevice,
    bar_va: usize,
) -> Option<(XhciController, Option<MassStorage>)> {
    println!(
        "usb: probando xHCI {:04x}:{:04x} PCI {:02x}:{:02x}.{}",
        xdev.vendor_id,
        xdev.device_id,
        xdev.bus,
        xdev.device,
        xdev.function,
    );
    // Linux quirk_ryzen_xhci_d3hot: Renoir/Cezanne 0x1639 necesita más settle.
    if xdev.vendor_id == AMD_VENDOR && xdev.device_id == AMD_RENOIR_XHCI {
        tsc::spin_us(20_000);
    }

    let mut ctrl = unsafe { XhciController::init(bar_va) };
    if !ctrl.any_root_port_connected() {
        ctrl.recover_root_ports();
    }
    // Una pasada unificada: HID + hubs + BOT (root o detrás de hub).
    if ctrl.any_root_port_connected() {
        ctrl.drain_port_events();
        if let Some(ms) = ctrl.enumerate_usb_devices() {
            return Some((ctrl, Some(ms)));
        }
    }
    if ctrl.has_keyboard() {
        println!(
            "usb: xHCI {:04x}:{:04x} — teclado HID (sin mass storage)",
            xdev.vendor_id, xdev.device_id
        );
    } else {
        println!(
            "usb: xHCI {:04x}:{:04x} — sin mass storage",
            xdev.vendor_id, xdev.device_id
        );
    }
    Some((ctrl, None))
}

pub fn init() {
    init_log();
    set_allocator(soso_dma_alloc);
    set_delay_us(soso_delay_us);

    pci::init_ecam();
    let xhcs: Vec<_> = pci::enumerate().into_iter().filter(is_xhci).collect();
    if xhcs.is_empty() {
        println!("usb: sin controlador xHCI en PCI");
        return;
    }

    let mut hosts = Vec::new();
    for xdev in &xhcs {
        pci_enable(xdev.bus, xdev.device, xdev.function);
        let Some((bar, size)) = pci::bar_info(xdev.bus, xdev.device, xdev.function, 0) else {
            continue;
        };
        // Mapear el BAR completo (puertos/runtime pueden quedar fuera de 256 KiB).
        mm::ensure_mmio_mapped(bar, size.max(64 * 1024));
        let bar_va = mm::phys_to_virt(bar).as_u64() as usize;

        let Some((ctrl, ms)) = try_probe_ctrl(xdev, bar_va) else {
            continue;
        };

        if let Some(ref ms) = ms {
            println!(
                "usb: mass storage {:04x}:{:04x} — {} sectores ({} MiB)",
                xdev.vendor_id,
                xdev.device_id,
                ms.sectors,
                ms.sectors * SECTOR as u64 / (1024 * 1024)
            );
        }
        hosts.push(UsbHost { ctrl, ms });
    }

    let n = hosts.len();
    let has_ms = hosts.iter().any(|h| h.ms.is_some());
    let has_kbd = hosts.iter().any(|h| h.ctrl.has_keyboard());
    *HOSTS.lock() = hosts;

    if !has_ms && !has_kbd {
        println!("usb: {n} xHCI activos, sin mass storage ni teclado HID");
        println!("usb: ¿lineas «boot port N ccs=1» arriba? si todas ccs=0, UEFI suelto el bus");
        println!("usb: alternativa: instalar en disco con /dev/sda4/SOSOINSTALL/install-soso.sh");
    } else {
        println!("usb: {n} xHCI activos ms={} kbd={}", has_ms, has_kbd);
    }
}

#[allow(dead_code)]
pub fn present() -> bool {
    HOSTS.lock().iter().any(|h| h.ms.is_some())
}

pub fn read_sector(lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), &'static str> {
    let mut guard = HOSTS.lock();
    let host = guard
        .iter_mut()
        .find(|h| h.ms.is_some())
        .ok_or("sin usb")?;
    let ms = host.ms.as_ref().ok_or("sin mass storage")?;
    if lba >= ms.sectors {
        return Err("lba");
    }
    if host.ctrl.read_sector10(ms, lba as u32, buf) {
        Ok(())
    } else {
        Err("usb read")
    }
}

/// `buf.len()/512` sectores consecutivos en una sola transacción BOT.
pub fn read_sectors(lba: u64, buf: &mut [u8]) -> Result<(), &'static str> {
    let mut guard = HOSTS.lock();
    let host = guard
        .iter_mut()
        .find(|h| h.ms.is_some())
        .ok_or("sin usb")?;
    let ms = host.ms.as_ref().ok_or("sin mass storage")?;
    let n = (buf.len() / SECTOR) as u64;
    if lba.saturating_add(n) > ms.sectors {
        return Err("lba");
    }
    if host.ctrl.read_sectors10(ms, lba as u32, buf) {
        Ok(())
    } else {
        Err("usb read")
    }
}

pub fn write_sector(lba: u64, buf: &[u8; SECTOR]) -> Result<(), &'static str> {
    let mut guard = HOSTS.lock();
    let host = guard
        .iter_mut()
        .find(|h| h.ms.is_some())
        .ok_or("sin usb")?;
    let ms = host.ms.as_ref().ok_or("sin mass storage")?;
    if lba >= ms.sectors {
        return Err("lba");
    }
    if host.ctrl.write_sectors10(ms, lba as u32, buf) {
        Ok(())
    } else {
        Err("usb write")
    }
}

/// `buf.len()/512` sectores consecutivos en una transacción BOT (o sector a sector si falla).
pub fn write_sectors(lba: u64, buf: &[u8]) -> Result<(), &'static str> {
    let mut guard = HOSTS.lock();
    let host = guard
        .iter_mut()
        .find(|h| h.ms.is_some())
        .ok_or("sin usb")?;
    let ms = host.ms.as_ref().ok_or("sin mass storage")?;
    let n = (buf.len() / SECTOR) as u64;
    if lba.saturating_add(n) > ms.sectors {
        return Err("lba");
    }
    if host.ctrl.write_sectors10(ms, lba as u32, buf) {
        return Ok(());
    }
    drop(guard);
    for (i, chunk) in buf.chunks(SECTOR).enumerate() {
        let sec: &[u8; SECTOR] = chunk.try_into().map_err(|_| "usb write")?;
        write_sector(lba + i as u64, sec)?;
    }
    Ok(())
}

pub fn active() -> bool {
    HOSTS.lock().iter().any(|h| h.ms.is_some())
}

pub fn sector_count() -> Option<u64> {
    HOSTS
        .lock()
        .iter()
        .find_map(|h| h.ms.as_ref().map(|m| m.sectors))
}

/// Evento de teclado USB normalizado para el driver PS/2/keymap.
pub struct UsbKbdEvent {
    pub scancode: u8,
    pub usage_id: u8,
    pub pressed: bool,
    pub shift: bool,
    pub altgr: bool,
}

/// Sondea el teclado HID. **`try_lock`**: ver comentario en el bloque anterior.
pub fn poll_keyboard_event() -> Option<UsbKbdEvent> {
    use xhci_nostd::hid::{MOD_LEFT_SHIFT, MOD_RIGHT_ALT, MOD_RIGHT_SHIFT};

    let mut guard = HOSTS.try_lock()?;
    for host in guard.iter_mut() {
        if let Some(evt) = host.ctrl.poll_keyboard() {
            let shift = evt.modifiers & (MOD_LEFT_SHIFT | MOD_RIGHT_SHIFT) != 0;
            let altgr = evt.modifiers & MOD_RIGHT_ALT != 0;
            return Some(UsbKbdEvent {
                scancode: evt.scancode,
                usage_id: evt.usage_id,
                pressed: evt.pressed,
                shift,
                altgr,
            });
        }
    }
    None
}

#[allow(dead_code)]
pub fn poll_keyboard_scancode() -> Option<u8> {
    poll_keyboard_event().and_then(|e| {
        if e.pressed && e.scancode != 0 {
            Some(e.scancode)
        } else {
            None
        }
    })
}

/// Suelta `HOSTS` a la fuerza. **Solo para el panic handler**: el volcado de
/// `SOSOLOG.TXT` escribe por USB, y si el panic salta con el candado cogido
/// —que es lo normal si el fallo viene del propio camino del disco— el volcado
/// se queda girando y el pendrive conserva el log ANTERIOR. O sea: justo en el
/// caso que más falta hace, el diagnóstico no llega. Mismo razonamiento que el
/// `force_unlock` de la consola y del logbuf.
///
/// # Safety
/// Rompe la exclusión mutua. Solo es aceptable cuando el kernel ya se está
/// muriendo y lo único que queda por hacer es dejar constancia.
pub unsafe fn force_unlock() {
    unsafe { HOSTS.force_unlock() };
}

pub fn has_usb_keyboard() -> bool {
    HOSTS.lock().iter().any(|h| h.ctrl.has_keyboard())
}

/// Un único reintento tras `init` (live_disk); evita bucle de slots.
pub fn rescan() {
    if RESCAN_DONE.swap(true, core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let mut guard = HOSTS.lock();
    for host in guard.iter_mut() {
        host.ctrl.drain_port_events();
        if !host.ctrl.any_root_port_connected() {
            host.ctrl.recover_root_ports();
        }
        if host.ms.is_none() && host.ctrl.any_root_port_connected() {
            if let Some(ms) = host.ctrl.enumerate_usb_devices() {
                println!(
                    "usb: mass storage (rescan) — {} sectores ({} MiB)",
                    ms.sectors,
                    ms.sectors * SECTOR as u64 / (1024 * 1024)
                );
                host.ms = Some(ms);
            }
        }
    }
}
