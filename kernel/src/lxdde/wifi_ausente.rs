//! Sustituto de [`super::wifi`] cuando el port iwlwifi no está enlazado.
//!
//! Igual que `gpu_ausente`: la capa lxdde se compila también en máquinas sin
//! WiFi Intel — la Steam Deck lleva un Qualcomm y su perfil compila el port
//! `ath11k`. Sin este doble, el kernel referencia los símbolos `lx_iwlwifi_*`
//! y el enlace falla aunque nadie los llame.
//!
//! Todo responde «no hay radio»: ni presente, ni viva, ni conectada. El scan
//! y las conexiones devuelven error en vez de un resultado vacío, para que
//! `sosh wifi` diga que no hay hardware en vez de que no hay redes.

use alloc::string::String;
use alloc::vec::Vec;

pub fn init() -> i32 {
    -1
}

pub fn start_firmware() -> i32 {
    -1
}

pub fn poll() {}

pub fn wifi_present() -> bool {
    false
}

pub fn alive() -> bool {
    false
}

pub fn phase() -> &'static str {
    "sin-iwlwifi"
}

pub fn mac() -> Option<[u8; 6]> {
    None
}

pub fn bssid() -> Option<[u8; 6]> {
    None
}

pub fn connected() -> bool {
    false
}

pub fn scan_results() -> Vec<(String, i8, u8, bool)> {
    Vec::new()
}

pub fn scan() -> i32 {
    -1
}

pub fn connect_open(_ssid: &str) -> i32 {
    -1
}

pub fn connect_wpa2(_ssid: &str, _psk: &[u8; 32]) -> i32 {
    -1
}

pub fn install_key(_key: &[u8; 16], _key_idx: i32) -> i32 {
    -1
}

pub fn receive(_buf: &mut [u8]) -> Option<usize> {
    None
}

pub fn send(_data: &[u8]) -> Result<(), ()> {
    Err(())
}

pub fn can_send() -> bool {
    false
}
