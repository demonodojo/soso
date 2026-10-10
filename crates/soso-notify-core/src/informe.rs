//! Montaje del cuerpo del informe a partir de colas de log.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::issue::CUERPO_GITHUB_MAX;

/// Presupuesto de bytes de cola por flujo (antes de UTF-8 y cabeceras de sección).
pub const PRESUPUESTO_KERNEL: usize = 40 * 1024;
pub const PRESUPUESTO_APPS: usize = 12 * 1024;
pub const PRESUPUESTO_OTA: usize = 8 * 1024;

pub fn presupuesto_flujo(nombre: &str) -> usize {
    match nombre {
        "kernel.log" => PRESUPUESTO_KERNEL,
        "aplicaciones.log" => PRESUPUESTO_APPS,
        "actualizaciones.log" => PRESUPUESTO_OTA,
        _ => 8 * 1024,
    }
}

pub struct FlujoEntrada<'a> {
    pub nombre: &'a str,
    pub activo: &'a [u8],
    pub rotado: Option<&'a [u8]>,
}

pub struct InformeEntrada<'a> {
    pub mensaje: &'a str,
    pub release: Option<&'a str>,
    pub hw: Option<&'a str>,
    pub flujos: &'a [FlujoEntrada<'a>],
}

/// Cola al final de `data` con tope `max`; devuelve (trozo, bytes_omitidos_al_inicio).
pub fn cola(data: &[u8], max: usize) -> (&[u8], usize) {
    if data.len() <= max {
        (data, 0)
    } else {
        let omit = data.len() - max;
        (&data[omit..], omit)
    }
}

/// Une activo (reciente) y, si cabe, `.1` (más antiguo dentro del presupuesto).
pub fn cola_flujo(activo: &[u8], rotado: Option<&[u8]>, presupuesto: usize) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    let mut omitidos = 0usize;

    if activo.len() >= presupuesto {
        let (tail, omit) = cola(activo, presupuesto);
        out.extend_from_slice(tail);
        omitidos += omit;
        if let Some(r) = rotado {
            omitidos += r.len();
        }
        return (out, omitidos);
    }

    out.extend_from_slice(activo);
    let restante = presupuesto - activo.len();
    if restante > 0 {
        if let Some(r) = rotado {
            let (tail, omit) = cola(r, restante);
            if !tail.is_empty() {
                if !out.is_empty() {
                    out.push(b'\n');
                    out.extend_from_slice(b"--- (continua desde .1) ---\n");
                }
                out.extend_from_slice(tail);
            }
            omitidos += omit;
            if omit == 0 && r.len() > tail.len() {
                omitidos += r.len() - tail.len();
            }
        }
    }
    (out, omitidos)
}

fn bytes_a_texto(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn seccion_flujo(nombre: &str, activo: &[u8], rotado: Option<&[u8]>) -> String {
    let presupuesto = presupuesto_flujo(nombre);
    let (bytes, omitidos) = cola_flujo(activo, rotado, presupuesto);
    let mut s = format!("### /var/log/{nombre}\n\n");
    if omitidos > 0 {
        s.push_str(&format!("(se omitieron {omitidos} bytes anteriores)\n\n"));
    }
    s.push_str(&bytes_a_texto(&bytes));
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Cuerpo markdown del issue (sin recorte global todavía).
pub fn construir_cuerpo(ent: InformeEntrada<'_>) -> String {
    let mut out = String::new();
    if !ent.mensaje.trim().is_empty() {
        out.push_str("## Mensaje\n\n");
        out.push_str(ent.mensaje.trim());
        out.push_str("\n\n");
    }
    out.push_str("## Sistema\n\n```\n");
    if let Some(rel) = ent.release {
        out.push_str(rel.trim());
        out.push('\n');
    } else {
        out.push_str("(sin /etc/soso-release)\n");
    }
    out.push_str("```\n\n");
    if let Some(hw) = ent.hw {
        if !hw.trim().is_empty() {
            out.push_str("## Hardware ( /etc/soso-hw )\n\n```\n");
            out.push_str(hw.trim());
            out.push_str("\n```\n\n");
        }
    }
    out.push_str("## Logs\n\n");
    for f in ent.flujos {
        out.push_str(&seccion_flujo(f.nombre, f.activo, f.rotado));
        out.push('\n');
    }
    acotar_cuerpo(out)
}

fn acotar_cuerpo(mut body: String) -> String {
    if body.len() <= CUERPO_GITHUB_MAX {
        return body;
    }
    body.truncate(CUERPO_GITHUB_MAX);
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str("\n… (informe recortado al límite de GitHub)\n");
    if body.len() > CUERPO_GITHUB_MAX {
        body.truncate(CUERPO_GITHUB_MAX);
    }
    body
}
