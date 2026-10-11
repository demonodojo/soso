//! `soso-improve agente`: el agente nativo (T35, D10) sobre un checkout del host.
//!
//! La lógica es `soso_improve_core::agente`; aquí sólo se cablean el sistema de archivos, los
//! procesos, el reloj y el conector TCP del host.


use serde_json::Value;
use soso_improve_core::agente::http::ModeloHttp;
use soso_improve_core::agente::{correr, Config, Fin, Politica, SISTEMA};
use soso_improve_core::cli::Codigo;
use soso_improve_core::transporte::Destino;
use soso_improve_core::{Error, Resultado};

use crate::sistema::{ConectorHost, Host, RelojHost};
use crate::Opciones;

pub fn agente(op: &Opciones) -> Result<i32, Error> {
    let repo = std::fs::canonicalize(op.exigido("repo")?)
        .map_err(|e| Error::uso(format!("--repo: {e}")))?
        .to_string_lossy()
        .into_owned();
    let tarea = op.exigido("tarea")?;
    let tarea = match tarea.strip_prefix('@') {
        Some(f) => std::fs::read_to_string(f).map_err(|e| Error::uso(format!("--tarea {f}: {e}")))?,
        None => tarea.to_string(),
    };
    let sistema = match op.uno("sistema").filter(|s| !s.is_empty()) {
        Some(f) => std::fs::read_to_string(f).map_err(|e| Error::uso(format!("--sistema {f}: {e}")))?,
        None => SISTEMA.to_string(),
    };
    let numero = |c: &str, defecto: u64| -> Resultado<u64> {
        match op.uno(c).filter(|v| !v.is_empty()) {
            Some(v) => v.parse().map_err(|_| Error::uso(format!("--{c} no es un número: {v}"))),
            None => Ok(defecto),
        }
    };
    let puerto = numero("puerto", 17299)? as u16;
    let mut cfg = Config::nueva(op.uno("modelo").filter(|s| !s.is_empty()).unwrap_or("soso-coder"), &sistema);
    cfg.max_turnos = numero("max-turnos", cfg.max_turnos as u64)? as usize;
    cfg.max_tokens = numero("max-tokens", cfg.max_tokens as u64)? as u32;
    cfg.plazo_turno_ms = numero("plazo-ms", cfg.plazo_turno_ms)?;

    let mut pol = Politica::nueva(&repo);
    if let Some(h) = op.uno("cargo-home").filter(|s| !s.is_empty()) {
        pol.entorno.push(("CARGO_HOME".into(), h.to_string()));
    }
    let reloj = RelojHost::default();
    let conector = ConectorHost;
    let mut modelo = ModeloHttp { conector: &conector, reloj: &reloj, destino: Destino::local(puerto), plazo_conexion_ms: 10_000,
        token: op.uno("token").filter(|t| !t.is_empty()).map(|t| t.to_string()).or_else(|| std::env::var("SOSO_LLM_API_KEY").ok()) };
    let sesion = op.uno("sesion").filter(|s| !s.is_empty()).map(|s| s.to_string());
    let mut host = Host;
    crate::digo!("agente: modelo {} en 127.0.0.1:{puerto}, checkout {repo}", cfg.modelo);
    let informe = correr(&mut host, &mut modelo, &reloj, &pol, &cfg, &tarea, &mut |mensajes: &[Value]| {
        // Una vuelta, una línea: lo que pidió el modelo y qué pasó.
        if let Some(m) = mensajes.iter().rev().find(|m| m["role"] == "assistant") {
            if let Some(ll) = m["tool_calls"].as_array() {
                for l in ll {
                    let a = l.pointer("/function/arguments").and_then(|v| v.as_str()).unwrap_or("");
                    crate::digo!("  → {} {}", l.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("?"), a.chars().take(160).collect::<String>());
                }
            }
        }
        if let Some(s) = &sesion {
            let _ = std::fs::write(s, serde_json::to_vec_pretty(mensajes).unwrap_or_default());
        }
    });
    crate::digo!("agente: {:?} tras {} turno(s) y {} llamada(s)", informe.fin, informe.turnos, informe.llamadas);
    if let Some(e) = &informe.error {
        crate::aviso!("agente: {e}");
    }
    if !informe.respuesta_final.is_empty() {
        crate::digo!("--- informe del agente ---\n{}", informe.respuesta_final);
    }
    Ok(if informe.fin == Fin::Terminado { Codigo::Exito.como_i32() } else { Codigo::Error.como_i32() })
}
