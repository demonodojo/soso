//! Huecos de actualización en la ESP: `SOSOUPD.TXT` (buzón) y `SOSOKRN.BIN` (kernel).
//!
//! Mismo patrón que `bootreq.rs`: ficheros 8.3 pre-creados y contiguos; el kernel
//! solo sobrescribe sectores de datos. Es el camino por el que userspace puede
//! preparar una actualización de kernel antes del reinicio.

use crate::drivers::espfat::{self, SECTOR, Slot};
use core::sync::atomic::{AtomicBool, Ordering};
use spin::Once;
use soso_abi as abi;

static MAILBOX: Once<Option<Slot>> = Once::new();
static KERNEL: Once<Option<Slot>> = Once::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static KERNEL_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn init() {
    if !crate::drivers::live_disk::esp_available() {
        return;
    }
    match espfat::locate(b"SOSOUPD ", b"TXT", abi::UPD_MAILBOX_SIZE) {
        Some(slot) => {
            MAILBOX.call_once(|| Some(slot));
            ACTIVE.store(true, Ordering::Relaxed);
            crate::println!("updslot: SOSOUPD.TXT LBA {}", slot.data_lba);
        }
        None => crate::println!(
            "updslot: SOSOUPD.TXT no encontrado; actualizaciones de kernel desactivadas"
        ),
    }
    let ksize = abi::UPD_KERNEL_SLOT_SIZE as usize;
    match espfat::locate(b"SOSOKRN ", b"BIN", ksize) {
        Some(slot) => {
            KERNEL.call_once(|| Some(slot));
            KERNEL_ACTIVE.store(true, Ordering::Relaxed);
            crate::println!("updslot: SOSOKRN.BIN LBA {} ({} MiB)", slot.data_lba, ksize / (1024 * 1024));
        }
        None => crate::println!(
            "updslot: SOSOKRN.BIN no encontrado; reflashea el live para habilitar actualizaciones de kernel"
        ),
    }
}

pub fn mailbox_available() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub fn kernel_slot_available() -> bool {
    KERNEL_ACTIVE.load(Ordering::Relaxed)
}

fn slot_for(which: u64) -> Result<Slot, i64> {
    match which {
        abi::UPD_WHICH_MAILBOX => MAILBOX
            .get()
            .and_then(|s| *s)
            .ok_or(-abi::ENOTSUP),
        abi::UPD_WHICH_KERNEL => KERNEL
            .get()
            .and_then(|s| *s)
            .ok_or(-abi::ENOTSUP),
        _ => Err(-abi::EINVAL),
    }
}

fn max_size(which: u64) -> Result<usize, i64> {
    Ok(match which {
        abi::UPD_WHICH_MAILBOX => abi::UPD_MAILBOX_SIZE,
        abi::UPD_WHICH_KERNEL => abi::UPD_KERNEL_SLOT_SIZE as usize,
        _ => return Err(-abi::EINVAL),
    })
}

pub fn write(which: u64, offset: u64, data: &[u8]) -> Result<(), i64> {
    if data.is_empty() {
        return Ok(());
    }
    if offset % SECTOR as u64 != 0 || data.len() % SECTOR != 0 {
        return Err(-abi::EINVAL);
    }
    let slot = slot_for(which)?;
    let cap = max_size(which)?;
    let end = offset as usize + data.len();
    if end > cap {
        return Err(-abi::EINVAL);
    }
    crate::drivers::logbuf::run_without_capture(|| {
        let mut lba = slot.data_lba + offset / SECTOR as u64;
        for chunk in data.chunks(SECTOR) {
            if espfat::write(lba, chunk).is_err() {
                return false;
            }
            lba += 1;
        }
        true
    })
    .then_some(())
    .ok_or(-abi::EIO)
}

pub fn read(which: u64, offset: u64, out: &mut [u8]) -> Result<usize, i64> {
    if out.is_empty() {
        return Ok(0);
    }
    if offset % SECTOR as u64 != 0 || out.len() % SECTOR != 0 {
        return Err(-abi::EINVAL);
    }
    let slot = slot_for(which)?;
    let cap = max_size(which)?;
    if offset as usize >= cap {
        return Ok(0);
    }
    let n = out.len().min(cap - offset as usize);
    crate::drivers::logbuf::run_without_capture(|| {
        let mut lba = slot.data_lba + offset / SECTOR as u64;
        let mut written = 0usize;
        while written < n {
            if espfat::read(lba, &mut out[written..written + SECTOR]).is_err() {
                return false;
            }
            lba += 1;
            written += SECTOR;
        }
        true
    })
    .then_some(n)
    .ok_or(-abi::EIO)
}

/// Sobrescribe el buzón entero (rellena con `\n` como bootreq).
pub fn write_mailbox(payload: &[u8]) -> Result<(), i64> {
    if !mailbox_available() {
        return Err(-abi::ENOTSUP);
    }
    if payload.len() > abi::UPD_MAILBOX_SIZE {
        return Err(-abi::EINVAL);
    }
    static mut BUF: [u8; abi::UPD_MAILBOX_SIZE] = [b'\n'; abi::UPD_MAILBOX_SIZE];
    let buf = unsafe {
        core::slice::from_raw_parts_mut(
            core::ptr::addr_of_mut!(BUF).cast(),
            abi::UPD_MAILBOX_SIZE,
        )
    };
    buf[..payload.len()].copy_from_slice(payload);
    buf[payload.len()..].fill(b'\n');
    write(abi::UPD_WHICH_MAILBOX, 0, buf)
}

pub fn read_mailbox(out: &mut [u8]) -> Result<usize, i64> {
    if !mailbox_available() {
        return Err(-abi::ENOTSUP);
    }
    let n = out.len().min(abi::UPD_MAILBOX_SIZE) / SECTOR * SECTOR;
    if n == 0 {
        return Ok(0);
    }
    read(abi::UPD_WHICH_MAILBOX, 0, &mut out[..n])
}
