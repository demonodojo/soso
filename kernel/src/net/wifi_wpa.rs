//! Conexión WiFi: credenciales, asociación y 4-way EAPOL.
//!
//! Aquí sólo hay E/S. La máquina de estados WPA2-PSK/CCMP vive en el crate
//! [`soso_wpa2`], que se compila y se prueba en el host contra vectores y
//! transcripciones; este módulo le da tramas y ejecuta lo que decide.
//!
//! Lee credenciales de `SOSOWIFI.TXT` (ESP live) o `/etc/wifi.conf`.
//! Tras un `wifi connect` correcto las persiste en ambos para el próximo arranque.

use alloc::string::{String, ToString};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use soso_wpa2::{Discard, State, Supplicant};

/// Tiempo máximo sin avanzar el diálogo antes de darlo por perdido.
const STEP_TIMEOUT_MS: u32 = 6_000;
const POLL_US: u32 = 2_000;
/// Tope de tramas EAPOL procesadas. El reloj solo corre cuando no llega nada,
/// así que sin este tope un AP (o un vecino) que inunde con EAPOL descartables
/// dejaría el bucle girando para siempre.
const MAX_EAPOL_FRAMES: u32 = 64;
/// Primer reintento tras una caída y tope del retroceso exponencial.
const REINTENTO_PRIMERO_MS: u64 = 2_000;
const REINTENTO_MAX_MS: u64 = 60_000;

/// Supplicant del enlace vigente. Sigue vivo tras el 4-way: el AP renueva la
/// GTK y repite M3 si no recibió M4, y sin nadie que conteste acaba
/// desautenticando.
static SESION: spin::Mutex<Option<Supplicant>> = spin::Mutex::new(None);

/// Red a la que volver tras una caída. La PMK va ya derivada: PBKDF2 son 4096
/// vueltas de SHA-1 y no hace falta guardar la passphrase.
#[derive(Clone, Copy)]
struct Red {
    ssid: [u8; 32],
    len: usize,
    pmk: Option<[u8; 32]>,
}
static RED: spin::Mutex<Option<Red>> = spin::Mutex::new(None);

/// Hubo enlace autorizado y hay que vigilar que siga.
static ENLACE: AtomicBool = AtomicBool::new(false);
/// Una conexión en curso (arranque, `wifi connect` o reconexión). Dos a la vez
/// entrelazarían comandos al firmware de secuencias distintas.
static CONECTANDO: AtomicBool = AtomicBool::new(false);
/// La pila IP aún no se enteró de la caída (su candado estaba tomado).
static CAIDA_PENDIENTE: AtomicBool = AtomicBool::new(false);
/// Uptime en ms del próximo intento; 0 = ninguno pendiente.
static REINTENTO_EN: AtomicU64 = AtomicU64::new(0);
static ESPERA_MS: AtomicU64 = AtomicU64::new(REINTENTO_PRIMERO_MS);

/// Deriva la PSK de 32 bytes (PMK) desde passphrase ASCII y SSID.
pub fn pbkdf2_psk(passphrase: &str, ssid: &str) -> [u8; 32] {
    soso_wpa2::pbkdf2_psk(passphrase, ssid)
}

fn read_wifi_config_text() -> Option<String> {
    #[cfg(feature = "drv-live-disk")]
    if crate::drivers::live_disk::esp_available() {
        if let Some(raw) = crate::drivers::wificonf::read_text() {
            let text = core::str::from_utf8(&raw).unwrap_or("");
            if parse_wifi_conf(text).is_some() {
                return Some(text.to_string());
            }
            crate::println!(
                "wifi: SOSOWIFI.TXT sin ssid= válido — elige red en sosh o escribe ssid=/psk="
            );
            return None;
        }
    }
    let data = crate::vfs::resolve("/etc/wifi.conf")
        .ok()
        .and_then(|ino| crate::vfs::read_file(ino).ok())?;
    let text = core::str::from_utf8(&data).unwrap_or("");
    if parse_wifi_conf(text).is_some() {
        Some(text.to_string())
    } else {
        None
    }
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
    connect_open(&ssid)
}

/// Conecta a red abierta. Pasa por aquí (no directo al driver) para que una
/// caída posterior sepa a qué red volver.
pub fn connect_open(ssid: &str) -> i32 {
    conectar(ssid, None)
}

/// Conecta a red WPA2-PSK: assoc + 4-way EAPOL + claves CCMP.
pub fn connect_wpa2(ssid: &str, passphrase: &str) -> i32 {
    let pmk = pbkdf2_psk(passphrase, ssid);
    crate::println!("wifi-wpa: PSK derivada para '{ssid}'");
    conectar(ssid, Some(&pmk))
}

fn conectar(ssid: &str, pmk: Option<&[u8; 32]>) -> i32 {
    if CONECTANDO.swap(true, Ordering::AcqRel) {
        crate::println!("wifi-wpa: ya hay una conexión en curso");
        return -1;
    }
    ENLACE.store(false, Ordering::Release);
    *SESION.lock() = None;
    let rc = match pmk {
        Some(pmk) => asociar_wpa2(ssid, pmk),
        None => asociar_abierta(ssid),
    };
    if rc == 0 {
        let mut red = Red { ssid: [0; 32], len: ssid.len().min(32), pmk: pmk.copied() };
        red.ssid[..red.len].copy_from_slice(&ssid.as_bytes()[..red.len]);
        *RED.lock() = Some(red);
        // Una caída aún sin aplicar borraría la configuración que el llamante
        // está a punto de pedir con `on_wifi_connected`.
        CAIDA_PENDIENTE.store(false, Ordering::Release);
        REINTENTO_EN.store(0, Ordering::Relaxed);
        ESPERA_MS.store(REINTENTO_PRIMERO_MS, Ordering::Relaxed);
        ENLACE.store(true, Ordering::Release);
    }
    CONECTANDO.store(false, Ordering::Release);
    rc
}

fn asociar_abierta(ssid: &str) -> i32 {
    let rc = crate::lxdde::wifi::connect_open(ssid);
    if rc == 0 {
        crate::println!("wifi-wpa: asociado a '{ssid}' (abierta)");
    }
    rc
}

fn asociar_wpa2(ssid: &str, pmk: &[u8; 32]) -> i32 {
    crate::lxdde::wifi_scan();
    // La ruta WPA2 va por `connect_wpa2` del driver, que exige que el BSS
    // ofrezca RSN con AKM PSK y CCMP. `connect_open` ahora rechaza las redes
    // protegidas (R7), así que pasar por ahí era quedarse sin conexión.
    if crate::lxdde::wifi::connect_wpa2(ssid, pmk) != 0 {
        crate::println!("wifi-wpa: fallo AUTH+ASSOC 802.11 (¿la red ofrece WPA2-PSK/CCMP?)");
        return -1;
    }
    crate::println!("wifi-wpa: AUTH+ASSOC 802.11 ok, 4-way EAPOL");

    let Some(sup) = four_way_handshake(pmk) else {
        // Asociada pero sin autorizar: que nadie confunda el enlace con red.
        crate::lxdde::wifi::set_authorized(false);
        return -1;
    };
    *SESION.lock() = Some(sup);
    crate::lxdde::wifi::set_authorized(true);
    crate::println!("wifi-wpa: 4-way completado, enlace autorizado");
    0
}

/// Trabajo del enlace que no cabe en `net::poll`, que corre también en el
/// tick: contestar EAPOL tras el 4-way, detectar la caída y reconectar.
/// Lo llama el scheduler en la BSP, fuera de IRQ y sin candados tomados.
pub fn mantener() {
    if !crate::lxdde::wifi_present() {
        return;
    }
    crate::lxdde::wifi::vigilar_contadores(super::device::wifi_tx_errores());
    if CONECTANDO.load(Ordering::Acquire) {
        return;
    }
    atender_eapol();
    vigilar_enlace();
    if CAIDA_PENDIENTE.load(Ordering::Acquire) && super::on_wifi_lost() {
        CAIDA_PENDIENTE.store(false, Ordering::Release);
    }
    reintentar();
}

fn atender_eapol() {
    let Some(mut sesion) = SESION.try_lock() else {
        return;
    };
    let Some(sup) = sesion.as_mut() else {
        return;
    };
    let mut buf = [0u8; 14 + soso_wpa2::MAX_EAPOL];
    // Pocas por vuelta: esto corre en el scheduler.
    for _ in 0..4 {
        let Some(n) = crate::lxdde::wifi::try_receive_eapol(&mut buf) else {
            return;
        };
        let out = sup.on_ethernet(&buf[..n]);
        if let Some(d) = out.dropped {
            crate::println!("wifi-wpa: EAPOL tras el 4-way descartada ({})", motivo(d));
            continue;
        }
        if out.send && crate::lxdde::wifi_send(sup.tx()).is_err() {
            crate::println!("wifi-wpa: no se pudo contestar al EAPOL del AP");
        }
        if let Some(tk) = out.install_tk
            && crate::lxdde::wifi::install_key(&tk, 0) != 0
        {
            crate::println!("wifi-wpa: el firmware rechazó la PTK renegociada");
        }
        if let Some(g) = out.install_gtk {
            if crate::lxdde::wifi::install_gtk(&g.key, g.key_id as i32, &g.rsc) != 0 {
                crate::println!("wifi-wpa: el firmware rechazó la GTK renovada");
            } else {
                crate::println!("wifi-wpa: GTK renovada (key id {})", g.key_id);
            }
        }
    }
}

fn vigilar_enlace() {
    if !ENLACE.load(Ordering::Acquire) || crate::lxdde::wifi_authorized() {
        return;
    }
    ENLACE.store(false, Ordering::Release);
    *SESION.lock() = None;
    let c = crate::lxdde::wifi::contadores();
    crate::println!(
        "wifi: enlace perdido ({}, motivo 802.11 {}); reconexión en {} s",
        c.causa_caida(),
        c.motivo(),
        REINTENTO_PRIMERO_MS / 1000
    );
    CAIDA_PENDIENTE.store(true, Ordering::Release);
    ESPERA_MS.store(REINTENTO_PRIMERO_MS, Ordering::Relaxed);
    REINTENTO_EN.store(
        crate::arch::pit::uptime_ms() + REINTENTO_PRIMERO_MS,
        Ordering::Relaxed,
    );
}

fn reintentar() {
    let en = REINTENTO_EN.load(Ordering::Relaxed);
    if en == 0 || crate::arch::pit::uptime_ms() < en {
        return;
    }
    REINTENTO_EN.store(0, Ordering::Relaxed);
    let Some(red) = *RED.lock() else {
        return;
    };
    let ssid = core::str::from_utf8(&red.ssid[..red.len]).unwrap_or("");
    crate::println!("wifi: reconectando a '{ssid}'…");
    // El intento anterior pudo dejar MAC/STA/colas a medias: firmware nuevo.
    crate::lxdde::wifi::reset_link();
    if conectar(ssid, red.pmk.as_ref()) == 0 {
        super::on_wifi_connected();
        crate::println!("wifi: reconectado a '{ssid}'");
        return;
    }
    let espera = ESPERA_MS.load(Ordering::Relaxed);
    ESPERA_MS.store((espera * 2).min(REINTENTO_MAX_MS), Ordering::Relaxed);
    REINTENTO_EN.store(crate::arch::pit::uptime_ms() + espera, Ordering::Relaxed);
    crate::println!("wifi: reconexión fallida; siguiente intento en {} s", espera / 1000);
}

/// Ejecuta el 4-way contra el AP. Devuelve el supplicant sólo si quedan
/// claves instaladas; sigue haciendo falta para la renovación de grupo.
fn four_way_handshake(pmk: &[u8; 32]) -> Option<Supplicant> {
    let Some(sta) = crate::lxdde::wifi_mac() else {
        crate::println!("wifi-wpa: sin MAC de la estación");
        return None;
    };
    let Some(bssid) = crate::lxdde::wifi_bssid() else {
        crate::println!("wifi-wpa: sin BSSID; ¿de verdad está asociada?");
        return None;
    };
    let Some(rsn_ie) = crate::lxdde::wifi::rsn_ie() else {
        crate::println!("wifi-wpa: el driver no expone el RSN IE anunciado");
        return None;
    };
    let mut snonce = [0u8; 32];
    if getrandom::getrandom(&mut snonce).is_err() {
        crate::println!("wifi-wpa: sin fuente de aleatoriedad para la SNonce");
        return None;
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
                return None;
            }
            // El reloj se reinicia con cada avance real del diálogo.
            waited_us = 0;
        }
        if let Some(tk) = out.install_tk
            && crate::lxdde::wifi::install_key(&tk, 0) != 0
        {
            crate::println!("wifi-wpa: el firmware rechazó la clave de pares");
            return None;
        }
        if let Some(g) = out.install_gtk
            && crate::lxdde::wifi::install_gtk(&g.key, g.key_id as i32, &g.rsc) != 0
        {
            crate::println!("wifi-wpa: el firmware rechazó la clave de grupo");
            return None;
        }
        if sup.state() == State::Authorized {
            return Some(sup);
        }
    }
    crate::println!(
        "wifi-wpa: 4-way sin terminar en estado {:?} ({frames} tramas EAPOL)",
        sup.state()
    );
    None
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
