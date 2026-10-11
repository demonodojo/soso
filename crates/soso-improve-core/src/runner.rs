//! Ejecutor con límites y registro (T25).
//!
//! Dos caminos, un mismo vocabulario de eventos y de informe:
//!
//! - **proceso** ([`ejecutar_proceso`]): un agente externo (OpenCode en el host) que habla JSON por
//!   líneas en stdout. El trait [`Procesos`] devuelve la salida **al terminar**, así que los topes de
//!   herramientas y pasos se juzgan *a posteriori* y sólo el plazo corta de verdad (lo aplica el
//!   adaptador con `timeout_s`). Se dice en el informe, no se disimula.
//! - **nativo** ([`ejecutar_agente`]): el agente de T35 corre en proceso y los topes se aplican en
//!   vivo, antes de lanzar la herramienta que los rebasaría.
//!
//! Reglas: los totales de uso se suman **por paso** (el campo `total` de OpenCode se ignora: sumarlo
//! duplicaría lo ya contado), una herramienta se cuenta **una vez por `callID`** aunque su evento
//! llegue dos veces (`running` y `completed`), y lo que no se pudo leer (línea cortada, no JSON) se
//! cuenta aparte en vez de perderse.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::Value;

use crate::entorno::{Entorno, Orden, Procesos};
use crate::tiempo::Reloj;
use crate::{Error, Resultado};

/// Presupuesto de una ejecución.
#[derive(Debug, Clone)]
pub struct Limites {
    pub max_herramientas: usize,
    pub max_pasos: usize,
    pub plazo_ms: u64,
    /// Tope de bytes del registro conservado (eventos serializados).
    pub max_log_bytes: usize,
}

impl Default for Limites {
    fn default() -> Self {
        Limites { max_herramientas: 40, max_pasos: 40, plazo_ms: 2 * 60 * 60 * 1000, max_log_bytes: 256 * 1024 }
    }
}

/// Evento normalizado: lo mismo venga de OpenCode o del agente nativo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evento {
    Paso,
    Herramienta { id: String, nombre: String, completada: bool },
    Texto { bytes: usize },
    Uso { entrada: u64, salida: u64 },
    /// El agente declaró un error.
    Error(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cuenta {
    pub herramientas: usize,
    pub pasos: usize,
    pub tokens_entrada: u64,
    pub tokens_salida: u64,
    /// Líneas que no eran JSON o venían cortadas.
    pub lineas_ilegibles: usize,
    ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fin {
    Completo,
    LimiteHerramientas,
    LimitePasos,
    /// El proceso no terminó: plazo agotado (lo cortó el adaptador).
    Plazo,
    /// Terminó pero el flujo acaba a mitad de una línea o sin `step_finish` final.
    SalidaIncompleta,
    /// Terminó con código distinto de 0.
    ProcesoFallo,
    /// No se pudo lanzar.
    SinProceso,
    /// Sólo nativo: el modelo falló.
    ErrorModelo,
}

#[derive(Debug, Clone)]
pub struct InformeRunner {
    pub fin: Fin,
    pub codigo: Option<i32>,
    pub cuenta: Cuenta,
    pub eventos: Vec<Evento>,
    /// `true` si el registro se recortó por `max_log_bytes`.
    pub log_truncado: bool,
    /// Qué límites se aplicaron en vivo y cuáles sólo a posteriori.
    pub limites_en_vivo: bool,
    pub detalle: String,
    /// Conversación completa (sólo en el camino nativo): va a un artefacto aparte, nunca al estado ni al prompt.
    pub conversacion: Vec<Value>,
}

/// Lector incremental de JSON por líneas: tolera trozos partidos en cualquier byte.
#[derive(Default)]
pub struct LectorEventos {
    pendiente: Vec<u8>,
    pub cuenta: Cuenta,
}

impl LectorEventos {
    pub fn nuevo() -> Self {
        Self::default()
    }

    /// Añade bytes y devuelve los eventos de las líneas ya completas.
    pub fn empujar(&mut self, trozo: &[u8]) -> Vec<Evento> {
        self.pendiente.extend_from_slice(trozo);
        let mut fuera = Vec::new();
        while let Some(i) = self.pendiente.iter().position(|&b| b == b'\n') {
            let linea: Vec<u8> = self.pendiente.drain(..=i).collect();
            self.linea(&linea[..linea.len() - 1], &mut fuera);
        }
        fuera
    }

    /// Cierra el flujo. Devuelve `true` si quedó media línea sin terminar.
    pub fn fin(&mut self) -> bool {
        let resto = core::mem::take(&mut self.pendiente);
        if resto.iter().all(|b| b.is_ascii_whitespace()) {
            return false;
        }
        self.cuenta.lineas_ilegibles += 1;
        true
    }

    fn linea(&mut self, bytes: &[u8], fuera: &mut Vec<Evento>) {
        let t = bytes.strip_suffix(b"\r").unwrap_or(bytes);
        if t.iter().all(|b| b.is_ascii_whitespace()) {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(t) else {
            self.cuenta.lineas_ilegibles += 1;
            return;
        };
        let part = &v["part"];
        match v["type"].as_str().unwrap_or("") {
            "step_start" => {
                self.cuenta.pasos += 1;
                fuera.push(Evento::Paso);
            }
            "tool_use" => {
                let id = part["callID"].as_str().unwrap_or("").to_string();
                let nombre = part["tool"].as_str().unwrap_or("?").to_string();
                let completada = matches!(part["state"]["status"].as_str(), Some("completed") | Some("error"));
                if self.cuenta.ids.iter().any(|x| *x == id) && !id.is_empty() {
                    // Mismo callID otra vez (p. ej. `running` y luego `completed`): no es otra llamada.
                    if completada {
                        fuera.push(Evento::Herramienta { id, nombre, completada });
                    }
                } else {
                    self.cuenta.herramientas += 1;
                    self.cuenta.ids.push(id.clone());
                    fuera.push(Evento::Herramienta { id, nombre, completada });
                }
            }
            "text" => fuera.push(Evento::Texto { bytes: part["text"].as_str().map_or(0, |s| s.len()) }),
            "step_finish" => {
                // Por paso. `tokens.total` se ignora: es input+output y sumarlo duplicaría.
                let e = part["tokens"]["input"].as_u64().unwrap_or(0);
                let s = part["tokens"]["output"].as_u64().unwrap_or(0);
                self.cuenta.tokens_entrada += e;
                self.cuenta.tokens_salida += s;
                fuera.push(Evento::Uso { entrada: e, salida: s });
            }
            "error" => fuera.push(Evento::Error(
                v["error"]["data"]["message"].as_str().or(v["error"]["name"].as_str()).unwrap_or("error sin mensaje").to_string(),
            )),
            _ => {}
        }
    }
}

/// Guarda eventos hasta el tope de bytes; lo que no cabe no se guarda pero **se cuenta** (la cuenta
/// vive aparte y sigue completa).
fn registrar(eventos: &mut Vec<Evento>, usados: &mut usize, tope: usize, truncado: &mut bool, nuevos: Vec<Evento>) {
    for e in nuevos {
        let coste = format!("{e:?}").len();
        if *usados + coste > tope {
            *truncado = true;
            continue;
        }
        *usados += coste;
        eventos.push(e);
    }
}

fn juzgar(cuenta: &Cuenta, lim: &Limites) -> Option<Fin> {
    if cuenta.herramientas > lim.max_herramientas {
        Some(Fin::LimiteHerramientas)
    } else if cuenta.pasos > lim.max_pasos {
        Some(Fin::LimitePasos)
    } else {
        None
    }
}

/// Lanza un agente externo y normaliza su salida. `orden.timeout_s` lo fija esta función a partir del
/// plazo (el adaptador mata al proceso al vencer).
pub fn ejecutar_proceso<P: Procesos>(p: &P, mut orden: Orden, lim: &Limites) -> InformeRunner {
    orden.timeout_s = Some(((lim.plazo_ms + 999) / 1000).min(u32::MAX as u64) as u32);
    let salida = match p.ejecutar(&orden) {
        Ok(s) => s,
        Err(e) => {
            return InformeRunner { fin: Fin::SinProceso, codigo: None, cuenta: Cuenta::default(), eventos: Vec::new(), log_truncado: false, limites_en_vivo: false, detalle: e.to_string(), conversacion: Vec::new() }
        }
    };
    let mut lector = LectorEventos::nuevo();
    let (mut eventos, mut usados, mut truncado) = (Vec::new(), 0usize, false);
    // En trozos: el lector no depende de que la salida llegue entera ni de dónde se parta.
    for trozo in salida.stdout.chunks(4096) {
        let ev = lector.empujar(trozo);
        registrar(&mut eventos, &mut usados, lim.max_log_bytes, &mut truncado, ev);
    }
    let cortada = lector.fin();
    let sin_cierre = lector.cuenta.pasos > 0 && !eventos.iter().rev().any(|e| matches!(e, Evento::Uso { .. }));
    let fin = match (salida.codigo, juzgar(&lector.cuenta, lim)) {
        (None, _) if salida.motivo.as_deref().map_or(false, |m| m.contains("plazo")) => Fin::Plazo,
        (None, _) => Fin::SinProceso,
        (Some(c), _) if c != 0 => Fin::ProcesoFallo,
        (Some(_), Some(f)) => f,
        (Some(_), None) if cortada || sin_cierre => Fin::SalidaIncompleta,
        (Some(_), None) => Fin::Completo,
    };
    let stderr = String::from_utf8_lossy(&salida.stderr);
    InformeRunner {
        fin,
        codigo: salida.codigo,
        cuenta: lector.cuenta,
        eventos,
        log_truncado: truncado,
        limites_en_vivo: false,
        conversacion: Vec::new(),
        detalle: format!("{} byte(s) de stdout, {} de stderr{}", salida.stdout.len(), salida.stderr.len(), if stderr.is_empty() { String::new() } else { format!("; stderr: {}", stderr.chars().take(300).collect::<String>()) }),
    }
}

// ----------------------------------------------------------------------------- nativo

use crate::agente::{self, Config, Modelo, Politica};

/// Corre el agente nativo con topes **en vivo**: si el siguiente turno rebasaría las herramientas,
/// no se lanza. El uso sale del campo `usage` de cada respuesta (por respuesta, no acumulado).
pub fn ejecutar_agente<E: Entorno, M: Modelo, R: Reloj>(
    e: &mut E,
    modelo: &mut M,
    reloj: &R,
    pol: &Politica,
    cfg: &Config,
    tarea: &str,
    lim: &Limites,
) -> InformeRunner {
    // El modelo se envuelve para leer `usage` sin tocar el bucle.
    struct Medido<'a, M: Modelo> {
        dentro: &'a mut M,
        entrada: u64,
        salida: u64,
    }
    impl<'a, M: Modelo> Modelo for Medido<'a, M> {
        fn completar(&mut self, p: &str, t: u64) -> Resultado<String> {
            let r = self.dentro.completar(p, t)?;
            if let Ok(v) = serde_json::from_str::<Value>(&r) {
                self.entrada += v["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
                self.salida += v["usage"]["completion_tokens"].as_u64().unwrap_or(0);
            }
            Ok(r)
        }
    }
    let mut cfg = cfg.clone();
    cfg.max_turnos = cfg.max_turnos.min(lim.max_pasos);
    cfg.plazo_total_ms = cfg.plazo_total_ms.min(lim.plazo_ms);
    cfg.max_llamadas = Some(cfg.max_llamadas.map_or(lim.max_herramientas, |m| m.min(lim.max_herramientas)));
    let mut m = Medido { dentro: modelo, entrada: 0, salida: 0 };
    let inf = agente::correr(e, &mut m, reloj, pol, &cfg, tarea, &mut |_, _| {});
    let fin = match inf.fin {
        agente::Fin::Terminado => Fin::Completo,
        agente::Fin::SinTurnos => Fin::LimitePasos,
        agente::Fin::SinTiempo => Fin::Plazo,
        agente::Fin::SinLlamadas => Fin::LimiteHerramientas,
        agente::Fin::ErrorModelo => Fin::ErrorModelo,
    };
    let (mut eventos, mut usados, mut truncado) = (Vec::new(), 0usize, false);
    let mut nuevos = Vec::new();
    for msg in &inf.mensajes {
        if msg["role"] == "assistant" {
            nuevos.push(Evento::Paso);
            if let Some(ll) = msg["tool_calls"].as_array() {
                for l in ll {
                    nuevos.push(Evento::Herramienta {
                        id: l["id"].as_str().unwrap_or("").to_string(),
                        nombre: l.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                        completada: true,
                    });
                }
            }
            if let Some(t) = msg["content"].as_str().filter(|t| !t.is_empty()) {
                nuevos.push(Evento::Texto { bytes: t.len() });
            }
        }
    }
    registrar(&mut eventos, &mut usados, lim.max_log_bytes, &mut truncado, nuevos);
    InformeRunner {
        fin,
        codigo: None,
        cuenta: Cuenta { herramientas: inf.llamadas, pasos: inf.turnos, tokens_entrada: m.entrada, tokens_salida: m.salida, ..Cuenta::default() },
        eventos,
        log_truncado: truncado,
        limites_en_vivo: true,
        detalle: inf.error.unwrap_or_default(),
        conversacion: inf.mensajes,
    }
}

/// Error de configuración incoherente (p. ej. plazo 0): se rechaza antes de lanzar nada.
pub fn validar(lim: &Limites) -> Resultado<()> {
    if lim.plazo_ms == 0 || lim.max_pasos == 0 {
        return Err(Error::uso("los límites deben ser positivos: un presupuesto de cero no ejecuta nada"));
    }
    Ok(())
}

// ----------------------------------------------------------------------------- intento de tarea

use crate::durable::Durable;
use crate::state::{self, Autoridad, Estado, Medida};

/// `tarea ejecutar`: abre un intento sobre la copia, corre el agente nativo con los límites **de la
/// tarea** (`spec.limites`) y cierra el intento con lo que se midió. No valida el parche (T26) ni
/// acepta nada: el intento acaba en `verificando`, que es donde el validador toma el relevo.
///
/// Se niega a empezar si hay un intento abierto (la ejecución se cayó a mitad y repetirla sin más
/// duplicaría efectos: eso es T27) o si no quedan intentos.
pub fn ejecutar_intento<E: Entorno + Durable, M: Modelo, R: Reloj>(
    e: &mut E,
    modelo: &mut M,
    reloj: &R,
    dir_estado: &str,
    run_id: &str,
    copia: &str,
    cfg: &Config,
    pol: &Politica,
) -> Resultado<InformeRunner> {
    let r = state::reanudar(&*e, dir_estado, run_id)?;
    if let Some(n) = r.intento_abierto {
        return Err(Error::uso(format!(
            "el intento {n} de {run_id} quedó abierto (la ejecución se cayó a mitad): repetirlo duplicaría efectos; eso lo resuelve T27"
        )));
    }
    if r.intentos_restantes == 0 {
        return Err(Error::uso(format!("{run_id}: agotados los {} intentos", r.spec.limites.intentos_max)));
    }
    let Some((mut m, mut hash)) = state::cargar(&*e, dir_estado, run_id)? else {
        return Err(Error::uso(format!("no hay ninguna ejecución «{run_id}»")));
    };
    // Camino feliz hasta `editando`, con la autoridad del coordinador.
    for paso in [Estado::Reproduciendo, Estado::Editando] {
        if m.estado == Estado::Pendiente && paso == Estado::Reproduciendo || m.estado == Estado::Reproduciendo && paso == Estado::Editando {
            m.transicion(paso, Autoridad::Coordinador)?;
        }
    }
    if m.estado == Estado::Verificando {
        m.transicion(Estado::Editando, Autoridad::Coordinador)?;
    }
    if m.estado != Estado::Editando {
        return Err(Error::uso(format!("la ejecución está en «{}»: no se puede abrir un intento", m.estado.nombre())));
    }
    let ahora = reloj.ahora_ms();
    let numero = m.abrir_intento(&r.spec.limites, ahora)?;
    // El intento abierto se publica **antes** de ejecutar: si el proceso muere, el estado lo dice.
    state::guardar(e, dir_estado, &m, Some(&hash))?;
    hash = state::cargar(&*e, dir_estado, run_id)?.map(|(_, h)| h).unwrap_or_default();

    let mut lim = Limites { max_herramientas: r.spec.limites.herramientas_max as usize, ..Limites::default() };
    if let Some(s) = r.spec.limites.segundos_max {
        lim.plazo_ms = s.saturating_mul(1000);
    }
    validar(&lim)?;
    // El encargo es el que dio GO en las campañas: el agente lee `TASK.md` él mismo con `read`.
    // Pegar su contenido en el mensaje cambia el comportamiento del modelo (probado con el 14B).
    let copia_t = copia.trim_end_matches('/');
    let tarea = if e.existe(&format!("{copia_t}/{}", crate::workspace::TASK_MD)) {
        String::from("Resuelve la tarea descrita en TASK.md y ejecuta sus comprobaciones.")
    } else {
        format!("Resuelve esta tarea y ejecuta sus comprobaciones: {}", r.spec.problema)
    };
    let inf = ejecutar_agente(e, modelo, reloj, pol, cfg, &tarea, &lim);

    // Registro del intento: un artefacto aparte y una referencia corta en el estado.
    let nombre = format!("{dir_estado}/{run_id}-intento{numero}.eventos.json");
    let registro = serde_json::json!({
        "intento": numero, "fin": format!("{:?}", inf.fin), "limites_en_vivo": inf.limites_en_vivo,
        "herramientas": inf.cuenta.herramientas, "pasos": inf.cuenta.pasos,
        "tokens_entrada": inf.cuenta.tokens_entrada, "tokens_salida": inf.cuenta.tokens_salida,
        "log_truncado": inf.log_truncado, "detalle": inf.detalle,
        "eventos": inf.eventos.iter().map(|x| format!("{x:?}")).collect::<Vec<_>>(),
    });
    e.escribir(&nombre, registro.to_string().as_bytes(), 0o644)?;
    m.anotar_artefacto(&nombre)?;
    // La conversación entera, aparte: sirve para diagnosticar un intento (qué leyó, qué dijo al rendirse).
    if !inf.conversacion.is_empty() {
        let conv = format!("{dir_estado}/{run_id}-intento{numero}.conversacion.json");
        e.escribir(&conv, serde_json::to_vec(&inf.conversacion).unwrap_or_default().as_slice(), 0o644)?;
    }
    // Tokens: sólo si el endpoint los dijo. Un cero inventado se suma y se cree.
    let tokens = if inf.cuenta.tokens_entrada + inf.cuenta.tokens_salida > 0 {
        Medida::Valor(inf.cuenta.tokens_entrada + inf.cuenta.tokens_salida)
    } else {
        Medida::desconocida("el endpoint no informó de uso")
    };
    let desenlace = if inf.fin == Fin::Completo { Estado::Verificando } else { Estado::Bloqueada };
    m.cerrar_intento(desenlace, Medida::Valor(inf.cuenta.herramientas as u64), tokens, reloj.ahora_ms().max(ahora))?;
    m.transicion(if inf.fin == Fin::Completo { Estado::Verificando } else { Estado::Bloqueada }, Autoridad::Coordinador)?;
    state::guardar(e, dir_estado, &m, Some(&hash))?;
    Ok(inf)
}
