//! Ampliar sosofs robando espacio al final de la partición de modelos.
//!
//! Solo en layout GPT live/instalado (`drv-live-disk`). QEMU virtio sin GPT
//! crece sosofs al montar vía `ftruncate` + `grow_to`.

use crate::drivers::live_disk::{self, GPT_MODELS, GPT_ROOT};
use crate::fs;
use crate::println;
use block_dev::BLOCK_SIZE;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use soso_resize_core::{
    self, Disk512, Error as ResizeError, Journal, JOURNAL_INTENT,
    JOURNAL_SHRINK_MODELS, apply_gpt_and_slides, validate_geometry,
};

const SECTOR: usize = 512;

static RESIZE_ACTIVE: AtomicBool = AtomicBool::new(false);
static PENDING_ROOT_GROW: AtomicU64 = AtomicU64::new(0);

struct ResizeGuard;
impl ResizeGuard {
    fn acquire() -> Result<Self, i64> {
        if RESIZE_ACTIVE.swap(true, Ordering::SeqCst) {
            println!("fs-resize: rechazado — otra operación en curso");
            return Err(-soso_abi::EBUSY);
        }
        Ok(Self)
    }
}
impl Drop for ResizeGuard {
    fn drop(&mut self) {
        RESIZE_ACTIVE.store(false, Ordering::SeqCst);
    }
}

struct LiveDisk512;

impl Disk512 for LiveDisk512 {
    fn read_sector(&self, lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), ResizeError> {
        live_disk::disk_read_sector(lba, buf).map_err(|_| ResizeError::Io)
    }

    fn write_sector(&mut self, lba: u64, buf: &[u8; SECTOR]) -> Result<(), ResizeError> {
        live_disk::disk_write_sector_nosync(lba, buf).map_err(|_| ResizeError::Io)
    }

    fn flush(&mut self) -> Result<(), ResizeError> {
        live_disk::disk_flush().map_err(|_| ResizeError::Io)
    }
}

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
            let used = sosomfs::import::next_free_lba(sb, &mfs.catalog);
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

/// Recuperación en disco **antes** de montar root/modelos (slide + GPT).
#[cfg(feature = "drv-live-disk")]
pub fn recover_before_mount() -> bool {
    if !live_disk::active() {
        return true;
    }
    let mut journal = match journal_read() {
        Err(code) if code == -soso_abi::ENOENT => {
            // Sin hueco no pudo haberse iniciado un resize durable.
            return true;
        }
        Err(code) if code == -soso_abi::EIO => {
            println!("fs-resize: no se pudo leer SOSORES.TXT — no se montará live");
            return false;
        }
        Err(_) => {
            println!("fs-resize: journal corrupto — no se montará live");
            return false;
        }
        Ok(j) => j,
    };
    if !journal.needs_recovery() {
        return true;
    }
    if !journal.valid_phase() {
        println!("fs-resize: journal corrupto — no se montará live");
        return false;
    }
    if !live_disk::backend_supports_durable_flush() {
        println!("fs-resize: recovery requiere backend con flush durable");
        return false;
    }
    println!(
        "fs-resize: recuperando fase {} ({} sectores)…",
        journal.phase, journal.delta_sectors
    );
    let delta_blocks = journal
        .delta_sectors
        .checked_div((BLOCK_SIZE / SECTOR) as u64)
        .unwrap_or(0);
    if journal.phase == JOURNAL_INTENT {
        println!("fs-resize: recovery INTENT — montando modelos");
        if !fs::mount_models_for_recovery() {
            println!("fs-resize: no se pudo montar modelos para shrink");
            return false;
        }
        println!("fs-resize: recovery INTENT — shrink {} bloques", delta_blocks);
        if shrink_models(delta_blocks).is_err() {
            println!("fs-resize: shrink en recovery falló");
            return false;
        }
        journal.phase = JOURNAL_SHRINK_MODELS;
        journal.slide_done = 0;
        if journal_write(&journal).is_err() {
            println!("fs-resize: no se pudo escribir journal tras shrink");
            return false;
        }
        println!("fs-resize: recovery INTENT — shrink hecho");
    }
    println!("fs-resize: recovery slide/GPT (fase {})", journal.phase);
    let mut disk = LiveDisk512;
    if soso_resize_core::recover_slides_and_gpt(&mut disk, &mut journal, |j| {
        let _ = journal_write(j);
    })
    .is_err()
    {
        println!("fs-resize: slide/GPT en recovery falló (fase {})", journal.phase);
        return false;
    }
    if !live_disk::refresh_geometry() {
        println!("fs-resize: no se pudo refrescar geometría GPT tras recovery");
        return false;
    }
    PENDING_ROOT_GROW.store(delta_blocks, Ordering::SeqCst);
    println!("fs-resize: disco recuperado; pendiente grow root ({} bloques)", delta_blocks);
    true
}

#[cfg(not(feature = "drv-live-disk"))]
pub fn recover_before_mount() -> bool {
    true
}

/// Tras montar sosofs: aplica grow pendiente de recovery.
#[cfg(feature = "drv-live-disk")]
pub fn finalize_after_mount() {
    let delta_blocks = PENDING_ROOT_GROW.swap(0, Ordering::SeqCst);
    if delta_blocks == 0 {
        return;
    }
    let Some(fs_mutex) = fs::FS.get() else {
        println!("fs-resize: finalize sin sosofs montado");
        return;
    };
    let mut fs = fs_mutex.lock();
    let current = fs.block_count();
    let part = live_disk::cached_part_sectors(GPT_ROOT)
        .map(|s| s / (BLOCK_SIZE / SECTOR) as u64)
        .unwrap_or(0);
    // Tras slide+GPT el montaje ya pudo hacer grow_to(partición).
    let target = if part > current {
        part
    } else {
        current
    };
    if target > current && fs.grow_to(target).is_err() {
        println!("fs-resize: grow root en finalize falló");
        return;
    }
    drop(fs);
    let _ = reload_models();
    let _ = journal_write(&Journal::idle());
    println!("fs-resize: recovery completa — rootfs ampliado");
}

#[cfg(not(feature = "drv-live-disk"))]
pub fn finalize_after_mount() {}

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

    preflight_resize(delta_blocks, delta_sectors)?;
    let _guard = ResizeGuard::acquire()?;

    println!(
        "fs-resize: rootfs +{} MiB ({} bloques) desde modelos",
        delta_blocks * BLOCK_SIZE as u64 / (1024 * 1024),
        delta_blocks
    );

    let mut journal = Journal {
        phase: JOURNAL_INTENT,
        delta_sectors,
        slide_done: 0,
    };
    journal_write(&journal)?;

    shrink_models(delta_blocks)?;
    journal.phase = JOURNAL_SHRINK_MODELS;
    journal.slide_done = 0;
    journal_write(&journal)?;

    let mut disk = LiveDisk512;
    apply_gpt_and_slides(&mut disk, delta_sectors, &mut journal, |j| {
        let _ = journal_write(j);
    })
    .map_err(|_| -soso_abi::EIO)?;
    if !live_disk::refresh_geometry() {
        println!("fs-resize: no se pudo refrescar geometría GPT");
        return Err(-soso_abi::EIO);
    }

    let new_root_blocks = {
        let fs_mutex = fs::FS.get().ok_or(-soso_abi::EIO)?;
        let mut fs = fs_mutex.lock();
        let target = fs.block_count().saturating_add(delta_blocks);
        fs.grow_to(target).map_err(|_| -soso_abi::EIO)?;
        fs.block_count()
    };

    reload_models()?;
    journal_write(&Journal::idle())?;

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
fn preflight_resize(delta_blocks: u64, delta_sectors: u64) -> Result<(), i64> {
    use crate::drivers::espfat;
    const JOURNAL_SIZE: usize = 4096;
    if espfat::locate(b"SOSORES ", b"TXT", JOURNAL_SIZE).is_none() {
        println!("fs-resize: rechazado — journal SOSORES.TXT ausente");
        return Err(-soso_abi::EIO);
    }
    if crate::som_import::import_active() {
        println!("fs-resize: rechazado — importación de modelos en curso");
        return Err(-soso_abi::EBUSY);
    }
    if !live_disk::backend_supports_durable_flush() {
        let be = live_disk::backend()
            .map(|b| alloc::format!("{b:?}"))
            .unwrap_or_else(|| "?".into());
        println!(
            "fs-resize: rechazado — backend {be} sin flush durable (redimensionado no soportado)"
        );
        return Err(-soso_abi::ENOTSUP);
    }
    let disk = LiveDisk512;
    validate_geometry(&disk, delta_sectors).map_err(|_| -soso_abi::EINVAL)?;
    let info = space_info()?;
    if delta_blocks > info.max_grow_blocks {
        return Err(-soso_abi::ENOSPC);
    }
    Ok(())
}

#[cfg(feature = "drv-live-disk")]
fn part_blocks(entry_index: usize) -> Result<u64, i64> {
    let sectors = live_disk::cached_part_sectors(entry_index).ok_or(-soso_abi::EIO)?;
    Ok(sectors / (BLOCK_SIZE / SECTOR) as u64)
}

#[cfg(feature = "drv-live-disk")]
fn shrink_models(delta_blocks: u64) -> Result<(), i64> {
    let mfs_mutex = fs::MODELS.get().ok_or(-soso_abi::EIO)?;
    let mut mfs = mfs_mutex.lock();
    let mut sb = *mfs.superblock();
    let new_total = sb.total_blocks.saturating_sub(delta_blocks);
    sosomfs::import::shrink_superblock(&mut sb, &mfs.catalog, new_total)
        .map_err(|_| -soso_abi::ENOSPC)?;
    sosomfs::import::commit_grow(mfs.cache.volume_mut().inner_mut(), &sb)
        .map_err(|_| -soso_abi::EIO)?;
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
        sosomfs::import::grow_superblock(&mut sb, part_blocks);
        sosomfs::import::commit_grow(mfs.cache.volume_mut().inner_mut(), &sb)
            .map_err(|_| -soso_abi::EIO)?;
        mfs.reload_from_disk().map_err(|_| -soso_abi::EIO)?;
    }
    Ok(())
}

#[cfg(feature = "drv-live-disk")]
fn journal_read() -> Result<Journal, i64> {
    use crate::drivers::espfat::{self, SECTOR as ESP_SEC};
    const JOURNAL_SIZE: usize = 4096;
    let slot = espfat::locate(b"SOSORES ", b"TXT", JOURNAL_SIZE).ok_or(-soso_abi::ENOENT)?;
    let mut buf = [0u8; ESP_SEC];
    espfat::read(slot.data_lba, &mut buf).map_err(|_| -soso_abi::EIO)?;
    Journal::decode(&buf).map_err(|_| -soso_abi::EINVAL)
}

#[cfg(feature = "drv-live-disk")]
fn journal_write(journal: &Journal) -> Result<(), i64> {
    use crate::drivers::espfat::{self, SECTOR as ESP_SEC};
    const JOURNAL_SIZE: usize = 4096;
    let slot = espfat::locate(b"SOSORES ", b"TXT", JOURNAL_SIZE).ok_or(-soso_abi::EIO)?;
    let mut buf = [0u8; ESP_SEC];
    journal.encode(&mut buf);
    espfat::write(slot.data_lba, &buf).map_err(|_| -soso_abi::EIO)?;
    Ok(())
}
