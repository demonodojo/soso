//! `report` — resumen de una campaña (T29). La cuenta es `soso_improve_core::report`.

use soso_improve_core::cli::Codigo;
use soso_improve_core::report::{self, Registro};
use soso_improve_core::Error;

use crate::Opciones;

pub fn reporte(op: &Opciones) -> Result<i32, Error> {
    let camp: serde_json::Value = serde_json::from_slice(&std::fs::read(op.exigido("campana-json")?).map_err(|e| Error::uso(format!("campaña: {e}")))?)
        .map_err(|e| Error::formato(format!("campaña ilegible: {e}")))?;
    let fijadas: Vec<String> = camp["tareas"]
        .as_array()
        .ok_or_else(|| Error::formato("la campaña no tiene `tareas`"))?
        .iter()
        .filter_map(|t| t["id"].as_str().map(|s| s.to_string()))
        .collect();
    let dir = op.exigido("registros")?;
    let mut registros: Vec<Registro> = Vec::new();
    for id in &fijadas {
        let f = format!("{dir}/{id}.json");
        match std::fs::read(&f) {
            Ok(d) => registros.push(serde_json::from_slice(&d).map_err(|e| Error::formato(format!("{f}: {e}")))?),
            Err(_) => return Err(Error::uso(format!("falta el registro de {id} ({f}): una tarea fijada no se puede omitir"))),
        }
    }
    let base_final = match op.uno("base-final") {
        Some("verde") => Some(true),
        Some("roja") => Some(false),
        _ => None,
    };
    let res = report::resumir(&fijadas, &registros, base_final)?;
    let md = report::markdown(&res, &registros);
    if let Some(out) = op.uno("out").filter(|o| !o.is_empty()) {
        std::fs::write(out, &md).map_err(|e| Error::entorno(format!("{out}: {e}")))?;
        let js = serde_json::to_string_pretty(&res).map_err(|e| Error::formato(e.to_string()))?;
        std::fs::write(format!("{out}.json"), js).map_err(|e| Error::entorno(format!("{out}.json: {e}")))?;
    }
    print!("{md}");
    Ok(if res.cumple_si4 { Codigo::Exito.como_i32() } else { Codigo::Verificacion.como_i32() })
}
