//! Disco live: lee particiones GPT (1=ESP, 2=sosofs, 3=sosomfs; p4 SOSOINSTALL la ignora) sobre un disco 512 B/LBA.
//!
//! Backend: USB mass storage, NVMe (dual-boot en disco dedicado), o virtio-blk0
//! (QEMU `SOSO_QEMU_LIVE`).

use block_dev::{Block, BlockDevice, BlockError, BLOCK_SIZE};
use core::sync::atomic::{AtomicU64, Ordering};
use sosofs::layout::MAGIC as SOSOFS_MAGIC;
use spin::{Mutex, Once};

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

/// Índices en la tabla GPT del live (particiones 1=ESP, 2=root, 3=modelos, 4=install).
pub const GPT_ESP: usize = 0;
pub const GPT_ROOT: usize = 1;
pub const GPT_MODELS: usize = 2;
pub const GPT_INSTALL: usize = 3;

static LIVE_BACKEND: Once<Option<LiveBackend>> = Once::new();
static LIVE_ESP: Mutex<Option<LivePart>> = Mutex::new(None);
static LIVE_ROOT: Mutex<Option<LivePart>> = Mutex::new(None);
static LIVE_MODELS: Mutex<Option<LivePart>> = Mutex::new(None);
static GPT_RELOADS: AtomicU64 = AtomicU64::new(0);

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
        register_log_esp();
    }
}

/// Sin live montado no había dónde dejar el log: `fatlog` se apagaba porque
/// `active()` era falso, y en una placa sin puerto serie eso deja el arranque
/// sin ningún canal legible. Pero la ESP es la partición 1 y el root la 2 — son
/// independientes. Si el pendrive se lee, el log puede ir a su ESP igualmente.
///
/// Sólo se mira el USB a propósito: es donde existe `SOSOLOG.TXT`, y así no hay
/// forma de acabar escribiendo en la ESP del disco interno.
fn register_log_esp() {
    let backend = LiveBackend::Usb;
    let Some(ents) = read_gpt_entries(backend, true) else {
        return;
    };
    let Some(esp) = parse_entry(&ents, 0, backend) else {
        return;
    };
    crate::println!(
        "live: sin root, pero hay ESP en {backend:?} LBA {} — el log de arranque va ahí",
        esp.first_lba
    );
    *LIVE_ESP.lock() = Some(esp);
}

/// Lee la cabecera GPT y las 4 primeras entradas. Con `quiet` no imprime nada
/// (el respaldo de log reintenta sobre un backend que ya se quejó una vez).
fn read_gpt_entries(backend: LiveBackend, quiet: bool) -> Option<[u8; SECTOR * 4]> {
    // Antes cada paso devolvía None en silencio: «no hay GPT» y «no pude leer»
    // se veían igual en pantalla, que es lo que escondió durante todo un arranque
    // un pendrive perfectamente detectado pero con capacidad falsa.
    let mut hdr = [0u8; SECTOR];
    if sector_reader(backend, GPT_HDR_LBA, &mut hdr).is_err() {
        if !quiet {
            crate::println!("live: {backend:?}: no pude leer la LBA {GPT_HDR_LBA} (cabecera GPT)");
        }
        return None;
    }
    if &hdr[0..8] != b"EFI PART" {
        if !quiet {
            crate::println!(
                "live: {backend:?}: LBA {GPT_HDR_LBA} sin firma «EFI PART» (empieza por {:02x?})",
                &hdr[0..8]
            );
        }
        return None;
    }
    let mut ents = [0u8; SECTOR * 4];
    for i in 0..4usize {
        let mut sec = [0u8; SECTOR];
        if sector_reader(backend, GPT_PARTS_LBA + i as u64, &mut sec).is_err() {
            if !quiet {
                crate::println!("live: {backend:?}: no pude leer la tabla de particiones");
            }
            return None;
        }
        ents[i * SECTOR..(i + 1) * SECTOR].copy_from_slice(&sec);
    }
    Some(ents)
}

fn try_backend(backend: LiveBackend) -> Option<()> {
    let ents = read_gpt_entries(backend, false)?;
    let p1 = parse_entry(&ents, 0, backend);
    let (Some(p2), Some(p3)) = (parse_entry(&ents, 1, backend), parse_entry(&ents, 2, backend))
    else {
        crate::println!("live: {backend:?}: GPT sin particiones 2 y 3 (¿imagen live incompleta?)");
        return None;
    };
    if !partition_has_sosofs(backend, p2.first_lba) {
        crate::println!(
            "live: {backend:?}: sin magic SOSOFS11 en la LBA {} (partición 2)",
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
    LIVE_BACKEND.call_once(|| Some(backend));
    if let Some(esp) = p1 {
        *LIVE_ESP.lock() = Some(esp);
    }
    *LIVE_ROOT.lock() = Some(p2);
    *LIVE_MODELS.lock() = Some(p3);
    GPT_RELOADS.fetch_add(1, Ordering::Relaxed);
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
    crate::drivers::raw_disk::nvme_read_512_range(slot as usize, gpt_lba, buf).map_err(|_| ())
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
    crate::drivers::raw_disk::nvme_write_512_range(slot as usize, gpt_lba, buf).map_err(|_| ())
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

fn current_part(entry_index: usize) -> Option<LivePart> {
    match entry_index {
        GPT_ESP => *LIVE_ESP.lock(),
        GPT_ROOT => *LIVE_ROOT.lock(),
        GPT_MODELS => *LIVE_MODELS.lock(),
        GPT_INSTALL => None,
        _ => None,
    }
}

/// Relee la GPT y sustituye la geometría en RAM. Llamar solo tras un
/// cambio GPT confirmado (resize/recovery), no en el camino de datos.
pub fn refresh_geometry() -> bool {
    let Some(backend) = backend() else {
        return false;
    };
    let Some(ents) = read_gpt_entries(backend, true) else {
        return false;
    };
    if let Some(esp) = parse_entry(&ents, GPT_ESP, backend) {
        *LIVE_ESP.lock() = Some(esp);
    }
    *LIVE_ROOT.lock() = parse_entry(&ents, GPT_ROOT, backend);
    *LIVE_MODELS.lock() = parse_entry(&ents, GPT_MODELS, backend);
    GPT_RELOADS.fetch_add(1, Ordering::Relaxed);
    LIVE_ROOT.lock().is_some()
}

/// Recargas de la tabla GPT (arranque + refresh explícito). Diagnóstico B6.
#[allow(dead_code)]
pub fn gpt_reloads() -> u64 {
    GPT_RELOADS.load(Ordering::Relaxed)
}

/// Sectores de una partición cacheada (sin releer GPT).
pub fn cached_part_sectors(entry_index: usize) -> Option<u64> {
    current_part(entry_index).map(|p| p.sectors)
}

pub fn backend() -> Option<LiveBackend> {
    LIVE_BACKEND.get().copied().flatten()
}

/// Virtio y NVMe: flush de protocolo. USB: solo si SYNCHRONIZE CACHE(10)
/// contestó al enumerar.
pub fn backend_supports_durable_flush() -> bool {
    match backend() {
        Some(LiveBackend::Virtio0) => true,
        Some(LiveBackend::Usb) => crate::drivers::usb_storage::supports_durable_flush(),
        Some(LiveBackend::Nvme(_)) => true,
        None => false,
    }
}

/// Lee un sector LBA 512 B del disco GPT (cabecera, MBR, datos de particiones).
pub fn disk_read_sector(lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), BlockError> {
    let backend = backend().ok_or(BlockError::Io)?;
    sector_reader(backend, lba, buf).map_err(|_| BlockError::Io)
}

/// Escribe un sector LBA 512 B del disco GPT.
#[allow(dead_code)]
pub fn disk_write_sector(lba: u64, buf: &[u8; SECTOR]) -> Result<(), BlockError> {
    disk_write_sector_nosync(lba, buf)?;
    disk_flush()
}

/// Escritura sin barrera. El resize agrupa el `disk_flush` al final del slide.
pub fn disk_write_sector_nosync(lba: u64, buf: &[u8; SECTOR]) -> Result<(), BlockError> {
    let backend = backend().ok_or(BlockError::Io)?;
    match backend {
        LiveBackend::Virtio0 => {
            crate::drivers::virtio_blk::write_sector_nosync(lba, buf).map_err(|_| BlockError::Io)
        }
        LiveBackend::Usb => {
            crate::drivers::usb_storage::write_sector(lba, buf).map_err(|_| BlockError::Io)
        }
        LiveBackend::Nvme(slot) => nvme_write_sector(slot, lba, buf),
    }
}

pub fn disk_flush() -> Result<(), BlockError> {
    match backend() {
        Some(LiveBackend::Virtio0) => {
            crate::drivers::virtio_blk::flush().map_err(|_| BlockError::Io)
        }
        Some(LiveBackend::Usb) => {
            crate::drivers::usb_storage::flush().map_err(|_| BlockError::Io)
        }
        Some(LiveBackend::Nvme(slot)) => {
            crate::drivers::nvme::flush_slot(slot as usize).map_err(|_| BlockError::Io)
        }
        None => Err(BlockError::Io),
    }
}

/// Copia `len` sectores dentro del disco (mismo backend), de atrás hacia delante.
#[allow(dead_code)]
pub fn disk_slide_sectors(src_lba: u64, dst_lba: u64, sectors: u64) -> Result<(), BlockError> {
    if sectors == 0 {
        return Ok(());
    }
    if dst_lba <= src_lba {
        return Err(BlockError::Io);
    }
    let mut sec = [0u8; SECTOR];
    for i in (0..sectors).rev() {
        disk_read_sector(src_lba + i, &mut sec)?;
        disk_write_sector(dst_lba + i, &sec)?;
    }
    Ok(())
}

pub struct LiveRootDev {
    backend: LiveBackend,
}

pub struct LiveModelsDev {
    backend: LiveBackend,
}

impl LiveRootDev {
    fn part(&self) -> Option<LivePart> {
        current_part(GPT_ROOT).filter(|p| p.backend == self.backend)
    }
}

impl LiveModelsDev {
    fn part(&self) -> Option<LivePart> {
        current_part(GPT_MODELS).filter(|p| p.backend == self.backend)
    }
}

impl BlockDevice for LiveRootDev {
    fn block_count(&self) -> u64 {
        self.part()
            .map(|p| p.sectors / (BLOCK_SIZE / SECTOR) as u64)
            .unwrap_or(0)
    }

    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        let part = self.part().ok_or(BlockError::Io)?;
        read_block_512(&part, block, buf)
    }

    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        let part = self.part().ok_or(BlockError::Io)?;
        write_block_512(&part, block, buf)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        disk_flush()
    }

    fn max_blocks_per_request(&self) -> usize {
        sosomfs::MAX_REQ_BLOCKS
    }

    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let part = self.part().ok_or(BlockError::Io)?;
        read_blocks_512(&part, start, buf)
    }
}

impl BlockDevice for LiveModelsDev {
    fn block_count(&self) -> u64 {
        self.part()
            .map(|p| p.sectors / (BLOCK_SIZE / SECTOR) as u64)
            .unwrap_or(0)
    }

    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        let part = self.part().ok_or(BlockError::Io)?;
        read_block_512(&part, block, buf)
    }

    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        let part = self.part().ok_or(BlockError::Io)?;
        write_block_512(&part, block, buf)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        disk_flush()
    }

    fn max_blocks_per_request(&self) -> usize {
        sosomfs::MAX_REQ_BLOCKS
    }

    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let part = self.part().ok_or(BlockError::Io)?;
        read_blocks_512(&part, start, buf)
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
    LIVE_BACKEND
        .get()
        .and_then(|b| b.map(|backend| LiveRootDev { backend }))
}

pub fn models_dev() -> Option<LiveModelsDev> {
    LIVE_BACKEND
        .get()
        .and_then(|b| b.map(|backend| LiveModelsDev { backend }))
}

pub fn active() -> bool {
    LIVE_ROOT.lock().is_some()
}

/// ¿Hay una ESP donde escribir los logs? Puede haberla sin live montado.
pub fn esp_available() -> bool {
    LIVE_ESP.lock().is_some()
}

/// Lee sectores de la partición 1 (ESP FAT).
pub fn esp_read_sectors(lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
    let part = LIVE_ESP.lock().ok_or(BlockError::Io)?;
    part.read_sectors(lba, buf)
}

/// Escribe sectores de la partición 1 (ESP FAT).
pub fn esp_write_sectors(lba: u64, buf: &[u8]) -> Result<(), BlockError> {
    let part = LIVE_ESP.lock().ok_or(BlockError::Io)?;
    part.write_sectors(lba, buf)
}
