//! Volcado del informe hwscan a `SOSODRV.TXT` en la ESP (FAT live).

use crate::drivers::espfat::{self, SECTOR, Slot};
use core::sync::atomic::{AtomicBool, Ordering};
use spin::Once;

const FILE_SIZE: usize = 16 * 1024;

static SLOT: Once<Option<Slot>> = Once::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn init() {
    if !crate::drivers::live_disk::esp_available() {
        return;
    }
    match espfat::locate(b"SOSODRV ", b"TXT", FILE_SIZE) {
        Some(slot) => {
            SLOT.call_once(|| Some(slot));
            ACTIVE.store(true, Ordering::Relaxed);
            crate::println!(
                "drvlog: SOSODRV.TXT LBA {} ({} KiB)",
                slot.data_lba,
                FILE_SIZE / 1024
            );
            let _ = flush();
        }
        None => crate::println!("drvlog: SOSODRV.TXT no encontrado; desactivado"),
    }
}

pub fn flush() -> Result<(), ()> {
    if !ACTIVE.load(Ordering::Relaxed) {
        return Err(());
    }
    let slot = SLOT.get().and_then(|s| *s).ok_or(())?;
    let lines = crate::drivers::registry::hwscan_lines();
    static mut BUF: [u8; FILE_SIZE] = [0; FILE_SIZE];
    let buf = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(BUF).cast(), FILE_SIZE)
    };
    let mut pos = 0usize;
    for line in &lines {
        let bytes = line.as_bytes();
        let nl = b"\n";
        if pos + bytes.len() + 1 > FILE_SIZE {
            break;
        }
        buf[pos..pos + bytes.len()].copy_from_slice(bytes);
        pos += bytes.len();
        buf[pos] = nl[0];
        pos += 1;
    }
    while pos < FILE_SIZE {
        buf[pos] = b'\n';
        pos += 1;
    }

    let write_ok = crate::drivers::logbuf::run_without_capture(|| {
        let mut lba = slot.data_lba;
        let mut written = 0usize;
        while written < FILE_SIZE {
            let chunk = (FILE_SIZE - written).min(SECTOR);
            if espfat::write(lba, &buf[written..written + chunk]).is_err() {
                return false;
            }
            lba += 1;
            written += chunk;
        }
        true
    });
    if write_ok {
        Ok(())
    } else {
        Err(())
    }
}
