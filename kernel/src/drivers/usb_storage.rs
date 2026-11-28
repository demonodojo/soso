//! USB mass storage (BOT) sobre xHCI — lectura del stick live (placa real / QEMU USB).
//!
//! Si no hay mass storage, el controlador puede quedar activo para teclado HID.
//! Se conservan todos los xHCI (como Linux: un HCD por controlador).

use crate::arch::irq;
use crate::arch::tsc;
use crate::drivers::dma;
use crate::drivers::pci;
use crate::mm;
use crate::{klogln, println};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use spin::Mutex;
use xhci_nostd::hid::{MOD_LEFT_SHIFT, MOD_RIGHT_ALT, MOD_RIGHT_SHIFT};
use xhci_nostd::{
    set_allocator, set_delay_us, KeyEvent, MassStorage, UsbHotplugEvent, UsbKind, XhciController,
};

const SECTOR: usize = 512;
const CLASS_SERIAL_USB: u8 = 0x0c;
const SUBCLASS_USB2: u8 = 0x03;
const PROG_XHCI: u8 = 0x30;
const AMD_VENDOR: u16 = 0x1022;
const AMD_RENOIR_XHCI: u16 = 0x1639;

struct UsbHost {
    /// Candado propio: el teclado HID puede sondearse con `try_lock` aunque
    /// otro hilo tenga cogido el vector `HOSTS` para un write BOT largo.
    ctrl: Mutex<XhciController>,
    ms: Option<MassStorage>,
    /// LBA 0 y 1 del disco del arranque. Si se desenchufa y vuelve, sólo se
    /// readopta si es el mismo: escribir el rootfs en otro pendrive lo
    /// destrozaría.
    ident: Option<alloc::boxed::Box<[u8; 2 * SECTOR]>>,
    /// Pendrives de datos de este controlador, con su id de disco.
    datos: Vec<(u32, MassStorage)>,
    bus: u8,
    device: u8,
    function: u8,
}

/// Punteros estables a cada `ctrl`. El sondeo HID no toma `HOSTS`: un write
/// del stick (el otro xHCI) no debe saltarse el teclado.
const MAX_XHCI: usize = 4;
static CTRL_PTR: [AtomicPtr<Mutex<XhciController>>; MAX_XHCI] = [
    AtomicPtr::new(core::ptr::null_mut()),
    AtomicPtr::new(core::ptr::null_mut()),
    AtomicPtr::new(core::ptr::null_mut()),
    AtomicPtr::new(core::ptr::null_mut()),
];
static CTRLS_READY: AtomicBool = AtomicBool::new(false);

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
            klogln!("{}", record.args());
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
    tsc::delay_us(us as u64);
}

/// MEM + Bus Master (como `pci_enable_device` mínimo de Linux).
fn pci_enable(bus: u8, dev: u8, func: u8) {
    let cmd = pci::read16(bus, dev, func, 0x04);
    let new = cmd | 0x6;
    pci::write16(bus, dev, func, 0x04, new);
    let after = pci::read16(bus, dev, func, 0x04);
    klogln!("usb: PCI {:02x}:{:02x}.{func} cmd {cmd:#06x}→{after:#06x}", bus, dev);
}

fn is_xhci(d: &pci::PciDevice) -> bool {
    d.class == CLASS_SERIAL_USB && d.subclass == SUBCLASS_USB2 && d.prog_if == PROG_XHCI
}

fn try_probe_ctrl(
    xdev: &pci::PciDevice,
    bar_va: usize,
) -> Option<(XhciController, Option<MassStorage>)> {
    klogln!(
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

    let mut ctrl = match unsafe { XhciController::init(bar_va) } {
        Ok(c) => c,
        Err(e) => {
            klogln!(
                "usb: xHCI {:04x}:{:04x} PCI {:02x}:{:02x}.{} omitido ({e})",
                xdev.vendor_id, xdev.device_id, xdev.bus, xdev.device, xdev.function
            );
            return None;
        }
    };
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
        klogln!(
            "usb: xHCI {:04x}:{:04x} — teclado HID (sin mass storage)",
            xdev.vendor_id, xdev.device_id
        );
    } else {
        klogln!(
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
    let xhcs: Vec<_> = pci::devices().iter().copied().filter(is_xhci).collect();
    if xhcs.is_empty() {
        println!("usb: sin controlador xHCI en PCI");
        return;
    }

    let mut hosts = Vec::new();
    for xdev in &xhcs {
        pci_enable(xdev.bus, xdev.device, xdev.function);
        let (bar, size) = if xdev.bar0 != 0 && xdev.bar0_size != 0 {
            (xdev.bar0, xdev.bar0_size)
        } else {
            continue;
        };
        // Mapear el BAR completo (puertos/runtime pueden quedar fuera de 256 KiB).
        mm::ensure_mmio_mapped(bar, size.max(64 * 1024));
        let bar_va = mm::phys_to_virt(bar).as_u64() as usize;

        let Some((ctrl, ms)) = try_probe_ctrl(xdev, bar_va) else {
            continue;
        };

        if let Some(ref ms) = ms {
            klogln!(
                "usb: mass storage {:04x}:{:04x} — {} sectores ({} MiB)",
                xdev.vendor_id,
                xdev.device_id,
                ms.sectors,
                ms.sectors * SECTOR as u64 / (1024 * 1024)
            );
        }
        let mut ctrl = ctrl;
        let extras = ctrl.take_extra_mass_storage();
        hosts.push(UsbHost {
            ctrl: Mutex::new(ctrl),
            ms,
            ident: None,
            datos: extras.into_iter().map(|m| (nuevo_id_datos(), m)).collect(),
            bus: xdev.bus,
            device: xdev.device,
            function: xdev.function,
        });
    }
    elegir_disco_de_arranque(&mut hosts);
    for host in hosts.iter_mut() {
        if let Some(ms) = host.ms {
            let mut ctrl = host.ctrl.lock();
            host.ident = disk_ident(&mut ctrl, &ms);
        }
        for (id, ms) in &host.datos {
            klogln!(
                "usb: pendrive de datos {} MiB (disco {id})",
                ms.sectors * SECTOR as u64 / (1024 * 1024)
            );
            CAMBIOS.lock().push(Cambio::Montar(*id, ms.sectors));
        }
    }

    let n = hosts.len();
    let has_ms = hosts.iter().any(|h| h.ms.is_some());
    let has_kbd = hosts.iter().any(|h| h.ctrl.lock().has_keyboard());
    *HOSTS.lock() = hosts;
    publish_ctrl_ptrs();

    if !has_ms && !has_kbd {
        println!("usb: {n} xHCI activos, sin mass storage ni teclado HID");
        println!("usb: ¿lineas «boot port N ccs=1» arriba? si todas ccs=0, UEFI suelto el bus");
        println!("usb: alternativa: instalar en disco con /dev/sda4/SOSOINSTALL/install-soso.sh");
    } else {
        klogln!("usb: {n} xHCI activos ms={} kbd={}", has_ms, has_kbd);
    }
}

/// Inventario USB para el informe de hardware.
///
/// En una máquina sin puerto serie (Steam Deck) el informe del USB es la única
/// descripción del bus, y de un HID que no habla el protocolo boot hace falta
/// el report descriptor entero para saber interpretarlo.
pub fn inventory_lines() -> Vec<alloc::string::String> {
    use core::fmt::Write;
    // Un report descriptor de teclado real no llega a 200 bytes; el tope evita
    // que un dispositivo hablador se coma el SOSODRV entero.
    const MAX_REPORT: usize = 512;
    let mut out = Vec::new();
    let hosts = HOSTS.lock();
    for host in hosts.iter() {
        let ctrl = host.ctrl.lock();
        for dev in ctrl.inventory() {
            let mut l = alloc::string::String::new();
            let _ = write!(
                l,
                "usb: dev slot={} port={} {:04x}:{:04x} clase {:02x}:{:02x}:{:02x}",
                dev.slot,
                dev.root_port,
                dev.vendor_id,
                dev.product_id,
                dev.class,
                dev.subclass,
                dev.protocol
            );
            out.push(l);
            for i in &dev.ifaces {
                let mut l = alloc::string::String::new();
                let _ = write!(
                    l,
                    "usb: if slot={} n={} alt={} clase {:02x}:{:02x}:{:02x} report={}",
                    dev.slot,
                    i.number,
                    i.alt,
                    i.class,
                    i.subclass,
                    i.protocol,
                    i.report_desc.len()
                );
                out.push(l);
                let n = i.report_desc.len().min(MAX_REPORT);
                for (k, trozo) in i.report_desc[..n].chunks(32).enumerate() {
                    let mut l = alloc::string::String::new();
                    let _ = write!(l, "usb: rep slot={} n={} {:03x} ", dev.slot, i.number, k * 32);
                    for b in trozo {
                        let _ = write!(l, "{b:02x}");
                    }
                    out.push(l);
                }
            }
        }
    }
    out
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
    let mut ctrl = host.ctrl.lock();
    if ctrl.read_sector10(ms, lba as u32, buf) {
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
    let mut ctrl = host.ctrl.lock();
    if ctrl.read_sectors10(ms, lba as u32, buf) {
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
    let mut ctrl = host.ctrl.lock();
    if ctrl.write_sectors10(ms, lba as u32, buf) {
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
    let mut ctrl = host.ctrl.lock();
    if ctrl.write_sectors10(ms, lba as u32, buf) {
        return Ok(());
    }
    drop(ctrl);
    drop(guard);
    for (i, chunk) in buf.chunks(SECTOR).enumerate() {
        let sec: &[u8; SECTOR] = chunk.try_into().map_err(|_| "usb write")?;
        write_sector(lba + i as u64, sec)?;
    }
    Ok(())
}

pub fn supports_durable_flush() -> bool {
    HOSTS
        .lock()
        .iter()
        .any(|h| h.ms.as_ref().is_some_and(|m| m.sync_cache))
}

pub fn flush() -> Result<(), &'static str> {
    let mut guard = HOSTS.lock();
    let host = guard
        .iter_mut()
        .find(|h| h.ms.is_some())
        .ok_or("sin usb")?;
    let ms = host.ms.as_ref().ok_or("sin mass storage")?;
    /* Linux sd_sync_cache: ILLEGAL_REQUEST / asc 0x20 → éxito; sin WCE no
     * emite flushes. Write-through asumido (sd.c:2974-2979 asc 0x24). */
    if !ms.sync_cache {
        return Ok(());
    }
    let mut ctrl = host.ctrl.lock();
    if ctrl.synchronize_cache10(ms) {
        Ok(())
    } else {
        Err("usb flush")
    }
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
    pub modifiers: u8,
}

/// Sondea el teclado HID. No toma `HOSTS`: el stick está en otro xHCI y un
/// write BOT no debe saltarse este sondeo. Solo `try_lock` del controlador
/// (Linux `hid_irq_in` no coge el candado del disco).
pub fn poll_pointer_event() -> Option<xhci_nostd::PointerEvent> {
    for_each_ctrl(|ctrl| ctrl.poll_pointer())
}

pub fn poll_keyboard_event() -> Option<UsbKbdEvent> {
    for_each_ctrl(|ctrl| ctrl.poll_keyboard().map(key_to_usb))
}

/// Vacía teclados. Si `deliver` devuelve false, la tecla vuelve a la cola
/// del controlador: ya no está en el anillo del xHCI.
pub fn drain_keyboard_events(mut deliver: impl FnMut(UsbKbdEvent) -> bool) {
    for_each_ctrl(|ctrl| {
        while let Some(raw) = ctrl.poll_keyboard() {
            if !deliver(key_to_usb(raw)) {
                ctrl.requeue_keyboard(raw);
                return Some(());
            }
        }
        None
    });
}

fn key_to_usb(evt: KeyEvent) -> UsbKbdEvent {
    UsbKbdEvent {
        modifiers: evt.modifiers,
        scancode: evt.scancode,
        usage_id: evt.usage_id,
        pressed: evt.pressed,
        shift: evt.modifiers & (MOD_LEFT_SHIFT | MOD_RIGHT_SHIFT) != 0,
        altgr: evt.modifiers & MOD_RIGHT_ALT != 0,
    }
}

fn for_each_ctrl<T>(mut f: impl FnMut(&mut XhciController) -> Option<T>) -> Option<T> {
    if CTRLS_READY.load(Ordering::Acquire) {
        for slot in &CTRL_PTR {
            let p = slot.load(Ordering::Acquire);
            if p.is_null() {
                continue;
            }
            let Some(mut ctrl) = (unsafe { &*p }).try_lock() else {
                continue;
            };
            if let Some(v) = f(&mut ctrl) {
                return Some(v);
            }
        }
        return None;
    }
    let guard = HOSTS.try_lock()?;
    for host in guard.iter() {
        let Some(mut ctrl) = host.ctrl.try_lock() else {
            continue;
        };
        if let Some(v) = f(&mut ctrl) {
            return Some(v);
        }
    }
    None
}

fn publish_ctrl_ptrs() {
    let hosts = HOSTS.lock();
    for (i, host) in hosts.iter().enumerate().take(MAX_XHCI) {
        CTRL_PTR[i].store(
            &host.ctrl as *const Mutex<XhciController> as *mut _,
            Ordering::Release,
        );
    }
    CTRLS_READY.store(true, Ordering::Release);
}

fn xhci_irq_0() {
    xhci_irq_slot(0);
}
fn xhci_irq_1() {
    xhci_irq_slot(1);
}
fn xhci_irq_2() {
    xhci_irq_slot(2);
}
fn xhci_irq_3() {
    xhci_irq_slot(3);
}

const XHCI_IRQS: [fn(); MAX_XHCI] = [xhci_irq_0, xhci_irq_1, xhci_irq_2, xhci_irq_3];

fn xhci_irq_slot(i: usize) {
    let p = CTRL_PTR[i].load(Ordering::Acquire);
    if p.is_null() {
        return;
    }
    let fired = {
        let Some(mut ctrl) = (unsafe { &*p }).try_lock() else {
            return;
        };
        ctrl.service_interrupt()
    };
    if !fired {
        return;
    }
    crate::drivers::diag_entrada::suma(&crate::drivers::diag_entrada::USB_IRQ);
    drain_keyboard_events(|evt| crate::drivers::kbd::deliver_usb_from_irq(evt));
}

/// MSI-X del interrupter 0, como Linux `xhci_try_enable_msi` + `xhci_irq`.
/// Solo en controladores con HID: el mass storage sigue en sondeo síncrono.
/// Los que ganen un HID en caliente se arman desde `poll_hotplug`.
pub fn arm_hid_irqs() {
    let n = HOSTS.lock().len().min(MAX_XHCI);
    for i in 0..n {
        let hid = {
            let hosts = HOSTS.lock();
            let ctrl = hosts[i].ctrl.lock();
            ctrl.has_keyboard() || ctrl.pointer_count() > 0
        };
        if hid && !IRQ_ARMED[i].load(Ordering::Acquire) {
            arm_hid_irq(i);
        }
    }
    IRQS_ARMED_ONCE.store(true, Ordering::Release);
}

fn arm_hid_irq(i: usize) {
    let (bus, device, function) = {
        let hosts = HOSTS.lock();
        let Some(h) = hosts.get(i) else {
            return;
        };
        (h.bus, h.device, h.function)
    };
    // Se marca antes de intentarlo: un vector que no se pudo pedir no se
    // vuelve a pedir en cada sondeo.
    IRQ_ARMED[i].store(true, Ordering::Release);
    let Some(vec) = irq::allocate(XHCI_IRQS[i]) else {
        klogln!("usb: xHCI {:02x}:{:02x}.{} sin vector IRQ", bus, device, function);
        return;
    };
    if let Some(msix) = pci::find_msix(bus, device, function) {
        if pci::msix_setup(&msix, 0, vec, pci::msix_default_dest()).is_ok() {
            klogln!("usb: xHCI {:02x}:{:02x}.{} MSI-X vector={vec:#x}", bus, device, function);
            return;
        }
    }
    let line = pci::read8(bus, device, function, 0x3c);
    let pin = pci::read8(bus, device, function, 0x3d);
    if pin != 0 && crate::arch::ioapic::route_isa(line, vec, pci::msix_default_dest()).is_ok() {
        klogln!(
            "usb: xHCI {:02x}:{:02x}.{} INTx IRQ{line} vector={vec:#x}",
            bus, device, function
        );
    } else {
        klogln!(
            "usb: xHCI {:02x}:{:02x}.{} sin MSI-X ni INTx; solo sondeo",
            bus, device, function
        );
    }
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
    usb_keyboard_count() > 0
}

pub fn usb_keyboard_count() -> u32 {
    HOSTS
        .lock()
        .iter()
        .map(|h| h.ctrl.lock().keyboard_count() as u32)
        .sum()
}

/// Un único reintento tras `init` (live_disk); evita bucle de slots.
pub fn rescan() {
    if RESCAN_DONE.swap(true, core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let mut guard = HOSTS.lock();
    for host in guard.iter_mut() {
        let mut ctrl = host.ctrl.lock();
        ctrl.drain_port_events();
        if !ctrl.any_root_port_connected() {
            ctrl.recover_root_ports();
        }
        if host.ms.is_none() && ctrl.any_root_port_connected() {
            if let Some(ms) = ctrl.enumerate_usb_devices() {
                klogln!(
                    "usb: mass storage (rescan) — {} sectores ({} MiB)",
                    ms.sectors,
                    ms.sectors * SECTOR as u64 / (1024 * 1024)
                );
                host.ident = disk_ident(&mut ctrl, &ms);
                host.ms = Some(ms);
            }
        }
        for ms in ctrl.take_extra_mass_storage() {
            let id = nuevo_id_datos();
            CAMBIOS.lock().push(Cambio::Montar(id, ms.sectors));
            host.datos.push((id, ms));
        }
    }
}

/// Un solo disco del arranque entre todos los controladores: el primero con
/// GPT (el live y una instalación de soso lo son; un pendrive de datos casi
/// nunca). Los demás pasan a ser pendrives de datos. Antes se quedaba el
/// primero que se enumeraba, y con dos pendrives podía tocarle al de datos.
fn elegir_disco_de_arranque(hosts: &mut [UsbHost]) {
    let mut cands: Vec<(usize, MassStorage, bool)> = Vec::new();
    for (i, h) in hosts.iter_mut().enumerate() {
        let mut todos: Vec<MassStorage> = h.ms.take().into_iter().collect();
        todos.extend(h.datos.drain(..).map(|(_, m)| m));
        let mut ctrl = h.ctrl.lock();
        for m in todos {
            let mut sec = [0u8; SECTOR];
            let gpt = ctrl.read_sector10(&m, 1, &mut sec) && &sec[0..8] == b"EFI PART";
            cands.push((i, m, gpt));
        }
    }
    let elegido = cands
        .iter()
        .position(|c| c.2)
        .or(if cands.is_empty() { None } else { Some(0) });
    for (k, (i, m, _)) in cands.into_iter().enumerate() {
        if Some(k) == elegido {
            hosts[i].ms = Some(m);
        } else {
            hosts[i].datos.push((nuevo_id_datos(), m));
        }
    }
}

static SIGUIENTE_ID_DATOS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(1);

fn nuevo_id_datos() -> u32 {
    SIGUIENTE_ID_DATOS.fetch_add(1, Ordering::Relaxed)
}

/// Montajes y desmontajes pendientes. Se apuntan con `HOSTS` cogido y se
/// atienden sin él: montar lee el disco, y eso vuelve a pedir `HOSTS`.
enum Cambio {
    Montar(u32, u64),
    Desmontar(u32),
}

static CAMBIOS: Mutex<Vec<Cambio>> = Mutex::new(Vec::new());

fn atender_cambios() {
    let pendientes: Vec<Cambio> = core::mem::take(&mut *CAMBIOS.lock());
    for c in pendientes {
        match c {
            Cambio::Montar(id, sectores) => crate::drivers::usbfs::montar(id, sectores),
            Cambio::Desmontar(id) => crate::drivers::usbfs::desmontar(id),
        }
    }
}

fn con_datos<T>(id: u32, f: impl FnOnce(&mut XhciController, &MassStorage) -> T) -> Option<T> {
    let hosts = HOSTS.lock();
    for h in hosts.iter() {
        if let Some((_, ms)) = h.datos.iter().find(|(i, _)| *i == id) {
            let ms = *ms;
            let mut ctrl = h.ctrl.lock();
            return Some(f(&mut ctrl, &ms));
        }
    }
    None
}

/// Lee `buf.len()/512` sectores de un pendrive de datos.
pub fn datos_leer(id: u32, lba: u64, buf: &mut [u8]) -> bool {
    con_datos(id, |ctrl, ms| {
        lba + (buf.len() / SECTOR) as u64 <= ms.sectors && ctrl.read_sectors10(ms, lba as u32, buf)
    })
    .unwrap_or(false)
}

/// Escribe `buf.len()/512` sectores en un pendrive de datos.
pub fn datos_escribir(id: u32, lba: u64, buf: &[u8]) -> bool {
    con_datos(id, |ctrl, ms| {
        lba + (buf.len() / SECTOR) as u64 <= ms.sectors && ctrl.write_sectors10(ms, lba as u32, buf)
    })
    .unwrap_or(false)
}

/// SYNCHRONIZE CACHE, si el pendrive lo admite.
pub fn datos_flush(id: u32) -> bool {
    con_datos(id, |ctrl, ms| !ms.sync_cache || ctrl.synchronize_cache10(ms)).unwrap_or(false)
}

/// Huella del disco: LBA 0 (MBR protector) y 1 (cabecera GPT con el GUID).
fn disk_ident(
    ctrl: &mut XhciController,
    ms: &MassStorage,
) -> Option<alloc::boxed::Box<[u8; 2 * SECTOR]>> {
    let mut out = alloc::boxed::Box::new([0u8; 2 * SECTOR]);
    for lba in 0..2u32 {
        let mut sec = [0u8; SECTOR];
        if !ctrl.read_sector10(ms, lba, &mut sec) {
            return None;
        }
        out[lba as usize * SECTOR..(lba as usize + 1) * SECTOR].copy_from_slice(&sec);
    }
    Some(out)
}

/// Cada cuánto mira el bus. Sin cambios son unas lecturas de PORTSC.
const HOTPLUG_PERIOD_MS: u64 = 250;
static HOTPLUG_LAST_MS: AtomicU64 = AtomicU64::new(0);
/// `arm_hid_irqs` ya corrió: los HID que lleguen después se arman aquí.
static IRQS_ARMED_ONCE: AtomicBool = AtomicBool::new(false);
static IRQ_ARMED: [AtomicBool; MAX_XHCI] = [
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
];

fn kind_name(k: UsbKind) -> &'static str {
    match k {
        UsbKind::Keyboard => "teclado",
        UsbKind::Pointer => "ratón",
        UsbKind::MassStorage => "almacenamiento",
        UsbKind::Hub => "hub",
        UsbKind::Other => "dispositivo",
    }
}

fn log_event(e: &UsbHotplugEvent) {
    let donde = if e.hub_port != 0 {
        alloc::format!("puerto {} (hub, puerto {})", e.root_port, e.hub_port)
    } else {
        alloc::format!("puerto {}", e.root_port)
    };
    klogln!(
        "usb: {} {} {:04x}:{:04x} en {donde}",
        kind_name(e.kind),
        if e.connected { "conectado" } else { "desconectado" },
        e.vendor_id,
        e.product_id,
    );
}

/// Corre `f` con las interrupciones abiertas y deja el estado como estaba.
///
/// Enumerar un dispositivo son cientos de ms de esperas activas. Con IF=0 en
/// la BSP se perdían los ticks del PIT (el reloj se retrasaba) y los IPI de
/// TLB shootdown de los otros núcleos se quedaban esperándola. Es seguro
/// abrirlas: en ring 0 el temporizador sólo cuenta ticks, y los handlers que
/// tocan el xHCI (IRQ del HID, teclado) usan `try_lock` y no esperan al
/// candado que tenemos.
fn con_interrupciones<T>(f: impl FnOnce() -> T) -> T {
    let antes = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::enable();
    let r = f();
    if !antes {
        x86_64::instructions::interrupts::disable();
    }
    r
}

/// Una vez, ya arrancado: si el sistema no arrancó del USB (instalado en
/// NVMe, virtio…), el pendrive que había enchufado no es el disco del
/// arranque sino uno de datos, y se monta como tal.
fn degradar_si_no_es_el_arranque(hosts: &mut [UsbHost]) {
    static HECHO: AtomicBool = AtomicBool::new(false);
    if HECHO.swap(true, Ordering::Relaxed) {
        return;
    }
    #[cfg(feature = "drv-live-disk")]
    let del_usb = crate::drivers::live_disk::backend()
        == Some(crate::drivers::live_disk::LiveBackend::Usb);
    #[cfg(not(feature = "drv-live-disk"))]
    let del_usb = false;
    if del_usb {
        return;
    }
    for h in hosts.iter_mut() {
        if let Some(ms) = h.ms.take() {
            h.ident = None;
            let id = nuevo_id_datos();
            klogln!("usb: el sistema no arrancó del USB; el pendrive pasa a ser de datos (disco {id})");
            h.datos.push((id, ms));
            CAMBIOS.lock().push(Cambio::Montar(id, ms.sectors));
        }
    }
}

/// Conexión y desconexión en caliente. La llama el bucle de la BSP.
///
/// Solo `try_lock`: si otro núcleo está en medio de un write BOT, se mira en
/// la pasada siguiente. Una conexión enumera aquí mismo (cientos de ms).
pub fn poll_hotplug() {
    if !CTRLS_READY.load(Ordering::Acquire) {
        return;
    }
    let now = crate::arch::pit::uptime_ms();
    let last = HOTPLUG_LAST_MS.load(Ordering::Relaxed);
    if now.wrapping_sub(last) < HOTPLUG_PERIOD_MS {
        return;
    }
    HOTPLUG_LAST_MS.store(now, Ordering::Relaxed);

    let Some(mut hosts) = HOSTS.try_lock() else {
        return;
    };
    let mut nuevos: Vec<(usize, MassStorage)> = Vec::new();
    let mut gana_hid: Vec<usize> = Vec::new();
    for (i, host) in hosts.iter_mut().enumerate() {
        let Some(mut ctrl) = host.ctrl.try_lock() else {
            continue;
        };
        let eventos = con_interrupciones(|| ctrl.poll_hotplug());
        for e in &eventos {
            log_event(e);
            if !e.connected && host.ms.as_ref().is_some_and(|m| m.slot_id == e.slot) {
                klogln!(
                    "usb: se ha retirado el disco del arranque; su E/S falla hasta que vuelva"
                );
                host.ms = None;
            }
            if !e.connected {
                if let Some(k) = host.datos.iter().position(|(_, m)| m.slot_id == e.slot) {
                    let (id, _) = host.datos.remove(k);
                    CAMBIOS.lock().push(Cambio::Desmontar(id));
                }
            }
            if e.connected && matches!(e.kind, UsbKind::Keyboard | UsbKind::Pointer) {
                gana_hid.push(i);
            }
        }
        for ms in ctrl.take_extra_mass_storage() {
            nuevos.push((i, ms));
        }
    }

    for (i, ms) in nuevos {
        let ident = {
            let mut ctrl = hosts[i].ctrl.lock();
            con_interrupciones(|| disk_ident(&mut ctrl, &ms))
        };
        // ¿Es el disco del arranque que vuelve? Su huella quedó en el host
        // donde estaba, que puede no ser éste (otro puerto, otro xHCI).
        let dueño = hosts.iter().position(|h| {
            h.ms.is_none()
                && h.ident.is_some()
                && ident.as_ref().is_some_and(|x| h.ident.as_ref() == Some(x))
        });
        let disco_activo = hosts.iter().any(|h| h.ms.is_some());
        match dueño {
            Some(j) if !disco_activo => {
                klogln!(
                    "usb: ha vuelto el disco del arranque ({} sectores); se reanuda su E/S",
                    ms.sectors
                );
                let huella = hosts[j].ident.take();
                hosts[i].ident = huella;
                hosts[i].ms = Some(ms);
            }
            _ => {
                let id = nuevo_id_datos();
                klogln!(
                    "usb: pendrive de datos {} MiB conectado (disco {id})",
                    ms.sectors * SECTOR as u64 / (1024 * 1024)
                );
                hosts[i].datos.push((id, ms));
                CAMBIOS.lock().push(Cambio::Montar(id, ms.sectors));
            }
        }
    }
    degradar_si_no_es_el_arranque(&mut hosts);
    drop(hosts);
    con_interrupciones(atender_cambios);

    if IRQS_ARMED_ONCE.load(Ordering::Acquire) {
        for i in gana_hid {
            if i < MAX_XHCI && !IRQ_ARMED[i].load(Ordering::Acquire) {
                arm_hid_irq(i);
            }
        }
    }
}
