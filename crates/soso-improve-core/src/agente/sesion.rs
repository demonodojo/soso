//! Sesión del agente: la conversación publicada por generaciones (T46) tras cada turno, para que un
//! agente que muere pueda seguir donde estaba y nunca vea una sesión a medias.

use alloc::string::String;
use alloc::vec::Vec;

use serde_json::Value;

use crate::durable::{leer_vigente, publicar, purgar, Durable};
use crate::{sha256_hex, Error, Resultado};

const NOMBRE: &str = "sesion.json";
/// Generaciones que se conservan además de la vigente.
const CONSERVAR: u64 = 2;

/// Publica la conversación. Devuelve la generación.
pub fn guardar<D: Durable>(d: &mut D, dir: &str, mensajes: &[Value]) -> Resultado<u64> {
    if !d.existe(dir) {
        d.crear_directorio(dir)?;
    }
    let datos = serde_json::to_vec(mensajes).map_err(Error::formato)?;
    let actual: Option<String> = leer_vigente(d, dir, NOMBRE)?.map(|(_, v)| sha256_hex(&v));
    let gen = publicar(d, dir, NOMBRE, &datos, actual.as_deref())?;
    // Un fallo al purgar sobra un fichero, no falta uno.
    let _ = purgar(d, dir, NOMBRE, gen.saturating_sub(CONSERVAR));
    Ok(gen)
}

/// La conversación vigente, si hay una íntegra.
pub fn cargar<D: Durable>(d: &D, dir: &str) -> Resultado<Option<Vec<Value>>> {
    if !d.existe(dir) {
        return Ok(None);
    }
    match leer_vigente(d, dir, NOMBRE)? {
        None => Ok(None),
        Some((_, datos)) => serde_json::from_slice(&datos).map(Some).map_err(Error::formato),
    }
}

/// Quita del final lo que no se puede continuar: llamadas del asistente cuyos resultados no constan.
pub fn normalizar(mensajes: &mut Vec<Value>) {
    loop {
        let Some(ultimo_asistente) = mensajes.iter().rposition(|m| m["role"] == "assistant") else {
            return;
        };
        let llamadas = mensajes[ultimo_asistente]["tool_calls"].as_array().map_or(0, |l| l.len());
        let resultados = mensajes[ultimo_asistente + 1..].iter().filter(|m| m["role"] == "tool").count();
        if llamadas > resultados {
            mensajes.truncate(ultimo_asistente);
        } else {
            return;
        }
    }
}

/// ¿La conversación ya terminó (el último mensaje es una respuesta sin llamadas)?
pub fn terminada(mensajes: &[Value]) -> bool {
    mensajes.last().map_or(false, |m| m["role"] == "assistant" && m["tool_calls"].as_array().map_or(true, |l| l.is_empty()))
}
