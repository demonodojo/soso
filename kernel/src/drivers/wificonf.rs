//! Credenciales WiFi en la ESP del live: `SOSOWIFI.TXT` (partición 1, FAT).
//!
//! Mismo formato que `/etc/wifi.conf` (`ssid=`, `psk=`). Si el fichero está
//! vacío o solo tiene comentarios, el kernel cae a `/etc/wifi.conf`.

use crate::drivers::espfat::{self, SECTOR, Slot};
use spin::Once;

pub const FILE_SIZE: usize = 4096;

static SLOT: Once<Option<Slot>> = Once::new();

pub fn init() {
    if !crate::drivers::live_disk::esp_available() {
        return;
    }
    match espfat::locate(b"SOSOWIFI", b"TXT", FILE_SIZE) {
        Some(slot) => {
            SLOT.call_once(|| Some(slot));
            crate::println!("wificonf: SOSOWIFI.TXT LBA {}", slot.data_lba);
        }
        None => crate::println!("wificonf: SOSOWIFI.TXT no encontrado (usa /etc/wifi.conf)"),
    }
}

pub fn read_text() -> Option<alloc::vec::Vec<u8>> {
    let slot = SLOT.get().and_then(|s| *s)?;
    let mut buf = alloc::vec::Vec::with_capacity(FILE_SIZE);
    buf.resize(FILE_SIZE, b'\n');
    espfat::read(slot.data_lba, &mut buf).ok()?;
    Some(buf)
}
