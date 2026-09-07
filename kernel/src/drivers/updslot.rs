//! Huecos de actualización en la ESP: `SOSOUPD.TXT`, `SOSOKRN.BIN`, `SOSOKRN.MET`.

use crate::drivers::espfat::{self, SECTOR, Slot};
use spin::Once;
use soso_abi as abi;

static MAILBOX: Once<Option<Slot>> = Once::new();
static KERNEL: Once<Option<Slot>> = Once::new();
static META: Once<Option<Slot>> = Once::new();

pub fn init() {
    if !crate::drivers::live_disk::esp_available() {
        return;
    }
    match espfat::locate(b"SOSOUPD ", b"TXT", abi::UPD_MAILBOX_SIZE) {
        Some(slot) => {
            MAILBOX.call_once(|| Some(slot));
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
            crate::println!("updslot: SOSOKRN.BIN LBA {} ({} MiB)", slot.data_lba, ksize / (1024 * 1024));
        }
        None => crate::println!(
            "updslot: SOSOKRN.BIN no encontrado; reflashea el live para habilitar actualizaciones de kernel"
        ),
    }
    match espfat::locate(b"SOSOKRN ", b"MET", abi::UPD_KERNEL_META_SIZE) {
        Some(slot) => {
            META.call_once(|| Some(slot));
            crate::println!("updslot: SOSOKRN.MET LBA {}", slot.data_lba);
        }
        None => crate::println!(
            "updslot: SOSOKRN.MET no encontrado; recuperación OTA limitada hasta reflashear"
        ),
    }
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
        abi::UPD_WHICH_META => META
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
        abi::UPD_WHICH_META => abi::UPD_KERNEL_META_SIZE,
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
