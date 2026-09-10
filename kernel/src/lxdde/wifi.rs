//! Puente Rust ↔ driver Intel AX211 (lxdde/iwlwifi).

use core::ffi::{c_char, c_int};

const MAX_SCAN: usize = 32;
const SSID_MAX: usize = 32;

/// Espejo de `struct iwl_ax211_bss` (lxdde/ports/iwlwifi/iwl_internal.h).
/// El driver copia el array con memcpy: mismo orden y mismo tamaño.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LxWifiBss {
    pub ssid: [u8; SSID_MAX + 1],
    pub bssid: [u8; 6],
    pub rssi: i8,
    pub channel: u8,
    pub open: u8,
    /// RSN IE presente en el beacon.
    pub rsn: u8,
    /// Ofrece AKM PSK (WPA2-Personal).
    pub akm_psk: u8,
    /// Cifrado por pares y de grupo CCMP-128.
    pub ccmp: u8,
    pub band24: u8,
}

unsafe extern "C" {
    fn lx_iwlwifi_init_module() -> c_int;
    fn lx_iwlwifi_start_module() -> c_int;
    fn lx_iwlwifi_fw_alive() -> c_int;
    fn lx_iwlwifi_probed() -> c_int;
    fn lx_iwlwifi_fw_phase() -> *const c_char;
    fn lx_iwlwifi_scan(out: *mut LxWifiBss, max: c_int, count: *mut c_int) -> c_int;
    fn lx_iwlwifi_get_scan_results(out: *mut LxWifiBss, max: c_int, count: *mut c_int) -> c_int;
    fn lx_iwlwifi_connect_open(ssid: *const c_char) -> c_int;
    fn lx_iwlwifi_connect_wpa2(ssid: *const c_char, psk: *const u8) -> c_int;
    fn lx_iwlwifi_install_key(key: *const u8, key_idx: c_int) -> c_int;
    fn lx_iwlwifi_connected() -> c_int;
    fn lx_iwlwifi_rx(buf: *mut u8, buflen: c_int) -> c_int;
    fn lx_iwlwifi_tx(buf: *const u8, len: c_int) -> c_int;
    fn lx_iwlwifi_mac(mac: *mut u8) -> c_int;
    fn lx_iwlwifi_bssid(bssid: *mut u8) -> c_int;
    fn lx_iwlwifi_poll();
}

static mut WIFI_REGISTERED: bool = false;

/// Propietario único del transporte iwlwifi (R4).
///
/// El anillo de comandos, el drenaje RX y el sondeo comparten estado en
/// `g_iwl`: sin este candado un `scan` desde una syscall en otro core podía
/// reservar el mismo slot que el drenaje de `poll()` estaba liberando. El
/// driver C rechaza además la reentrada (`in_trans`), que es lo que se puede
/// comprobar en el banco host; esto es la exclusión real entre cores.
static TRANS_LOCK: spin::Mutex<()> = spin::Mutex::new(());

pub fn init() -> i32 {
    let rc = unsafe { lx_iwlwifi_init_module() };
    if rc == 0 {
        unsafe { WIFI_REGISTERED = true; }
    }
    rc
}

pub fn start_firmware() -> i32 {
    if unsafe { !WIFI_REGISTERED } {
        return -1;
    }
    let _g = TRANS_LOCK.lock();
    unsafe { lx_iwlwifi_start_module() }
}

pub fn poll() {
    if !wifi_present() {
        return;
    }
    // El sondeo cede si otro camino ya es el propietario: volverá al próximo
    // tick en vez de girar detrás de un scan de varios segundos.
    let Some(_g) = TRANS_LOCK.try_lock() else {
        return;
    };
    unsafe { lx_iwlwifi_poll() };
}

pub fn wifi_present() -> bool {
    unsafe { lx_iwlwifi_probed() != 0 }
}

pub fn alive() -> bool {
    unsafe { lx_iwlwifi_fw_alive() != 0 }
}

pub fn phase() -> &'static str {
    unsafe {
        let p = lx_iwlwifi_fw_phase();
        if p.is_null() {
            return "?";
        }
        core::ffi::CStr::from_ptr(p).to_str().unwrap_or("?")
    }
}

pub fn mac() -> Option<[u8; 6]> {
    let mut mac = [0u8; 6];
    if unsafe { lx_iwlwifi_mac(mac.as_mut_ptr()) } != 0 {
        return None;
    }
    Some(mac)
}

pub fn bssid() -> Option<[u8; 6]> {
    let mut bssid = [0u8; 6];
    if unsafe { lx_iwlwifi_bssid(bssid.as_mut_ptr()) } != 0 {
        return None;
    }
    Some(bssid)
}

pub fn connected() -> bool {
    unsafe { lx_iwlwifi_connected() != 0 }
}

pub fn scan_results() -> alloc::vec::Vec<(alloc::string::String, i8, u8, bool)> {
    let mut out = [LxWifiBss {
        ssid: [0; SSID_MAX + 1],
        bssid: [0; 6],
        rssi: 0,
        channel: 0,
        open: 0,
        rsn: 0,
        akm_psk: 0,
        ccmp: 0,
        band24: 0,
    }; MAX_SCAN];
    let mut count = 0i32;
    let rc = {
        let _g = TRANS_LOCK.lock();
        unsafe { lx_iwlwifi_get_scan_results(out.as_mut_ptr(), MAX_SCAN as c_int, &mut count) }
    };
    if rc != 0 || count <= 0 {
        return alloc::vec::Vec::new();
    }
    let mut v = alloc::vec::Vec::new();
    for b in &out[..count as usize] {
        let n = b.ssid.iter().position(|&c| c == 0).unwrap_or(SSID_MAX);
        let ssid = alloc::string::String::from_utf8_lossy(&b.ssid[..n]).into_owned();
        v.push((ssid, b.rssi, b.channel, b.open != 0));
    }
    v
}

pub fn scan() -> i32 {
    let _g = TRANS_LOCK.lock();
    unsafe { lx_iwlwifi_scan(core::ptr::null_mut(), 0, core::ptr::null_mut()) }
}

pub fn connect_open(ssid: &str) -> i32 {
    let mut buf = [0u8; SSID_MAX + 1];
    let bytes = ssid.as_bytes();
    let n = bytes.len().min(SSID_MAX);
    buf[..n].copy_from_slice(&bytes[..n]);
    let _g = TRANS_LOCK.lock();
    unsafe { lx_iwlwifi_connect_open(buf.as_ptr() as *const c_char) }
}

pub fn connect_wpa2(ssid: &str, psk: &[u8; 32]) -> i32 {
    let mut buf = [0u8; SSID_MAX + 1];
    let bytes = ssid.as_bytes();
    let n = bytes.len().min(SSID_MAX);
    buf[..n].copy_from_slice(&bytes[..n]);
    let _g = TRANS_LOCK.lock();
    unsafe { lx_iwlwifi_connect_wpa2(buf.as_ptr() as *const c_char, psk.as_ptr()) }
}

pub fn install_key(key: &[u8; 16], key_idx: i32) -> i32 {
    let _g = TRANS_LOCK.lock();
    unsafe { lx_iwlwifi_install_key(key.as_ptr(), key_idx) }
}

pub fn receive(buf: &mut [u8]) -> Option<usize> {
    let _g = TRANS_LOCK.lock();
    let n = unsafe { lx_iwlwifi_rx(buf.as_mut_ptr(), buf.len() as c_int) };
    if n > 0 {
        Some(n as usize)
    } else {
        None
    }
}

pub fn send(data: &[u8]) -> Result<(), ()> {
    let _g = TRANS_LOCK.lock();
    let rc = unsafe { lx_iwlwifi_tx(data.as_ptr(), data.len() as c_int) };
    if rc >= 0 {
        Ok(())
    } else {
        Err(())
    }
}

pub fn can_send() -> bool {
    connected()
}
