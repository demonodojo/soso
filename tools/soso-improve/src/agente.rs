//! `soso-improve agente`: el agente nativo (T35, D10) sobre un checkout del host.
//!
//! La lógica es `soso_improve_core::agente`; aquí sólo se cablean el sistema de archivos, los
//! procesos, el reloj y el conector TCP del host.


use serde_json::Value;
use soso_improve_core::agente::http::ModeloHttp;
use soso_improve_core::agente::{correr, correr_desde, sesion, Config, Fin, Politica, SISTEMA};
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
    let mut al_turno = |h: &mut Host, mensajes: &[Value]| {
        // Una vuelta, una línea: lo que pidió el modelo y qué pasó.
        if let Some(m) = mensajes.iter().rev().find(|m| m["role"] == "assistant") {
            if let Some(ll) = m["tool_calls"].as_array() {
                for l in ll {
                    let a = l.pointer("/function/arguments").and_then(|v| v.as_str()).unwrap_or("");
                    crate::digo!("  → {} {}", l.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("?"), a.chars().take(160).collect::<String>());
                }
            }
        }
        if let Some(dir) = &sesion {
            if let Err(e) = sesion::guardar(h, dir, mensajes) {
                crate::aviso!("agente: no se pudo guardar la sesión: {e}");
            }
        }
    };
    let previa = match (&sesion, op.bandera("reanudar")) {
        (Some(dir), true) => sesion::cargar(&host, dir)?,
        (None, true) => return Err(Error::uso("--reanudar necesita --sesion <directorio>")),
        _ => None,
    };
    let informe = match previa {
        Some(m) => {
            crate::digo!("agente: reanudando sesión ({} mensaje(s))", m.len());
            correr_desde(&mut host, &mut modelo, &reloj, &pol, &cfg, m, &mut al_turno)
        }
        None => correr(&mut host, &mut modelo, &reloj, &pol, &cfg, &tarea, &mut al_turno),
    };
    crate::digo!("agente: {:?} tras {} turno(s) y {} llamada(s)", informe.fin, informe.turnos, informe.llamadas);
    if let Some(e) = &informe.error {
        crate::aviso!("agente: {e}");
    }
    if !informe.respuesta_final.is_empty() {
        crate::digo!("--- informe del agente ---\n{}", informe.respuesta_final);
    }
    Ok(if informe.fin == Fin::Terminado { Codigo::Exito.como_i32() } else { Codigo::Error.como_i32() })
}

/// `agente-prueba`: las herramientas reales sobre un directorio temporal del host.
pub fn prueba(op: &Opciones) -> Result<i32, Error> {
    if let Some(raiz) = op.uno("sembrar").filter(|r| !r.is_empty()) {
        soso_improve_core::agente::prueba::sembrar(&mut Host, raiz.trim_end_matches('/'))?;
        crate::digo!("sembrado: {raiz}");
        return Ok(0);
    }
    let tmp = crate::sistema::Temporal::nuevo("agente-prueba")?;
    let raiz = tmp.0.to_string_lossy().into_owned();
    let mut host = Host;
    let r = soso_improve_core::agente::prueba::autoprueba(&mut host, &RelojHost::default(), &raiz, &["true"])?;
    let mal = r.iter().filter(|(_, ok, _)| !ok).count();
    for (n, ok, d) in &r {
        crate::digo!("{n}: {}", if *ok { "ok".to_string() } else { format!("FALLA {d:?}") });
    }
    crate::digo!("agente-prueba: {mal} caso(s) mal");
    Ok(if mal == 0 { Codigo::Exito.como_i32() } else { Codigo::Verificacion.como_i32() })
}

/// `tarea ejecutar` (T25): un intento del agente nativo sobre la copia, con los límites de la tarea.
pub fn ejecutar_tarea(op: &Opciones) -> Result<i32, Error> {
    let estado = op.exigido("estado")?;
    let run = op.exigido("run")?;
    let copia = std::fs::canonicalize(op.exigido("copia")?)
        .map_err(|e| Error::uso(format!("--copia: {e}")))?
        .to_string_lossy()
        .into_owned();
    let numero = |c: &str, d: u64| -> Resultado<u64> {
        match op.uno(c).filter(|v| !v.is_empty()) {
            Some(v) => v.parse().map_err(|_| Error::uso(format!("--{c} no es un número: {v}"))),
            None => Ok(d),
        }
    };
    let puerto = numero("puerto", 17299)? as u16;
    let mut cfg = Config::nueva(op.uno("modelo").filter(|s| !s.is_empty()).unwrap_or("soso-coder"), SISTEMA);
    cfg.max_turnos = numero("max-turnos", cfg.max_turnos as u64)? as usize;
    cfg.max_tokens = numero("max-tokens", cfg.max_tokens as u64)? as u32;
    let mut pol = Politica::nueva(&copia);
    if let Some(h) = op.uno("cargo-home").filter(|s| !s.is_empty()) {
        pol.entorno.push(("CARGO_HOME".into(), h.to_string()));
    }
    let reloj = RelojHost::default();
    let conector = ConectorHost;
    let mut modelo = ModeloHttp {
        conector: &conector,
        reloj: &reloj,
        destino: Destino::local(puerto),
        plazo_conexion_ms: 10_000,
        token: op.uno("token").filter(|t| !t.is_empty()).map(|t| t.to_string()).or_else(|| std::env::var("SOSO_LLM_API_KEY").ok()),
    };
    let mut host = Host;
    let inf = soso_improve_core::runner::ejecutar_intento(&mut host, &mut modelo, &reloj, estado, run, &copia, &cfg, &pol)?;
    crate::digo!(
        "tarea: intento {:?}: {} paso(s), {} herramienta(s), {} token(s) de entrada y {} de salida; límites {}",
        inf.fin,
        inf.cuenta.pasos,
        inf.cuenta.herramientas,
        inf.cuenta.tokens_entrada,
        inf.cuenta.tokens_salida,
        if inf.limites_en_vivo { "en vivo" } else { "a posteriori" }
    );
    if !inf.detalle.is_empty() {
        crate::aviso!("tarea: {}", inf.detalle);
    }
    Ok(if inf.fin == soso_improve_core::runner::Fin::Completo { Codigo::Exito.como_i32() } else { Codigo::Error.como_i32() })
}
