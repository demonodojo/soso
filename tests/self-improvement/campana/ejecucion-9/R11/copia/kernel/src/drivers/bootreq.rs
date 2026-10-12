//! Petición de entrada de arranque UEFI: `SOSOBOOT.TXT` en la ESP del live.
//!
//! El kernel no puede registrar un `Boot####` en la NVRAM: `BootInfo` no trae
//! la System Table y los Runtime Services ya no son alcanzables después de
//! `ExitBootServices`. Quien sí está en contexto UEFI completo es el shim
//! (`boot-shim/`), que corre en cada arranque del pendrive. Así que
//! `soso-install` deja aquí lo que quiere que se registre y el shim lo ejecuta
//! en el siguiente arranque.
//!
//! El fichero lo pre-crea `xtask package-usb-live` con tamaño fijo y clusters
//! contiguos; aquí solo se sobrescriben sus sectores de datos. Es
//! deliberadamente el único camino por el que userspace puede escribir en el
//! disco de arranque: `raw_disk::write` lo rechaza con `EBUSY`.

use crate::drivers::espfat::{self, SECTOR, Slot};
use core::sync::atomic::{AtomicBool, Ordering};
use spin::Once;

pub const FILE_SIZE: usize = 4096;

static SLOT: Once<Option<Slot>> = Once::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn init() {
    if !crate::drivers::live_disk::esp_available() {
        return;
    }
    match espfat::locate(b"SOSOBOOT", b"TXT", FILE_SIZE) {
        Some(slot) => {
            SLOT.call_once(|| Some(slot));
            ACTIVE.store(true, Ordering::Relaxed);
            crate::println!("bootreq: SOSOBOOT.TXT LBA {}", slot.data_lba);
        }
        None => crate::println!(
            "bootreq: SOSOBOOT.TXT no encontrado o no contiguo; \
             el instalador no podrá pedir entrada UEFI"
        ),
    }
}

pub fn available() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Sobrescribe el fichero con `payload` y rellena el resto con `\n` para que
/// siga midiendo lo mismo que dice el directorio.
pub fn write(payload: &[u8]) -> Result<(), i64> {
    if !available() {
        return Err(-soso_abi::ENOTSUP);
    }
    if payload.len() > FILE_SIZE {
        return Err(-soso_abi::EINVAL);
    }
    let slot = SLOT.get().and_then(|s| *s).ok_or(-soso_abi::ENOTSUP)?;

    static mut BUF: [u8; FILE_SIZE] = [b'\n'; FILE_SIZE];
    // SAFETY: mismo patrón que fatlog/drvlog — solo se llama desde el hilo que
    // atiende la syscall, sin reentrada.
    let buf =
        unsafe { core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(BUF).cast(), FILE_SIZE) };
    buf[..payload.len()].copy_from_slice(payload);
    buf[payload.len()..].fill(b'\n');

    let ok = crate::drivers::logbuf::run_without_capture(|| {
        let mut lba = slot.data_lba;
        let mut written = 0usize;
        while written < FILE_SIZE {
            if espfat::write(lba, &buf[written..written + SECTOR]).is_err() {
                return false;
            }
            lba += 1;
            written += SECTOR;
        }
        true
    });
    if ok { Ok(()) } else { Err(-soso_abi::EIO) }
}

/// Lee el contenido actual: el instalador lo usa para enseñar si el shim ya
/// atendió la petición del arranque anterior.
pub fn read(out: &mut [u8]) -> Result<usize, i64> {
    if !available() {
        return Err(-soso_abi::ENOTSUP);
    }
    let slot = SLOT.get().and_then(|s| *s).ok_or(-soso_abi::ENOTSUP)?;
    let n = out.len().min(FILE_SIZE) / SECTOR * SECTOR;
    if n == 0 {
        return Ok(0);
    }
    espfat::read(slot.data_lba, &mut out[..n]).map_err(|_| -soso_abi::EIO)?;
    Ok(n)
}
