//! Disco live: lee particiones GPT (2=sosofs, 3=sosomfs) sobre un disco 512 B/LBA.
//!
//! Backend: virtio-blk0 (QEMU `SOSO_QEMU_LIVE`) o USB mass storage (placa real).

use block_dev::{Block, BlockDevice, BlockError, BLOCK_SIZE};
use sosofs::layout::MAGIC as SOSOFS_MAGIC;
use spin::Once;

const SECTOR: usize = 512;
const GPT_HDR_LBA: u64 = 1;
const GPT_PARTS_LBA: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveBackend {
    Virtio0,
    Usb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LivePart {
    backend: LiveBackend,
    first_lba: u64,
    sectors: u64,
}

static LIVE_ROOT: Once<Option<LivePart>> = Once::new();
static LIVE_MODELS: Once<Option<LivePart>> = Once::new();

pub fn init() {
    // Preferir USB (stick de arranque) sobre virtio.
    if try_backend(LiveBackend::Usb).is_some() {
        return;
    }
    let _ = try_backend(LiveBackend::Virtio0);
}

fn try_backend(backend: LiveBackend) -> Option<()> {
    if !sector_reader(backend, GPT_HDR_LBA, &mut [0u8; SECTOR]).is_ok() {
        return None;
    }
    let mut hdr = [0u8; SECTOR];
    sector_reader(backend, GPT_HDR_LBA, &mut hdr).ok()?;
    if &hdr[0..8] != b"EFI PART" {
        return None;
    }
    let mut ents = [0u8; SECTOR * 4];
    for i in 0..4usize {
        let mut sec = [0u8; SECTOR];
        sector_reader(backend, GPT_PARTS_LBA + i as u64, &mut sec).ok()?;
        ents[i * SECTOR..(i + 1) * SECTOR].copy_from_slice(&sec);
    }
    let p2 = parse_entry(&ents, 1, backend)?;
    let p3 = parse_entry(&ents, 2, backend)?;
    if !partition_has_sosofs(backend, p2.first_lba) {
        return None;
    }
    crate::println!(
        "live: GPT backend={backend:?} root LBA {} ({} MiB) models LBA {} ({} MiB)",
        p2.first_lba,
        p2.sectors * SECTOR as u64 / (1024 * 1024),
        p3.first_lba,
        p3.sectors * SECTOR as u64 / (1024 * 1024)
    );
    LIVE_ROOT.call_once(|| {
        Some(LivePart {
            backend,
            first_lba: p2.first_lba,
            sectors: p2.sectors,
        })
    });
    LIVE_MODELS.call_once(|| {
        Some(LivePart {
            backend,
            first_lba: p3.first_lba,
            sectors: p3.sectors,
        })
    });
    Some(())
}

fn parse_entry(table: &[u8], index: usize, backend: LiveBackend) -> Option<LivePart> {
    let off = index * 128;
    let ent = table.get(off..off + 128)?;
    // GUID type all-zero => partición vacía
    if ent[0..16].iter().all(|&b| b == 0) {
        return None;
    }
    let first = u64::from_le_bytes(ent[32..40].try_into().ok()?);
    let last = u64::from_le_bytes(ent[40..48].try_into().ok()?);
    if last < first {
        return None;
    }
    Some(LivePart {
        backend,
        first_lba: first,
        sectors: last - first + 1,
    })
}

fn partition_has_sosofs(backend: LiveBackend, first_lba: u64) -> bool {
    let mut sec = [0u8; SECTOR];
    if sector_reader(backend, first_lba, &mut sec).is_err() {
        return false;
    }
    sec.starts_with(&SOSOFS_MAGIC)
}

fn sector_reader(backend: LiveBackend, lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), ()> {
    match backend {
        LiveBackend::Virtio0 => crate::drivers::virtio_blk::read_sector(lba, buf).map_err(|_| ()),
        LiveBackend::Usb => crate::drivers::usb_storage::read_sector(lba, buf).map_err(|_| ()),
    }
}

fn range_reader(backend: LiveBackend, lba: u64, buf: &mut [u8]) -> Result<(), ()> {
    match backend {
        LiveBackend::Virtio0 => crate::drivers::virtio_blk::read_sectors(lba, buf).map_err(|_| ()),
        LiveBackend::Usb => crate::drivers::usb_storage::read_sectors(lba, buf).map_err(|_| ()),
    }
}

impl LivePart {
    fn read_sector(&self, lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), BlockError> {
        sector_reader(self.backend, self.first_lba + lba, buf).map_err(|_| BlockError::Io)
    }

    fn read_sectors(&self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        range_reader(self.backend, self.first_lba + lba, buf).map_err(|_| BlockError::Io)
    }
}

pub struct LiveRootDev(pub LivePart);
pub struct LiveModelsDev(pub LivePart);

impl BlockDevice for LiveRootDev {
    fn block_count(&self) -> u64 {
        self.0.sectors / (BLOCK_SIZE / SECTOR) as u64
    }

    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        read_block_512(&self.0, block, buf)
    }

    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        write_block_512(&self.0, block, buf)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }

    fn max_blocks_per_request(&self) -> usize {
        sosomfs::MAX_REQ_BLOCKS
    }

    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        read_blocks_512(&self.0, start, buf)
    }
}

impl BlockDevice for LiveModelsDev {
    fn block_count(&self) -> u64 {
        self.0.sectors / (BLOCK_SIZE / SECTOR) as u64
    }

    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        read_block_512(&self.0, block, buf)
    }

    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        write_block_512(&self.0, block, buf)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }

    fn max_blocks_per_request(&self) -> usize {
        sosomfs::MAX_REQ_BLOCKS
    }

    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        read_blocks_512(&self.0, start, buf)
    }
}

/// Un bloque de 4 KiB = 8 sectores, en **una** transacción.
///
/// Antes eran ocho: ocho peticiones al dispositivo y ocho `copy_from_slice` de
/// rebote por cada bloque del FS. Sobre USB eso son ocho CBW/datos/CSW.
fn read_block_512(part: &LivePart, block: u64, buf: &mut Block) -> Result<(), BlockError> {
    part.read_sectors(block * (BLOCK_SIZE / SECTOR) as u64, buf)
}

fn read_blocks_512(part: &LivePart, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
    if buf.len() % BLOCK_SIZE != 0 {
        return Err(BlockError::OutOfRange);
    }
    part.read_sectors(start * (BLOCK_SIZE / SECTOR) as u64, buf)
}

fn write_block_512(part: &LivePart, block: u64, buf: &Block) -> Result<(), BlockError> {
    let base = block * (BLOCK_SIZE / SECTOR) as u64;
    for i in 0..(BLOCK_SIZE / SECTOR) {
        let mut sec = [0u8; SECTOR];
        sec.copy_from_slice(&buf[i * SECTOR..(i + 1) * SECTOR]);
        write_sector(part, base + i as u64, &sec)?;
    }
    Ok(())
}

fn write_sector(part: &LivePart, lba: u64, buf: &[u8; SECTOR]) -> Result<(), BlockError> {
    match part.backend {
        LiveBackend::Virtio0 => crate::drivers::virtio_blk::write_sector(part.first_lba + lba, buf)
            .map_err(|_| BlockError::Io),
        LiveBackend::Usb => crate::drivers::usb_storage::write_sector(part.first_lba + lba, buf)
            .map_err(|_| BlockError::Io),
    }
}

pub fn root_dev() -> Option<LiveRootDev> {
    LIVE_ROOT.get().and_then(|p| p.as_ref().map(|x| LiveRootDev(*x)))
}

pub fn models_dev() -> Option<LiveModelsDev> {
    LIVE_MODELS.get().and_then(|p| p.as_ref().map(|x| LiveModelsDev(*x)))
}

pub fn active() -> bool {
    LIVE_ROOT.get().is_some_and(|p| p.is_some())
}
