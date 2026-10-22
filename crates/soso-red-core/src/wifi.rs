//! Redes WiFi guardadas: `wifi.conf` / `SOSOWIFI.TXT`.
//!
//! Cada `ssid=` abre una red y el `psk=` que le sigue es suya; sin `psk=` es
//! abierta. Un fichero con una sola red es el formato de siempre. La primera
//! de la lista es la última a la que se conectó el usuario.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Redes que caben en la lista. Con 4 el fichero queda muy por debajo del
/// hueco de 4 KiB de `SOSOWIFI.TXT`.
pub const MAX_REDES: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedWifi {
    pub ssid: String,
    pub psk: Option<String>,
}

impl RedWifi {
    pub fn new(ssid: &str, psk: Option<&str>) -> Self {
        RedWifi {
            ssid: ssid.to_string(),
            psk: psk.filter(|p| !p.is_empty()).map(|p| p.to_string()),
        }
    }
}

/// Lee la lista. Un `psk=` antes del primer `ssid=` pertenece a esa primera
/// red (el orden de las dos líneas nunca estuvo fijado). Un `ssid=` repetido
/// no duplica la red: se queda la primera. Lo que sobra de `MAX_REDES` se
/// ignora.
pub fn parsear_redes(texto: &str) -> Vec<RedWifi> {
    let mut redes: Vec<RedWifi> = Vec::new();
    let mut psk_suelta: Option<String> = None;
    // Una red repetida absorbe sus líneas `psk=` sin sumarlas a la lista.
    let mut descartando = false;
    for linea in texto.lines() {
        let linea = linea.trim();
        if linea.is_empty() || linea.starts_with('#') {
            continue;
        }
        if let Some(v) = linea.strip_prefix("ssid=") {
            let ssid = v.trim();
            descartando = ssid.is_empty()
                || redes.iter().any(|r| r.ssid == ssid)
                || redes.len() == MAX_REDES;
            if !descartando {
                redes.push(RedWifi::new(ssid, None));
            }
        } else if let Some(v) = linea.strip_prefix("psk=") {
            let psk = v.trim();
            if descartando {
                continue;
            }
            match redes.last_mut() {
                Some(r) => r.psk = (!psk.is_empty()).then(|| psk.to_string()),
                None => psk_suelta = Some(psk.to_string()),
            }
        }
        if redes.len() == 1 && redes[0].psk.is_none() {
            if let Some(p) = psk_suelta.take().filter(|p| !p.is_empty()) {
                redes[0].psk = Some(p);
            }
        }
    }
    redes
}

/// Escribe la lista en el formato que lee [`parsear_redes`].
pub fn formatear_redes(redes: &[RedWifi]) -> String {
    let mut out = String::new();
    for r in redes.iter().take(MAX_REDES) {
        out.push_str("ssid=");
        out.push_str(&r.ssid);
        out.push('\n');
        if let Some(p) = r.psk.as_deref().filter(|p| !p.is_empty()) {
            out.push_str("psk=");
            out.push_str(p);
            out.push('\n');
        }
    }
    out
}

/// La red a la que acaba de conectarse el usuario pasa al frente. Si ya estaba
/// se sustituye (la clave pudo cambiar) y no se duplica; si la lista está
/// llena se pierde la última.
pub fn promover(mut redes: Vec<RedWifi>, nueva: RedWifi) -> Vec<RedWifi> {
    redes.retain(|r| r.ssid != nueva.ssid);
    redes.insert(0, nueva);
    redes.truncate(MAX_REDES);
    redes
}
