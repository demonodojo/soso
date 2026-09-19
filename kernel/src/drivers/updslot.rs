//! Huecos de actualización en la ESP: `SOSOUPD.TXT`, `SOSOKRN.BIN`, `SOSOKRN.MET`.

use crate::drivers::espfat::{self, SECTOR, Slot};
use spin::Once;
use soso_abi as abi;

static MAILBOX: Once<Option<Slot>> = Once::new();
static KERNEL: Once<Option<Slot>> = Once::new();
static META: Once<Option<Slot>> = Once::new();
static TXN: Once<Option<Slot>> = Once::new();
static MODE: Once<Option<Slot>> = Once::new();

pub fn init() {
    if !crate::drivers::live_disk::esp_available() {
        return;
    }
    match espfat::locate(b"SOSOUPD ", b"TXT", abi::UPD_MAILBOX_SIZE) {
        Some(slot) => {
            MAILBOX.call_once(|| Some(slot));
            crate::println!("updslot: SOSOUPD.TXT LBA {}", slot.data_lba);
            crate::otalog!("hueco SOSOUPD.TXT disponible LBA {}", slot.data_lba);
        }
        None => {
            crate::println!(
                "updslot: SOSOUPD.TXT no encontrado; actualizaciones de kernel desactivadas"
            );
            crate::otalog!("hueco SOSOUPD.TXT ausente: OTA de kernel desactivada");
        }
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
    // Huecos de la transacción (U5a) y de la identidad (U2). Que falten no es
    // fatal: sólo desactiva esa vía, como el resto.
    if let Some(slot) = espfat::locate(b"SOSOTXN ", b"BIN", soso_update_core::UPD_BOOTREC_SIZE) {
        TXN.call_once(|| Some(slot));
    } else {
        crate::println!("updslot: SOSOTXN.BIN no encontrado; sin registro de transacción");
    }
    if let Some(slot) = espfat::locate(b"SOSOMODE", b"TXT", soso_update_core::UPD_MODE_SIZE) {
        MODE.call_once(|| Some(slot));
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

/// Por qué no hay hueco: que **no esté** y que esté pero no sirva piden arreglos
/// distintos —reflashear entero o sólo reponerlo—, así que el errno lo dice.
/// Confundirlos manda a quien diagnostica a buscar fragmentación donde sólo
/// había un fichero que nunca se creó.
fn falta(nombre: &[u8; 8], ext: &[u8; 3]) -> i64 {
    if espfat::existe(nombre, ext) {
        -abi::ENOTSUP
    } else {
        -abi::ENOENT
    }
}

fn nombre_83(which: u64) -> (&'static [u8; 8], &'static [u8; 3]) {
    match which {
        abi::UPD_WHICH_MAILBOX => (b"SOSOUPD ", b"TXT"),
        abi::UPD_WHICH_KERNEL => (b"SOSOKRN ", b"BIN"),
        abi::UPD_WHICH_META => (b"SOSOKRN ", b"MET"),
        abi::UPD_WHICH_TXN => (b"SOSOTXN ", b"BIN"),
        _ => (b"SOSOMODE", b"TXT"),
    }
}

fn slot_for(which: u64) -> Result<Slot, i64> {
    let encontrado = match which {
        abi::UPD_WHICH_MAILBOX => MAILBOX.get().and_then(|s| *s),
        abi::UPD_WHICH_KERNEL => KERNEL.get().and_then(|s| *s),
        abi::UPD_WHICH_TXN => TXN.get().and_then(|s| *s),
        abi::UPD_WHICH_MODE => MODE.get().and_then(|s| *s),
        abi::UPD_WHICH_META => META.get().and_then(|s| *s),
        _ => return Err(-abi::EINVAL),
    };
    encontrado.ok_or_else(|| {
        let (n, e) = nombre_83(which);
        falta(n, e)
    })
}

fn max_size(which: u64) -> Result<usize, i64> {
    Ok(match which {
        abi::UPD_WHICH_MAILBOX => abi::UPD_MAILBOX_SIZE,
        abi::UPD_WHICH_KERNEL => abi::UPD_KERNEL_SLOT_SIZE as usize,
        abi::UPD_WHICH_META => abi::UPD_KERNEL_META_SIZE,
        abi::UPD_WHICH_TXN => soso_update_core::UPD_BOOTREC_SIZE,
        abi::UPD_WHICH_MODE => soso_update_core::UPD_MODE_SIZE,
        _ => return Err(-abi::EINVAL),
    })
}

/// Nombre del hueco para el registro de actualizaciones.
fn nombre(which: u64) -> &'static str {
    match which {
        abi::UPD_WHICH_MAILBOX => "SOSOUPD.TXT",
        abi::UPD_WHICH_TXN => "SOSOTXN.BIN",
        abi::UPD_WHICH_MODE => "SOSOMODE.TXT",
        abi::UPD_WHICH_KERNEL => "SOSOKRN.BIN",
        abi::UPD_WHICH_META => "SOSOKRN.MET",
        _ => "?",
    }
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
    let ok = crate::drivers::logbuf::run_without_capture(|| {
        let mut lba = slot.data_lba + offset / SECTOR as u64;
        for chunk in data.chunks(SECTOR) {
            if espfat::write(lba, chunk).is_err() {
                return false;
            }
            lba += 1;
        }
        true
    });
    // Los huecos pequeños son las decisiones durables de la actualización, así
    // que se registran enteros. El kernel llega en miles de trozos: sólo se
    // anotan el principio, el final y un hito cada 16 MiB, para no ahogar el
    // registro con lo mismo repetido.
    const HITO: u64 = 16 * 1024 * 1024;
    let hito = offset == 0
        || end == cap
        || offset / HITO != (offset + data.len() as u64 - 1) / HITO;
    if which != abi::UPD_WHICH_KERNEL || hito || !ok {
        crate::otalog!(
            "hueco {} escritura off={} len={} {}",
            nombre(which),
            offset,
            data.len(),
            if ok { "ok" } else { "FALLO" }
        );
    }
    ok.then_some(()).ok_or(-abi::EIO)
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
