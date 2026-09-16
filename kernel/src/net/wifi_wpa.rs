//! Conexión WiFi: credenciales, asociación y 4-way EAPOL.
//!
//! Aquí sólo hay E/S. La máquina de estados WPA2-PSK/CCMP vive en el crate
//! [`soso_wpa2`], que se compila y se prueba en el host contra vectores y
//! transcripciones; este módulo le da tramas y ejecuta lo que decide.
//!
//! Lee credenciales de `SOSOWIFI.TXT` (ESP live) o `/etc/wifi.conf`.
//! Tras un `wifi connect` correcto las persiste en ambos para el próximo arranque.

use alloc::string::{String, ToString};
use soso_wpa2::{Discard, State, Supplicant};

/// Tiempo máximo sin avanzar el diálogo antes de darlo por perdido.
const STEP_TIMEOUT_MS: u32 = 6_000;
const POLL_US: u32 = 2_000;
/// Tope de tramas EAPOL procesadas. El reloj solo corre cuando no llega nada,
/// así que sin este tope un AP (o un vecino) que inunde con EAPOL descartables
/// dejaría el bucle girando para siempre.
const MAX_EAPOL_FRAMES: u32 = 64;

/// Deriva la PSK de 32 bytes (PMK) desde passphrase ASCII y SSID.
pub fn pbkdf2_psk(passphrase: &str, ssid: &str) -> [u8; 32] {
    soso_wpa2::pbkdf2_psk(passphrase, ssid)
}

fn read_wifi_config_text() -> Option<String> {
    #[cfg(feature = "drv-live-disk")]
    if let Some(raw) = crate::drivers::wificonf::read_text() {
        let text = core::str::from_utf8(&raw).unwrap_or("");
        if parse_wifi_conf(text).is_some() {
            return Some(text.to_string());
        }
    }
    let data = crate::vfs::resolve("/etc/wifi.conf")
        .ok()
        .and_then(|ino| crate::vfs::read_file(ino).ok())?;
    Some(core::str::from_utf8(&data).unwrap_or("").to_string())
}

/// Autoconnect: scan, elige SSID de config, conecta (abierta o WPA2).
pub fn autoconnect() -> i32 {
    let text = match read_wifi_config_text() {
        Some(t) => t,
        None => return -1,
    };
    let Some((ssid, psk)) = parse_wifi_conf(&text) else {
        crate::println!("wifi-wpa: falta ssid= en configuración");
        return -1;
    };
    crate::lxdde::wifi_scan();
    if let Some(pass) = psk {
        return connect_wpa2(&ssid, &pass);
    }
    let rc = crate::lxdde::wifi::connect_open(&ssid);
    if rc == 0 {
        crate::println!("wifi-wpa: asociado a '{ssid}' (abierta)");
    }
    rc
}

/// Conecta a red WPA2-PSK: assoc + 4-way EAPOL + claves CCMP.
pub fn connect_wpa2(ssid: &str, passphrase: &str) -> i32 {
    let pmk = pbkdf2_psk(passphrase, ssid);
    crate::println!("wifi-wpa: PSK derivada para '{ssid}'");

    crate::lxdde::wifi_scan();
    // La ruta WPA2 va por `connect_wpa2` del driver, que exige que el BSS
    // ofrezca RSN con AKM PSK y CCMP. `connect_open` ahora rechaza las redes
    // protegidas (R7), así que pasar por ahí era quedarse sin conexión.
    if crate::lxdde::wifi::connect_wpa2(ssid, &pmk) != 0 {
        crate::println!("wifi-wpa: fallo AUTH+ASSOC 802.11 (¿la red ofrece WPA2-PSK/CCMP?)");
        return -1;
    }
    crate::println!("wifi-wpa: AUTH+ASSOC 802.11 ok, 4-way EAPOL");

    if four_way_handshake(&pmk) != 0 {
        // Asociada pero sin autorizar: que nadie confunda el enlace con red.
        crate::lxdde::wifi::set_authorized(false);
        return -1;
    }
    crate::lxdde::wifi::set_authorized(true);
    crate::println!("wifi-wpa: 4-way completado, enlace autorizado");
    0
}

/// Ejecuta el 4-way contra el AP. Devuelve 0 sólo si quedan claves instaladas.
fn four_way_handshake(pmk: &[u8; 32]) -> i32 {
    let Some(sta) = crate::lxdde::wifi_mac() else {
        crate::println!("wifi-wpa: sin MAC de la estación");
        return -1;
    };
    let Some(bssid) = crate::lxdde::wifi_bssid() else {
        crate::println!("wifi-wpa: sin BSSID; ¿de verdad está asociada?");
        return -1;
    };
    let Some(rsn_ie) = crate::lxdde::wifi::rsn_ie() else {
        crate::println!("wifi-wpa: el driver no expone el RSN IE anunciado");
        return -1;
    };
    let mut snonce = [0u8; 32];
    if getrandom::getrandom(&mut snonce).is_err() {
        crate::println!("wifi-wpa: sin fuente de aleatoriedad para la SNonce");
        return -1;
    }

    let mut sup = Supplicant::new(*pmk, sta, bssid, &rsn_ie, snonce);
    let mut buf = [0u8; 2048];
    let mut waited_us = 0u32;
    let mut frames = 0u32;

    while waited_us < STEP_TIMEOUT_MS * 1000 && frames < MAX_EAPOL_FRAMES {
        crate::lxdde::wifi::poll();
        let Some(n) = crate::lxdde::wifi_receive_eapol(&mut buf) else {
            crate::arch::tsc::spin_us(POLL_US as u64);
            waited_us += POLL_US;
            continue;
        };
        frames += 1;
        let out = sup.on_ethernet(&buf[..n]);
        if let Some(d) = out.dropped {
            // Un descarte silencioso era lo que dejaba el handshake colgado sin
            // decir en qué punto.
            crate::println!("wifi-wpa: EAPOL descartada ({})", motivo(d));
        }
        if out.send {
            let frame = sup.tx();
            if crate::lxdde::wifi_send(frame).is_err() {
                crate::println!("wifi-wpa: no se pudo enviar la respuesta EAPOL");
                return -1;
            }
            // El reloj se reinicia con cada avance real del diálogo.
            waited_us = 0;
        }
        if let Some(tk) = out.install_tk
            && crate::lxdde::wifi::install_key(&tk, 0) != 0
        {
            crate::println!("wifi-wpa: el firmware rechazó la clave de pares");
            return -1;
        }
        if let Some(g) = out.install_gtk
            && crate::lxdde::wifi::install_gtk(&g.key, g.key_id as i32, &g.rsc) != 0
        {
            crate::println!("wifi-wpa: el firmware rechazó la clave de grupo");
            return -1;
        }
        if sup.state() == State::Authorized {
            return 0;
        }
    }
    crate::println!(
        "wifi-wpa: 4-way sin terminar en estado {:?} ({frames} tramas EAPOL)",
        sup.state()
    );
    -1
}

fn motivo(d: Discard) -> &'static str {
    match d {
        Discard::NotEapol(_) => "no es un EAPOL-Key válido",
        Discard::TooBig => "demasiado grande",
        Discard::NotFromAp => "sin ACK: no viene del AP",
        Discard::BadVersion => "descriptor que no es WPA2/CCMP",
        Discard::Replay => "contador de reenvío repetido",
        Discard::BadMic => "MIC incorrecto",
        Discard::NonceMismatch => "ANonce distinta de la de M1",
        Discard::KeyDataUnwrap => "Key Data que no se puede descifrar",
        Discard::NoGtk => "M3 sin GTK utilizable",
        Discard::Unexpected => "mensaje fuera de secuencia",
        Discard::NoSpace => "respuesta que no cabe",
    }
}

/// Serializa el formato que lee `parse_wifi_conf` / el autoconnect.
pub fn format_wifi_conf(ssid: &str, psk: Option<&str>) -> String {
    match psk {
        Some(p) if !p.is_empty() => alloc::format!("ssid={ssid}\npsk={p}\n"),
        _ => alloc::format!("ssid={ssid}\n"),
    }
}

fn credenciales_validas(ssid: &str, psk: Option<&str>) -> bool {
    if ssid.is_empty() || ssid.contains('\n') || ssid.contains('\r') {
        return false;
    }
    match psk {
        Some(p) => !p.contains('\n') && !p.contains('\r'),
        None => true,
    }
}

fn write_etc_wifi_conf(text: &str) -> bool {
    if crate::fs::FS.get().is_none() {
        return false;
    }
    let mtime = crate::time::wall_secs();
    let Ok(etc) = crate::vfs::resolve("/etc") else {
        return false;
    };
    match crate::vfs::create_file(etc, "wifi.conf", text.as_bytes(), mtime) {
        Ok(_) => true,
        Err(e) => {
            crate::println!("wifi: no pude escribir /etc/wifi.conf ({e:?})");
            false
        }
    }
}

/// Guarda SSID/clave para el autoconnect y DHCP del siguiente arranque.
/// ESP primero (`SOSOWIFI.TXT`); `/etc/wifi.conf` como respaldo e instalación.
pub fn persist_credentials(ssid: &str, psk: Option<&str>) {
    if !credenciales_validas(ssid, psk) {
        crate::println!("wifi: no se guardan credenciales (SSID o clave inválidos)");
        return;
    }
    let text = format_wifi_conf(ssid, psk);
    let mut ok = false;
    #[cfg(feature = "drv-live-disk")]
    match crate::drivers::wificonf::write_text(&text) {
        Ok(true) => {
            crate::println!("wifi: credenciales en SOSOWIFI.TXT");
            ok = true;
        }
        Ok(false) => {}
        Err(()) => crate::println!("wifi: no se pudo escribir SOSOWIFI.TXT"),
    }
    if write_etc_wifi_conf(&text) {
        crate::println!("wifi: credenciales en /etc/wifi.conf");
        ok = true;
    }
    if !ok {
        crate::println!("wifi: no se pudieron guardar las credenciales");
    }
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
