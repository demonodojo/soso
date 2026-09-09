//! Redimensionado recuperable: journal, slide reanudable y planificación GPT.
//!
//! Orden live (particiones GPT p1=ESP fija):
//! 1. Journal INTENT → shrink sosomfs (p3)
//! 2. Slide p3 (modelos) hacia delante
//! 3. Slide p4 (SOSOINSTALL) hacia delante
//! 4. Actualizar GPT (p2 crece al final; p3/p4 se desplazan)
//! 5. Grow sosofs (p2) — en kernel, tras montar root

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use gptdisk::{Header, entry, entry_first_lba, entry_last_lba, entry_mut, set_entry_first_lba,
              set_entry_last_lba};

pub const SECTOR: usize = 512;
pub const MAGIC: &[u8; 7] = b"SOSORES";

pub const JOURNAL_IDLE: u8 = 0;
pub const JOURNAL_SHRINK_MODELS: u8 = 1;
pub const JOURNAL_SLIDE_P3: u8 = 2;
pub const JOURNAL_SLIDE_P4: u8 = 3;
pub const JOURNAL_GPT: u8 = 4;
pub const JOURNAL_INTENT: u8 = 5;

/// Índices GPT live: 0=ESP, 1=root, 2=modelos, 3=install.
pub const GPT_ESP: usize = 0;
pub const GPT_ROOT: usize = 1;
pub const GPT_MODELS: usize = 2;
pub const GPT_INSTALL: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Io,
    InvalidJournal,
    InvalidGeometry,
    OutOfRange,
}

/// Entrada del journal en ESP (`SOSORES.TXT`, primer sector 512 B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Journal {
    pub phase: u8,
    pub delta_sectors: u64,
    /// Sectores ya copiados en la fase de slide activa (p3 o p4).
    pub slide_done: u64,
}

impl Journal {
    pub fn idle() -> Self {
        Self {
            phase: JOURNAL_IDLE,
            delta_sectors: 0,
            slide_done: 0,
        }
    }

    pub fn encode(&self, out: &mut [u8; SECTOR]) {
        out.fill(0);
        out[0] = self.phase;
        out[1..8].copy_from_slice(MAGIC);
        out[8..16].copy_from_slice(&self.delta_sectors.to_le_bytes());
        out[16..24].copy_from_slice(&self.slide_done.to_le_bytes());
    }

    pub fn decode(sec: &[u8; SECTOR]) -> Result<Self, Error> {
        if &sec[1..8] != MAGIC {
            // Hueco ESP de fábrica (`package-usb-live` rellena `\n`) o sector
            // a ceros: no hay resize en curso. Basura con magic ausente sí es
            // journal corrupto.
            if is_uninitialized(sec) {
                return Ok(Self::idle());
            }
            return Err(Error::InvalidJournal);
        }
        Ok(Self {
            phase: sec[0],
            delta_sectors: u64::from_le_bytes(sec[8..16].try_into().unwrap()),
            slide_done: u64::from_le_bytes(sec[16..24].try_into().unwrap()),
        })
    }

    pub fn valid_phase(&self) -> bool {
        matches!(
            self.phase,
            JOURNAL_IDLE
                | JOURNAL_INTENT
                | JOURNAL_SHRINK_MODELS
                | JOURNAL_SLIDE_P3
                | JOURNAL_SLIDE_P4
                | JOURNAL_GPT
        )
    }

    pub fn needs_recovery(&self) -> bool {
        self.valid_phase() && self.phase != JOURNAL_IDLE
    }
}

fn is_uninitialized(sec: &[u8; SECTOR]) -> bool {
    sec.iter()
        .all(|&b| matches!(b, 0 | b'\n' | b'\r' | b' ' | b'\t'))
}

/// Orden monótono de fases para reanudación.
pub fn phase_rank(phase: u8) -> Option<u8> {
    match phase {
        JOURNAL_IDLE => Some(0),
        JOURNAL_INTENT => Some(1),
        JOURNAL_SHRINK_MODELS => Some(2),
        JOURNAL_SLIDE_P3 => Some(3),
        JOURNAL_SLIDE_P4 => Some(4),
        JOURNAL_GPT => Some(5),
        _ => None,
    }
}

fn phase_at_most(phase: u8, target: u8) -> bool {
    matches!(
        (phase_rank(phase), phase_rank(target)),
        (Some(a), Some(b)) if a <= b
    )
}

fn phase_before(phase: u8, target: u8) -> bool {
    matches!(
        (phase_rank(phase), phase_rank(target)),
        (Some(a), Some(b)) if a < b
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartGeom {
    pub first: u64,
    pub last: u64,
    pub sectors: u64,
}

impl PartGeom {
    pub fn from_lba(first: u64, last: u64) -> Self {
        Self {
            first,
            sectors: last.saturating_sub(first) + 1,
            last,
        }
    }
}

/// E/S de sectores 512 B (GPT y slide).
pub trait Disk512 {
    fn read_sector(&self, lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), Error>;
    fn write_sector(&mut self, lba: u64, buf: &[u8; SECTOR]) -> Result<(), Error>;
    fn flush(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

/// Copia sectores hacia delante (destino > origen), de atrás hacia delante.
/// `*done` sectores ya están en destino; reanuda desde ahí. Actualiza `*done`.
pub fn slide_sectors<D: Disk512>(
    disk: &mut D,
    src_lba: u64,
    dst_lba: u64,
    sectors: u64,
    done: &mut u64,
) -> Result<(), Error> {
    if sectors == 0 {
        return Ok(());
    }
    if dst_lba <= src_lba {
        return Err(Error::InvalidGeometry);
    }
    if *done > sectors {
        return Err(Error::OutOfRange);
    }
    let mut sec = [0u8; SECTOR];
    let start = *done;
    while *done < sectors {
        let idx = sectors - 1 - *done;
        disk.read_sector(src_lba + idx, &mut sec)?;
        disk.write_sector(dst_lba + idx, &sec)?;
        *done += 1;
    }
    if let Err(e) = disk.flush() {
        // Sin flush durable el progreso no cuenta: reanudar reescribe.
        *done = start;
        return Err(e);
    }
    Ok(())
}

/// Lee geometría de una entrada GPT primaria.
pub fn read_part(disk: &impl Disk512, entry_index: usize) -> Result<PartGeom, Error> {
    let mut hdr_sec = [0u8; SECTOR];
    disk.read_sector(1, &mut hdr_sec)?;
    let hdr = Header::parse(&hdr_sec).map_err(|_| Error::InvalidGeometry)?;
    let bytes = hdr.entries_bytes();
    let padded = bytes.div_ceil(SECTOR) * SECTOR;
    let mut entries = alloc::vec![0u8; padded];
    for i in 0..hdr.entries_sectors() {
        let mut sec = [0u8; SECTOR];
        disk.read_sector(hdr.entries_lba + i, &mut sec)?;
        entries[i as usize * SECTOR..(i as usize + 1) * SECTOR].copy_from_slice(&sec);
    }
    entries.truncate(bytes);
    let ent = entry(&entries, &hdr, entry_index).ok_or(Error::InvalidGeometry)?;
    Ok(PartGeom::from_lba(entry_first_lba(ent), entry_last_lba(ent)))
}

/// Valida que el relayout post-resize encaja en el disco.
pub fn validate_geometry(
    disk: &impl Disk512,
    delta_sectors: u64,
) -> Result<(), Error> {
    let p2 = read_part(disk, GPT_ROOT)?;
    let p3 = read_part(disk, GPT_MODELS)?;
    let p4 = read_part(disk, GPT_INSTALL)?;
    if p3.sectors <= delta_sectors {
        return Err(Error::InvalidGeometry);
    }
    let new_p2_last = p2.last + delta_sectors;
    let new_p3_first = p3.first + delta_sectors;
    let new_p3_last = p3.last;
    let new_p4_first = p4.first + delta_sectors;
    let new_p4_last = p4.last + delta_sectors;

    let mut hdr_sec = [0u8; SECTOR];
    disk.read_sector(1, &mut hdr_sec)?;
    let mut hdr = Header::parse(&hdr_sec).map_err(|_| Error::InvalidGeometry)?;
    let bytes = hdr.entries_bytes();
    let padded = bytes.div_ceil(SECTOR) * SECTOR;
    let mut entries = alloc::vec![0u8; padded];
    for i in 0..hdr.entries_sectors() {
        let mut sec = [0u8; SECTOR];
        disk.read_sector(hdr.entries_lba + i, &mut sec)?;
        entries[i as usize * SECTOR..(i as usize + 1) * SECTOR].copy_from_slice(&sec);
    }
    entries.truncate(bytes);

    if let Some(e) = entry_mut(&mut entries, &hdr, GPT_ROOT) {
        set_entry_last_lba(e, new_p2_last);
    }
    if let Some(e) = entry_mut(&mut entries, &hdr, GPT_INSTALL) {
        set_entry_first_lba(e, new_p4_first);
        set_entry_last_lba(e, new_p4_last);
    }
    if let Some(e) = entry_mut(&mut entries, &hdr, GPT_MODELS) {
        set_entry_first_lba(e, new_p3_first);
        set_entry_last_lba(e, new_p3_last);
    }

    let disk_sectors = disk_sectors(disk)?;
    gptdisk::relayout(&mut hdr, &mut entries, disk_sectors).map_err(|_| Error::InvalidGeometry)?;
    if new_p4_last > hdr.last_usable {
        return Err(Error::InvalidGeometry);
    }
    Ok(())
}

fn disk_sectors(disk: &impl Disk512) -> Result<u64, Error> {
    let mut hdr_sec = [0u8; SECTOR];
    disk.read_sector(1, &mut hdr_sec)?;
    let hdr = Header::parse(&hdr_sec).map_err(|_| Error::InvalidGeometry)?;
    Ok(hdr.alternate_lba + 1)
}

/// Reanuda slide+GPT según journal (shrink sosomfs es responsabilidad del llamador).
pub fn recover_slides_and_gpt<D: Disk512>(
    disk: &mut D,
    journal: &mut Journal,
    on_progress: impl FnMut(&Journal),
) -> Result<(), Error> {
    if !journal.valid_phase() {
        return Err(Error::InvalidJournal);
    }
    if !journal.needs_recovery() {
        return Ok(());
    }
    let rank = phase_rank(journal.phase).ok_or(Error::InvalidJournal)?;
    if rank < phase_rank(JOURNAL_SHRINK_MODELS).unwrap() {
        return Err(Error::InvalidJournal);
    }
    apply_gpt_and_slides(disk, journal.delta_sectors, journal, on_progress)
}

/// Tras shrink de modelos: desliza p3 y p4, actualiza GPT primario+backup.
pub fn apply_gpt_and_slides<D: Disk512>(
    disk: &mut D,
    delta: u64,
    journal: &mut Journal,
    mut on_progress: impl FnMut(&Journal),
) -> Result<(), Error> {
    let p2 = read_part(disk, GPT_ROOT)?;
    let p3 = read_part(disk, GPT_MODELS)?;
    let p4 = read_part(disk, GPT_INSTALL)?;
    let p3_after = p3.sectors.saturating_sub(delta);

    if phase_at_most(journal.phase, JOURNAL_SLIDE_P3) {
        if phase_before(journal.phase, JOURNAL_SLIDE_P3) {
            journal.phase = JOURNAL_SLIDE_P3;
            journal.slide_done = 0;
            on_progress(journal);
        }
        slide_sectors(
            disk,
            p3.first,
            p3.first + delta,
            p3_after,
            &mut journal.slide_done,
        )?;
        on_progress(journal);
    }

    if p4.sectors > 0 && phase_at_most(journal.phase, JOURNAL_SLIDE_P4) {
        if phase_before(journal.phase, JOURNAL_SLIDE_P4) {
            journal.phase = JOURNAL_SLIDE_P4;
            journal.slide_done = 0;
            on_progress(journal);
        }
        slide_sectors(
            disk,
            p4.first,
            p4.first + delta,
            p4.sectors,
            &mut journal.slide_done,
        )?;
        on_progress(journal);
    }

    if phase_at_most(journal.phase, JOURNAL_GPT) {
        journal.phase = JOURNAL_GPT;
        journal.slide_done = 0;
        on_progress(journal);
        write_gpt_update(disk, delta, p2, p3, p4)?;
    }

    journal.phase = JOURNAL_IDLE;
    journal.slide_done = 0;
    on_progress(journal);
    Ok(())
}

fn write_gpt_update<D: Disk512>(
    disk: &mut D,
    delta: u64,
    p2: PartGeom,
    p3: PartGeom,
    p4: PartGeom,
) -> Result<(), Error> {
    let mut hdr_sec = [0u8; SECTOR];
    disk.read_sector(1, &mut hdr_sec)?;
    let mut hdr = Header::parse(&hdr_sec).map_err(|_| Error::InvalidGeometry)?;
    let bytes = hdr.entries_bytes();
    let padded = bytes.div_ceil(SECTOR) * SECTOR;
    let mut entries = alloc::vec![0u8; padded];
    for i in 0..hdr.entries_sectors() {
        let mut sec = [0u8; SECTOR];
        disk.read_sector(hdr.entries_lba + i, &mut sec)?;
        entries[i as usize * SECTOR..(i as usize + 1) * SECTOR].copy_from_slice(&sec);
    }
    entries.truncate(bytes);

    let new_p2_last = p2.last + delta;
    let new_p3_first = p3.first + delta;
    let new_p3_last = p3.last;
    let new_p4_first = p4.first + delta;
    let new_p4_last = p4.last + delta;

    if let Some(e) = entry_mut(&mut entries, &hdr, GPT_ROOT) {
        set_entry_last_lba(e, new_p2_last);
    }
    if let Some(e) = entry_mut(&mut entries, &hdr, GPT_INSTALL) {
        set_entry_first_lba(e, new_p4_first);
        set_entry_last_lba(e, new_p4_last);
    }
    if let Some(e) = entry_mut(&mut entries, &hdr, GPT_MODELS) {
        set_entry_first_lba(e, new_p3_first);
        set_entry_last_lba(e, new_p3_last);
    }

    let disk_sz = disk_sectors(disk)?;
    let plan = gptdisk::relayout(&mut hdr, &mut entries, disk_sz).map_err(|_| Error::InvalidGeometry)?;

    // `relayout` estira la última partición usada; restaurar tamaños planificados.
    if let Some(e) = entry_mut(&mut entries, &hdr, GPT_ROOT) {
        set_entry_last_lba(e, new_p2_last);
    }
    if let Some(e) = entry_mut(&mut entries, &hdr, GPT_MODELS) {
        set_entry_first_lba(e, new_p3_first);
        set_entry_last_lba(e, new_p3_last);
    }
    if let Some(e) = entry_mut(&mut entries, &hdr, GPT_INSTALL) {
        set_entry_first_lba(e, new_p4_first);
        set_entry_last_lba(e, new_p4_last);
    }

    write_gpt_sectors(disk, plan.backup_entries_lba, &entries)?;
    let backup = gptdisk::render_backup(&hdr, &entries, &plan);
    write_gpt_sector(disk, plan.backup_header_lba, &backup)?;

    write_gpt_sectors(disk, plan.primary_entries_lba, &entries)?;
    let primary = gptdisk::render_primary(&hdr, &entries, &plan);
    write_gpt_sector(disk, plan.primary_header_lba, &primary)?;
    disk.flush()?;
    Ok(())
}

fn write_gpt_sectors<D: Disk512>(disk: &mut D, lba: u64, buf: &[u8]) -> Result<(), Error> {
    for (i, chunk) in buf.chunks(SECTOR).enumerate() {
        let mut sec = [0u8; SECTOR];
        sec[..chunk.len()].copy_from_slice(chunk);
        disk.write_sector(lba + i as u64, &sec)?;
    }
    Ok(())
}

fn write_gpt_sector<D: Disk512>(disk: &mut D, lba: u64, sec: &[u8; SECTOR]) -> Result<(), Error> {
    disk.write_sector(lba, sec)
}

/// Geometría GPT en memoria. Las consultas no tocan el disco; hay que
/// `reload` de forma explícita tras un cambio GPT confirmado.
#[derive(Debug, Clone, Default)]
pub struct GeomCache {
    parts: [Option<PartGeom>; 4],
    reloads: u64,
}

impl GeomCache {
    pub fn reload(&mut self, disk: &impl Disk512) -> Result<(), Error> {
        self.parts[GPT_ROOT] = Some(read_part(disk, GPT_ROOT)?);
        self.parts[GPT_MODELS] = Some(read_part(disk, GPT_MODELS)?);
        self.parts[GPT_ESP] = read_part(disk, GPT_ESP).ok();
        self.parts[GPT_INSTALL] = read_part(disk, GPT_INSTALL).ok();
        self.reloads = self.reloads.saturating_add(1);
        Ok(())
    }

    pub fn get(&self, index: usize) -> Option<PartGeom> {
        self.parts.get(index).copied().flatten()
    }

    pub fn reloads(&self) -> u64 {
        self.reloads
    }
}
