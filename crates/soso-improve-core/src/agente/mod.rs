//! Agente nativo (T35, D10): el bucle «modelo → llamadas a herramientas → modelo» que antes ponía
//! OpenCode. Vive aquí, portable, y habla con el modelo por la API de `soso-llm serve`.
//!
//! Reglas de diseño: el modelo recibe **todo** lo que pasa (errores incluidos, ninguna herramienta
//! calla), el bucle tiene tope de turnos y de tiempo, y una respuesta sin llamadas termina la tarea.

pub mod herramientas;
pub mod http;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{json, Value};

use crate::entorno::Entorno;
use crate::tiempo::Reloj;
use crate::Resultado;

pub use herramientas::{ejecutar, Politica, Respuesta};

/// Quien contesta. `peticion` es el cuerpo JSON de `/v1/chat/completions`; devuelve el cuerpo JSON.
pub trait Modelo {
    fn completar(&mut self, peticion: &str, plazo_ms: u64) -> Resultado<String>;
}

#[derive(Debug, Clone)]
pub struct Config {
    pub modelo: String,
    pub sistema: String,
    pub max_turnos: usize,
    pub max_tokens: u32,
    pub temperatura: f64,
    /// Plazo de cada petición al modelo.
    pub plazo_turno_ms: u64,
    /// Plazo de toda la tarea.
    pub plazo_total_ms: u64,
}

impl Config {
    pub fn nueva(modelo: &str, sistema: &str) -> Self {
        Config {
            modelo: modelo.to_string(),
            sistema: sistema.to_string(),
            max_turnos: 40,
            max_tokens: 2048,
            temperatura: 0.0,
            plazo_turno_ms: 2 * 60 * 60 * 1000,
            plazo_total_ms: 6 * 60 * 60 * 1000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fin {
    /// El modelo contestó sin pedir más herramientas.
    Terminado,
    /// Se agotaron los turnos.
    SinTurnos,
    /// Se agotó el plazo total.
    SinTiempo,
    /// El modelo falló (red, estado, formato).
    ErrorModelo,
}

#[derive(Debug, Clone)]
pub struct Informe {
    pub fin: Fin,
    pub turnos: usize,
    pub llamadas: usize,
    pub respuesta_final: String,
    pub error: Option<String>,
    /// Conversación completa (formato de la API), para auditar.
    pub mensajes: Vec<Value>,
}

/// Ejecuta una tarea. `al_turno` recibe la conversación tras cada turno (para persistirla).
pub fn correr<E: Entorno, M: Modelo, R: Reloj>(
    e: &mut E,
    modelo: &mut M,
    reloj: &R,
    pol: &Politica,
    cfg: &Config,
    tarea: &str,
    al_turno: &mut dyn FnMut(&[Value]),
) -> Informe {
    let mut mensajes: Vec<Value> = alloc::vec![
        json!({"role": "system", "content": cfg.sistema}),
        json!({"role": "user", "content": tarea}),
    ];
    let t0 = reloj.ahora_ms();
    let mut informe = Informe { fin: Fin::SinTurnos, turnos: 0, llamadas: 0, respuesta_final: String::new(), error: None, mensajes: Vec::new() };
    for _ in 0..cfg.max_turnos {
        let gastado = reloj.ahora_ms().saturating_sub(t0);
        if gastado >= cfg.plazo_total_ms {
            informe.fin = Fin::SinTiempo;
            break;
        }
        let plazo = cfg.plazo_turno_ms.min(cfg.plazo_total_ms - gastado);
        let peticion = json!({
            "model": cfg.modelo, "messages": mensajes, "tools": herramientas::definiciones(),
            "temperature": cfg.temperatura, "max_tokens": cfg.max_tokens, "stream": false,
        })
        .to_string();
        informe.turnos += 1;
        let cuerpo = match modelo.completar(&peticion, plazo) {
            Ok(c) => c,
            Err(err) => {
                informe.fin = Fin::ErrorModelo;
                informe.error = Some(err.to_string());
                break;
            }
        };
        let Some(msg) = serde_json::from_str::<Value>(&cuerpo).ok().and_then(|v| v.pointer("/choices/0/message").cloned()) else {
            informe.fin = Fin::ErrorModelo;
            informe.error = Some(format!("respuesta sin choices[0].message: {}", cuerpo.chars().take(300).collect::<String>()));
            break;
        };
        let texto = msg.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
        let llamadas: Vec<Value> = msg.get("tool_calls").and_then(|t| t.as_array()).cloned().unwrap_or_default();
        let mut asistente = json!({"role": "assistant", "content": texto});
        if !llamadas.is_empty() {
            asistente["tool_calls"] = Value::Array(llamadas.clone());
        }
        mensajes.push(asistente);
        if llamadas.is_empty() {
            informe.fin = Fin::Terminado;
            informe.respuesta_final = texto;
            al_turno(&mensajes);
            break;
        }
        for (i, ll) in llamadas.iter().enumerate() {
            informe.llamadas += 1;
            let id = ll.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_else(|| format!("call_{}_{i}", informe.turnos));
            let nombre = ll.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("");
            let crudo = ll.pointer("/function/arguments");
            let args: Option<Value> = match crudo {
                Some(Value::String(s)) => serde_json::from_str(s).ok(),
                Some(v @ Value::Object(_)) => Some(v.clone()),
                _ => None,
            };
            let r = match args {
                Some(a) => ejecutar(e, pol, nombre, &a),
                None => Respuesta { texto: "Los argumentos de la llamada no son un objeto JSON válido. Vuelve a llamar con JSON correcto.".to_string(), error: true },
            };
            mensajes.push(json!({"role": "tool", "tool_call_id": id, "content": r.texto}));
        }
        al_turno(&mensajes);
    }
    informe.mensajes = mensajes;
    informe
}

/// Prompt de sistema por defecto: el protocolo que dio GO con el 14B (D9), adaptado a las
/// herramientas nativas (D10).
pub const SISTEMA: &str = "Eres el agente de mejora de soso. Trabajas sólo dentro del checkout actual.\n\n\
PROTOCOLO (obligatorio):\n\
- En cada respuesta escribes **una sola llamada a una herramienta y nada más**: sin texto antes ni después.\n\
- El resultado de la herramienta llega en el mensaje siguiente. **Nunca lo inventes**: espera a recibirlo.\n\
- No des por leído un archivo que no hayas leído con `read`; no edites sin haber leído antes el archivo.\n\
- Sólo cuando hayas terminado todos los pasos respondes con texto, y entonces es el informe final.\n\n\
PASOS:\n\
1. Llama a `read` con `TASK.md` (raíz del checkout): contiene el objetivo, el archivo que puedes tocar y las comprobaciones.\n\
2. Localiza el código. Si el archivo es corto (menos de 150 líneas) lee entero; si es largo, `grep` con el nombre de la función y `read` sólo del tramo (`offset` y `limit`).\n\
3. Llama a `reemplazar_texto` con `filePath`, `buscar` (un fragmento corto de lo que leíste; los espacios no importan) y `nuevo`. Cambio mínimo: no toques otros archivos. Si dice que no lo encuentra o que aparece varias veces, repite con otro fragmento.\n\
4. Llama a `bash` con las comprobaciones de TASK.md (`cargo test …`) y lee el resultado.\n\
5. Informe final en texto corto: qué cambiaste, qué comprobaciones corrieron y su salida resumida, y lo que no pudiste hacer. No inventes resultados.";
