//! Equipo de tres roles sobre el mismo modelo (`Config::multiagente`): **planificador** (sólo lee y entrega un plan), **programador**
//! (el agente de siempre, con el plan en el encargo) y **revisor independiente** (petición aparte con `TASK.md` y el diff acumulado).
//! Si la comprobación del programador falla o el revisor señala fallos, el planificador replantea la corrección con el diff y los
//! problemas, y el programador vuelve a empezar con una conversación **nueva** (sin arrastrar el enredo de la anterior), hasta que
//! todo pase o se agoten los ciclos.
//!
//! Por qué: en las campañas 33–35 el agente se enredaba parcheando sus propios parches dentro de una conversación larga, y revisarse a
//! sí mismo no servía (se autoconfirmaba). Aquí cada rol trabaja con un contexto limpio y un encargo pequeño.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{json, Value};

use crate::entorno::Entorno;
use crate::tiempo::Reloj;

use super::herramientas::Politica;
use super::{correr_con_originales, correr_desde, revision, Config, Fin, Informe, Modelo};

/// Turnos máximos del planificador por ciclo: sólo lee y escribe un plan.
const TURNOS_PLANIFICADOR: usize = 15;

pub const SISTEMA_PLANIFICADOR: &str = "Eres el PLANIFICADOR de un equipo de tres: tú planificas, un programador edita el código y un revisor busca fallos. Tú NO escribes código ni editas archivos: sólo tienes read, grep y glob. Trabajas dentro del checkout actual.\n\n\
1. Lee TASK.md (con `read`) y el código que haga falta (con `read` y `grep`).\n\
2. Entrega un PLAN breve y concreto para el programador, como texto (sin llamadas a herramientas), con estas secciones:\n\
   - **Dónde:** archivo(s) y función(es) a cambiar, con rutas y líneas.\n\
   - **Casos:** cada caso límite que exige el enunciado (valores raros, empates, vacíos, infinitos, errores), uno por línea: entrada → resultado esperado.\n\
   - **Pasos:** pocos, ordenados y mínimos; cambios pequeños y bloques completos con sus llaves.\n\
   - **Comprobar:** las órdenes `cargo test` que debe ejecutar al acabar.\n\
3. Máximo 40 líneas. Si recibes un estado actual y problemas a resolver, el plan debe ser SOLO la corrección: qué cambiar sobre lo que ya está hecho, sin rehacerlo todo.";

/// Resultado de la última comprobación del programador: `(pasó, salida)` según el último `exit code:` que vio, o `None` si no ejecutó ninguna.
pub fn ultima_comprobacion(mensajes: &[Value]) -> Option<(bool, String)> {
    for m in mensajes.iter().rev() {
        if m["role"] != "tool" {
            continue;
        }
        let c = m["content"].as_str().unwrap_or("");
        let Some(pos) = c.find("exit code: ") else { continue };
        let resto = &c[pos + "exit code: ".len()..];
        let codigo: i64 = resto.split(|ch: char| !ch.is_ascii_digit() && ch != '-').next().and_then(|n| n.parse().ok())?;
        let salida: String = if c.chars().count() > 1800 { c.chars().skip(c.chars().count() - 1800).collect() } else { c.to_string() };
        return Some((codigo == 0, salida));
    }
    None
}

fn sumar(t: &mut Informe, i: &Informe) {
    t.turnos += i.turnos;
    t.llamadas += i.llamadas;
    t.empujones += i.empujones;
    t.formato += i.formato;
    t.repetidas += i.repetidas;
    t.recortadas += i.recortadas;
    t.bloqueadas += i.bloqueadas;
}

pub fn correr<E: Entorno, M: Modelo, R: Reloj>(
    e: &mut E,
    modelo: &mut M,
    reloj: &R,
    pol: &Politica,
    cfg: &Config,
    tarea: &str,
    al_turno: &mut dyn FnMut(&mut E, &[Value]),
) -> Informe {
    let t0 = reloj.ahora_ms();
    let mut total = Informe { fin: Fin::SinTurnos, turnos: 0, llamadas: 0, respuesta_final: String::new(), error: None, empujones: 0, formato: 0, repetidas: 0, recortadas: 0, sin_editar: 0, sin_pruebas: 0, sin_ejecutar: 0, comprobaciones_rojas: 0, alertas_panico: 0, pocas_pruebas: 0, redirecciones: 0, podadas: Vec::new(), bloqueadas: 0, revisiones: 0, ciclos: 0, mensajes: Vec::new() };
    let mut originales: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut problemas: Option<String> = None;
    let mut ultimo: Option<Informe> = None;
    for ciclo in 1..=cfg.ciclos_max.max(1) {
        let restante_turnos = cfg.max_turnos.saturating_sub(total.turnos);
        let restante_llamadas = cfg.max_llamadas.map(|m| m.saturating_sub(total.llamadas));
        if restante_turnos < 5 || restante_llamadas == Some(0) || reloj.ahora_ms().saturating_sub(t0) >= cfg.plazo_total_ms {
            break;
        }
        total.ciclos = ciclo;
        // 1. Planificador: sólo lectura, conversación nueva.
        let entrada = match &problemas {
            None => tarea.to_string(),
            Some(p) => format!(
                "{tarea}\n\n## Estado actual (diff de lo que el programador ya hizo)\n{}\n\n## Problemas a resolver\n{p}",
                revision::contenido(e, &pol.raiz, &originales)
            ),
        };
        let mut cp = cfg.clone();
        cp.multiagente = false;
        cp.solo_lectura = true;
        cp.revision = false;
        cp.revisor = false;
        cp.exigir_pruebas = false;
        cp.avisos_sin_editar = 0;
        cp.empujones = 0;
        cp.sistema = SISTEMA_PLANIFICADOR.to_string();
        cp.max_turnos = TURNOS_PLANIFICADOR.min(restante_turnos / 3).max(3);
        cp.max_llamadas = restante_llamadas.map(|m| m.min(30));
        let msgs = alloc::vec![json!({"role": "system", "content": cp.sistema}), json!({"role": "user", "content": entrada})];
        let ip = correr_desde(e, modelo, reloj, pol, &cp, msgs, &mut |_, _| {});
        sumar(&mut total, &ip);
        if ip.fin == Fin::ErrorModelo {
            total.fin = Fin::ErrorModelo;
            total.error = ip.error.clone();
            break;
        }
        let plan = if ip.respuesta_final.trim().is_empty() { "(el planificador no entregó plan: sigue TASK.md)".to_string() } else { ip.respuesta_final.trim().to_string() };
        // 2. Programador: conversación nueva con el plan en el encargo.
        let restante_turnos = cfg.max_turnos.saturating_sub(total.turnos);
        let restante_llamadas = cfg.max_llamadas.map(|m| m.saturating_sub(total.llamadas));
        let mut cg = cfg.clone();
        cg.multiagente = false;
        cg.revision = false;
        cg.revisor = false;
        cg.max_turnos = restante_turnos.max(1);
        cg.max_llamadas = restante_llamadas;
        let nota_ciclo = if ciclo > 1 {
            "\n\nESTE ES UN CICLO DE CORRECCIÓN: los archivos ya contienen cambios tuyos de antes (no los repitas ni los deshagas sin motivo). Lee el tramo actual antes de editar y corrige sólo lo que dice el plan."
        } else {
            ""
        };
        let encargo = format!("{tarea}\n\n## Plan del planificador (es una guía: si algo no cuadra con el código real, adáptalo)\n{plan}{nota_ciclo}");
        let msgs = alloc::vec![json!({"role": "system", "content": cg.sistema}), json!({"role": "user", "content": encargo})];
        let ig = correr_con_originales(e, modelo, reloj, pol, &cg, msgs, &mut originales, al_turno);
        sumar(&mut total, &ig);
        let fin_prog = ig.fin;
        let msgs_prog = ig.mensajes.clone();
        let final_prog = ig.respuesta_final.clone();
        let err_prog = ig.error.clone();
        ultimo = Some(ig);
        if fin_prog != Fin::Terminado {
            total.fin = fin_prog;
            total.error = err_prog;
            total.respuesta_final = final_prog;
            total.mensajes = msgs_prog;
            return total;
        }
        total.respuesta_final = final_prog;
        total.mensajes = msgs_prog.clone();
        // 3. Comprobación objetiva: si la última que ejecutó falla, no hace falta revisor.
        if let Some((false, salida)) = ultima_comprobacion(&msgs_prog) {
            problemas = Some(format!("La última comprobación del programador FALLA:\n{salida}"));
            continue;
        }
        // 4. Revisor independiente.
        if originales.is_empty() {
            break; // no editó nada: no hay nada que revisar
        }
        let base = revision::contenido(e, &pol.raiz, &originales);
        match modelo.completar(&revision::peticion_revisor(&cfg.modelo, &base, cfg.max_tokens), cfg.plazo_turno_ms.min(cfg.plazo_total_ms)) {
            Ok(c) => match revision::critica(&c) {
                None => break,
                Some(crit) => problemas = Some(format!("Un revisor independiente, que sólo ha visto los requisitos de TASK.md y el diff, señala:\n{crit}")),
            },
            Err(_) => break,
        }
    }
    // Se acabó el equipo (todo pasa, o sin ciclos/presupuesto): decide el validador sobre lo que haya en disco.
    if total.fin == Fin::SinTurnos {
        total.fin = if ultimo.is_some() { Fin::Terminado } else { Fin::SinTurnos };
    }
    total
}
