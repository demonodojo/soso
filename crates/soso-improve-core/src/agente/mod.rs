//! Agente nativo (T35, D10): el bucle «modelo → llamadas a herramientas → modelo» que antes ponía
//! OpenCode. Vive aquí, portable, y habla con el modelo por la API de `soso-llm serve`.
//!
//! Reglas de diseño: el modelo recibe **todo** lo que pasa (errores incluidos, ninguna herramienta
//! calla), el bucle tiene tope de turnos y de tiempo, y una respuesta sin llamadas termina la tarea.

pub mod herramientas;
pub mod http;
pub mod prueba;
pub mod orquestador;
pub mod revision;
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
    /// Cuántas veces se le dice al modelo, al intentar terminar, que **no ha modificado ningún archivo** (campaña 11:
    /// 3 de 10 tareas acababan con «el candidato no cambió nada»). Presupuesto aparte de `empujones`.
    pub avisos_sin_editar: usize,
    /// Si `true`, al intentar terminar tras haber editado sin escribir **ninguna prueba propia** (un `#[test]` insertado) se le
    /// avisa hasta `avisos_sin_pruebas` veces: el agente debe convertir cada caso límite del enunciado en una prueba suya.
    pub exigir_pruebas: bool,
    pub avisos_sin_pruebas: usize,
    /// Con `exigir_pruebas`: veces que se le avisa de que sus pruebas no se han ejecutado EN VERDE tras su último cambio.
    pub avisos_sin_ejecutar: usize,
    /// Antes de dar por terminado un trabajo con ediciones, el propio arnés ejecuta las comprobaciones de «Qué tiene que pasar» de `TASK.md`
    /// (las órdenes `cargo …` entre comillas invertidas) y, si fallan, se las devuelve al modelo (hasta `avisos_comprobacion` veces). Caza el
    /// código que no compila o rompe las pruebas visibles aunque el modelo no lo haya comprobado (R38 de la campaña 39: llamaba a dos funciones
    /// que nunca definió y terminó sin ejecutar nada).
    pub comprobar_al_terminar: bool,
    pub avisos_comprobacion: usize,
    /// Un intento que agota los turnos o las herramientas tras haber EDITADO se da por completo para que lo evalúe el validador independiente
    /// (campañas 41–42: R31 y R33 se quedaban en `LimitePasos` con el cambio a medias en disco, y un error no se valida). Sin ediciones sigue
    /// siendo un límite superado.
    pub validar_al_agotar: bool,
    /// Al cerrar el intento, las pruebas propias (`pruebas_agente::…`) que sigan en rojo se eliminan del archivo: la comprobación de la tarea ejecuta TODAS
    /// las pruebas del crate, y una prueba del propio agente en rojo (campaña 42: R33, R36, R37 y R38) hacía fallar el conjunto aunque el código cumpliera
    /// el enunciado. Lo que se mide es el código, no la calidad de sus pruebas de andamiaje.
    pub podar_pruebas_rojas: bool,
    /// Si la tarea habla de pánico o desbordamiento, antes de terminar se pasa Clippy (`indexing_slicing`, `arithmetic_side_effects`) y se le enseñan
    /// los avisos que caen en líneas que el agente ha añadido o cambiado (hasta `avisos_panico` veces).
    pub alertas_panico: bool,
    pub avisos_panico: usize,
    /// Antes de terminar, si el agente ha quitado o cambiado la firma de algún ítem `pub` de los archivos que editó, se le avisa (hasta `avisos_api` veces):
    /// las pruebas reservadas compilan contra la API original (campaña 53: R33 borró `resample_to_16k`; R34 y R38 cambiaron el tipo de retorno).
    pub proteger_api: bool,
    pub avisos_api: usize,
    /// Con `exigir_pruebas`: veces que se le avisa de que ha escrito menos pruebas que puntos tiene «Qué se pide».
    pub avisos_pocas_pruebas: usize,
    /// A la `n`-ésima llamada **repetida** de una que ya salió bien desde la última edición (aunque no sea la inmediata anterior) no
    /// se la ejecuta: su resultado no puede cambiar si no se edita nada entre medias (campaña 12: Gemma repetía 24 veces un `cargo test` ya en verde y agotaba las herramientas sin
    /// dar el trabajo por hecho). Con dos repeticiones más sin cambiar de conducta se da el intento por terminado. `0` lo apaga.
    pub repetidas_ok_max: usize,
    /// Tope de mensajes de la conversación. El servidor valida `MAX_MENSAJES` = 64 y devuelve 400 al pasarse: con 60 herramientas por
    /// intento la conversación llegaba a 71 y 5 de 10 tareas de la campaña 23 morían con «messages: 71 supera el máximo de 64».
    /// Al acercarse se pliegan los intercambios (llamada + resultados) más antiguos. `None` lo apaga.
    pub mensajes_max: Option<usize>,
    /// Al pasar de `mensajes_max` se pliega de golpe hasta `mensajes_max - plegado_holgura` en vez de uno a uno: el pliegue reescribe el principio de la conversación
    /// (la tarea lleva el historial) y con ello se invalida la caché de prefijo del servidor; plegando cada turno se reprocesaban ~18 000 tokens en 1 de cada 4
    /// peticiones (el 87 % del prefill de la campaña 54). En bloques, el prefijo se mantiene estable entre pliegues.
    pub plegado_holgura: usize,
    /// Si `true`, antes de dar por terminado un trabajo en el que hubo ediciones se le muestra al modelo la revisión (`revision::texto`:
    /// su diff y los requisitos de `TASK.md`) hasta `avisos_revision` veces, para que contraste cada punto con lo que hizo.
    pub revision: bool,
    pub avisos_revision: usize,
    /// Con `revision`, la revisión la hace una petición aparte al modelo (revisor independiente, sin la conversación del agente) y sólo
    /// se le pasa al agente lo que el revisor señale; si no encuentra fallos, el agente termina sin más avisos.
    pub revisor: bool,
    /// Equipo de tres roles (`orquestador`): un planificador de sólo lectura prepara el plan, el programador lo ejecuta, un revisor
    /// independiente señala fallos y el planificador replantea la corrección, hasta `ciclos_max` ciclos o hasta que todo pase.
    pub multiagente: bool,
    pub ciclos_max: usize,
    /// Sólo `read`, `grep` y `glob` (el planificador no edita ni ejecuta).
    pub solo_lectura: bool,
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
            avisos_sin_editar: 2,
            exigir_pruebas: false,
            avisos_sin_pruebas: 2,
            avisos_sin_ejecutar: 2,
            comprobar_al_terminar: false,
            avisos_comprobacion: 2,
            validar_al_agotar: false,
            podar_pruebas_rojas: false,
            alertas_panico: false,
            avisos_panico: 3,
            proteger_api: false,
            avisos_api: 1,
            avisos_pocas_pruebas: 1,
            repetidas_ok_max: 3,
            mensajes_max: Some(60),
            plegado_holgura: 16,
            revision: false,
            avisos_revision: 1,
            revisor: false,
            multiagente: false,
            ciclos_max: 3,
            solo_lectura: false,
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
    /// Avisos de «no has modificado nada» enviados (ver `Config::avisos_sin_editar`).
    pub sin_editar: usize,
    /// Avisos de «escribe tus propias pruebas» enviados (ver `Config::exigir_pruebas`).
    pub sin_pruebas: usize,
    /// Avisos de «ejecuta tus pruebas en verde» enviados (ver `Config::avisos_sin_ejecutar`).
    pub sin_ejecutar: usize,
    /// Comprobaciones de la tarea que el arnés ejecutó al final y que fallaron (ver `Config::comprobar_al_terminar`).
    pub comprobaciones_rojas: usize,
    /// Avisos de Clippy sobre líneas del agente enviados (ver `Config::alertas_panico`).
    pub alertas_panico: usize,
    /// Avisos de «menos pruebas que puntos» enviados.
    pub pocas_pruebas: usize,
    /// Veces que, tras releer lo mismo sin editar, se le redirigió a escribir en vez de cerrar el intento.
    pub redirecciones: usize,
    /// Pruebas propias en rojo que se eliminaron al cerrar (ver `Config::podar_pruebas_rojas`).
    pub podadas: Vec<String>,
    /// Llamadas que el guardián de repeticiones no ejecutó (no cuentan como llamadas a herramientas).
    pub bloqueadas: usize,
    /// Revisiones impuestas al modelo antes de terminar (ver `Config::revision`).
    pub revisiones: usize,
    /// Ciclos planificar → programar → revisar que se han recorrido (ver `Config::multiagente`).
    pub ciclos: usize,
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
    if cfg.multiagente {
        return orquestador::correr(e, modelo, reloj, pol, cfg, tarea, al_turno);
    }
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
    mensajes: Vec<Value>,
    al_turno: &mut dyn FnMut(&mut E, &[Value]),
) -> Informe {
    let mut originales = alloc::collections::BTreeMap::new();
    correr_con_originales(e, modelo, reloj, pol, cfg, mensajes, &mut originales, al_turno)
}

/// Como `correr_desde`, pero los archivos originales (antes de la primera edición) se llevan fuera: el orquestador los comparte entre
/// los ciclos del programador para que el revisor vea el diff acumulado respecto al inicio de la tarea.
pub fn correr_con_originales<E: Entorno, M: Modelo, R: Reloj>(
    e: &mut E,
    modelo: &mut M,
    reloj: &R,
    pol: &Politica,
    cfg: &Config,
    mut mensajes: Vec<Value>,
    originales: &mut alloc::collections::BTreeMap<String, Option<String>>,
    al_turno: &mut dyn FnMut(&mut E, &[Value]),
) -> Informe {
    sesion::normalizar(&mut mensajes);
    if sesion::terminada(&mensajes) {
        let r = mensajes.last().and_then(|m| m["content"].as_str()).unwrap_or("").to_string();
        return Informe { fin: Fin::Terminado, turnos: 0, llamadas: 0, respuesta_final: r, error: None, empujones: 0, formato: 0, repetidas: 0, recortadas: 0, sin_editar: 0, sin_pruebas: 0, sin_ejecutar: 0, comprobaciones_rojas: 0, alertas_panico: 0, pocas_pruebas: 0, redirecciones: 0, podadas: Vec::new(), bloqueadas: 0, revisiones: 0, ciclos: 0, mensajes };
    }
    let t0 = reloj.ahora_ms();
    let mut informe = Informe { fin: Fin::SinTurnos, turnos: 0, llamadas: 0, respuesta_final: String::new(), error: None, empujones: 0, formato: 0, repetidas: 0, recortadas: 0, sin_editar: 0, sin_pruebas: 0, sin_ejecutar: 0, comprobaciones_rojas: 0, alertas_panico: 0, pocas_pruebas: 0, redirecciones: 0, podadas: Vec::new(), bloqueadas: 0, revisiones: 0, ciclos: 0, mensajes: Vec::new() };
    // Última llamada ejecutada, si falló, y cuántas veces seguidas se ha repetido igual.
    let mut ultima: Option<(String, String, bool)> = None;
    let mut seguidas = 0usize;
    // Veces que cada llamada ha FALLADO desde la última edición que salió bien, seguidas o no: el modelo alternaba dos llamadas que
    // fallaban igual (A, B, A, B…) y la guarda de «la misma que acaba de fallar» nunca saltaba (campaña 27, R31: 60 turnos así).
    let mut fallidas: alloc::collections::BTreeMap<(String, String), usize> = alloc::collections::BTreeMap::new();
    // Huellas de los contenidos por los que ha pasado cada archivo editado (el original primero) y cuántas veces una edición ha devuelto
    // un archivo a un estado anterior: el modelo alternaba A→B→A→B (cada edición «Hecho») hasta agotar los turnos (campaña 36, R31).
    let mut estados: alloc::collections::BTreeMap<String, Vec<u64>> = alloc::collections::BTreeMap::new();
    let mut oscilaciones = 0usize;
    // Respuestas mal formadas SEGUIDAS: a T=0 el modelo repite la misma respuesta rota (campaña 41, R38: `call_5` tres veces idéntica), así que los
    // reintentos suben la temperatura.
    let mut formato_seguidos = 0usize;
    // Ediciones «sin efecto» (el modelo reenvía el código que ya está) y llamadas bloqueadas por repetirse, SEGUIDAS desde la última edición buena. A T=0
    // el mismo contexto da la misma respuesta: en las campañas 45–47 el 35–40 % de las llamadas eran de este tipo (R34: 23 reenvíos idénticos de `sample`
    // tras una prueba propia en rojo). Se sube la temperatura para que pueda salir otra cosa; la campaña 29 (rondas a T=0,7) tuvo 22 % → 16 % → 4 %.
    let mut sin_efecto_seguidos = 0usize;
    let mut api_avisos = 0usize;
    // Pruebas rojas de la última ejecución y cuántas veces seguidas han sido EXACTAMENTE las mismas: reescribir otra función sin que cambie el
    // resultado (campaña 42, R31: 15 `reemplazar_funcion` con las mismas pruebas propias en rojo) indica que la causa está en otro sitio o en la propia prueba.
    let mut rojos_previos: alloc::collections::BTreeSet<String> = alloc::collections::BTreeSet::new();
    let mut rojos_seguidos = 0usize;
    let mut atascado = false;
    let mut vistas: Vec<(String, String)> = Vec::new();
    let mut racha_ok = 0usize;
    for _ in 0..cfg.max_turnos {
        let gastado = reloj.ahora_ms().saturating_sub(t0);
        if gastado >= cfg.plazo_total_ms {
            informe.fin = Fin::SinTiempo;
            break;
        }
        let plazo = cfg.plazo_turno_ms.min(cfg.plazo_total_ms - gastado);
        if let Some(max) = cfg.mensajes_max {
            let n = limitar_mensajes(&mut mensajes, max, cfg.plegado_holgura);
            if n > 0 {
                vistas.clear();
                fallidas.clear();
                racha_ok = 0;
                informe.recortadas += n;
            }
        }
        if let Some(limite) = cfg.contexto_tokens {
            let n = recortar_contexto(&mut mensajes, limite, cfg.chars_por_token);
            if n > 0 {
                // El resultado de lo ya hecho se ha recortado y el propio mensaje invita a «volver a pedirlo»: repetir la llamada
                // deja de ser inútil, así que no se cuenta como repetición.
                vistas.clear();
                fallidas.clear();
                racha_ok = 0;
            }
            informe.recortadas += n;
        }
        let peticion = json!({
            "model": cfg.modelo, "messages": mensajes, "tools": if cfg.solo_lectura { herramientas::solo_lectura(herramientas::definiciones(pol)) } else { herramientas::definiciones(pol) },
            "temperature": if formato_seguidos > 0 { cfg.temperatura.max((0.3 + 0.2 * formato_seguidos as f64).min(1.0)) } else if sin_efecto_seguidos >= 2 { cfg.temperatura.max((0.1 + 0.2 * sin_efecto_seguidos as f64).min(0.9)) } else { cfg.temperatura }, "max_tokens": cfg.max_tokens, "stream": false,
        })
        .to_string();
        informe.turnos += 1;
        let cuerpo = match modelo.completar(&peticion, plazo) {
            Ok(c) => {
                formato_seguidos = 0;
                c
            }
            // llama-server contesta 500 («Failed to parse tool call arguments as JSON») cuando la llamada llega cortada o con el JSON roto (campaña 50:
            // R38 con Qwen3.6, un `insertar` de un módulo de pruebas entero cortado por `max_tokens`); el servidor propio contestaba 400.
            Err(err) if (err.to_string().contains("Some(400)") || err.to_string().contains("Failed to parse tool call")) && informe.formato < cfg.errores_de_formato => {
                // El servidor descartó una respuesta mal formada: no entra en el historial. Se le dice al modelo qué
                // estuvo mal (el motivo viene del propio servidor) y se le deja corregirse.
                informe.formato += 1;
                formato_seguidos += 1;
                let t = err.to_string();
                let motivo = t.split("\"message\":\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or("formato inválido");
                let motivo: String = motivo.trim_end_matches('\\').chars().take(200).collect();
                let cortada = t.contains("Failed to parse tool call") || t.contains("missing closing quote");
                let consejo = if formato_seguidos >= 2 || cortada {
                    " Si el JSON se te rompe, haz la llamada MÁS CORTA: una sola cosa por llamada y, si escribes código, pocas líneas cada vez (por ejemplo una prueba por `insertar`); escapa las comillas dobles dentro del texto (\\\") y los saltos de línea (\\n)."
                } else {
                    ""
                };
                mensajes.push(json!({"role": "user", "content": format!(
                    "Tu última respuesta no fue una llamada válida a una herramienta ({motivo}). Hazla de nuevo: una sola llamada, con los argumentos como un objeto JSON válido, y sin texto antes ni después.{consejo}"
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
            if informe.sin_editar < cfg.avisos_sin_editar && !hubo_edicion(&mensajes) {
                informe.sin_editar += 1;
                mensajes.push(json!({"role": "user", "content": AVISO_SIN_EDITAR}));
                al_turno(e, &mensajes);
                continue;
            }
            if cfg.exigir_pruebas && informe.sin_pruebas < cfg.avisos_sin_pruebas && !hubo_prueba_propia(&mensajes) {
                informe.sin_pruebas += 1;
                mensajes.push(json!({"role": "user", "content": AVISO_SIN_PRUEBAS}));
                al_turno(e, &mensajes);
                continue;
            }
            if cfg.exigir_pruebas && informe.sin_ejecutar < cfg.avisos_sin_ejecutar && hubo_prueba_propia(&mensajes) && !pruebas_propias_en_verde(&mensajes) {
                informe.sin_ejecutar += 1;
                mensajes.push(json!({"role": "user", "content": AVISO_PRUEBAS_SIN_EJECUTAR}));
                al_turno(e, &mensajes);
                continue;
            }
            if cfg.exigir_pruebas && informe.pocas_pruebas < cfg.avisos_pocas_pruebas && hubo_prueba_propia(&mensajes) {
                let puntos = herramientas::puntos_de_la_tarea(e, pol);
                let escritas = pruebas_propias_escritas(&mensajes);
                if puntos > 0 && escritas < puntos {
                    informe.pocas_pruebas += 1;
                    mensajes.push(json!({"role": "user", "content": format!(
                        "«Qué se pide» de TASK.md tiene {puntos} puntos y has escrito {escritas} prueba(s) propia(s). Cada punto (y cada valor límite que menciona) necesita al menos una prueba tuya con valores concretos: vuelve a leer la lista, añade las pruebas que falten dentro de `mod pruebas_agente`, ejecútalas y corrige el código si alguna falla. Después escribe el informe final."
                    )}));
                    al_turno(e, &mensajes);
                    continue;
                }
            }
            if cfg.alertas_panico && hubo_edicion(&mensajes) {
                let pedido = herramientas::pide_sin_panico(e, pol);
                // Si la tarea no habla de pánico, sólo se avisa una vez y sólo de las operaciones que pueden desbordar.
                let tope = if pedido { cfg.avisos_panico } else { 1 };
                if informe.alertas_panico < tope {
                    let anadidas = revision::lineas_anadidas(e, &pol.raiz, &originales);
                    if let Some(alertas) = herramientas::alertas_de_panico(e, pol, &anadidas) {
                        informe.alertas_panico += 1;
                        let texto = if pedido {
                            format!("La tarea pide que no haya pánico ni desbordamiento. Clippy (con los lints `indexing_slicing` y `arithmetic_side_effects`) marca estas líneas que has añadido o cambiado como capaces de entrar en pánico o desbordar:\n\n{alertas}\n\nComprueba cada una: usa `.get(..)` en vez de indexar, `checked_add`/`checked_mul` (o `saturating_*`) en las cuentas con valores que vienen de los datos, y una comprobación de longitud ANTES de leer. Ojo con las sumas de rangos como `off..off + N`: calcula `let fin = off.checked_add(N).ok_or(..)?;` y usa `get(off..fin)`. Corrige TODAS las líneas de la lista (no sólo la primera) y verás de nuevo los avisos que queden. Después ejecuta las comprobaciones y escribe el informe final.")
                        } else {
                            format!("Clippy (`arithmetic_side_effects`) marca estas operaciones que has añadido o cambiado como capaces de DESBORDAR (las pruebas se compilan en `debug`, donde un desbordamiento es un pánico):\n\n{alertas}\n\nSi los operandos pueden ser grandes, ensancha el tipo antes de operar (por ejemplo `i16` → `i32`: `(a as i32 + b as i32) / 2`) o usa `checked_*`/`saturating_*`/`wrapping_*` según lo que pida la tarea. Si la operación no puede desbordar de verdad (un contador pequeño, un índice acotado), ignora el aviso. Después ejecuta las comprobaciones y escribe el informe final.")
                        };
                        mensajes.push(json!({"role": "user", "content": texto}));
                        al_turno(e, &mensajes);
                        continue;
                    }
                }
            }
            if cfg.proteger_api && api_avisos < cfg.avisos_api && hubo_edicion(&mensajes) {
                if let Some(lista) = herramientas::api_publica_alterada(e, &originales) {
                    api_avisos += 1;
                    mensajes.push(json!({"role": "user", "content": format!(
                        "Has quitado o cambiado la firma de ítems PÚBLICOS que ya existían (las pruebas de la tarea compilan contra la API original, así que esto las rompe):\n\n{lista}\n\nRestáuralos tal como estaban (mismo nombre, mismos parámetros y mismo tipo de retorno) salvo que TASK.md pida expresamente cambiarlos, y haz tu cambio por dentro. Si reescribiste el archivo entero, comprueba que no has perdido ninguna función. Después ejecuta las comprobaciones y escribe el informe final."
                    )}));
                    al_turno(e, &mensajes);
                    continue;
                }
            }
            if cfg.comprobar_al_terminar && informe.comprobaciones_rojas < cfg.avisos_comprobacion && hubo_edicion(&mensajes) {
                if let Some(rojo) = herramientas::comprobar_tarea(e, pol) {
                    informe.comprobaciones_rojas += 1;
                    mensajes.push(json!({"role": "user", "content": format!(
                        "Antes de terminar he ejecutado yo la comprobación de la tarea sobre tu cambio y FALLA:\n\n{rojo}\n\nCorrige el código (lee el tramo que señala el error antes de editar), vuelve a ejecutar la comprobación y, cuando pase, escribe el informe final."
                    )}));
                    al_turno(e, &mensajes);
                    continue;
                }
            }
            if cfg.revision && informe.revisiones < cfg.avisos_revision && hubo_edicion(&mensajes) {
                informe.revisiones += 1;
                if cfg.revisor {
                    let base = revision::contenido(e, &pol.raiz, &originales);
                    let cuerpo = modelo.completar(&revision::peticion_revisor(&cfg.modelo, &base, cfg.max_tokens), plazo);
                    if let Ok(c) = cuerpo {
                        match revision::critica(&c) {
                            Some(crit) => {
                                mensajes.push(json!({"role": "user", "content": format!(
                                    "Antes de dar el trabajo por terminado: un revisor independiente, que sólo ha visto los requisitos de TASK.md y tu diff, señala estos posibles fallos:\n\n{crit}\n\nPara cada punto: si es un fallo real, corrígelo y ejecuta las comprobaciones; si crees que ya está cubierto, indica en qué línea de tu diff. No repitas cambios que ya están hechos."
                                )}));
                                al_turno(e, &mensajes);
                                continue;
                            }
                            // Sin fallos que señalar: el agente termina.
                            None => {
                                informe.fin = Fin::Terminado;
                                informe.respuesta_final = texto;
                                al_turno(e, &mensajes);
                                break;
                            }
                        }
                    }
                }
                let r = revision::texto(e, &pol.raiz, &originales);
                mensajes.push(json!({"role": "user", "content": format!("Antes de dar el trabajo por terminado, revisa lo que has hecho.\n\n{r}")}));
                al_turno(e, &mensajes);
                continue;
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
                    // Llamada que no edita y que ya se ejecutó con éxito desde la última edición: su resultado no puede cambiar.
                    // Cuenta aunque no sea consecutiva (Gemma alternaba dos `cargo test` casi iguales); una llamada nueva reinicia la racha.
                    let edita = matches!(nombre, "reemplazar_texto" | "reemplazar_lineas" | "reemplazar_funcion" | "insertar");
                    let repite_ok = !edita && vistas.iter().any(|v| *v == clave);
                    if !edita {
                        if repite_ok { racha_ok += 1 } else { racha_ok = 0 }
                    }
                    seguidas = if repite { seguidas + 1 } else { 1 };
                    let veces_fallo = fallidas.get(&clave).copied().unwrap_or(0);
                    let mut bloqueada_ok = false;
                    let mut bloqueada = false;
                    let r = if cfg.repetidas_ok_max > 0 && repite_ok && racha_ok >= cfg.repetidas_ok_max {
                        bloqueada_ok = true;
                        bloqueada = true;
                        informe.repetidas += 1;
                        if racha_ok >= cfg.repetidas_ok_max + 2 {
                            atascado = true;
                        }
                        Respuesta { texto: aviso_de_repeticion_ok(nombre, racha_ok + 1), error: true }
                    } else if cfg.repetidas_max > 0 && ((repite && seguidas >= cfg.repetidas_max) || veces_fallo >= cfg.repetidas_max) {
                        bloqueada = true;
                        informe.repetidas += 1;
                        Respuesta { texto: aviso_de_repeticion(nombre, seguidas.max(veces_fallo + 1)), error: true }
                    } else {
                        let mut abs_edit: Option<String> = None;
                        if edita {
                            // Estado del archivo ANTES de su primera edición, para poder enseñarle después su propio diff.
                            let ruta = ["filePath", "file_path", "filepath", "file", "path", "archivo", "ruta"].iter().find_map(|k| a.get(*k).and_then(|v| v.as_str()));
                            if let Some(Ok((abs, _))) = ruta.map(|r| herramientas::resolver(pol, r)) {
                                if !originales.contains_key(&abs) {
                                    let antes = e.leer(&abs).ok().and_then(|d| String::from_utf8(d).ok());
                                    originales.insert(abs.clone(), antes);
                                }
                                abs_edit = Some(abs);
                            }
                        }
                        let mut r = if cfg.solo_lectura && !matches!(nombre, "read" | "grep" | "glob") {
                            Respuesta { texto: format!("«{nombre}» no está disponible para ti: eres el planificador y sólo puedes leer (read, grep, glob). Entrega el plan como texto."), error: true }
                        } else if nombre == "revisar" { Respuesta { texto: revision::texto(e, &pol.raiz, &originales), error: false } } else { ejecutar(e, pol, nombre, &a) };
                        if nombre == "bash" {
                            let rojos = nombres_de_pruebas_rojas(&r.texto);
                            if !rojos.is_empty() && rojos == rojos_previos {
                                rojos_seguidos += 1;
                            } else {
                                rojos_seguidos = if rojos.is_empty() { 0 } else { 1 };
                            }
                            if rojos_seguidos == 3 {
                                let lista: Vec<&str> = rojos.iter().take(5).map(|x| x.as_str()).collect();
                                r.texto.push_str(&format!("\n\nAVISO: es la tercera ejecución seguida con las MISMAS pruebas en rojo ({}). Si tus cambios no las mueven, el fallo está en otro sitio o la propia prueba está mal (el valor esperado). Antes de reescribir otra función, lee el mensaje de fallo de ESA prueba (`left`/`right`, o el panic) y razona qué línea concreta produce ese resultado; si la prueba contradice el enunciado, corrígela o elimínala.", lista.join(", ")));
                            }
                            rojos_previos = rojos;
                        }
                        if edita && !r.error {
                            if let Some(abs) = &abs_edit {
                                let h = huella(e.leer(abs).ok().as_deref());
                                let ini = originales.get(abs).map(|o| huella(o.as_deref().map(|t| t.as_bytes()))).unwrap_or(0);
                                let v = estados.entry(abs.clone()).or_insert_with(|| alloc::vec![ini]);
                                // El último elemento es el estado anterior a esta edición (una edición idéntica ya es un error): si el nuevo coincide
                                // con uno MÁS antiguo, la edición ha deshecho trabajo previo.
                                let n = v.len();
                                if n >= 2 && v[..n - 1].contains(&h) {
                                    oscilaciones += 1;
                                    r.texto.push_str("\n\nAVISO: este cambio devuelve el archivo a un estado en el que ya estuvo: has deshecho una edición anterior. Estás alternando entre dos versiones. Decide una, quédate con ella y pasa a otra cosa (otro punto de la tarea, o ejecuta las comprobaciones).");
                                    if oscilaciones >= 3 {
                                        atascado = true;
                                    }
                                }
                                v.push(h);
                            }
                        }
                        if repite && r.error {
                            r.texto.push_str(&format!("\n\nAVISO: es la misma llamada que acabas de hacer y ha fallado igual ({seguidas} veces). No la repitas: cambia lo que envías (ver la pista de arriba) o usa otra herramienta."));
                        }
                        r
                    };
                    // Una llamada bloqueada por repetirse con éxito sigue contando como «salió bien»: así se cuenta hasta cerrar.
                    if edita && !r.error {
                        sin_efecto_seguidos = 0;
                        fallidas.clear();
                        vistas.clear();
                        racha_ok = 0;
                    } else if !edita && !r.error && !bloqueada_ok && !vistas.contains(&clave) {
                        vistas.push(clave.clone());
                    }
                    // Una llamada que el guardián no llegó a ejecutar no gasta presupuesto de herramientas (el límite de turnos y el cierre
                    // por atasco siguen acotando el bucle).
                    if bloqueada || r.texto.contains("no cambia nada") || r.texto.contains("YA está aplicado") {
                        sin_efecto_seguidos += 1;
                    }
                    if bloqueada {
                        informe.llamadas = informe.llamadas.saturating_sub(1);
                        informe.bloqueadas += 1;
                        if informe.bloqueadas >= 8 {
                            atascado = true;
                        }
                    }
                    if r.error && !bloqueada {
                        *fallidas.entry(clave.clone()).or_insert(0) += 1;
                    }
                    ultima = Some((clave.0, clave.1, r.error && !bloqueada_ok));
                    r
                }
                None => Respuesta { texto: "Los argumentos de la llamada no son un objeto JSON válido. Vuelve a llamar con JSON correcto.".to_string(), error: true },
            };
            mensajes.push(json!({"role": "tool", "tool_call_id": id, "content": r.texto}));
        }
        al_turno(e, &mensajes);
        if atascado && informe.redirecciones < 2 && (!hubo_edicion(&mensajes) || sin_codigo_nuevo(e, &pol.raiz, &originales)) {
            // Releer lo mismo una y otra vez SIN haber editado nada (campaña 41: R32, R35 y R36 acabaron «sin cambios» tras seis lecturas idénticas): en
            // vez de cerrar el intento se le dice qué hacer ahora, con las herramientas que ya tiene, y se le da otra oportunidad.
            informe.redirecciones += 1;
            atascado = false;
            informe.bloqueadas = 0;
            vistas.clear();
            fallidas.clear();
            racha_ok = 0;
            let texto = if hubo_edicion(&mensajes) { REDIRECCION_SOLO_PRUEBAS } else { REDIRECCION_SIN_EDITAR };
            mensajes.push(json!({"role": "user", "content": texto}));
            al_turno(e, &mensajes);
            continue;
        }
        if atascado {
            // Repite sin cesar una llamada que ya salió bien: no hay nada más que averiguar. Se cierra el intento y decide el validador.
            informe.fin = Fin::Terminado;
            informe.respuesta_final = String::from("(intento cerrado: el modelo repetía llamadas o alternaba ediciones sin avanzar)");
            break;
        }
    }
    if cfg.exigir_pruebas && cfg.podar_pruebas_rojas && hubo_edicion(&mensajes) && informe.fin != Fin::ErrorModelo {
        let archivos: Vec<String> = originales.keys().cloned().collect();
        let anadidas = revision::lineas_anadidas(e, &pol.raiz, &originales);
        informe.podadas = herramientas::podar_pruebas_rojas(e, pol, &archivos, &anadidas);
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
2. Localiza el código. Si el archivo es corto (menos de 150 líneas) lee entero; si es largo, **no lo recorras por tramos**: llama primero a `grep` con el nombre de la función o un texto que aparezca en ella y después a `read` sólo del tramo que rodea la línea (`offset` y `limit`). Antes de editar una función, léela ENTERA (`read` con `simbolo`), no sólo el tramo que te llamó la atención: una comprobación de entrada (versión, longitud, contador, rango, puerto) va al PRINCIPIO de la función, justo tras leer ese dato y antes de cualquier rama que lo use; si la pones dentro de un `else` o un `match` sólo vale para esa rama.\n\
3. Para **cambiar** algo que ya existe, llama a `reemplazar_texto` con `filePath`, `buscar` (un fragmento corto de lo que leíste; los espacios no importan) y `nuevo`. Para **añadir** código nuevo (una función, una rama), llama a `insertar` con `filePath`, `linea` (el número que te mostró `read`; se inserta después de esa línea) y `texto`. Siempre bloques completos: con su `{` y su `}`. Cambio mínimo: no toques otros archivos. Para reescribir una función ENTERA usa `reemplazar_funcion` (`nombre`, `nuevo`): no hay números de línea ni nada que copiar (`read` con `esquema` lista las funciones; con `simbolo` lees una entera). Si `reemplazar_texto` no encuentra el fragmento, cambia ese tramo por números de línea con `reemplazar_lineas` (`desde`, `hasta`, `nuevo`). Si una herramienta dice que aparece varias veces o que desequilibra las llaves, corrige y repite.\n\
4. Llama a `bash` con las comprobaciones de TASK.md (`cargo test …`) y lee el resultado.\n\
5. Antes de terminar, **relee TASK.md punto por punto**: cada comportamiento que describe tiene que estar cubierto por tu cambio, no sólo el caso que ya ves fallar. Si falta alguno, impleméntalo y vuelve a comprobar. Una tarea sin ningún archivo editado no está terminada.\n\
6. Informe final en texto corto: qué cambiaste, qué comprobaciones corrieron y su salida resumida, y lo que no pudiste hacer. No inventes resultados.";

/// Variante de [`SISTEMA`] que obliga a convertir cada caso límite del enunciado en una **prueba propia** antes de implementar
/// (campaña 14): las mismas tareas se atascaban con tres modelos distintos en los casos límite de la prueba reservada.
pub const SISTEMA_PRUEBAS: &str = "Eres el agente de mejora de soso. Trabajas sólo dentro del checkout actual.\n\n\
PROTOCOLO (obligatorio):\n\
- En cada respuesta escribes **una sola llamada a una herramienta y nada más**: sin texto antes ni después.\n\
- El resultado de la herramienta llega en el mensaje siguiente. **Nunca lo inventes**: espera a recibirlo.\n\
- No des por leído un archivo que no hayas leído con `read`; no edites sin haber leído antes el archivo.\n\
- Sólo cuando hayas terminado todos los pasos respondes con texto, y entonces es el informe final.\n\n\
PASOS:\n\
1. Llama a `read` con `TASK.md` (raíz del checkout): contiene el objetivo, el archivo que puedes tocar y las comprobaciones.\n\
2. Localiza el código. Si el archivo es corto (menos de 150 líneas) lee entero; si es largo, **no lo recorras por tramos**: llama primero a `grep` con el nombre de la función o un texto que aparezca en ella y después a `read` sólo del tramo que rodea la línea (`offset` y `limit`). Antes de editar una función, léela ENTERA (`read` con `simbolo`), no sólo el tramo que te llamó la atención: una comprobación de entrada (versión, longitud, contador, rango, puerto) va al PRINCIPIO de la función, justo tras leer ese dato y antes de cualquier rama que lo use; si la pones dentro de un `else` o un `match` sólo vale para esa rama.\n\
3. **Escribe primero tus pruebas.** De la sección «Qué se pide» de TASK.md saca TODOS los comportamientos y casos límite: cada viñeta, cada valor raro, cada «en caso de empate», «nunca», «si todos…». Añade al final del MISMO archivo que puedes tocar (con `insertar` y `linea`: -1, que significa «al final del archivo»; no hace falta leer el final) un módulo `#[cfg(test)] mod pruebas_agente { use super::*; … }` con **una prueba `#[test]` por caso**, con valores concretos y la salida exacta que pide el enunciado. Ejecútalas con `bash` (`cargo test …` con el filtro `pruebas_agente`) y comprueba que las que describen el fallo de hoy **fallan**.\n\
4. Para **cambiar** algo que ya existe, llama a `reemplazar_texto` con `filePath`, `buscar` (un fragmento corto de lo que leíste; los espacios no importan) y `nuevo`. Para **añadir** código nuevo, llama a `insertar` con `filePath`, `linea` y `texto`. Siempre bloques completos: con su `{` y su `}`. Cambio mínimo en el código; no toques otros archivos. Para reescribir una función ENTERA usa `reemplazar_funcion` (`nombre`, `nuevo`): no hay números de línea ni nada que copiar (`read` con `esquema` lista las funciones; con `simbolo` lees una entera). Si `reemplazar_texto` no encuentra el fragmento, cambia ese tramo por números de línea con `reemplazar_lineas` (`desde`, `hasta`, `nuevo`). Si una herramienta dice que aparece varias veces o que desequilibra las llaves, corrige y repite.\n\
5. Ejecuta con `bash` las comprobaciones de TASK.md **y** tus pruebas: todas en verde. Si alguna falla, corrige el CÓDIGO (no borres la prueba salvo que contradiga el enunciado).\n\
6. Antes de terminar, repasa el enunciado punto por punto: ¿hay una prueba tuya por cada uno? Si falta alguno, añádelo, ejecútalo y corrige.\n\
7. Informe final en texto corto: qué cambiaste, qué pruebas escribiste, qué comprobaciones corrieron y su salida resumida, y lo que no pudiste hacer. No inventes resultados.";

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
                "reemplazar_texto" | "reemplazar_lineas" | "reemplazar_funcion" | "insertar" if c.starts_with("Hecho") => {
                    sin_comprobar = true;
                    ultima_fallo = false;
                }
                // Un `bash` que el guardián no llegó a ejecutar no es una comprobación (ni verde ni roja): no cambia el estado.
                "bash" | "forja" if c.starts_with("No he ejecutado") => {}
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

/// Lo que se le dice al modelo que intenta terminar sin haber cambiado nada.
const REDIRECCION_SOLO_PRUEBAS: &str = "Llevas varias llamadas bloqueadas y lo único que has escrito son comentarios o un módulo de pruebas vacío: el CÓDIGO de la tarea sigue sin cambiar. Haz ahora, sin volver a leer lo mismo: cambia la función que pide la tarea con `reemplazar_funcion` (`nombre` = la función, `nuevo` = su definición completa) o con `reemplazar_texto`, y después ejecuta las comprobaciones. Si dudas de un detalle, haz el cambio mínimo: el compilador o las pruebas te dirán qué falta.";

/// ¿Lo añadido hasta ahora son sólo comentarios, líneas en blanco o el módulo `pruebas_agente` (ningún código de la tarea)?
fn sin_codigo_nuevo<E: Entorno>(e: &mut E, raiz: &str, originales: &alloc::collections::BTreeMap<String, Option<String>>) -> bool {
    let anadidas = revision::lineas_anadidas(e, raiz, originales);
    for (rel, set) in &anadidas {
        let Ok(datos) = e.leer(&format!("{}/{}", raiz.trim_end_matches('/'), rel)) else { continue };
        let texto = String::from_utf8_lossy(&datos).into_owned();
        let lineas: Vec<&str> = texto.lines().collect();
        let pruebas = lineas.iter().position(|l| l.trim_start().starts_with("mod pruebas_agente")).map(|i| i.saturating_sub(1)).unwrap_or(usize::MAX);
        for n in set {
            let i = n - 1;
            let t = lineas.get(i).map(|l| l.trim()).unwrap_or("");
            if i < pruebas && !t.is_empty() && !t.starts_with("//") {
                return false;
            }
        }
    }
    true
}

const REDIRECCION_SIN_EDITAR: &str = "Llevas varias lecturas idénticas y NINGUNA edición: ya has leído lo suficiente. Haz ahora, sin volver a leer lo mismo: (1) si la tarea te pide pruebas propias, añade tu módulo `#[cfg(test)] mod pruebas_agente { use super::*; … }` con `insertar` (`linea`: -1 = al final del archivo) con una prueba por punto de «Qué se pide»; (2) cambia el código con `reemplazar_funcion` (`nombre` de la función, `nuevo` = su definición completa) o con `reemplazar_texto`. Si dudas de un detalle, haz el cambio mínimo y ejecuta las comprobaciones: el error del compilador te lo dirá.";

const AVISO_SIN_EDITAR: &str = "No has modificado ningún archivo, y esta tarea exige un cambio en el código. Vuelve a leer TASK.md: busca el comportamiento que hoy falla, localízalo en el archivo que puedes tocar y corrígelo con `reemplazar_texto` o `insertar`. Después ejecuta las comprobaciones. No termines sin haber editado.";

/// Lo que se le dice al modelo que quiere terminar sin haber escrito ninguna prueba propia.
const AVISO_SIN_PRUEBAS: &str = "No has escrito ninguna prueba propia. Vuelve a leer la sección «Qué se pide» de TASK.md y haz una lista de cada comportamiento y caso límite que menciona (cada viñeta, cada valor raro, cada empate, cada «nunca» y cada «si todos…»). Añade al final del MISMO archivo que puedes tocar un módulo `#[cfg(test)] mod pruebas_agente { use super::*; … }` con una prueba `#[test]` por caso, con valores concretos; ejecútalas con `bash` y corrige el código hasta que todas pasen. Después escribe el informe final.";

const AVISO_PRUEBAS_SIN_EJECUTAR: &str = "Has escrito pruebas propias, pero tras tu ÚLTIMO cambio no las has ejecutado en verde. Ejecútalas ahora con `bash` (`cargo test -p CRATE --features std -- pruebas_agente`, con el CRATE y las opciones que uses para las comprobaciones de TASK.md): tienen que ejecutarse (si el filtro no coincide con ninguna, el resultado no demuestra nada) y pasar. Si alguna falla, corrige el CÓDIGO. Después escribe el informe final.";

/// Nombres de las pruebas que `cargo test` da por falladas (`test x::y ... FAILED` o `x --- FAILED`).
pub(crate) fn nombres_de_pruebas_rojas(texto: &str) -> alloc::collections::BTreeSet<String> {
    let mut v = alloc::collections::BTreeSet::new();
    for l in texto.lines() {
        let t = l.trim();
        if let Some(r) = t.strip_prefix("test ") {
            if let Some(n) = r.strip_suffix(" ... FAILED") {
                v.insert(n.trim().to_string());
            }
        } else if let Some(n) = t.strip_suffix(" --- FAILED") {
            v.insert(n.trim().to_string());
        }
    }
    v
}

/// Cuántas pruebas `#[test]` ha escrito el modelo con ediciones que salieron bien.
fn pruebas_propias_escritas(mensajes: &[Value]) -> usize {
    let mut candidatas: Vec<(String, usize)> = Vec::new();
    let mut total = 0;
    for m in mensajes {
        if m["role"] == "assistant" {
            for l in m["tool_calls"].as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
                let nombre = l.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("");
                let args = match l.pointer("/function/arguments") {
                    Some(Value::String(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => String::new(),
                };
                if matches!(nombre, "insertar" | "reemplazar_texto" | "reemplazar_lineas" | "reemplazar_funcion") {
                    candidatas.push((l["id"].as_str().unwrap_or("").to_string(), args.matches("#[test]").count()));
                }
            }
        } else if m["role"] == "tool" {
            let id = m["tool_call_id"].as_str().unwrap_or("");
            if let Some((_, n)) = candidatas.iter().find(|(c, _)| c == id) {
                if m["content"].as_str().unwrap_or("").starts_with("Hecho") {
                    total += n;
                }
            }
        }
    }
    total
}

/// ¿Hay, tras la última edición que salió bien, un `cargo test` con el filtro `pruebas_agente` que acabó con exit code 0 y sí ejecutó pruebas?
fn pruebas_propias_en_verde(mensajes: &[Value]) -> bool {
    let mut llamadas: Vec<(String, String, String)> = Vec::new(); // id, nombre, argumentos
    let mut verde = false;
    for m in mensajes {
        if m["role"] == "assistant" {
            for l in m["tool_calls"].as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
                let args = match l.pointer("/function/arguments") {
                    Some(Value::String(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => String::new(),
                };
                llamadas.push((l["id"].as_str().unwrap_or("").to_string(), l.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("").to_string(), args));
            }
        } else if m["role"] == "tool" {
            let id = m["tool_call_id"].as_str().unwrap_or("");
            let c = m["content"].as_str().unwrap_or("");
            let Some((_, nombre, args)) = llamadas.iter().rev().find(|(i, _, _)| i == id) else { continue };
            match nombre.as_str() {
                "reemplazar_texto" | "reemplazar_lineas" | "reemplazar_funcion" | "insertar" if c.starts_with("Hecho") => verde = false,
                "bash" if args.contains("pruebas_agente") && c.contains("exit code: 0") && !c.contains("NINGUNA prueba") => verde = true,
                _ => {}
            }
        }
    }
    verde
}

/// ¿Ha insertado el modelo alguna prueba propia (`#[test]`) con éxito?
fn hubo_prueba_propia(mensajes: &[Value]) -> bool {
    let mut candidatas: Vec<String> = Vec::new();
    for m in mensajes {
        if m["role"] == "assistant" {
            for l in m["tool_calls"].as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
                let nombre = l.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("");
                let args = match l.pointer("/function/arguments") {
                    Some(Value::String(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => String::new(),
                };
                if matches!(nombre, "insertar" | "reemplazar_texto" | "reemplazar_lineas" | "reemplazar_funcion") && args.contains("#[test]") {
                    candidatas.push(l["id"].as_str().unwrap_or("").to_string());
                }
            }
        } else if m["role"] == "tool" {
            let id = m["tool_call_id"].as_str().unwrap_or("");
            if candidatas.iter().any(|c| c == id) && m["content"].as_str().unwrap_or("").starts_with("Hecho") {
                return true;
            }
        }
    }
    false
}

/// ¿Ha logrado el modelo editar algún archivo (`reemplazar_texto` o `insertar` con resultado «Hecho…»)?
pub fn hubo_edicion(mensajes: &[Value]) -> bool {
    // Con los mensajes viejos plegados (`limitar_mensajes`) la respuesta «Hecho…» ya no está: sólo queda su línea del historial. Sin esto, una edición
    // hecha antes del primer plegado «desaparecía» (R33 de la campaña 47: acabó en `LimitePasos` sin validar y sin podar, con la edición en disco).
    if let Some(h) = mensajes.iter().filter_map(|m| m["content"].as_str()).find_map(|c| c.find(MARCA_HISTORIAL).map(|i| &c[i..])) {
        if h.lines().any(|l| ["- insertar ", "- reemplazar_texto ", "- reemplazar_lineas ", "- reemplazar_funcion "].iter().any(|p| l.starts_with(p)) && l.contains("→ Hecho")) {
            return true;
        }
    }
    let mut nombres: Vec<(String, String)> = Vec::new();
    for m in mensajes {
        if m["role"] == "assistant" {
            for l in m["tool_calls"].as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
                nombres.push((l["id"].as_str().unwrap_or("").to_string(), l.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("").to_string()));
            }
        } else if m["role"] == "tool" {
            let id = m["tool_call_id"].as_str().unwrap_or("");
            let nombre = nombres.iter().rev().find(|(i, _)| i == id).map(|(_, n)| n.as_str()).unwrap_or("");
            if matches!(nombre, "reemplazar_texto" | "reemplazar_lineas" | "reemplazar_funcion" | "insertar") && m["content"].as_str().unwrap_or("").starts_with("Hecho") {
                return true;
            }
        }
    }
    false
}

/// Qué decirle al modelo que repite una llamada que ya salió bien (su resultado no puede cambiar).
/// Huella (FNV-1a) del contenido de un archivo; `None` (no existe) tiene la suya.
fn huella(datos: Option<&[u8]>) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    match datos {
        None => h ^= 0xff,
        Some(d) => {
            for b in d {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        }
    }
    h
}

fn aviso_de_repeticion_ok(herramienta: &str, veces: usize) -> String {
    format!("No he ejecutado esta llamada de `{herramienta}`: es idéntica a la anterior ({} veces seguidas) y como no has cambiado nada entre medias daría el mismo resultado. Si las comprobaciones de la tarea ya pasan y has cubierto lo que pide TASK.md, escribe ahora el informe final (texto, sin llamada). Si falta algo, edita con `reemplazar_texto` o `insertar` antes de volver a comprobar.", veces - 1)
}

/// Qué decirle al modelo cuando repite una llamada que ya falló: no se ejecuta, y se le da una vía distinta.
fn aviso_de_repeticion(herramienta: &str, veces: usize) -> String {
    let consejo = match herramienta {
        "reemplazar_texto" => "Vuelve a leer el tramo exacto con `read` (con `offset` y `limit`) y copia en `buscar` UNA sola línea completa y corta, sin los números de línea que muestra `read`. Para añadir código nuevo usa `insertar` con el número de línea.",
        "reemplazar_funcion" => "Mira los nombres con `read` y `esquema`; `nuevo` debe ser la definición COMPLETA (firma, cuerpo y su última llave) y equilibrada.",
        "reemplazar_lineas" => "Comprueba el rango con `read` (con `offset` y `limit`): `desde` y `hasta` son números de línea, ambos incluidos, y `nuevo` debe traer bloques completos con sus llaves.",
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

/// Marca del historial de lo ya hecho que se anota en la tarea al plegar intercambios antiguos.
const MARCA_HISTORIAL: &str = "\n\n[Historial de lo que ya hiciste (los mensajes antiguos se han plegado para no pasar del límite; los cambios SIGUEN aplicados en los archivos, no los repitas):";

/// Una línea de historial por llamada plegada: herramienta, archivo/rango/orden y cómo acabó.
fn linea_historial(llamada: &Value, resultado: &str) -> String {
    let nombre = llamada.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("?");
    let args: Value = match llamada.pointer("/function/arguments") {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    };
    let texto = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("");
    let num = |k: &str| args.get(k).and_then(|v| v.as_u64());
    let corto = |t: &str, n: usize| {
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.chars().count() > n { format!("{}…", t.chars().take(n).collect::<String>()) } else { t }
    };
    let donde = match nombre {
        "reemplazar_funcion" => format!("{} función «{}»", texto("filePath"), corto(texto("nombre"), 50)),
        "reemplazar_lineas" => format!("{} líneas {}–{}", texto("filePath"), num("desde").unwrap_or(0), num("hasta").unwrap_or(0)),
        "reemplazar_texto" => format!("{} «{}»", texto("filePath"), corto(texto("buscar"), 50)),
        "insertar" => format!("{} línea {}", texto("filePath"), num("linea").map_or("?".to_string(), |n| n.to_string())),
        "bash" => corto(texto("command"), 70),
        "read" => format!("{} desde {}", texto("filePath"), num("offset").unwrap_or(1)),
        "grep" | "glob" => format!("«{}»", corto(texto("pattern"), 50)),
        _ => String::new(),
    };
    let fin = resultado.lines().next().unwrap_or("");
    format!("- {nombre} {donde} → {}", corto(fin, 70))
}

/// Mantiene la conversación en `max` mensajes plegando los intercambios más antiguos: una respuesta del asistente con sus llamadas y
/// los resultados que la siguen (siempre completos, para que ningún resultado quede huérfano). El sistema, la tarea y los últimos
/// mensajes no se tocan. Lo plegado no se pierde sin más: queda una línea por llamada en la tarea, para que el modelo sepa qué ya
/// hizo (con un simple recorte, en la campaña 27 olvidaba sus ediciones y las repetía hasta agotar los turnos). Devuelve cuántos
/// intercambios se pliegan.
fn limitar_mensajes(mensajes: &mut Vec<Value>, max: usize, holgura: usize) -> usize {
    const HISTORIAL_MAX: usize = 40;
    if mensajes.len() <= max {
        return 0;
    }
    // Se pliega hasta `objetivo` (no sólo hasta `max`): así no hace falta volver a plegar en los turnos siguientes.
    let objetivo = max.saturating_sub(holgura).max(6.min(max));
    let mut nuevas: Vec<String> = Vec::new();
    let mut quitados = 0;
    while mensajes.len() > objetivo {
        let Some(i) = (2..mensajes.len()).find(|&i| mensajes[i]["role"] == "assistant") else { break };
        let mut fin = i + 1;
        while fin < mensajes.len() && mensajes[fin]["role"] == "tool" {
            fin += 1;
        }
        if mensajes.len() - fin < 4 {
            break;
        }
        if let Some(llamadas) = mensajes[i]["tool_calls"].as_array() {
            for ll in llamadas {
                let id = ll.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let res = mensajes[i + 1..fin].iter().find(|m| m["tool_call_id"] == id).and_then(|m| m["content"].as_str()).unwrap_or("");
                nuevas.push(linea_historial(ll, res));
            }
        }
        mensajes.drain(i..fin);
        quitados += 1;
    }
    if quitados > 0 && mensajes.len() > 1 {
        if let Some(t) = mensajes[1]["content"].as_str() {
            let (cuerpo, previo) = match t.split_once(MARCA_HISTORIAL) {
                Some((c, h)) => (c.to_string(), h.trim_end_matches(']').lines().filter(|l| l.starts_with("- ")).map(|l| l.to_string()).collect::<Vec<_>>()),
                None => (t.to_string(), Vec::new()),
            };
            let mut todas = previo;
            todas.extend(nuevas);
            // Las ediciones que salieron bien no se olvidan nunca (hasta 20): son lo que `hubo_edicion` y el modelo necesitan saber que ya está hecho.
            let es_edicion = |l: &String| ["- insertar ", "- reemplazar_texto ", "- reemplazar_lineas ", "- reemplazar_funcion "].iter().any(|p| l.starts_with(p)) && l.contains("→ Hecho");
            let (ediciones, otras): (Vec<String>, Vec<String>) = todas.into_iter().partition(es_edicion);
            let ediciones: Vec<String> = ediciones.into_iter().rev().take(20).rev().collect();
            let hueco = HISTORIAL_MAX.saturating_sub(ediciones.len());
            let exceso = otras.len().saturating_sub(hueco);
            let lista = otras[exceso..].iter().chain(ediciones.iter()).cloned().collect::<Vec<_>>().join("\n");
            mensajes[1]["content"] = Value::String(format!("{cuerpo}{MARCA_HISTORIAL}\n{lista}]"));
        }
    }
    quitados
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
