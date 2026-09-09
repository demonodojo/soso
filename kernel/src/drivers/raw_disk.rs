//! Acceso raw a discos de instalación (512 B/LBA): USB live, virtio-blk0, NVMe.

use soso_abi::{
    DiskInfo, DISK_FLAG_BOOT, DISK_FLAG_EMPTY, DISK_FLAG_READONLY, DISK_FLAG_SOSO,
    DISK_KIND_NVME, DISK_KIND_USB, DISK_KIND_VIRTIO,
};
const SECTOR: usize = 512;
const NVME_SLOTS: usize = 2;
/// Bytes por transferencia al dispositivo. Clonar el live son lecturas USB
/// largas: 512 KiB por syscall, troceadas en el mass storage con TRBs
/// encadenados (escritura NVMe sin límite BOT).
const MAX_XFER: usize = 512 * 1024;

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

    #[cfg(feature = "drv-usb")]
    if crate::drivers::usb_storage::active() {
        if n < out.len() {
            out[n] = info_usb(boot == Some(RawId::Usb));
            out[n].flags |= probe_content(RawId::Usb);
            n += 1;
        }
    }
    #[cfg(feature = "drv-virtio-blk")]
    if crate::drivers::virtio_blk::BLK0.get().is_some() {
        if n < out.len() {
            out[n] = info_virtio0(boot == Some(RawId::Virtio0));
            out[n].flags |= probe_content(RawId::Virtio0);
            n += 1;
        }
    }
    #[cfg(feature = "drv-nvme")]
    for slot in 0..NVME_SLOTS {
        if crate::drivers::nvme::present_slot(slot) {
            if n < out.len() {
                let id = if slot == 0 {
                    RawId::Nvme0
                } else {
                    RawId::Nvme1
                };
                out[n] = info_nvme(slot, boot == Some(id));
                out[n].flags |= probe_content(id);
                n += 1;
            }
        }
    }
    n
}

fn boot_source() -> Option<RawId> {
    #[cfg(feature = "drv-live-disk")]
    if !crate::drivers::live_disk::active() {
        return None;
    }
    #[cfg(not(feature = "drv-live-disk"))]
    return None;

    #[cfg(all(feature = "drv-live-disk", feature = "drv-usb"))]
    if crate::drivers::usb_storage::active() && probe_gpt_usb() {
        return Some(RawId::Usb);
    }
    #[cfg(all(feature = "drv-live-disk", feature = "drv-virtio-blk"))]
    if crate::drivers::virtio_blk::BLK0.get().is_some() && probe_gpt_virtio() {
        return Some(RawId::Virtio0);
    }
    // Instalación dual-boot: el rootfs live vive en un NVMe dedicado, no en USB
    // ni virtio (`live_disk::init()` prueba NVMe como tercer backend). Sin esto,
    // `is_boot()`/`writable()` nunca reconocían el NVMe de arranque como tal, y
    // `sys_disk_write` dejaba escribir sectores crudos del propio disco en marcha.
    #[cfg(all(feature = "drv-live-disk", feature = "drv-nvme"))]
    for slot in 0..NVME_SLOTS {
        if crate::drivers::nvme::present_slot(slot) && probe_gpt_nvme(slot) {
            return Some(if slot == 0 { RawId::Nvme0 } else { RawId::Nvme1 });
        }
    }
    None
}

#[cfg(all(feature = "drv-live-disk", feature = "drv-usb"))]
fn probe_gpt_usb() -> bool {
    let mut sec = [0u8; SECTOR];
    crate::drivers::usb_storage::read_sector(1, &mut sec).is_ok() && &sec[0..8] == b"EFI PART"
}

#[cfg(all(feature = "drv-live-disk", feature = "drv-virtio-blk"))]
fn probe_gpt_virtio() -> bool {
    let mut sec = [0u8; SECTOR];
    crate::drivers::virtio_blk::read_sector(1, &mut sec).is_ok() && &sec[0..8] == b"EFI PART"
}

#[cfg(all(feature = "drv-live-disk", feature = "drv-nvme"))]
fn probe_gpt_nvme(slot: usize) -> bool {
    let mut sec = [0u8; SECTOR];
    nvme_read_512_range(slot, 1, &mut sec).is_ok() && &sec[0..8] == b"EFI PART"
}

#[cfg(feature = "drv-usb")]
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

#[cfg(feature = "drv-virtio-blk")]
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

#[cfg(feature = "drv-nvme")]
fn info_nvme(slot: usize, boot: bool) -> DiskInfo {
    let lba_size = crate::drivers::nvme::lba_size_slot(slot).unwrap_or(512);
    let n = crate::drivers::nvme::capacity_lba_slot(slot).unwrap_or(0);
    let sectors = n * lba_size as u64 / SECTOR as u64;
    let mut name = [0u8; 16];
    let label = if slot == 0 { b"nvme0" } else { b"nvme1" };
    copy_name(&mut name, label);
    DiskInfo {
        id: if slot == 0 {
            RawId::Nvme0 as u32
        } else {
            RawId::Nvme1 as u32
        },
        kind: DISK_KIND_NVME,
        slot: slot as u32,
        sectors,
        flags: if boot { DISK_FLAG_BOOT } else { 0 },
        name,
    }
}

fn copy_name(dst: &mut [u8; 16], src: &[u8]) {
    let n = src.len().min(15);
    dst[..n].copy_from_slice(&src[..n]);
}

/// Clasifica el contenido del disco para `soso-install list` y la guarda de destino.
/// Solo marcamos estados seguros (`SOSO`, `VACÍO`); lo demás queda sin flag.
fn probe_content(id: RawId) -> u32 {
    let mut sec = [0u8; SECTOR];
    if read_sector(id, 1, &mut sec).is_err() {
        return 0;
    }
    if &sec[0..8] == b"EFI PART" {
        let entry_lba = u64::from_le_bytes(sec[72..80].try_into().unwrap_or([0; 8]));
        return gpt_part2_flags(id, entry_lba);
    }

    if read_sector(id, 0, &mut sec).is_err() {
        return 0;
    }
    if sec[510] == 0x55 && sec[511] == 0xAA {
        for i in 0..4usize {
            if sec[446 + i * 16 + 4] != 0 {
                return 0;
            }
        }
    }
    if sec.iter().all(|&b| b == 0) {
        return DISK_FLAG_EMPTY;
    }
    0
}

fn gpt_part2_flags(id: RawId, entry_lba: u64) -> u32 {
    let mut sec = [0u8; SECTOR];
    if read_sector(id, entry_lba, &mut sec).is_err() {
        return 0;
    }
    let ent = &sec[128..256];
    if ent[0..16].iter().all(|&b| b == 0) {
        return 0;
    }
    let first = u64::from_le_bytes(ent[32..40].try_into().unwrap_or([0; 8]));
    if read_sector(id, first, &mut sec).is_err() {
        return 0;
    }
    if sosofs::layout::looks_like_sosofs(&sec) {
        DISK_FLAG_SOSO
    } else {
        0
    }
}

pub fn read(id: u32, lba: u64, buf: &mut [u8]) -> Result<(), i64> {
    if buf.len() % SECTOR != 0 || buf.is_empty() {
        return Err(-soso_abi::EINVAL);
    }
    let id = RawId::from_u32(id).ok_or(-soso_abi::EINVAL)?;
    for (i, part) in buf.chunks_mut(MAX_XFER).enumerate() {
        read_range(id, lba + (i * MAX_XFER / SECTOR) as u64, part)?;
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
    for (i, part) in buf.chunks(MAX_XFER).enumerate() {
        write_range(id, lba + (i * MAX_XFER / SECTOR) as u64, part)?;
    }
    Ok(())
}

fn is_boot(id: RawId) -> bool {
    boot_source() == Some(id)
}

fn writable(id: RawId) -> bool {
    matches!(id, RawId::Nvme0 | RawId::Nvme1)
}

fn read_range(id: RawId, lba: u64, buf: &mut [u8]) -> Result<(), i64> {
    match id {
        #[cfg(feature = "drv-usb")]
        RawId::Usb => {
            crate::drivers::usb_storage::read_sectors(lba, buf).map_err(|_| -soso_abi::EIO)
        }
        #[cfg(feature = "drv-virtio-blk")]
        RawId::Virtio0 => {
            crate::drivers::virtio_blk::read_sectors(lba, buf).map_err(|_| -soso_abi::EIO)
        }
        #[cfg(feature = "drv-nvme")]
        RawId::Nvme0 => nvme_read_512_range(0, lba, buf),
        #[cfg(feature = "drv-nvme")]
        RawId::Nvme1 => nvme_read_512_range(1, lba, buf),
        #[allow(unreachable_patterns)]
        _ => Err(-soso_abi::ENOENT),
    }
}

fn write_range(id: RawId, lba: u64, buf: &[u8]) -> Result<(), i64> {
    match id {
        #[cfg(feature = "drv-nvme")]
        RawId::Nvme0 => nvme_write_512_range(0, lba, buf),
        #[cfg(feature = "drv-nvme")]
        RawId::Nvme1 => nvme_write_512_range(1, lba, buf),
        _ => Err(-soso_abi::EROFS),
    }
}

/// Traduce un rango en LBA de 512 B (los que usa GPT) a los del namespace, que
/// puede tener bloques de 4096 B: extremos desalineados por read-modify-write y
/// el tramo central de una tacada.
#[cfg(feature = "drv-nvme")]
pub(crate) fn nvme_read_512_range(slot: usize, gpt_lba: u64, buf: &mut [u8]) -> Result<(), i64> {
    let lba_size = crate::drivers::nvme::lba_size_slot(slot).ok_or(-soso_abi::EIO)?;
    if lba_size == SECTOR {
        return crate::drivers::nvme::read_lba_slot(slot, gpt_lba, buf)
            .map_err(|_| -soso_abi::EIO);
    }
    if lba_size != 4096 {
        return Err(-soso_abi::ENOTSUP);
    }
    let per_page = 4096 / SECTOR as u64; // 8 sectores por bloque NVMe
    let mut lba = gpt_lba;
    let mut off = 0usize;
    let mut page = [0u8; 4096];

    if lba % per_page != 0 {
        let skip = (lba % per_page) as usize * SECTOR;
        let n = (4096 - skip).min(buf.len());
        nvme_page(slot, lba / per_page, &mut page)?;
        buf[..n].copy_from_slice(&page[skip..skip + n]);
        off = n;
        lba += (n / SECTOR) as u64;
    }

    let whole = (buf.len() - off) / 4096 * 4096;
    if whole > 0 {
        crate::drivers::nvme::read_lba_slot(slot, lba / per_page, &mut buf[off..off + whole])
            .map_err(|_| -soso_abi::EIO)?;
        off += whole;
        lba += (whole / SECTOR) as u64;
    }

    if off < buf.len() {
        let n = buf.len() - off;
        nvme_page(slot, lba / per_page, &mut page)?;
        buf[off..].copy_from_slice(&page[..n]);
    }
    Ok(())
}

#[cfg(feature = "drv-nvme")]
pub(crate) fn nvme_write_512_range(slot: usize, gpt_lba: u64, buf: &[u8]) -> Result<(), i64> {
    let lba_size = crate::drivers::nvme::lba_size_slot(slot).ok_or(-soso_abi::EIO)?;
    if lba_size == SECTOR {
        return crate::drivers::nvme::write_lba_slot(slot, gpt_lba, buf)
            .map_err(|_| -soso_abi::EIO);
    }
    if lba_size != 4096 {
        return Err(-soso_abi::ENOTSUP);
    }
    let per_page = 4096 / SECTOR as u64;
    let mut lba = gpt_lba;
    let mut off = 0usize;
    let mut page = [0u8; 4096];

    if lba % per_page != 0 {
        let skip = (lba % per_page) as usize * SECTOR;
        let n = (4096 - skip).min(buf.len());
        nvme_page(slot, lba / per_page, &mut page)?;
        page[skip..skip + n].copy_from_slice(&buf[..n]);
        nvme_page_write(slot, lba / per_page, &page)?;
        off = n;
        lba += (n / SECTOR) as u64;
    }

    let whole = (buf.len() - off) / 4096 * 4096;
    if whole > 0 {
        crate::drivers::nvme::write_lba_slot(slot, lba / per_page, &buf[off..off + whole])
            .map_err(|_| -soso_abi::EIO)?;
        off += whole;
        lba += (whole / SECTOR) as u64;
    }

    if off < buf.len() {
        let n = buf.len() - off;
        nvme_page(slot, lba / per_page, &mut page)?;
        page[..n].copy_from_slice(&buf[off..]);
        nvme_page_write(slot, lba / per_page, &page)?;
    }
    Ok(())
}

#[cfg(feature = "drv-nvme")]
fn nvme_page(slot: usize, nvme_lba: u64, page: &mut [u8; 4096]) -> Result<(), i64> {
    crate::drivers::nvme::read_lba_slot(slot, nvme_lba, page).map_err(|_| -soso_abi::EIO)
}

#[cfg(feature = "drv-nvme")]
fn nvme_page_write(slot: usize, nvme_lba: u64, page: &[u8; 4096]) -> Result<(), i64> {
    crate::drivers::nvme::write_lba_slot(slot, nvme_lba, page).map_err(|_| -soso_abi::EIO)
}

/// Un solo sector, para los sondeos de contenido (`probe_content`).
fn read_sector(id: RawId, lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), i64> {
    read_range(id, lba, buf)
}
