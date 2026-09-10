//! Mini-supplicant WPA2-PSK para Intel AX211.
//!
//! Lee credenciales de `SOSOWIFI.TXT` (ESP live) o `/etc/wifi.conf`.
//! Completa el 4-way handshake EAPOL tras asociación MLME.

use alloc::string::{String, ToString};

const ETH_P_EAPOL: u16 = 0x888e;
const EAPOL_KEY: u8 = 3;
const WPA_KEY_INFO_MIC: u16 = 0x0100;
const WPA_KEY_INFO_ACK: u16 = 0x0080;
const WPA_KEY_INFO_INSTALL: u16 = 0x0040;
const WPA_KEY_INFO_KEY_TYPE: u16 = 0x0008;
const WPA_KEY_INFO_SECURE: u16 = 0x0200;

/// Deriva la PSK de 32 bytes (PMK) desde passphrase ASCII y SSID.
pub fn pbkdf2_psk(passphrase: &str, ssid: &str) -> [u8; 32] {
    use pbkdf2::pbkdf2_hmac;
    use sha1::Sha1;
    let mut out = [0u8; 32];
    pbkdf2_hmac::<Sha1>(passphrase.as_bytes(), ssid.as_bytes(), 4096, &mut out);
    out
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
        crate::println!("wifi-wpa: fallo assoc MLME (¿la red ofrece WPA2-PSK/CCMP?)");
        return -1;
    }

    if four_way_handshake(ssid, &pmk) != 0 {
        return -1;
    }
    crate::println!("wifi-wpa: 4-way handshake completado");
    0
}

fn four_way_handshake(_ssid: &str, pmk: &[u8; 32]) -> i32 {
    let mut buf = [0u8; 2048];
    for _ in 0..500 {
        crate::lxdde::wifi::poll();
        let Some(n) = crate::lxdde::wifi_receive(&mut buf) else {
            crate::arch::tsc::spin_us(10_000);
            continue;
        };
        if n < 14 {
            continue;
        }
        let ethertype = u16::from_be_bytes([buf[12], buf[13]]);
        if ethertype != ETH_P_EAPOL {
            continue;
        }
        let eapol = &buf[14..n];
        if eapol.len() < 4 || eapol[1] != EAPOL_KEY {
            continue;
        }
        let key_info = u16::from_be_bytes([eapol[5], eapol[6]]);
        if key_info & WPA_KEY_INFO_KEY_TYPE == 0 {
            continue;
        }
        if key_info & WPA_KEY_INFO_ACK != 0 {
            continue;
        }
        if eapol.len() < 99 {
            continue;
        }
        let anonce = &eapol[17..49];
        let mut snonce = [0u8; 32];
        let _ = getrandom::getrandom(&mut snonce);

        let mut ptk = [0u8; 48];
        derive_ptk(pmk, anonce, &snonce, &mut ptk);

        let mut m2 = [0u8; 256];
        let m2_len = build_eapol_m2(eapol, &snonce, &ptk, &mut m2);
        if m2_len == 0 {
            return -1;
        }
        let mut frame = [0u8; 512];
        let flen = wrap_eapol_tx(&mut frame, &m2[..m2_len]);
        if crate::lxdde::wifi_send(&frame[..flen]).is_err() {
            return -1;
        }

        for _ in 0..500 {
            crate::lxdde::wifi::poll();
            let Some(n2) = crate::lxdde::wifi_receive(&mut buf) else {
                crate::arch::tsc::spin_us(10_000);
                continue;
            };
            if n2 < 14 {
                continue;
            }
            if u16::from_be_bytes([buf[12], buf[13]]) != ETH_P_EAPOL {
                continue;
            }
            let m3 = &buf[14..n2];
            if m3.len() < 99 || m3[1] != EAPOL_KEY {
                continue;
            }
            let mut m3v = [0u8; 512];
            let ml = m3.len().min(m3v.len());
            m3v[..ml].copy_from_slice(&m3[..ml]);
            if !verify_eapol_mic(&ptk, &mut m3v[..ml]) {
                continue;
            }
            let mut gtk = [0u8; 16];
            if extract_gtk(&m3v[..ml], &mut gtk).is_err() {
                gtk.fill(0);
            }
            let mut m4 = [0u8; 128];
            let m4_len = build_eapol_m4(&m3v[..ml], &ptk, &mut m4);
            let mut frame4 = [0u8; 256];
            let flen4 = wrap_eapol_tx(&mut frame4, &m4[..m4_len]);
            let _ = crate::lxdde::wifi_send(&frame4[..flen4]);

            let mut ccmp_ptk = [0u8; 16];
            ccmp_ptk.copy_from_slice(&ptk[..16]);
            crate::lxdde::wifi::install_key(&ccmp_ptk, 0);
            if gtk != [0u8; 16] {
                crate::lxdde::wifi::install_key(&gtk, 1);
            }
            return 0;
        }
        return -1;
    }
    -1
}

fn cmp_bytes(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    let n = a.len().min(b.len());
    for i in 0..n {
        match a[i].cmp(&b[i]) {
            core::cmp::Ordering::Equal => {}
            o => return o,
        }
    }
    a.len().cmp(&b.len())
}

fn derive_ptk(pmk: &[u8; 32], anonce: &[u8], snonce: &[u8], out: &mut [u8; 48]) {
    use hmac::{Hmac, Mac};
    use sha1::Sha1;
    type HmacSha1 = Hmac<Sha1>;

    let sta = crate::lxdde::wifi_mac().unwrap_or([0; 6]);
    let bssid = crate::lxdde::wifi_bssid().unwrap_or([0xff; 6]);

    let mut prefix = [0u8; 128];
    let mut pos = 0usize;
    prefix[pos..pos + 23].copy_from_slice(b"Pairwise key expansion\0");
    pos += 23;
    let (a, b) = if cmp_bytes(&sta, &bssid) != core::cmp::Ordering::Greater {
        (sta, bssid)
    } else {
        (bssid, sta)
    };
    prefix[pos..pos + 6].copy_from_slice(&a);
    pos += 6;
    prefix[pos..pos + 6].copy_from_slice(&b);
    pos += 6;
    let (an, sn) = if cmp_bytes(anonce, snonce) != core::cmp::Ordering::Greater {
        (anonce, snonce)
    } else {
        (snonce, anonce)
    };
    prefix[pos..pos + an.len()].copy_from_slice(an);
    pos += an.len();
    prefix[pos..pos + sn.len()].copy_from_slice(sn);
    pos += sn.len();

    let mut mac = HmacSha1::new_from_slice(pmk).expect("hmac");
    mac.update(&prefix[..pos]);
    let t = mac.finalize().into_bytes();
    out[..20].copy_from_slice(&t);
    let mut mac2 = HmacSha1::new_from_slice(pmk).expect("hmac");
    mac2.update(&prefix[..pos]);
    mac2.update(&[0u8]);
    let t2 = mac2.finalize().into_bytes();
    out[20..40].copy_from_slice(&t2[..20]);
}

fn eapol_key_mic(ptk: &[u8; 48], eapol: &[u8]) -> [u8; 16] {
    use hmac::{Hmac, Mac};
    use sha1::Sha1;
    let mut mac = Hmac::<Sha1>::new_from_slice(&ptk[0..16]).expect("hmac");
    mac.update(eapol);
    let t = mac.finalize().into_bytes();
    let mut out = [0u8; 16];
    out.copy_from_slice(&t[..16]);
    out
}

fn set_eapol_mic(ptk: &[u8; 48], eapol: &mut [u8]) {
    if eapol.len() < 97 {
        return;
    }
    eapol[81..97].fill(0);
    let mic = eapol_key_mic(ptk, eapol);
    eapol[81..97].copy_from_slice(&mic);
}

fn verify_eapol_mic(ptk: &[u8; 48], eapol: &mut [u8]) -> bool {
    if eapol.len() < 97 {
        return false;
    }
    let mut saved = [0u8; 16];
    saved.copy_from_slice(&eapol[81..97]);
    eapol[81..97].fill(0);
    let calc = eapol_key_mic(ptk, eapol);
    eapol[81..97].copy_from_slice(&saved);
    calc == saved
}

fn extract_gtk(m3: &[u8], out: &mut [u8; 16]) -> Result<(), ()> {
    if m3.len() < 99 {
        return Err(());
    }
    let kd_len = u16::from_be_bytes([m3[97], m3[98]]) as usize;
    if kd_len == 0 || 99 + kd_len > m3.len() {
        return Err(());
    }
    let kd = &m3[99..99 + kd_len];
    let mut i = 0usize;
    while i + 2 <= kd.len() {
        let id = kd[i];
        let elen = kd[i + 1] as usize;
        if i + 2 + elen > kd.len() {
            break;
        }
        if id == 0xdd && elen >= 6 && kd[i + 2] == 0x00 && kd[i + 3] == 0x0f && kd[i + 4] == 0xac
        {
            if kd[i + 5] == 1 && elen >= 7 {
                let gtk_len = elen - 6;
                if gtk_len >= 16 {
                    out.copy_from_slice(&kd[i + 7..i + 7 + 16]);
                    return Ok(());
                }
            }
        }
        i += 2 + elen;
    }
    Err(())
}

fn wrap_eapol_tx(out: &mut [u8], eapol: &[u8]) -> usize {
    let len = 14 + eapol.len();
    if len > out.len() {
        return 0;
    }
    let sta = crate::lxdde::wifi_mac().unwrap_or([0; 6]);
    let bssid = crate::lxdde::wifi_bssid().unwrap_or([0xff; 6]);
    out[0..6].copy_from_slice(&bssid);
    out[6..12].copy_from_slice(&sta);
    out[12] = 0x88;
    out[13] = 0x8e;
    out[14..14 + eapol.len()].copy_from_slice(eapol);
    len
}

fn build_eapol_m2(m1: &[u8], snonce: &[u8; 32], ptk: &[u8; 48], out: &mut [u8]) -> usize {
    if m1.len() < 95 {
        return 0;
    }
    out[..95].copy_from_slice(&m1[..95]);
    out[0] = 0x02;
    out[1] = EAPOL_KEY;
    out[5] = 0x03;
    let key_info = WPA_KEY_INFO_KEY_TYPE | WPA_KEY_INFO_ACK | WPA_KEY_INFO_MIC;
    out[6] = (key_info & 0xff) as u8;
    out[7] = (key_info >> 8) as u8;
    out[17..49].copy_from_slice(snonce);
    out[97] = 0;
    out[98] = 0;
    set_eapol_mic(ptk, &mut out[..99]);
    99
}

fn build_eapol_m4(m3: &[u8], ptk: &[u8; 48], out: &mut [u8]) -> usize {
    if m3.len() < 99 {
        return 0;
    }
    out[..99].copy_from_slice(&m3[..99]);
    out[0] = 0x02;
    out[1] = EAPOL_KEY;
    out[5] = 0x03;
    let key_info = WPA_KEY_INFO_KEY_TYPE | WPA_KEY_INFO_ACK | WPA_KEY_INFO_MIC | WPA_KEY_INFO_SECURE;
    out[6] = (key_info & 0xff) as u8;
    out[7] = (key_info >> 8) as u8;
    out[97] = 0;
    out[98] = 0;
    set_eapol_mic(ptk, &mut out[..99]);
    99
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
