//! Ampliar sosofs robando espacio al final de la partición de modelos.
//!
//! Solo en layout GPT live/instalado (`drv-live-disk`). QEMU virtio sin GPT
//! crece sosofs al montar vía `ftruncate` + `grow_to`.

use crate::drivers::live_disk::{self, GPT_INSTALL, GPT_MODELS, GPT_ROOT};
use crate::fs;
use crate::println;
use block_dev::BLOCK_SIZE;
use gptdisk::{Header, entry, entry_first_lba, entry_last_lba, entry_mut, set_entry_first_lba,
              set_entry_last_lba};
use sosomfs::import;

const SECTOR: usize = 512;
const GPT_HDR_LBA: u64 = 1;

const JOURNAL_IDLE: u8 = 0;
const JOURNAL_SHRINK_MODELS: u8 = 1;
const JOURNAL_SLIDE_P3: u8 = 2;
const JOURNAL_SLIDE_P4: u8 = 3;
const JOURNAL_GPT: u8 = 4;

#[cfg(feature = "drv-live-disk")]
pub fn space_info() -> Result<soso_abi::FsSpaceInfo, i64> {
    if !live_disk::active() {
        return Err(-soso_abi::ENOSYS);
    }
    let root_part = part_blocks(GPT_ROOT)?;
    let models_part = part_blocks(GPT_MODELS)?;
    let (root_fs, root_free) = fs::FS
        .get()
        .map(|fs| {
            let fs = fs.lock();
            (fs.block_count(), fs.free_blocks())
        })
        .unwrap_or((0, 0));
    let (models_fs, models_used) = fs::MODELS
        .get()
        .map(|mfs| {
            let mfs = mfs.lock();
            let sb = mfs.superblock();
            let used = import::next_free_lba(sb, &mfs.catalog);
            (mfs.total_blocks(), used)
        })
        .unwrap_or((0, 0));
    let max_grow = models_fs.saturating_sub(models_used);
    Ok(soso_abi::FsSpaceInfo {
        root_fs_blocks: root_fs,
        root_free_blocks: root_free,
        root_part_blocks: root_part,
        models_fs_blocks: models_fs,
        models_used_blocks: models_used,
        models_part_blocks: models_part,
        max_grow_blocks: max_grow,
    })
}

#[cfg(not(feature = "drv-live-disk"))]
pub fn space_info() -> Result<soso_abi::FsSpaceInfo, i64> {
    Err(-soso_abi::ENOSYS)
}

#[cfg(feature = "drv-live-disk")]
pub fn grow_root(delta_blocks: u64) -> Result<u64, i64> {
    if !live_disk::active() {
        return Err(-soso_abi::ENOSYS);
    }
    if delta_blocks == 0 {
        return Err(-soso_abi::EINVAL);
    }
    let info = space_info()?;
    if delta_blocks > info.max_grow_blocks {
        return Err(-soso_abi::ENOSPC);
    }
    let delta_sectors = delta_blocks
        .checked_mul((BLOCK_SIZE / SECTOR) as u64)
        .ok_or(-soso_abi::EINVAL)?;

    println!(
        "fs-resize: rootfs +{} MiB ({} bloques) desde modelos",
        delta_blocks * BLOCK_SIZE as u64 / (1024 * 1024),
        delta_blocks
    );

    shrink_models(delta_blocks)?;
    journal_write(JOURNAL_SHRINK_MODELS, delta_sectors)?;

    let p2 = part_geometry(GPT_ROOT)?;
    let p3 = part_geometry(GPT_MODELS)?;
    let p4 = part_geometry(GPT_INSTALL)?;

    let p3_sectors_after = p3.sectors.saturating_sub(delta_sectors);
    if p3_sectors_after == 0 {
        return Err(-soso_abi::ENOSPC);
    }

    live_disk::disk_slide_sectors(p3.first, p3.first + delta_sectors, p3_sectors_after)
        .map_err(|_| -soso_abi::EIO)?;
    journal_write(JOURNAL_SLIDE_P3, delta_sectors)?;

    if p4.sectors > 0 {
        live_disk::disk_slide_sectors(p4.first, p4.first + delta_sectors, p4.sectors)
            .map_err(|_| -soso_abi::EIO)?;
    }
    journal_write(JOURNAL_SLIDE_P4, delta_sectors)?;

    update_gpt(delta_sectors, p2, p3, p4)?;
    journal_write(JOURNAL_GPT, delta_sectors)?;

    let new_root_blocks = {
        let fs_mutex = fs::FS.get().ok_or(-soso_abi::EIO)?;
        let mut fs = fs_mutex.lock();
        let target = fs.block_count().saturating_add(delta_blocks);
        fs.grow_to(target).map_err(|_| -soso_abi::EIO)?;
        fs.block_count()
    };

    reload_models()?;
    journal_write(JOURNAL_IDLE, 0)?;

    println!(
        "fs-resize: rootfs ahora {} bloques ({} MiB libres)",
        new_root_blocks,
        fs::FS
            .get()
            .map(|f| f.lock().free_blocks() * BLOCK_SIZE as u64 / (1024 * 1024))
            .unwrap_or(0)
    );
    Ok(0)
}

#[cfg(not(feature = "drv-live-disk"))]
pub fn grow_root(_delta_blocks: u64) -> Result<u64, i64> {
    Err(-soso_abi::ENOSYS)
}

#[cfg(feature = "drv-live-disk")]
struct PartGeom {
    first: u64,
    last: u64,
    sectors: u64,
}

#[cfg(feature = "drv-live-disk")]
fn part_geometry(entry_index: usize) -> Result<PartGeom, i64> {
    let (first, last) = read_gpt_entry(entry_index)?;
    Ok(PartGeom {
        first,
        sectors: last.saturating_sub(first) + 1,
        last,
    })
}

#[cfg(feature = "drv-live-disk")]
fn part_blocks(entry_index: usize) -> Result<u64, i64> {
    Ok(part_geometry(entry_index)?.sectors / (BLOCK_SIZE / SECTOR) as u64)
}

#[cfg(feature = "drv-live-disk")]
fn read_gpt_entry(entry_index: usize) -> Result<(u64, u64), i64> {
    let mut hdr_sec = [0u8; SECTOR];
    live_disk::disk_read_sector(GPT_HDR_LBA, &mut hdr_sec).map_err(|_| -soso_abi::EIO)?;
    let hdr = Header::parse(&hdr_sec).map_err(|_| -soso_abi::EIO)?;
    let bytes = hdr.entries_bytes();
    let padded = bytes.div_ceil(SECTOR) * SECTOR;
    let mut entries = alloc::vec![0u8; padded];
    for i in 0..hdr.entries_sectors() {
        let mut sec = [0u8; SECTOR];
        live_disk::disk_read_sector(hdr.entries_lba + i, &mut sec).map_err(|_| -soso_abi::EIO)?;
        entries[i as usize * SECTOR..(i as usize + 1) * SECTOR].copy_from_slice(&sec);
    }
    entries.truncate(bytes);
    let ent = entry(&entries, &hdr, entry_index).ok_or(-soso_abi::EIO)?;
    Ok((entry_first_lba(ent), entry_last_lba(ent)))
}

#[cfg(feature = "drv-live-disk")]
fn disk_sectors() -> Result<u64, i64> {
    let mut hdr_sec = [0u8; SECTOR];
    live_disk::disk_read_sector(GPT_HDR_LBA, &mut hdr_sec).map_err(|_| -soso_abi::EIO)?;
    let hdr = Header::parse(&hdr_sec).map_err(|_| -soso_abi::EIO)?;
    Ok(hdr.alternate_lba + 1)
}

#[cfg(feature = "drv-live-disk")]
fn update_gpt(
    delta: u64,
    p2: PartGeom,
    p3: PartGeom,
    p4: PartGeom,
) -> Result<(), i64> {
    let mut hdr_sec = [0u8; SECTOR];
    live_disk::disk_read_sector(GPT_HDR_LBA, &mut hdr_sec).map_err(|_| -soso_abi::EIO)?;
    let mut hdr = Header::parse(&hdr_sec).map_err(|_| -soso_abi::EIO)?;
    let bytes = hdr.entries_bytes();
    let padded = bytes.div_ceil(SECTOR) * SECTOR;
    let mut entries = alloc::vec![0u8; padded];
    for i in 0..hdr.entries_sectors() {
        let mut sec = [0u8; SECTOR];
        live_disk::disk_read_sector(hdr.entries_lba + i, &mut sec).map_err(|_| -soso_abi::EIO)?;
        entries[i as usize * SECTOR..(i as usize + 1) * SECTOR].copy_from_slice(&sec);
    }
    entries.truncate(bytes);

    if let Some(e) = entry_mut(&mut entries, &hdr, GPT_ROOT) {
        set_entry_last_lba(e, p2.last + delta);
    }
    if let Some(e) = entry_mut(&mut entries, &hdr, GPT_INSTALL) {
        set_entry_first_lba(e, p4.first + delta);
        set_entry_last_lba(e, p4.last + delta);
    }
    if let Some(e) = entry_mut(&mut entries, &hdr, GPT_MODELS) {
        set_entry_first_lba(e, p3.first + delta);
        set_entry_last_lba(e, p3.last);
    }

    let disk = disk_sectors()?;
    let plan = gptdisk::relayout(&mut hdr, &mut entries, disk).map_err(|_| -soso_abi::EIO)?;

    let backup = gptdisk::render_backup(&hdr, &entries, &plan);
    write_gpt_sectors(plan.backup_entries_lba, &entries)?;
    write_gpt_sector(plan.backup_header_lba, &backup)?;

    write_gpt_sectors(plan.primary_entries_lba, &entries)?;
    let primary = gptdisk::render_primary(&hdr, &entries, &plan);
    write_gpt_sector(plan.primary_header_lba, &primary)?;
    Ok(())
}

#[cfg(feature = "drv-live-disk")]
fn write_gpt_sectors(lba: u64, buf: &[u8]) -> Result<(), i64> {
    for (i, chunk) in buf.chunks(SECTOR).enumerate() {
        let mut sec = [0u8; SECTOR];
        sec[..chunk.len()].copy_from_slice(chunk);
        live_disk::disk_write_sector(lba + i as u64, &sec).map_err(|_| -soso_abi::EIO)?;
    }
    Ok(())
}

#[cfg(feature = "drv-live-disk")]
fn write_gpt_sector(lba: u64, sec: &[u8; SECTOR]) -> Result<(), i64> {
    live_disk::disk_write_sector(lba, sec).map_err(|_| -soso_abi::EIO)
}

#[cfg(feature = "drv-live-disk")]
fn shrink_models(delta_blocks: u64) -> Result<(), i64> {
    let mfs_mutex = fs::MODELS.get().ok_or(-soso_abi::EIO)?;
    let mut mfs = mfs_mutex.lock();
    let mut sb = *mfs.superblock();
    let new_total = sb.total_blocks.saturating_sub(delta_blocks);
    import::shrink_superblock(&mut sb, &mfs.catalog, new_total).map_err(|_| -soso_abi::ENOSPC)?;
    import::commit_grow(mfs.cache.volume_mut().inner_mut(), &sb).map_err(|_| -soso_abi::EIO)?;
    mfs.reload_from_disk().map_err(|_| -soso_abi::EIO)?;
    Ok(())
}

#[cfg(feature = "drv-live-disk")]
fn reload_models() -> Result<(), i64> {
    let mfs_mutex = fs::MODELS.get().ok_or(-soso_abi::EIO)?;
    let mut mfs = mfs_mutex.lock();
    mfs.reload_from_disk().map_err(|_| -soso_abi::EIO)?;
    let part_blocks = live_disk::models_dev()
        .map(|d| block_dev::BlockDevice::block_count(&d))
        .unwrap_or(0);
    if part_blocks > mfs.total_blocks() {
        let mut sb = *mfs.superblock();
        import::grow_superblock(&mut sb, part_blocks);
        import::commit_grow(mfs.cache.volume_mut().inner_mut(), &sb).map_err(|_| -soso_abi::EIO)?;
        mfs.reload_from_disk().map_err(|_| -soso_abi::EIO)?;
    }
    Ok(())
}

#[cfg(feature = "drv-live-disk")]
fn journal_write(phase: u8, delta_sectors: u64) -> Result<(), i64> {
    use crate::drivers::espfat::{self, SECTOR as ESP_SEC};
    const JOURNAL_SIZE: usize = 4096;
    let slot = match espfat::locate(b"SOSORES ", b"TXT", JOURNAL_SIZE) {
        Some(s) => s,
        None => return Ok(()),
    };
    let mut buf = [0u8; ESP_SEC];
    buf[0] = phase;
    buf[8..16].copy_from_slice(&delta_sectors.to_le_bytes());
    espfat::write(slot.data_lba, &buf).map_err(|_| -soso_abi::EIO)?;
    Ok(())
}
