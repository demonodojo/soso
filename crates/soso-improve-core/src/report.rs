//! Resumen de una campaña (T29): aceptación, intentos, tiempo y tokens por mejora aceptada, con los
//! fallos separados por **de quién son** (infraestructura, protocolo, programación).
//!
//! Reglas: el denominador es el número de tareas **fijadas** (no se quita ninguna por haber fallado ni
//! por ser de infraestructura); una medida que no se tomó no se rellena (si falta en una aceptada, el
//! promedio por aceptada es «desconocido», no un número menor); y el criterio de SI-4 se cumple sólo si
//! hay ≥ 5 aceptadas, **recuperación acreditada** y la base final en verde.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::{Error, Resultado};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cierre {
    Aceptada,
    Rechazada,
    Bloqueada,
    /// No llegó a medirse nada del candidato (el coordinador falló).
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FalloDe {
    /// El servidor del modelo, la red, el coordinador, el arnés.
    Infraestructura,
    /// El modelo no usó las herramientas como debía (no editó, llamadas mal formadas, se rindió).
    Protocolo,
    /// Editó y el validador lo rechazó: el código no resuelve la tarea.
    Programacion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registro {
    pub id: String,
    pub resultado: Cierre,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallo: Option<FalloDe>,
    pub intentos: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub herramientas: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    pub segundos: u64,
    /// Si se inyectó un fallo en esta tarea: qué se hizo y si la recuperación fue correcta.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inyeccion: Option<Inyeccion>,
    #[serde(default)]
    pub nota: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inyeccion {
    pub que: String,
    /// `true` sólo si, tras la inyección, la ejecución terminó sin duplicar efectos (o se bloqueó
    /// explícitamente, que el contrato de T27 también acepta).
    pub recuperada: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Resumen {
    pub total: usize,
    pub aceptadas: usize,
    pub rechazadas: usize,
    pub bloqueadas: usize,
    pub errores: usize,
    pub por_fallo: BTreeMap<FalloDe, usize>,
    pub segundos_total: u64,
    pub segundos_por_aceptada: Option<u64>,
    pub tokens_por_aceptada: Option<u64>,
    pub intentos_por_aceptada: Option<f64>,
    pub herramientas_por_aceptada: Option<u64>,
    /// Mensajes de lo que **no** se pudo calcular y por qué.
    pub sin_medir: Vec<String>,
    pub inyecciones: usize,
    pub recuperaciones_acreditadas: usize,
    pub base_final_verde: Option<bool>,
    pub minimo_aceptadas: usize,
    pub cumple_si4: bool,
    pub motivos_no_cumple: Vec<String>,
}

pub const MINIMO_SI4: usize = 5;

/// Calcula el resumen. `fijadas` son los ids de la campaña **tal como se fijaron antes de ejecutarla**:
/// falta alguno → error (no se cuenta una campaña a la que se le quitaron tareas), sobra o repite → error.
pub fn resumir(fijadas: &[String], registros: &[Registro], base_final_verde: Option<bool>) -> Resultado<Resumen> {
    for f in fijadas {
        let n = registros.iter().filter(|r| &r.id == f).count();
        if n == 0 {
            return Err(Error::uso(format!("falta la tarea fijada {f}: el denominador no se puede reducir")));
        }
        if n > 1 {
            return Err(Error::uso(format!("la tarea {f} aparece {n} veces")));
        }
    }
    if let Some(x) = registros.iter().find(|r| !fijadas.contains(&r.id)) {
        return Err(Error::uso(format!("{} no estaba entre las tareas fijadas", x.id)));
    }
    let cuenta = |q: Cierre| registros.iter().filter(|r| r.resultado == q).count();
    let acept: Vec<&Registro> = registros.iter().filter(|r| r.resultado == Cierre::Aceptada).collect();
    let mut por_fallo: BTreeMap<FalloDe, usize> = BTreeMap::new();
    for r in registros.iter().filter(|r| r.resultado != Cierre::Aceptada) {
        if let Some(f) = r.fallo {
            *por_fallo.entry(f).or_default() += 1;
        }
    }
    let mut sin_medir = Vec::new();
    for r in registros.iter().filter(|r| r.resultado != Cierre::Aceptada && r.fallo.is_none()) {
        sin_medir.push(format!("{}: no se clasificó el fallo", r.id));
    }
    let segundos_total: u64 = registros.iter().map(|r| r.segundos).sum();
    let n = acept.len() as u64;
    // Tiempo, tokens y herramientas se reparten sobre **todo** lo gastado (también lo que no aceptó):
    // una mejora aceptada cuesta lo suyo y lo que costaron los intentos que no salieron.
    let segundos_por_aceptada = if n > 0 { Some(segundos_total / n) } else { None };
    let suma = |f: &dyn Fn(&Registro) -> Option<u64>, nombre: &str, sin: &mut Vec<String>| -> Option<u64> {
        if n == 0 {
            return None;
        }
        let mut total = 0u64;
        for r in registros {
            match f(r) {
                Some(v) => total += v,
                None => {
                    sin.push(format!("{nombre}: no se midió en {}", r.id));
                    return None;
                }
            }
        }
        Some(total / n)
    };
    let tokens_por_aceptada = suma(&|r| r.tokens, "tokens", &mut sin_medir);
    let herramientas_por_aceptada = suma(&|r| r.herramientas, "herramientas", &mut sin_medir);
    let intentos_por_aceptada = if n > 0 { Some(registros.iter().map(|r| r.intentos as f64).sum::<f64>() / n as f64) } else { None };
    let inyecciones = registros.iter().filter(|r| r.inyeccion.is_some()).count();
    let recuperaciones_acreditadas = registros.iter().filter(|r| r.inyeccion.as_ref().map_or(false, |i| i.recuperada)).count();

    let mut motivos = Vec::new();
    if acept.len() < MINIMO_SI4 {
        motivos.push(format!("{} aceptadas de {}: se exigen al menos {MINIMO_SI4}", acept.len(), registros.len()));
    }
    if inyecciones == 0 {
        motivos.push("no se inyectó ningún fallo: la recuperación no está acreditada".to_string());
    } else if recuperaciones_acreditadas < inyecciones {
        motivos.push(format!("{} de {inyecciones} inyecciones se recuperaron", recuperaciones_acreditadas));
    }
    match base_final_verde {
        Some(true) => {}
        Some(false) => motivos.push("la base final no pasa los checks (regresión)".to_string()),
        None => motivos.push("la base final no se comprobó".to_string()),
    }
    Ok(Resumen {
        total: registros.len(),
        aceptadas: acept.len(),
        rechazadas: cuenta(Cierre::Rechazada),
        bloqueadas: cuenta(Cierre::Bloqueada),
        errores: cuenta(Cierre::Error),
        por_fallo,
        segundos_total,
        segundos_por_aceptada,
        tokens_por_aceptada,
        intentos_por_aceptada,
        herramientas_por_aceptada,
        sin_medir,
        inyecciones,
        recuperaciones_acreditadas,
        base_final_verde,
        minimo_aceptadas: MINIMO_SI4,
        cumple_si4: motivos.is_empty(),
        motivos_no_cumple: motivos,
    })
}

/// Informe legible (Markdown) para `docs/self-improvement/campana.md`.
pub fn markdown(r: &Resumen, registros: &[Registro]) -> String {
    let mut t = String::new();
    t.push_str(&format!("**Resultado:** {} aceptadas de {} (mínimo SI-4: {}) → **{}**\n\n", r.aceptadas, r.total, r.minimo_aceptadas, if r.cumple_si4 { "GO" } else { "NO-GO" }));
    for m in &r.motivos_no_cumple {
        t.push_str(&format!("- no cumple: {m}\n"));
    }
    t.push_str("\n| tarea | resultado | fallo de | intentos | herramientas | tokens | segundos | inyección |\n|---|---|---|---|---|---|---|---|\n");
    for x in registros {
        t.push_str(&format!(
            "| {} | {:?} | {} | {} | {} | {} | {} | {} |\n",
            x.id,
            x.resultado,
            x.fallo.map_or("-".to_string(), |f| format!("{f:?}")),
            x.intentos,
            x.herramientas.map_or("?".to_string(), |v| v.to_string()),
            x.tokens.map_or("?".to_string(), |v| v.to_string()),
            x.segundos,
            x.inyeccion.as_ref().map_or("-".to_string(), |i| format!("{} ({})", i.que, if i.recuperada { "recuperada" } else { "NO recuperada" })),
        ));
    }
    t.push_str(&format!(
        "\n**Por mejora aceptada:** {} s, {} tokens, {} herramientas, {} intentos (sobre todo lo gastado).\n",
        r.segundos_por_aceptada.map_or("?".to_string(), |v| v.to_string()),
        r.tokens_por_aceptada.map_or("?".to_string(), |v| v.to_string()),
        r.herramientas_por_aceptada.map_or("?".to_string(), |v| v.to_string()),
        r.intentos_por_aceptada.map_or("?".to_string(), |v| format!("{v:.2}")),
    ));
    t.push_str(&format!("\n**Fallos por responsable:** {:?}\n", r.por_fallo));
    for s in &r.sin_medir {
        t.push_str(&format!("- sin medir: {s}\n"));
    }
    t
}
