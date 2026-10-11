//! Cliente HTTP del agente: una petición JSON, una respuesta JSON, sobre un [`Conector`].
//!
//! Sin streaming a propósito: el agente necesita el mensaje entero (texto y llamadas) para decidir,
//! y un cuerpo con `Connection: close` se delimita por el cierre, lo que el transporte ya distingue
//! de un plazo agotado (T48).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::tiempo::{Plazo, Reloj};
use crate::transporte::{escribir_todo, leer_hasta_fin, Conector, Destino, Transporte};
use crate::{Error, Resultado};

use super::Modelo;

pub struct ModeloHttp<'a, C: Conector, R: Reloj> {
    pub conector: &'a C,
    pub reloj: &'a R,
    pub destino: Destino,
    pub plazo_conexion_ms: u64,
    /// Clave de la API (`Authorization: Bearer`). `None` = sin cabecera.
    pub token: Option<String>,
    /// Cómo esperar (ms) cuando el servidor contesta `429` («generación en curso»): lo pone el adaptador.
    pub dormir: fn(u64),
}

impl<'a, C: Conector, R: Reloj> ModeloHttp<'a, C, R> {
    fn una_vez(&mut self, peticion: &str, plazo: Plazo) -> Resultado<Vec<u8>> {
        let con = Plazo::en(self.reloj, self.plazo_conexion_ms);
        let mut enlace = self.conector.conectar(&self.destino, con, self.reloj)?;
        let auth = self.token.as_ref().map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
        let cabeza = format!(
            "POST /v1/chat/completions HTTP/1.1\r\nHost: {}\r\n{auth}Content-Type: application/json\r\nAccept: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
            self.destino.texto(),
            peticion.len()
        );
        let mut bytes = cabeza.into_bytes();
        bytes.extend_from_slice(peticion.as_bytes());
        escribir_todo(&mut enlace, self.reloj, plazo, &bytes)?;
        let crudo = leer_hasta_fin(&mut enlace, self.reloj, plazo, 8 * 1024 * 1024);
        enlace.cerrar();
        crudo
    }
}

/// Espera entre reintentos cuando el servidor está ocupado.
pub const ESPERA_BUSY_MS: u64 = 5_000;

impl<'a, C: Conector, R: Reloj> Modelo for ModeloHttp<'a, C, R> {
    /// Un servidor con una sola generación a la vez contesta `429` mientras genera para otro cliente (o
    /// para uno que murió a mitad). Eso **no** es un fallo del modelo: se espera y se reintenta hasta
    /// agotar el plazo del turno.
    fn completar(&mut self, peticion: &str, plazo_ms: u64) -> Resultado<String> {
        let plazo = Plazo::en(self.reloj, plazo_ms);
        loop {
            let crudo = self.una_vez(peticion, plazo)?;
            if estado_de(&crudo) == Some(429) {
                if plazo.vencido(self.reloj) {
                    return Err(Error::plazo("el servidor siguió ocupado (429) hasta agotar el plazo del turno"));
                }
                (self.dormir)(ESPERA_BUSY_MS);
                continue;
            }
            return cuerpo_de(&crudo);
        }
    }
}

/// Código de estado de una respuesta HTTP cruda, si se puede leer.
pub fn estado_de(crudo: &[u8]) -> Option<u16> {
    let fin = crudo.windows(2).position(|w| w == b"\r\n").unwrap_or(crudo.len().min(64));
    String::from_utf8_lossy(&crudo[..fin]).split_whitespace().nth(1).and_then(|c| c.parse().ok())
}

/// Valida estado 200 y devuelve el cuerpo (decodificando `chunked` si hace falta).
pub fn cuerpo_de(crudo: &[u8]) -> Resultado<String> {
    let Some(i) = crudo.windows(4).position(|w| w == b"\r\n\r\n") else {
        return Err(Error::entorno(format!("respuesta HTTP sin cabeceras ({} bytes)", crudo.len())));
    };
    let cabeza = String::from_utf8_lossy(&crudo[..i]).to_lowercase();
    let estado: Option<u16> = cabeza.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|c| c.parse().ok());
    let cuerpo = &crudo[i + 4..];
    let cuerpo = if cabeza.contains("transfer-encoding: chunked") { deschunk(cuerpo)? } else { cuerpo.to_vec() };
    let texto = String::from_utf8_lossy(&cuerpo).into_owned();
    if estado != Some(200) {
        return Err(Error::entorno(format!("el servidor contestó {:?}: {}", estado, texto.chars().take(400).collect::<String>())));
    }
    Ok(texto)
}

fn deschunk(mut d: &[u8]) -> Resultado<Vec<u8>> {
    let mut fuera = Vec::new();
    loop {
        let Some(p) = d.windows(2).position(|w| w == b"\r\n") else {
            return Err(Error::entorno("cuerpo chunked truncado"));
        };
        let n = usize::from_str_radix(String::from_utf8_lossy(&d[..p]).split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| Error::entorno("tamaño de chunk ilegible"))?;
        d = &d[p + 2..];
        if n == 0 {
            return Ok(fuera);
        }
        if d.len() < n {
            return Err(Error::entorno("cuerpo chunked truncado"));
        }
        fuera.extend_from_slice(&d[..n]);
        d = &d[n..];
        d = d.strip_prefix(b"\r\n").unwrap_or(d);
    }
}

