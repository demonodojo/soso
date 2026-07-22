//! Discos virtio-blk sobre PCI (ECAM de q35). v1: blk0=sosofs, blk1=sosomfs.

use crate::drivers::virtio_hal::HalImpl;
use crate::println;
use alloc::vec::Vec;
use spin::{Mutex, Once};
use virtio_drivers::device::blk::{SECTOR_SIZE, VirtIOBlk};
use virtio_drivers::transport::DeviceType;
use virtio_drivers::transport::pci::bus::{Cam, Command, MmioCam, PciRoot};
use virtio_drivers::transport::pci::{PciTransport, virtio_device_type};

type BlkDev = VirtIOBlk<HalImpl, PciTransport>;

pub static BLK0: Once<Mutex<BlkDev>> = Once::new();
pub static BLK1: Once<Mutex<BlkDev>> = Once::new();

pub fn init() {
    let (ecam_base, ecam_size) = crate::drivers::pci::ecam_mmio();
    crate::mm::ensure_mmio_mapped(ecam_base, ecam_size);
    let ecam_ptr = crate::mm::phys_to_virt(ecam_base).as_mut_ptr();
    let mut root = PciRoot::new(unsafe { MmioCam::new(ecam_ptr, Cam::Ecam) });

    let ecam = crate::drivers::pci::ecam();
    let mut dispositivos = Vec::new();
    for bus in ecam.bus_start..=ecam.bus_end {
        dispositivos.extend(root.enumerate_bus(bus));
    }
    let mut slot = 0u32;
    for (df, info) in dispositivos {
        if virtio_device_type(&info) != Some(DeviceType::Block) {
            continue;
        }
        println!("pci: virtio-blk[{slot}] en {df} ({info})");
        root.set_command(
            df,
            Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
        );
        let transport = PciTransport::new::<HalImpl, _>(&mut root, df)
            .expect("fallo creando el transporte virtio PCI");
        let blk = VirtIOBlk::new(transport).expect("fallo inicializando virtio-blk");
        println!(
            "blk[{slot}]: {} sectores ({} MiB)",
            blk.capacity(),
            blk.capacity() * SECTOR_SIZE as u64 / (1024 * 1024)
        );
        if slot == 0 {
            BLK0.call_once(|| Mutex::new(blk));
        } else if slot == 1 {
            BLK1.call_once(|| Mutex::new(blk));
        } else {
            println!("blk: disco extra ignorado (slot {slot})");
        }
        slot += 1;
    }
    if BLK0.get().is_none() {
        println!("blk: no se encontró ningún virtio-blk en el bus PCI 0");
    }
}

fn read_sector_slot(slot: u32, sector: u64, buf: &mut [u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    let blk = match slot {
        0 => BLK0.get().ok_or("no hay disco 0")?,
        1 => BLK1.get().ok_or("no hay disco 1")?,
        _ => return Err("slot inválido"),
    };
    blk.lock()
        .read_blocks(sector as usize, buf)
        .map_err(|_| "error de lectura")
}

fn write_sector_slot(slot: u32, sector: u64, buf: &[u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    let blk = match slot {
        0 => BLK0.get().ok_or("no hay disco 0")?,
        1 => BLK1.get().ok_or("no hay disco 1")?,
        _ => return Err("slot inválido"),
    };
    let mut blk = blk.lock();
    blk.write_blocks(sector as usize, buf)
        .map_err(|_| "error de escritura")?;
    blk.flush().map_err(|_| "error de flush")
}

pub fn read_sector(sector: u64, buf: &mut [u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    read_sector_slot(0, sector, buf)
}

pub fn write_sector(sector: u64, buf: &[u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    write_sector_slot(0, sector, buf)
}

pub fn read_sector1(sector: u64, buf: &mut [u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    read_sector_slot(1, sector, buf)
}

pub fn capacity_sectors() -> Option<u64> {
    BLK0.get().map(|b| b.lock().capacity())
}

pub fn capacity_sectors1() -> Option<u64> {
    BLK1.get().map(|b| b.lock().capacity())
}
