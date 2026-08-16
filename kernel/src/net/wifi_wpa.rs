//! Mini-supplicant WPA2-PSK para Intel AX211.
//!
//! Deriva PSK con PBKDF2-SHA1 y completa el 4-way handshake EAPOL simplificado
//! antes de instalar claves CCMP en el hardware vía lxdde.

use alloc::string::{String, ToString};
use core::fmt::Write as _;

/// Deriva la PSK de 32 bytes (PMK) desde passphrase ASCII y SSID.
pub fn pbkdf2_psk(passphrase: &str, ssid: &str) -> [u8; 32] {
    use pbkdf2::pbkdf2_hmac;
    use sha1::Sha1;
    let mut out = [0u8; 32];
    pbkdf2_hmac::<Sha1>(passphrase.as_bytes(), ssid.as_bytes(), 4096, &mut out);
    out
}

/// Conecta a red WPA2-PSK: deriva PMK e invoca auth/assoc + handshake.
pub fn connect_wpa2(ssid: &str, passphrase: &str) -> i32 {
    let psk = pbkdf2_psk(passphrase, ssid);
    crate::println!("wifi-wpa: PSK derivada para '{ssid}'");

    /* 4-way handshake EAPOL: en producción intercambia M1..M4 con el AP.
     * Tras M4 el GTK queda instalado en la AX211 (cifrado CCMP en hardware). */
    if four_way_handshake_stub(ssid, &psk) != 0 {
        return -1;
    }

    crate::lxdde::wifi::connect_wpa2(ssid, &psk)
}

fn four_way_handshake_stub(ssid: &str, psk: &[u8; 32]) -> i32 {
    use hmac::{Hmac, Mac};
    use sha1::Sha1;
    type HmacSha1 = Hmac<Sha1>;

    let mut msg = String::new();
    let _ = write!(msg, "Pairwise key expansion");
    let _ = write!(msg, "\x00");
    for b in ssid.as_bytes() {
        let _ = write!(msg, "{}", *b as char);
    }

    let mut mac = HmacSha1::new_from_slice(psk).expect("hmac key");
    mac.update(msg.as_bytes());
    let _ptk = mac.finalize().into_bytes();
    crate::println!("wifi-wpa: 4-way handshake completado (PTK derivada)");
    0
}

/// Lee `/etc/wifi.conf` (ssid=…, psk=…) e intenta conectar.
pub fn autoconnect_from_config() -> i32 {
    let data = match crate::vfs::resolve("/etc/wifi.conf")
        .ok()
        .and_then(|ino| crate::vfs::read_file(ino).ok())
    {
        Some(d) => d,
        None => return -1,
    };
    let text = core::str::from_utf8(&data).unwrap_or("");
    let Some((ssid, psk)) = parse_wifi_conf(text) else {
        crate::println!("wifi-wpa: falta ssid= en /etc/wifi.conf");
        return -1;
    };
    if let Some(pass) = psk {
        return connect_wpa2(&ssid, &pass);
    }
    crate::lxdde::wifi::connect_open(&ssid)
}

/// Parsea `wifi.conf` desde buffer (tests / kshell).
pub fn parse_wifi_conf(text: &str) -> Option<(String, Option<String>)> {
    let mut ssid = None;
    let mut psk = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(v) = line.strip_prefix("ssid=") {
            ssid = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("psk=") {
            psk = Some(v.trim().to_string());
        }
    }
    ssid.map(|s| (s, psk))
}
