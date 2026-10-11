//! Reanudar tras una caída sin repetir efectos (T27).
//!
//! Una ejecución es una cadena de operaciones con efectos: **lanzar** al agente sobre la copia,
//! **exportar** su paquete y **validar** (que, si acepta, **promueve** la base). Antes de cada una se
//! escribe en un diario durable su *intención* con una clave estable (`run/op/ronda`) y después su
//! *resultado*. Tras una caída, `avanzar` no se fía de la memoria ni de un PID: lee el diario y
//! **reconcilia con la evidencia** (la copia, el hash del paquete, la referencia de base):
//!
//! - resultado escrito → la operación ya está hecha, no se repite;
//! - intención sin resultado en **validar** → si la referencia de base ya lleva el hash de este
//!   paquete, la promoción ocurrió: se reconoce y no se vuelve a publicar; si no, no hubo efecto y se
//!   repite (validar sólo promueve al final);
//! - intención sin resultado en **lanzar** → si la copia no tiene cambios, no hubo efecto y se
//!   relanza; si los tiene, **no se puede saber** qué hizo el agente y se bloquea con la comprobación
//!   necesaria (inspeccionar la copia) en vez de duplicar. `adoptar_copia` resuelve el bloqueo;
//! - el presupuesto de intentos vive en el estado durable: reanudar nunca lo reinicia.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::captura::Manifiesto;
use crate::durable::{self, Durable};
use crate::entorno::Entorno;
use crate::state::{self, Autoridad, Estado, Medida, TaskSpec};
use crate::tiempo::Reloj;
use crate::validate::{self, Exigencias, Sitios, Veredicto};
use crate::workspace;
use crate::{sha256_hex, Error, Resultado};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Lanzar,
    Exportar,
    Validar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fase {
    Intencion,
    Resultado,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entrada {
    pub clave: String,
    pub op: Op,
    pub fase: Fase,
    pub detalle: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diario {
    pub entradas: Vec<Entrada>,
}

impl Diario {
    fn tiene(&self, clave: &str, fase: Fase) -> bool {
        self.entradas.iter().any(|e| e.clave == clave && e.fase == fase)
    }
    fn detalle(&self, clave: &str, fase: Fase) -> Option<&str> {
        self.entradas.iter().find(|e| e.clave == clave && e.fase == fase).map(|e| e.detalle.as_str())
    }
    /// Rondas ya cerradas (un resultado de validar cierra una ronda).
    pub fn rondas_cerradas(&self) -> usize {
        self.entradas.iter().filter(|e| e.op == Op::Validar && e.fase == Fase::Resultado).count()
    }
}

fn nombre_diario(run: &str) -> String {
    format!("diario-{run}")
}

pub fn cargar_diario<D: crate::entorno::Archivos>(d: &D, dir: &str, run: &str) -> Resultado<Diario> {
    match durable::leer_vigente(d, dir, &nombre_diario(run))? {
        None => Ok(Diario::default()),
        Some((_, datos)) => serde_json::from_slice(&datos).map_err(|e| Error::formato(format!("diario de {run} ilegible: {e}"))),
    }
}

fn anotar<D: Durable>(d: &mut D, dir: &str, run: &str, diario: &mut Diario, e: Entrada) -> Resultado<()> {
    diario.entradas.push(e);
    let datos = serde_json::to_vec(&*diario).map_err(Error::formato)?;
    let actual = durable::leer_vigente(&*d, dir, &nombre_diario(run))?.map(|(_, v)| sha256_hex(&v));
    let g = durable::publicar(d, dir, &nombre_diario(run), &datos, actual.as_deref())?;
    let _ = durable::purgar(d, dir, &nombre_diario(run), g.saturating_sub(2));
    Ok(())
}

/// Todo lo que `avanzar` necesita saber de dónde está cada cosa.
pub struct Contexto<'a> {
    pub dir_estado: &'a str,
    pub run_id: &'a str,
    pub captura: &'a str,
    pub copia: &'a str,
    pub sitios: Sitios<'a>,
    pub spec: &'a TaskSpec,
    pub exigencias: &'a Exigencias,
    /// Resuelve un bloqueo de «lanzar»: el operador inspeccionó la copia y la adopta como el resultado.
    pub adoptar_copia: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Desenlace {
    Validado(Veredicto),
    /// No se puede decidir sin una comprobación: se deja constancia y no se toca nada más.
    Bloqueado { motivo: String, comprobacion: String },
}

#[derive(Debug, Clone)]
pub struct Reanudado {
    pub desenlace: Desenlace,
    /// Operaciones que **no** se repitieron porque su resultado ya constaba o se reconoció.
    pub reutilizadas: Vec<String>,
    pub ejecutadas: Vec<String>,
}

/// Lleva la ejecución hasta el final desde donde esté. Se puede llamar tantas veces como haga falta.
///
/// `lanzar` es el efecto caro y no idempotente (en producción, `runner::ejecutar_intento`).
pub fn avanzar<E: Entorno + Durable, R: Reloj>(
    e: &mut E,
    reloj: &R,
    cx: &Contexto<'_>,
    lanzar: &mut dyn FnMut(&mut E) -> Resultado<()>,
) -> Resultado<Reanudado> {
    // Una ejecución ya juzgada es terminal: llamar otra vez no abre otra ronda.
    if let Some((m, _)) = state::cargar(&*e, cx.dir_estado, cx.run_id)? {
        if m.estado.terminal() {
            let v = if m.estado == Estado::Aceptada { Veredicto::Aceptado } else { Veredicto::Rechazado };
            return Ok(Reanudado { desenlace: Desenlace::Validado(v), reutilizadas: alloc::vec![format!("{} (terminal)", cx.run_id)], ejecutadas: Vec::new() });
        }
    }
    let mut d = cargar_diario(&*e, cx.dir_estado, cx.run_id)?;
    let ronda = d.rondas_cerradas();
    let clave = |op: &str| format!("{}/{op}/{ronda}", cx.run_id);
    let mut hechas: Vec<String> = Vec::new();
    let mut ejec: Vec<String> = Vec::new();
    let bloquear = |e: &mut E, motivo: String, comprobacion: String, hechas, ejec| -> Resultado<Reanudado> {
        if let Some((mut m, h)) = state::cargar(&*e, cx.dir_estado, cx.run_id)? {
            if m.estado != Estado::Bloqueada && !m.estado.terminal() {
                m.transicion(Estado::Bloqueada, Autoridad::Coordinador)?;
                state::guardar(e, cx.dir_estado, &m, Some(&h))?;
            }
        }
        Ok(Reanudado { desenlace: Desenlace::Bloqueado { motivo, comprobacion }, reutilizadas: hechas, ejecutadas: ejec })
    };

    let man: Manifiesto = crate::captura::leer_manifiesto(&*e, cx.captura)?;
    let copia = workspace::leer_marca(&*e, cx.copia)?;
    let exportar_ahora = |e: &E| workspace::exportar(e, &copia, &man, &man.politica);

    // ---------------------------------------------------------------- validar ya a medias
    if d.tiene(&clave("validar"), Fase::Intencion) && !d.tiene(&clave("validar"), Fase::Resultado) {
        let hash = d.detalle(&clave("exportar"), Fase::Resultado).unwrap_or("").to_string();
        if let Some((b, _)) = validate::base_vigente(&*e, cx.sitios.referencias, cx.sitios.campana)? {
            if !hash.is_empty() && b.paquete == hash {
                // La promoción ocurrió antes de que se escribiera el resultado: se reconoce, no se repite.
                llevar_a(e, cx.dir_estado, cx.run_id, Estado::Verificando)?;
                validate::marcar_veredicto(e, cx.dir_estado, cx.run_id, &Veredicto::Aceptado)?;
                anotar(e, cx.dir_estado, cx.run_id, &mut d, Entrada { clave: clave("validar"), op: Op::Validar, fase: Fase::Resultado, detalle: format!("Aceptado promovido-reconocido {hash}") })?;
                hechas.push(clave("validar"));
                return Ok(Reanudado { desenlace: Desenlace::Validado(Veredicto::Aceptado), reutilizadas: hechas, ejecutadas: ejec });
            }
        }
    }
    if d.tiene(&clave("validar"), Fase::Resultado) {
        let det = d.detalle(&clave("validar"), Fase::Resultado).unwrap_or("");
        let v = if det.starts_with("Aceptado") { Veredicto::Aceptado } else if det.starts_with("Rechazado") { Veredicto::Rechazado } else { Veredicto::Bloqueado };
        hechas.push(clave("validar"));
        return Ok(Reanudado { desenlace: Desenlace::Validado(v), reutilizadas: hechas, ejecutadas: ejec });
    }

    // ---------------------------------------------------------------- lanzar
    if d.tiene(&clave("lanzar"), Fase::Resultado) {
        hechas.push(clave("lanzar"));
    } else {
        if d.tiene(&clave("lanzar"), Fase::Intencion) {
            let cambios = exportar_ahora(&*e)?;
            if !cambios.vacia() && !cx.adoptar_copia {
                return bloquear(
                    e,
                    format!("el agente se cayó a mitad de {}: la copia ya tiene {} cambio(s) y no se sabe qué más iba a hacer", clave("lanzar"), cambios.paquete.operaciones.len()),
                    format!("inspeccionar la copia {} (`tarea exportar`) y, si es aceptable, reanudar con adoptar_copia", cx.copia),
                    hechas,
                    ejec,
                );
            }
            // Un intento que quedó abierto se cierra como bloqueado: consume presupuesto, no lo reinicia.
            if let Some((mut m, h)) = state::cargar(&*e, cx.dir_estado, cx.run_id)? {
                if m.intento_abierto().is_some() {
                    let ahora = reloj.ahora_ms();
                    let desc = if cambios.vacia() { "caída sin efectos en la copia" } else { "adoptado tras la caída" };
                    m.cerrar_intento(Estado::Bloqueada, Medida::desconocida(desc), Medida::desconocida(desc), ahora.max(m.intentos.last().map_or(0, |a| a.inicio_ms)))?;
                    state::guardar(e, cx.dir_estado, &m, Some(&h))?;
                }
            }
            if !cambios.vacia() {
                anotar(e, cx.dir_estado, cx.run_id, &mut d, Entrada { clave: clave("lanzar"), op: Op::Lanzar, fase: Fase::Resultado, detalle: "adoptado".to_string() })?;
                hechas.push(format!("{} (adoptado)", clave("lanzar")));
            }
        } else {
            anotar(e, cx.dir_estado, cx.run_id, &mut d, Entrada { clave: clave("lanzar"), op: Op::Lanzar, fase: Fase::Intencion, detalle: String::new() })?;
        }
        if !d.tiene(&clave("lanzar"), Fase::Resultado) {
            llevar_a(e, cx.dir_estado, cx.run_id, Estado::Pendiente)?;
            lanzar(e)?;
            ejec.push(clave("lanzar"));
            anotar(e, cx.dir_estado, cx.run_id, &mut d, Entrada { clave: clave("lanzar"), op: Op::Lanzar, fase: Fase::Resultado, detalle: String::new() })?;
        }
    }

    // ---------------------------------------------------------------- exportar
    let exp = exportar_ahora(&*e)?;
    if exp.vacia() {
        return bloquear(e, "el candidato no cambió nada".to_string(), "revisar el registro del intento".to_string(), hechas, ejec);
    }
    let hash = validate::hash_paquete(&exp.paquete)?;
    if let Some(previo) = d.detalle(&clave("exportar"), Fase::Resultado) {
        if previo != hash {
            return bloquear(e, format!("la copia cambió desde que se exportó el paquete ({previo} → {hash})"), "decidir qué paquete es el candidato".to_string(), hechas, ejec);
        }
        hechas.push(clave("exportar"));
    } else {
        anotar(e, cx.dir_estado, cx.run_id, &mut d, Entrada { clave: clave("exportar"), op: Op::Exportar, fase: Fase::Intencion, detalle: String::new() })?;
        validate::empaquetar_objetos(e, cx.copia, &exp.paquete, cx.sitios.objetos)?;
        anotar(e, cx.dir_estado, cx.run_id, &mut d, Entrada { clave: clave("exportar"), op: Op::Exportar, fase: Fase::Resultado, detalle: hash.clone() })?;
        ejec.push(clave("exportar"));
    }

    // ---------------------------------------------------------------- validar (y promover)
    if !d.tiene(&clave("validar"), Fase::Intencion) {
        anotar(e, cx.dir_estado, cx.run_id, &mut d, Entrada { clave: clave("validar"), op: Op::Validar, fase: Fase::Intencion, detalle: hash.clone() })?;
    }
    llevar_a(e, cx.dir_estado, cx.run_id, Estado::Verificando)?;
    vaciar(e, cx.sitios.arbol)?;
    let inf = validate::validar_candidato(e, &cx.sitios, &exp.paquete, Some(&hash), cx.spec, cx.exigencias, Some((cx.dir_estado, cx.run_id)))?;
    anotar(e, cx.dir_estado, cx.run_id, &mut d, Entrada { clave: clave("validar"), op: Op::Validar, fase: Fase::Resultado, detalle: format!("{:?} {hash}", inf.veredicto) })?;
    ejec.push(clave("validar"));
    Ok(Reanudado { desenlace: Desenlace::Validado(inf.veredicto), reutilizadas: hechas, ejecutadas: ejec })
}

/// Deja vacío el árbol del validador (el validador no reutiliza árboles).
fn vaciar<E: Entorno>(e: &mut E, raiz: &str) -> Resultado<()> {
    fn rec<E: Entorno>(e: &mut E, dir: &str) -> Resultado<()> {
        for x in e.listar(dir)? {
            let r = crate::unir(dir, &x.ruta);
            if x.tipo == crate::entorno::Tipo::Directorio {
                rec(e, &r)?;
            }
            e.borrar(&r)?;
        }
        Ok(())
    }
    if e.existe(raiz) {
        rec(e, raiz)?;
    }
    Ok(())
}

/// Si una caída o un error dejó la ejecución `bloqueada` y luego se resuelve el bloqueo (la promoción se
/// **demuestra** ya hecha, el operador adopta la copia, o se relanza), el estado vuelve a donde hace falta
/// por el camino permitido del coordinador. Nada de esto concede `aceptada`: eso sigue siendo del validador.
fn llevar_a<E: Entorno + Durable>(e: &mut E, dir: &str, run: &str, destino: Estado) -> Resultado<()> {
    let Some((mut m, h)) = state::cargar(&*e, dir, run)? else { return Ok(()) };
    let antes = m.estado;
    let camino = [Estado::Pendiente, Estado::Reproduciendo, Estado::Editando, Estado::Verificando];
    if m.estado == Estado::Bloqueada {
        for paso in camino {
            m.transicion(paso, Autoridad::Coordinador)?;
            if paso == destino {
                break;
            }
        }
    }
    if m.estado != antes {
        state::guardar(e, dir, &m, Some(&h))?;
    }
    Ok(())
}
