//! Tarjeta de red virtio-net sobre PCI (ECAM de q35).

use crate::drivers::virtio_hal::HalImpl;
use crate::println;
use alloc::vec::Vec;
use spin::{Mutex, Once};
use virtio_drivers::device::net::VirtIONet;
use virtio_drivers::transport::DeviceType;
use virtio_drivers::transport::pci::bus::{Cam, Command, MmioCam, PciRoot};
use virtio_drivers::transport::pci::{PciTransport, virtio_device_type};

const ECAM_BASE: u64 = 0xB000_0000;
const ECAM_BUS0_SIZE: u64 = 256 * 4096;

pub const QUEUE_SIZE: usize = 16;
/// Buffer por paquete: MTU ethernet + cabecera virtio-net.
pub const BUF_LEN: usize = 2048;

pub type Nic = VirtIONet<HalImpl, PciTransport, QUEUE_SIZE>;

pub static NET: Once<Mutex<Nic>> = Once::new();

/// Devuelve la MAC si encontró tarjeta.
pub fn init() -> Option<[u8; 6]> {
    crate::mm::ensure_mmio_mapped(ECAM_BASE, ECAM_BUS0_SIZE);
    let ecam_ptr = crate::mm::phys_to_virt(ECAM_BASE).as_mut_ptr();
    let mut root = PciRoot::new(unsafe { MmioCam::new(ecam_ptr, Cam::Ecam) });

    let dispositivos: Vec<_> = root.enumerate_bus(0).collect();
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
        return Some(mac);
    }
    println!("net: no se encontró ningún virtio-net en el bus PCI 0");
    None
}
