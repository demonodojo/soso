//! Syscalls de import atómico en sosomfs (`/models/`).

use sosomfs::{ImportError, ImportSession};
use spin::Mutex;

use crate::fs::MODELS;

struct ActiveImport {
    session: ImportSession,
}

static IMPORT: Mutex<Option<ActiveImport>> = Mutex::new(None);

fn import_err(e: ImportError) -> i64 {
    e.to_errno()
}

pub fn begin(name_ptr: u64, name_len: u64) -> Result<(), i64> {
    let name = crate::task::syscall::user_str(name_ptr, name_len)?;
    if IMPORT.lock().is_some() {
        return Err(-soso_abi::EBUSY);
    }
    let mfs = MODELS.get().ok_or(-soso_abi::ENOSYS)?;
    let mut guard = mfs.lock();
    let session = ImportSession::begin(
        *guard.superblock(),
        guard.catalog_ref().clone(),
        name,
    )
    .map_err(import_err)?;
    *IMPORT.lock() = Some(ActiveImport { session });
    Ok(())
}

pub fn put(rel_ptr: u64, rel_len: u64, buf_ptr: u64, buf_len: u64) -> Result<(), i64> {
    let rel = crate::task::syscall::user_str(rel_ptr, rel_len)?;
    let data = crate::task::syscall::user_slice(buf_ptr, buf_len)?;
    let mut imp = IMPORT.lock();
    let active = imp.as_mut().ok_or(-soso_abi::EINVAL)?;
    let mfs = MODELS.get().ok_or(-soso_abi::ENOSYS)?;
    let mut guard = mfs.lock();
    active
        .session
        .put_file(guard.cache.volume_mut().inner_mut(), rel, data)
        .map_err(import_err)
}

pub fn commit() -> Result<(), i64> {
    let active = IMPORT.lock().take().ok_or(-soso_abi::EINVAL)?;
    let mfs = MODELS.get().ok_or(-soso_abi::ENOSYS)?;
    let mut guard = mfs.lock();
    let (_sb, _cat) = active
        .session
        .commit(guard.cache.volume_mut().inner_mut())
        .map_err(import_err)?;
    guard.reload_from_disk().map_err(|_| -soso_abi::EIO)?;
    Ok(())
}

pub fn abort() -> Result<(), i64> {
    if let Some(active) = IMPORT.lock().take() {
        active.session.abort();
    }
    Ok(())
}

pub fn scratch_alloc(size: u64) -> Result<u64, i64> {
    let mut imp = IMPORT.lock();
    let active = imp.as_mut().ok_or(-soso_abi::EINVAL)?;
    let region = active.session.scratch_alloc(size).map_err(import_err)?;
    Ok(region.start_lba)
}

pub fn scratch_write(start_lba: u64, offset: u64, buf_ptr: u64, buf_len: u64) -> Result<(), i64> {
    let data = crate::task::syscall::user_slice(buf_ptr, buf_len)?;
    let mut imp = IMPORT.lock();
    let active = imp.as_mut().ok_or(-soso_abi::EINVAL)?;
    let region = active.session.scratch_region().ok_or(-soso_abi::EINVAL)?;
    if region.start_lba != start_lba {
        return Err(-soso_abi::EINVAL);
    }
    let mfs = MODELS.get().ok_or(-soso_abi::ENOSYS)?;
    let mut guard = mfs.lock();
    sosomfs::import::scratch_write(
        guard.cache.volume_mut().inner_mut(),
        region,
        offset,
        data,
    )
    .map_err(import_err)
}

pub fn scratch_read(start_lba: u64, offset: u64, buf_ptr: u64, buf_len: u64) -> Result<(), i64> {
    if buf_len == 0 {
        return Ok(());
    }
    let mut imp = IMPORT.lock();
    let active = imp.as_mut().ok_or(-soso_abi::EINVAL)?;
    let region = active.session.scratch_region().ok_or(-soso_abi::EINVAL)?;
    if region.start_lba != start_lba {
        return Err(-soso_abi::EINVAL);
    }
    let mfs = MODELS.get().ok_or(-soso_abi::ENOSYS)?;
    let mut guard = mfs.lock();
    let mut tmp = alloc::vec::Vec::with_capacity(buf_len as usize);
    tmp.resize(buf_len as usize, 0);
    sosomfs::import::scratch_read(
        guard.cache.volume_mut().inner_mut(),
        region,
        offset,
        &mut tmp,
    )
    .map_err(import_err)?;
    let dst = crate::task::syscall::user_slice_mut(buf_ptr, buf_len)?;
    dst.copy_from_slice(&tmp);
    Ok(())
}

pub fn scratch_free() -> Result<(), i64> {
    let mut imp = IMPORT.lock();
    let active = imp.as_mut().ok_or(-soso_abi::EINVAL)?;
    active.session.scratch_free();
    Ok(())
}
