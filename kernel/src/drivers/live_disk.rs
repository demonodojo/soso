//! Disco live: lee particiones GPT (1=ESP, 2=sosofs, 3=sosomfs) sobre un disco 512 B/LBA.
//!
//! Backend: USB mass storage, NVMe (dual-boot en disco dedicado), o virtio-blk0
//! (QEMU `SOSO_QEMU_LIVE`).

use block_dev::{Block, BlockDevice, BlockError, BLOCK_SIZE};
use sosofs::layout::MAGIC as SOSOFS_MAGIC;
use spin::Once;

const SECTOR: usize = 512;
const GPT_HDR_LBA: u64 = 1;
const GPT_PARTS_LBA: u64 = 2;
const NVME_SLOTS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveBackend {
    Virtio0,
    Usb,
    Nvme(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LivePart {
    backend: LiveBackend,
    first_lba: u64,
    sectors: u64,
}

static LIVE_ESP: Once<Option<LivePart>> = Once::new();
static LIVE_ROOT: Once<Option<LivePart>> = Once::new();
static LIVE_MODELS: Once<Option<LivePart>> = Once::new();

pub fn init() {
    crate::drivers::usb_storage::rescan();
    if try_backend(LiveBackend::Usb).is_some() {
        return;
    }
    for slot in 0..NVME_SLOTS {
        if crate::drivers::nvme::present_slot(slot) {
            if try_backend(LiveBackend::Nvme(slot as u8)).is_some() {
                return;
            }
        }
    }
    let _ = try_backend(LiveBackend::Virtio0);
    if !active() {
        crate::println!("live: sin GPT sosofs (USB/NVMe/virtio); solo kernel-shell");
    }
}

fn try_backend(backend: LiveBackend) -> Option<()> {
    // Antes esto devolvía None en silencio en cada paso: «no hay GPT» y «no pude
    // leer» se veían igual en pantalla, que es lo que escondió durante todo un
    // arranque un pendrive perfectamente detectado pero con capacidad falsa.
    let mut hdr = [0u8; SECTOR];
    if sector_reader(backend, GPT_HDR_LBA, &mut hdr).is_err() {
        crate::println!("live: {backend:?}: no pude leer la LBA {GPT_HDR_LBA} (cabecera GPT)");
        return None;
    }
    if &hdr[0..8] != b"EFI PART" {
        crate::println!(
            "live: {backend:?}: LBA {GPT_HDR_LBA} sin firma «EFI PART» (empieza por {:02x?})",
            &hdr[0..8]
        );
        return None;
    }
    let mut ents = [0u8; SECTOR * 4];
    for i in 0..4usize {
        let mut sec = [0u8; SECTOR];
        if sector_reader(backend, GPT_PARTS_LBA + i as u64, &mut sec).is_err() {
            crate::println!("live: {backend:?}: no pude leer la tabla de particiones");
            return None;
        }
        ents[i * SECTOR..(i + 1) * SECTOR].copy_from_slice(&sec);
    }
    let p1 = parse_entry(&ents, 0, backend);
    let (Some(p2), Some(p3)) = (parse_entry(&ents, 1, backend), parse_entry(&ents, 2, backend))
    else {
        crate::println!("live: {backend:?}: GPT sin particiones 2 y 3 (¿imagen live incompleta?)");
        return None;
    };
    if !partition_has_sosofs(backend, p2.first_lba) {
        crate::println!(
            "live: {backend:?}: sin magic SOSOFS10 en la LBA {} (partición 2)",
            p2.first_lba
        );
        return None;
    }
    crate::println!(
        "live: GPT backend={backend:?} root LBA {} ({} MiB) models LBA {} ({} MiB)",
        p2.first_lba,
        p2.sectors * SECTOR as u64 / (1024 * 1024),
        p3.first_lba,
        p3.sectors * SECTOR as u64 / (1024 * 1024)
    );
    if let Some(esp) = p1 {
        LIVE_ESP.call_once(|| Some(esp));
    }
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
        LiveBackend::Nvme(slot) => nvme_read_sector(slot, lba, buf),
    }
}

fn range_reader(backend: LiveBackend, lba: u64, buf: &mut [u8]) -> Result<(), ()> {
    if buf.is_empty() {
        return Ok(());
    }
    if buf.len() % SECTOR != 0 {
        return Err(());
    }
    match backend {
        LiveBackend::Virtio0 => crate::drivers::virtio_blk::read_sectors(lba, buf).map_err(|_| ()),
        LiveBackend::Usb => crate::drivers::usb_storage::read_sectors(lba, buf).map_err(|_| ()),
        LiveBackend::Nvme(slot) => nvme_read_sectors(slot, lba, buf),
    }
}

/// GPT y `sgdisk` usan LBA de 512 B; el namespace NVMe puede ser 512 o 4096 B.
fn nvme_read_sector(slot: u8, gpt_lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), ()> {
    let slot = slot as usize;
    let lba_size = crate::drivers::nvme::lba_size_slot(slot).ok_or(())?;
    match lba_size {
        512 => crate::drivers::nvme::read_lba_slot(slot, gpt_lba, buf).map_err(|_| ()),
        4096 => {
            let nvme_lba = gpt_lba / 8;
            let off = (gpt_lba % 8) as usize * SECTOR;
            let mut page = [0u8; 4096];
            crate::drivers::nvme::read_lba_slot(slot, nvme_lba, &mut page).map_err(|_| ())?;
            buf.copy_from_slice(&page[off..off + SECTOR]);
            Ok(())
        }
        _ => {
            crate::println!("live: NVMe slot {slot} LBA {lba_size} B no soportado (solo 512/4096)");
            Err(())
        }
    }
}

fn nvme_read_sectors(slot: u8, gpt_lba: u64, buf: &mut [u8]) -> Result<(), ()> {
    let mut lba = gpt_lba;
    for chunk in buf.chunks_mut(SECTOR) {
        let mut sec = [0u8; SECTOR];
        nvme_read_sector(slot, lba, &mut sec)?;
        chunk.copy_from_slice(&sec);
        lba += 1;
    }
    Ok(())
}

fn range_writer(backend: LiveBackend, lba: u64, buf: &[u8]) -> Result<(), ()> {
    if buf.is_empty() {
        return Ok(());
    }
    if buf.len() % SECTOR != 0 {
        return Err(());
    }
    match backend {
        LiveBackend::Virtio0 => {
            for (i, chunk) in buf.chunks(SECTOR).enumerate() {
                let sec: &[u8; SECTOR] = chunk.try_into().map_err(|_| ())?;
                crate::drivers::virtio_blk::write_sector(lba + i as u64, sec).map_err(|_| ())?;
            }
            Ok(())
        }
        LiveBackend::Usb => crate::drivers::usb_storage::write_sectors(lba, buf).map_err(|_| ()),
        LiveBackend::Nvme(slot) => nvme_write_sectors(slot, lba, buf),
    }
}

fn nvme_write_sectors(slot: u8, gpt_lba: u64, buf: &[u8]) -> Result<(), ()> {
    let mut lba = gpt_lba;
    for chunk in buf.chunks(SECTOR) {
        let sec: &[u8; SECTOR] = chunk.try_into().map_err(|_| ())?;
        nvme_write_sector(slot, lba, sec).map_err(|_| ())?;
        lba += 1;
    }
    Ok(())
}

fn nvme_write_sector(slot: u8, gpt_lba: u64, buf: &[u8; SECTOR]) -> Result<(), BlockError> {
    let slot = slot as usize;
    let lba_size = crate::drivers::nvme::lba_size_slot(slot).ok_or(BlockError::Io)?;
    match lba_size {
        512 => crate::drivers::nvme::write_lba_slot(slot, gpt_lba, buf).map_err(|_| BlockError::Io),
        4096 => {
            let nvme_lba = gpt_lba / 8;
            let off = (gpt_lba % 8) as usize * SECTOR;
            let mut page = [0u8; 4096];
            crate::drivers::nvme::read_lba_slot(slot, nvme_lba, &mut page).map_err(|_| BlockError::Io)?;
            page[off..off + SECTOR].copy_from_slice(buf);
            crate::drivers::nvme::write_lba_slot(slot, nvme_lba, &page).map_err(|_| BlockError::Io)
        }
        _ => {
            crate::println!("live: NVMe slot {slot} LBA {lba_size} B no soportado (solo 512/4096)");
            Err(BlockError::Io)
        }
    }
}

impl LivePart {
    fn read_sectors(&self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        range_reader(self.backend, self.first_lba + lba, buf).map_err(|_| BlockError::Io)
    }

    fn write_sectors(&self, lba: u64, buf: &[u8]) -> Result<(), BlockError> {
        range_writer(self.backend, self.first_lba + lba, buf).map_err(|_| BlockError::Io)
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
        LiveBackend::Nvme(slot) => nvme_write_sector(slot, part.first_lba + lba, buf),
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

/// Lee sectores de la partición 1 (ESP FAT).
pub fn esp_read_sectors(lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
    let part = LIVE_ESP.get().and_then(|p| *p).ok_or(BlockError::Io)?;
    part.read_sectors(lba, buf)
}

/// Escribe sectores de la partición 1 (ESP FAT).
pub fn esp_write_sectors(lba: u64, buf: &[u8]) -> Result<(), BlockError> {
    let part = LIVE_ESP.get().and_then(|p| *p).ok_or(BlockError::Io)?;
    part.write_sectors(lba, buf)
}
