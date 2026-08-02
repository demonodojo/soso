//! Acceso raw a discos de instalación (512 B/LBA): USB live, virtio-blk0, NVMe.

use soso_abi::{DiskInfo, DISK_FLAG_BOOT, DISK_FLAG_READONLY, DISK_KIND_NVME, DISK_KIND_USB, DISK_KIND_VIRTIO};

const SECTOR: usize = 512;
const NVME_SLOTS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum RawId {
    Usb = 0,
    Virtio0 = 1,
    Nvme0 = 2,
    Nvme1 = 3,
}

impl RawId {
    fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Usb),
            1 => Some(Self::Virtio0),
            2 => Some(Self::Nvme0),
            3 => Some(Self::Nvme1),
            _ => None,
        }
    }
}

pub fn list(out: &mut [DiskInfo]) -> usize {
    let mut n = 0usize;
    let boot = boot_source();

    if crate::drivers::usb_storage::active() {
        if n < out.len() {
            out[n] = info_usb(boot == Some(RawId::Usb));
            n += 1;
        }
    }
    if crate::drivers::virtio_blk::BLK0.get().is_some() {
        if n < out.len() {
            out[n] = info_virtio0(boot == Some(RawId::Virtio0));
            n += 1;
        }
    }
    for slot in 0..NVME_SLOTS {
        if crate::drivers::nvme::present_slot(slot) {
            if n < out.len() {
                out[n] = info_nvme(slot);
                n += 1;
            }
        }
    }
    n
}

/// Disco del que arrancó el live (origen de la imagen a clonar).
fn boot_source() -> Option<RawId> {
    if !crate::drivers::live_disk::active() {
        return None;
    }
    // live_disk no exporta backend; re-probar en el mismo orden que init().
    if crate::drivers::usb_storage::active() && probe_gpt_usb() {
        return Some(RawId::Usb);
    }
    if crate::drivers::virtio_blk::BLK0.get().is_some() && probe_gpt_virtio() {
        return Some(RawId::Virtio0);
    }
    None
}

fn probe_gpt_usb() -> bool {
    let mut sec = [0u8; SECTOR];
    crate::drivers::usb_storage::read_sector(1, &mut sec).is_ok() && &sec[0..8] == b"EFI PART"
}

fn probe_gpt_virtio() -> bool {
    let mut sec = [0u8; SECTOR];
    crate::drivers::virtio_blk::read_sector(1, &mut sec).is_ok() && &sec[0..8] == b"EFI PART"
}

fn info_usb(boot: bool) -> DiskInfo {
    let sectors = crate::drivers::usb_storage::sector_count().unwrap_or(0);
    let mut name = [0u8; 16];
    copy_name(&mut name, b"usb");
    DiskInfo {
        id: RawId::Usb as u32,
        kind: DISK_KIND_USB,
        slot: 0,
        sectors,
        flags: DISK_FLAG_READONLY | if boot { DISK_FLAG_BOOT } else { 0 },
        name,
    }
}

fn info_virtio0(boot: bool) -> DiskInfo {
    let sectors = crate::drivers::virtio_blk::capacity_sectors().unwrap_or(0);
    let mut name = [0u8; 16];
    copy_name(&mut name, b"virtio0");
    DiskInfo {
        id: RawId::Virtio0 as u32,
        kind: DISK_KIND_VIRTIO,
        slot: 0,
        sectors,
        flags: if boot { DISK_FLAG_BOOT } else { 0 },
        name,
    }
}

fn info_nvme(slot: usize) -> DiskInfo {
    let lba_size = crate::drivers::nvme::lba_size_slot(slot).unwrap_or(512);
    let n = crate::drivers::nvme::capacity_lba_slot(slot).unwrap_or(0);
    let sectors = n * lba_size as u64 / SECTOR as u64;
    let mut name = [0u8; 16];
    let label = if slot == 0 { b"nvme0" } else { b"nvme1" };
    copy_name(&mut name, label);
    DiskInfo {
        id: if slot == 0 { RawId::Nvme0 as u32 } else { RawId::Nvme1 as u32 },
        kind: DISK_KIND_NVME,
        slot: slot as u32,
        sectors,
        flags: 0,
        name,
    }
}

fn copy_name(dst: &mut [u8; 16], src: &[u8]) {
    let n = src.len().min(15);
    dst[..n].copy_from_slice(&src[..n]);
}

pub fn read(id: u32, lba: u64, buf: &mut [u8]) -> Result<(), i64> {
    if buf.len() % SECTOR != 0 || buf.is_empty() {
        return Err(-soso_abi::EINVAL);
    }
    let id = RawId::from_u32(id).ok_or(-soso_abi::EINVAL)?;
    if is_boot(id) && !writable(id) {
        // lectura OK
    }
    let mut off = 0usize;
    while off < buf.len() {
        let mut sec = [0u8; SECTOR];
        read_sector(id, lba + (off / SECTOR) as u64, &mut sec)?;
        let n = SECTOR.min(buf.len() - off);
        buf[off..off + n].copy_from_slice(&sec[..n]);
        off += n;
    }
    Ok(())
}

pub fn write(id: u32, lba: u64, buf: &[u8]) -> Result<(), i64> {
    if buf.len() % SECTOR != 0 || buf.is_empty() {
        return Err(-soso_abi::EINVAL);
    }
    let id = RawId::from_u32(id).ok_or(-soso_abi::EINVAL)?;
    if is_boot(id) {
        return Err(-soso_abi::EBUSY);
    }
    if !writable(id) {
        return Err(-soso_abi::EROFS);
    }
    let mut off = 0usize;
    while off < buf.len() {
        let mut sec = [0u8; SECTOR];
        sec.copy_from_slice(&buf[off..off + SECTOR]);
        write_sector(id, lba + (off / SECTOR) as u64, &sec)?;
        off += SECTOR;
    }
    Ok(())
}

fn is_boot(id: RawId) -> bool {
    boot_source() == Some(id)
}

fn writable(id: RawId) -> bool {
    matches!(id, RawId::Nvme0 | RawId::Nvme1)
}

fn read_sector(id: RawId, lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), i64> {
    match id {
        RawId::Usb => crate::drivers::usb_storage::read_sector(lba, buf).map_err(|_| -soso_abi::EIO),
        RawId::Virtio0 => {
            crate::drivers::virtio_blk::read_sector(lba, buf).map_err(|_| -soso_abi::EIO)
        }
        RawId::Nvme0 => nvme_read_512(0, lba, buf),
        RawId::Nvme1 => nvme_read_512(1, lba, buf),
    }
}

fn write_sector(id: RawId, lba: u64, buf: &[u8; SECTOR]) -> Result<(), i64> {
    match id {
        RawId::Nvme0 => nvme_write_512(0, lba, buf),
        RawId::Nvme1 => nvme_write_512(1, lba, buf),
        _ => Err(-soso_abi::EROFS),
    }
}

fn nvme_read_512(slot: usize, gpt_lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), i64> {
    if !crate::drivers::nvme::present_slot(slot) {
        return Err(-soso_abi::ENOENT);
    }
    let lba_size = crate::drivers::nvme::lba_size_slot(slot).ok_or(-soso_abi::EIO)?;
    match lba_size {
        512 => crate::drivers::nvme::read_lba_slot(slot, gpt_lba, buf).map_err(|_| -soso_abi::EIO),
        4096 => {
            let nvme_lba = gpt_lba / 8;
            let off = (gpt_lba % 8) as usize * SECTOR;
            let mut page = [0u8; 4096];
            crate::drivers::nvme::read_lba_slot(slot, nvme_lba, &mut page).map_err(|_| -soso_abi::EIO)?;
            buf.copy_from_slice(&page[off..off + SECTOR]);
            Ok(())
        }
        _ => Err(-soso_abi::ENOTSUP),
    }
}

fn nvme_write_512(slot: usize, gpt_lba: u64, buf: &[u8; SECTOR]) -> Result<(), i64> {
    if !crate::drivers::nvme::present_slot(slot) {
        return Err(-soso_abi::ENOENT);
    }
    let lba_size = crate::drivers::nvme::lba_size_slot(slot).ok_or(-soso_abi::EIO)?;
    match lba_size {
        512 => {
            crate::drivers::nvme::write_lba_slot(slot, gpt_lba, buf).map_err(|_| -soso_abi::EIO)
        }
        4096 => {
            let nvme_lba = gpt_lba / 8;
            let off = (gpt_lba % 8) as usize * SECTOR;
            let mut page = [0u8; 4096];
            crate::drivers::nvme::read_lba_slot(slot, nvme_lba, &mut page).map_err(|_| -soso_abi::EIO)?;
            page[off..off + SECTOR].copy_from_slice(buf);
            crate::drivers::nvme::write_lba_slot(slot, nvme_lba, &page).map_err(|_| -soso_abi::EIO)
        }
        _ => Err(-soso_abi::ENOTSUP),
    }
}
