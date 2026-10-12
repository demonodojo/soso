//! Tarjeta de red virtio-net sobre PCI (ECAM dinámico).
//!
//! Intenta MSI-X (un vector compartido para RX/TX/config); si falla, sigue
//! en modo polled como hasta ahora.

use crate::arch::irq;
use crate::drivers::pci;
use crate::drivers::virtio_hal::HalImpl;
use crate::println;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use spin::{Mutex, Once};
use virtio_drivers::device::net::VirtIONet;
use virtio_drivers::transport::DeviceType;
use virtio_drivers::transport::pci::bus::{Cam, Command, DeviceFunction, MmioCam, PciRoot};
use virtio_drivers::transport::pci::{PciTransport, virtio_device_type};

pub const QUEUE_SIZE: usize = 16;
/// Buffer por paquete: MTU ethernet + cabecera virtio-net.
pub const BUF_LEN: usize = 2048;

pub type Nic = VirtIONet<HalImpl, PciTransport, QUEUE_SIZE>;

pub static NET: Once<Mutex<Nic>> = Once::new();

/// Contador de interrupciones MSI-X recibidas (diagnóstico).
pub static MSI_IRQ_COUNT: AtomicU64 = AtomicU64::new(0);
static MSI_ARMED: AtomicBool = AtomicBool::new(false);

const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
const VIRTIO_PCI_CAP_VENDOR: u8 = 0x09;
const NO_VECTOR: u16 = 0xffff;

/// Devuelve la MAC si encontró tarjeta. Sonda una sola vez: `net::poll()` llama
/// aquí en cada vuelta del bucle ocioso mientras no haya pila, y reenumerar el
/// ECAM entero por vuelta dejaba el live de placa real girando sobre el bus e
/// inundando la consola con «no se encontró ningún virtio-net» (2026-08-31).
pub fn init() -> Option<[u8; 6]> {
    *SONDA.call_once(probe)
}

static SONDA: Once<Option<[u8; 6]>> = Once::new();

fn probe() -> Option<[u8; 6]> {
    let (ecam_base, ecam_size) = pci::ecam_mmio();
    crate::mm::ensure_mmio_mapped(ecam_base, ecam_size);
    let ecam_ptr = crate::mm::phys_to_virt(ecam_base).as_mut_ptr();
    let mut root = PciRoot::new(unsafe { MmioCam::new(ecam_ptr, Cam::Ecam) });

    let ecam = pci::ecam();
    let mut dispositivos = Vec::new();
    for bus in ecam.bus_start..=ecam.bus_end {
        dispositivos.extend(root.enumerate_bus(bus));
    }
    for (df, info) in dispositivos {
        if virtio_device_type(&info) != Some(DeviceType::Network) {
            continue;
        }
        println!("pci: virtio-net en {df} ({info})");
        root.set_command(
            df,
            Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
        );
        let transport = PciTransport::new::<HalImpl, _>(&mut root, df)
            .expect("fallo creando el transporte virtio PCI");
        let nic: Nic = VirtIONet::new(transport, BUF_LEN)
            .expect("fallo inicializando virtio-net");
        let mac = nic.mac_address();
        NET.call_once(|| Mutex::new(nic));

        // MSI-X después de crear el dispositivo (colas ya habilitadas).
        if let Err(e) = try_enable_msix(df) {
            println!("net: MSI-X no disponible ({e}); modo polled");
        }
        return Some(mac);
    }
    println!("net: no se encontró ningún virtio-net");
    None
}

fn try_enable_msix(df: DeviceFunction) -> Result<(), &'static str> {
    let bus = df.bus;
    let dev = df.device;
    let func = df.function;

    let msix = pci::find_msix(bus, dev, func).ok_or("sin capability MSI-X")?;
    let vector = irq::allocate(net_irq_handler).ok_or("sin vectores IRQ libres")?;
    let dest = pci::msix_default_dest();

    // Entrada 0 de la tabla MSI-X → nuestro vector.
    pci::msix_setup(&msix, 0, vector, dest)?;

    // Apuntar colas y config change al vector MSI-X 0.
    bind_virtio_msix_vectors(bus, dev, func, 0)?;

    MSI_ARMED.store(true, Ordering::SeqCst);
    println!(
        "net: MSI-X armado vector={vector:#x} dest_apic={dest} entradas={}",
        msix.table_size
    );
    Ok(())
}

/// Escribe `queue_msix_vector` / `msix_config` en el common cfg de virtio.
fn bind_virtio_msix_vectors(
    bus: u8,
    dev: u8,
    func: u8,
    msix_entry: u16,
) -> Result<(), &'static str> {
    let (cfg_phys, _) = find_virtio_common_cfg(bus, dev, func).ok_or("sin common cfg")?;
    crate::mm::ensure_mmio_mapped(cfg_phys, 64);
    let base = crate::mm::phys_to_virt(cfg_phys).as_mut_ptr::<u8>();

    unsafe {
        // msix_config @ offset 16
        core::ptr::write_volatile(base.add(16) as *mut u16, msix_entry);
        // Para cada cola: select + queue_msix_vector @ 26
        for q in 0u16..4 {
            core::ptr::write_volatile(base.add(22) as *mut u16, q); // queue_select
            let enabled = core::ptr::read_volatile(base.add(28) as *const u16);
            if enabled == 0 {
                continue;
            }
            core::ptr::write_volatile(base.add(26) as *mut u16, msix_entry);
        }
        // Asegurar que no dejamos NO_VECTOR en config
        let cfg_vec = core::ptr::read_volatile(base.add(16) as *const u16);
        if cfg_vec == NO_VECTOR {
            core::ptr::write_volatile(base.add(16) as *mut u16, msix_entry);
        }
    }
    Ok(())
}

fn find_virtio_common_cfg(bus: u8, dev: u8, func: u8) -> Option<(u64, u32)> {
    let status = pci::read16(bus, dev, func, 0x06);
    if status & (1 << 4) == 0 {
        return None;
    }
    let mut off = pci::read8(bus, dev, func, 0x34);
    for _ in 0..48 {
        if off < 0x40 || off == 0xff {
            break;
        }
        let id = pci::read8(bus, dev, func, off);
        let next = pci::read8(bus, dev, func, off + 1);
        if id == VIRTIO_PCI_CAP_VENDOR {
            let cfg_type = pci::read8(bus, dev, func, off + 3);
            if cfg_type == VIRTIO_PCI_CAP_COMMON_CFG {
                let bar = pci::read8(bus, dev, func, off + 4);
                let bar_off = pci::read32(bus, dev, func, off + 8);
                let (bar_addr, _) = pci::bar_info(bus, dev, func, bar)?;
                return Some((bar_addr + bar_off as u64, pci::read32(bus, dev, func, off + 12)));
            }
        }
        if next == 0 || next == off {
            break;
        }
        off = next;
    }
    None
}

/// Handler de la MSI-X de virtio-net. **Sólo hace ack y agenda: no procesa la
/// pila de red.**
///
/// AVERÍA (2026-08-01): aquí se llamaba a `crate::net::poll()`, así que la IRQ
/// ejecutaba smoltcp + sunset + `ssh::drive` con IF=0. Ese camino toma PROCS
/// (`task::exists` desde `net/ssh.rs`), las colas RX/TX de ssh, el VFS
/// (`spawn_console` carga /bin/sosh) y el heap del kernel (talc, con un
/// `spin::Mutex` pelado que el propio camino de TX pide al reservar su búfer).
/// Una syscall corre en ring 0 con las interrupciones ABIERTAS —el `sti` de
/// `syscall_entry`— sosteniendo esos mismos candados, y `sosh` hace una syscall
/// `write` POR CARÁCTER echoado: en el arranque de una sesión la ventana es
/// enorme. Si la IRQ caía dentro, monocore = giro eterno con IF=0.
///
/// Se medía como «la sesión SSH recibe banner y prompt (117 bytes) y el stdin no
/// vuelve nunca», 1 de cada 5 arranques, con el serie mudo desde «ssh: sesión
/// abierta» y sin poder abrir otra sesión («timed out during banner exchange»).
///
/// Es el mismo problema que el serie ya resolvía con `without_interrupts`
/// (`drivers/serial.rs`: «en monocore eso sería un interbloqueo»), y la solución
/// es la de Linux: la IRQ dura sólo hace ack y `__napi_schedule`; la pila corre
/// en softirq.
fn net_irq_handler() {
    let n = MSI_IRQ_COUNT.fetch_add(1, Ordering::Relaxed);
    if n == 0 {
        // `println!` sí es seguro desde IRQ: el serie toma su candado siempre
        // bajo `without_interrupts`, así que nadie puede sostenerlo con IF=1.
        println!("net: primera IRQ MSI-X recibida");
    }
    if let Some(nic) = NET.get() {
        if let Some(mut nic) = nic.try_lock() {
            let _ = nic.ack_interrupt();
        }
    }
    // Agendar y salir. Lo recoge `irq::dispatch` al volver a ring 3, el bucle
    // del scheduler o el tick del timer: los tres son contextos que no pueden
    // estar dentro de esos candados.
    crate::net::marcar_trabajo_pendiente();
}

/// ¿MSI-X activo?
#[allow(dead_code)]
pub fn msix_armed() -> bool {
    MSI_ARMED.load(Ordering::Relaxed)
}
