//! Disco virtio-blk sobre PCI (ECAM de q35).

use crate::drivers::virtio_hal::HalImpl;
use crate::println;
use alloc::vec::Vec;
use spin::{Mutex, Once};
use virtio_drivers::device::blk::{SECTOR_SIZE, VirtIOBlk};
use virtio_drivers::transport::DeviceType;
use virtio_drivers::transport::pci::bus::{Cam, Command, MmioCam, PciRoot};
use virtio_drivers::transport::pci::{PciTransport, virtio_device_type};

/// Base MMCONFIG (ECAM) de la máquina q35 de QEMU.
const ECAM_BASE: u64 = 0xB000_0000;
/// Bus 0 completo: 32 dispositivos x 8 funciones x 4 KiB de config.
const ECAM_BUS0_SIZE: u64 = 256 * 4096;

pub static BLK: Once<Mutex<VirtIOBlk<HalImpl, PciTransport>>> = Once::new();

pub fn init() {
    crate::mm::ensure_mmio_mapped(ECAM_BASE, ECAM_BUS0_SIZE);
    let ecam_ptr = crate::mm::phys_to_virt(ECAM_BASE).as_mut_ptr();
    let mut root = PciRoot::new(unsafe { MmioCam::new(ecam_ptr, Cam::Ecam) });

    let dispositivos: Vec<_> = root.enumerate_bus(0).collect();
    for (df, info) in dispositivos {
        if virtio_device_type(&info) != Some(DeviceType::Block) {
            continue;
        }
        println!("pci: virtio-blk en {df} ({info})");
        root.set_command(
            df,
            Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
        );
        let transport = PciTransport::new::<HalImpl, _>(&mut root, df)
            .expect("fallo creando el transporte virtio PCI");
        let blk = VirtIOBlk::new(transport).expect("fallo inicializando virtio-blk");
        println!(
            "blk: disco de {} sectores ({} MiB)",
            blk.capacity(),
            blk.capacity() * SECTOR_SIZE as u64 / (1024 * 1024)
        );
        BLK.call_once(|| Mutex::new(blk));
        return;
    }
    println!("blk: no se encontró ningún virtio-blk en el bus PCI 0");
}

pub fn read_sector(sector: u64, buf: &mut [u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    let blk = BLK.get().ok_or("no hay disco")?;
    blk.lock()
        .read_blocks(sector as usize, buf)
        .map_err(|_| "error de lectura")
}

pub fn write_sector(sector: u64, buf: &[u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    let blk = BLK.get().ok_or("no hay disco")?;
    let mut blk = blk.lock();
    blk.write_blocks(sector as usize, buf)
        .map_err(|_| "error de escritura")?;
    blk.flush().map_err(|_| "error de flush")
}

pub fn capacity_sectors() -> Option<u64> {
    BLK.get().map(|b| b.lock().capacity())
}
