//! Agente nativo (T35, D10): el bucle «modelo → llamadas a herramientas → modelo» que antes ponía
//! OpenCode. Vive aquí, portable, y habla con el modelo por la API de `soso-llm serve`.
//!
//! Reglas de diseño: el modelo recibe **todo** lo que pasa (errores incluidos, ninguna herramienta
//! calla), el bucle tiene tope de turnos y de tiempo, y una respuesta sin llamadas termina la tarea.

pub mod herramientas;
pub mod http;
pub mod prueba;
pub mod sesion;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{json, Value};

use crate::entorno::Entorno;
use crate::tiempo::Reloj;
use crate::Resultado;

pub use herramientas::{ejecutar, Forja, Politica, Respuesta};

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
    /// Tope de llamadas a herramientas, aplicado **antes** de ejecutar la que lo rebasaría.
    pub max_llamadas: Option<usize>,
    /// Cuántas veces se le recuerda al modelo, sin gastar una llamada a herramienta, que no ha acabado: respuesta vacía,
    /// llamada escrita como texto, cambio sin comprobar o comprobación fallida. Se cuentan en el informe.
    pub empujones: usize,
    /// Cuántas veces se acepta que el servidor rechace la respuesta del modelo por mal formada (`400`: texto tras
    /// una llamada, argumentos que no son JSON…). Se le devuelve el motivo y se reintenta; pasado el tope, es error.
    pub errores_de_formato: usize,
    /// A la `n`-ésima llamada **idéntica** seguida a otra que ya falló, no se ejecuta: se le devuelve al modelo que se
    /// ha repetido `n` veces y qué cambiar. (Con T=0 un modelo que falla una edición suele repetirla veinte veces:
    /// campañas 6 y 7.) `0` lo apaga.
    pub repetidas_max: usize,
    /// Ventana de contexto del modelo, en tokens. Con ella se recortan las salidas de herramienta **antiguas**
    /// antes de que la conversación llegue al límite (el servidor rechaza con 422 lo que no cabe). `None` lo apaga.
    pub contexto_tokens: Option<usize>,
    /// Caracteres por token con los que se estima el tamaño de la conversación. Depende del tokenizador: ~3,5 en
    /// castellano o código con un BPE normal, pero ~1,5 con Mellum, que parte los dígitos uno a uno y hace carísimos los
    /// volcados con número de línea (campaña 11: 56 k caracteres = 36 k tokens)..
    pub chars_por_token: f32,
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
            max_llamadas: None,
            empujones: 2,
            errores_de_formato: 5,
            repetidas_max: 3,
            contexto_tokens: Some(32_768),
            chars_por_token: 3.0,
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
    /// El siguiente turno rebasaba el tope de llamadas a herramientas.
    SinLlamadas,
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
    /// Recordatorios enviados al modelo (ver `Config::empujones`).
    pub empujones: usize,
    /// Respuestas del modelo que el servidor rechazó por mal formadas y se le devolvieron para que se corrigiera.
    pub formato: usize,
    /// Llamadas idénticas a una que ya había fallado y que no se ejecutaron (ver `Config::repetidas_max`).
    pub repetidas: usize,
    /// Salidas de herramienta antiguas recortadas para no pasar de la ventana (ver `Config::contexto_tokens`).
    pub recortadas: usize,
    /// Conversación completa (formato de la API), para auditar.
    pub mensajes: Vec<Value>,
}

/// Ejecuta una tarea. `al_turno` recibe el entorno y la conversación tras cada turno (para
/// persistirla; ver [`sesion`]).
pub fn correr<E: Entorno, M: Modelo, R: Reloj>(
    e: &mut E,
    modelo: &mut M,
    reloj: &R,
    pol: &Politica,
    cfg: &Config,
    tarea: &str,
    al_turno: &mut dyn FnMut(&mut E, &[Value]),
) -> Informe {
    let mensajes: Vec<Value> = alloc::vec![
        json!({"role": "system", "content": cfg.sistema}),
        json!({"role": "user", "content": tarea}),
    ];
    correr_desde(e, modelo, reloj, pol, cfg, mensajes, al_turno)
}

/// Sigue una conversación ya empezada (sesión recuperada). Una llamada del asistente sin sus
/// resultados (el agente murió a mitad de turno) se descarta: no se re-ejecuta nada a ciegas, el
/// modelo vuelve a decidir con lo que sí consta.
pub fn correr_desde<E: Entorno, M: Modelo, R: Reloj>(
    e: &mut E,
    modelo: &mut M,
    reloj: &R,
    pol: &Politica,
    cfg: &Config,
    mut mensajes: Vec<Value>,
    al_turno: &mut dyn FnMut(&mut E, &[Value]),
) -> Informe {
    sesion::normalizar(&mut mensajes);
    if sesion::terminada(&mensajes) {
        let r = mensajes.last().and_then(|m| m["content"].as_str()).unwrap_or("").to_string();
        return Informe { fin: Fin::Terminado, turnos: 0, llamadas: 0, respuesta_final: r, error: None, empujones: 0, formato: 0, repetidas: 0, recortadas: 0, mensajes };
    }
    let t0 = reloj.ahora_ms();
    let mut informe = Informe { fin: Fin::SinTurnos, turnos: 0, llamadas: 0, respuesta_final: String::new(), error: None, empujones: 0, formato: 0, repetidas: 0, recortadas: 0, mensajes: Vec::new() };
    // Última llamada ejecutada, si falló, y cuántas veces seguidas se ha repetido igual.
    let mut ultima: Option<(String, String, bool)> = None;
    let mut seguidas = 0usize;
    for _ in 0..cfg.max_turnos {
        let gastado = reloj.ahora_ms().saturating_sub(t0);
        if gastado >= cfg.plazo_total_ms {
            informe.fin = Fin::SinTiempo;
            break;
        }
        let plazo = cfg.plazo_turno_ms.min(cfg.plazo_total_ms - gastado);
        if let Some(limite) = cfg.contexto_tokens {
            informe.recortadas += recortar_contexto(&mut mensajes, limite, cfg.chars_por_token);
        }
        let peticion = json!({
            "model": cfg.modelo, "messages": mensajes, "tools": herramientas::definiciones(pol),
            "temperature": cfg.temperatura, "max_tokens": cfg.max_tokens, "stream": false,
        })
        .to_string();
        informe.turnos += 1;
        let cuerpo = match modelo.completar(&peticion, plazo) {
            Ok(c) => c,
            Err(err) if err.to_string().contains("Some(400)") && informe.formato < cfg.errores_de_formato => {
                // El servidor descartó una respuesta mal formada: no entra en el historial. Se le dice al modelo qué
                // estuvo mal (el motivo viene del propio servidor) y se le deja corregirse.
                informe.formato += 1;
                let t = err.to_string();
                let motivo = t.split("\"message\":\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or("formato inválido");
                mensajes.push(json!({"role": "user", "content": format!(
                    "Tu última respuesta no fue una llamada válida a una herramienta ({motivo}). Hazla de nuevo: una sola llamada, con los argumentos como un objeto JSON válido, y sin texto antes ni después."
                )}));
                al_turno(e, &mensajes);
                continue;
            }
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
            if informe.empujones < cfg.empujones {
                if let Some(aviso) = aviso_de_empuje(&mensajes, &texto) {
                    informe.empujones += 1;
                    mensajes.push(json!({"role": "user", "content": aviso}));
                    al_turno(e, &mensajes);
                    continue;
                }
            }
            informe.fin = Fin::Terminado;
            informe.respuesta_final = texto;
            al_turno(e, &mensajes);
            break;
        }
        if cfg.max_llamadas.map_or(false, |m| informe.llamadas + llamadas.len() > m) {
            // Sin ejecutar nada: la llamada que rebasa el presupuesto no se lanza.
            mensajes.pop();
            informe.fin = Fin::SinLlamadas;
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
                Some(a) => {
                    let clave = (nombre.to_string(), serde_json::to_string(&a).unwrap_or_default());
                    let repite = ultima.as_ref().is_some_and(|(n, c, fallo)| *fallo && *n == clave.0 && *c == clave.1);
                    seguidas = if repite { seguidas + 1 } else { 1 };
                    let r = if cfg.repetidas_max > 0 && repite && seguidas >= cfg.repetidas_max {
                        informe.repetidas += 1;
                        Respuesta { texto: aviso_de_repeticion(nombre, seguidas), error: true }
                    } else {
                        let mut r = ejecutar(e, pol, nombre, &a);
                        if repite && r.error {
                            r.texto.push_str(&format!("\n\nAVISO: es la misma llamada que acabas de hacer y ha fallado igual ({seguidas} veces). No la repitas: cambia lo que envías (ver la pista de arriba) o usa otra herramienta."));
                        }
                        r
                    };
                    ultima = Some((clave.0, clave.1, r.error));
                    r
                }
                None => Respuesta { texto: "Los argumentos de la llamada no son un objeto JSON válido. Vuelve a llamar con JSON correcto.".to_string(), error: true },
            };
            mensajes.push(json!({"role": "tool", "tool_call_id": id, "content": r.texto}));
        }
        al_turno(e, &mensajes);
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
2. Localiza el código. Si el archivo es corto (menos de 150 líneas) lee entero; si es largo, **no lo recorras por tramos**: llama primero a `grep` con el nombre de la función o un texto que aparezca en ella y después a `read` sólo del tramo que rodea la línea (`offset` y `limit`).\n\
3. Para **cambiar** algo que ya existe, llama a `reemplazar_texto` con `filePath`, `buscar` (un fragmento corto de lo que leíste; los espacios no importan) y `nuevo`. Para **añadir** código nuevo (una función, una rama), llama a `insertar` con `filePath`, `linea` (el número que te mostró `read`; se inserta después de esa línea) y `texto`. Siempre bloques completos: con su `{` y su `}`. Cambio mínimo: no toques otros archivos. Si una herramienta dice que no lo encuentra, que aparece varias veces o que desequilibra las llaves, corrige y repite.\n\
4. Llama a `bash` con las comprobaciones de TASK.md (`cargo test …`) y lee el resultado.\n\
5. Informe final en texto corto: qué cambiaste, qué comprobaciones corrieron y su salida resumida, y lo que no pudiste hacer. No inventes resultados.";

/// Prompt del perfil Forja (T37): el agente edita fuentes de userspace y compila **en el host** por
/// la herramienta `forja`; el diagnóstico vuelve a él sin que nadie lo copie.
pub const SISTEMA_FORJA: &str = "Eres el agente de mejora de soso. Trabajas dentro del checkout /src/soso, en un sistema sin shell: sólo tienes las herramientas.\n\n\
PROTOCOLO (obligatorio):\n\
- En cada respuesta escribes **una sola llamada a una herramienta y nada más**: sin texto antes ni después.\n\
- El resultado de la herramienta llega en el mensaje siguiente. **Nunca lo inventes**: espera a recibirlo.\n\
- No edites un archivo sin haberlo leído antes con `read`.\n\
- Sólo cuando hayas terminado todos los pasos respondes con texto: el informe final.\n\n\
PASOS:\n\
1. Llama a `read` con el archivo que indica la tarea. Las rutas son **relativas a /src/soso** (por ejemplo `user/hola-std/src/main.rs`); si una ruta no existe, la respuesta te dice qué hay en el directorio más cercano: úsala.\n\
2. Llama a `reemplazar_texto` con `filePath`, `buscar` (un fragmento corto de lo que leíste; los espacios no importan) y `nuevo`. Cambio mínimo.\n\
3. Llama a `forja` con `accion` = `all`: sube las fuentes, compila en el host y aplica el binario. Si devuelve un error de compilación, léelo, corrige el archivo con `reemplazar_texto` y vuelve a llamar a `forja`.\n\
4. Llama a `bash` con `/bin/hola-std` para comprobar lo que imprime el programa ya compilado.\n\
5. Informe final en texto corto: qué cambiaste y qué imprimió el programa. No inventes resultados.";

/// ¿Ha acabado el modelo de verdad o se ha quedado a medias? Devuelve el recordatorio que toca, si toca.
/// Casos (por este orden): la llamada escrita como texto; respuesta vacía; un cambio hecho y nunca
/// comprobado; la última comprobación falló y el modelo se rinde.
fn aviso_de_empuje(mensajes: &[Value], texto: &str) -> Option<String> {
    let t = texto.trim();
    if t.contains("\"arguments\"") && t.contains("\"name\"") {
        return Some("Has escrito la llamada a la herramienta como texto y así no se ejecuta. Hazla como una llamada de herramienta (una sola, sin texto antes ni después).".to_string());
    }
    if t.is_empty() {
        return Some("Tu respuesta estaba vacía. Haz ahora la siguiente llamada a una herramienta (una sola, sin texto).".to_string());
    }
    // Recorrido: ¿hay un cambio sin comprobar? ¿falló la última comprobación?
    let mut nombres: Vec<(String, String)> = Vec::new();
    let (mut sin_comprobar, mut ultima_fallo) = (false, false);
    for m in mensajes {
        if m["role"] == "assistant" {
            for l in m["tool_calls"].as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
                nombres.push((l["id"].as_str().unwrap_or("").to_string(), l.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("").to_string()));
            }
        } else if m["role"] == "tool" {
            let id = m["tool_call_id"].as_str().unwrap_or("");
            let nombre = nombres.iter().rev().find(|(i, _)| i == id).map(|(_, n)| n.as_str()).unwrap_or("");
            let c = m["content"].as_str().unwrap_or("");
            match nombre {
                "reemplazar_texto" | "insertar" if c.starts_with("Hecho") => {
                    sin_comprobar = true;
                    ultima_fallo = false;
                }
                "bash" | "forja" => {
                    sin_comprobar = false;
                    ultima_fallo = !c.starts_with("exit code: 0");
                }
                _ => {}
            }
        }
    }
    if sin_comprobar {
        return Some("Todavía no has comprobado tu cambio. Llama ahora a `bash` con las comprobaciones de la tarea (una sola llamada, sin texto antes ni después).".to_string());
    }
    if ultima_fallo {
        return Some("La última comprobación ha fallado. Lee el error que te devolvió, corrige el archivo con `reemplazar_texto` o `insertar` y vuelve a ejecutar la comprobación. No termines con la prueba en rojo.".to_string());
    }
    None
}

/// Qué decirle al modelo cuando repite una llamada que ya falló: no se ejecuta, y se le da una vía distinta.
fn aviso_de_repeticion(herramienta: &str, veces: usize) -> String {
    let consejo = match herramienta {
        "reemplazar_texto" => "Vuelve a leer el tramo exacto con `read` (con `offset` y `limit`) y copia en `buscar` UNA sola línea completa y corta, sin los números de línea que muestra `read`. Para añadir código nuevo usa `insertar` con el número de línea.",
        "insertar" => "Lee con `read` el tramo alrededor y comprueba el número de línea; inserta bloques completos (con su `{` y su `}`).",
        "bash" => "Lee el error de la salida anterior: la orden no va a cambiar de resultado si no cambias antes el archivo con `reemplazar_texto` o `insertar`.",
        "read" | "grep" | "glob" => "Ya tienes esa información en la conversación; úsala para dar el siguiente paso (editar o comprobar).",
        _ => "Cambia los argumentos o usa otra herramienta.",
    };
    format!("No he ejecutado esta llamada: es la misma que ya falló {} veces seguidas y volvería a fallar. {consejo}", veces - 1)
}

/// Tokens que ocupa la conversación, por exceso (una cadena de código o castellano da ~3,5 caracteres por token).
fn tokens_estimados(mensajes: &[Value], chars_por_token: f32) -> usize {
    (mensajes.iter().map(|m| m.to_string().len()).sum::<usize>() as f32 / chars_por_token) as usize
}

/// Recorta las salidas de herramienta **antiguas** cuando la conversación pasa del 70 % de la ventana (y, si aún sobra,
/// del 90 %, más a fondo). Se conservan el sistema, la tarea y los últimos mensajes tal cual; de las salidas viejas
/// queda el principio. Las llamadas no se tocan, así que cada resultado sigue atado a su `tool_call_id`.
/// Devuelve cuántas salidas se recortaron en esta pasada.
fn recortar_contexto(mensajes: &mut [Value], limite: usize, chars_por_token: f32) -> usize {
    const MARCA: &str = "\n[… salida antigua recortada para no agotar el contexto; vuelve a pedirla si la necesitas]";
    let mut recortadas = 0;
    for (umbral, conservar, cabeza) in [(limite * 7 / 10, 10usize, 400usize), (limite * 9 / 10, 4, 120)] {
        if tokens_estimados(mensajes, chars_por_token) <= umbral || mensajes.len() <= conservar + 2 {
            continue;
        }
        let hasta = mensajes.len() - conservar;
        for m in mensajes[2..hasta].iter_mut() {
            if m["role"] != "tool" {
                continue;
            }
            let c = m["content"].as_str().unwrap_or("");
            if c.len() <= cabeza + MARCA.len() || c.ends_with(MARCA) {
                continue;
            }
            let corte = c.char_indices().take_while(|(i, _)| *i < cabeza).last().map_or(0, |(i, ch)| i + ch.len_utf8());
            let nuevo = format!("{}{MARCA}", &c[..corte]);
            m["content"] = Value::String(nuevo);
            recortadas += 1;
        }
    }
    recortadas
}
