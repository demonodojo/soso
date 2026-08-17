//! `ask`: preguntar al modelo escribiendo el texto tal cual.
//!
//! Dos modos, los dos sin una sola línea de diagnóstico:
//!
//! - `ask <pregunta>` — una respuesta y a otra cosa.
//! - `ask` — prompt propio `?> `, una pregunta por línea. Aquí la shell no
//!   interviene en absoluto (el texto se lee directo de la tty), y el modelo se
//!   carga **una vez** para todas las preguntas de la sesión.
//!
//! Qué modelo se usa y con qué límite sale de `/etc/llm.conf`; si falta, del
//! primero que haya en `/models`. No hay flags: todo lo que va detrás de `ask`
//! es texto para el modelo, y ese es justamente el punto.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use libsoso::linea::Lector;
use libsoso::{abi, println, sys};
use soso_llm_core::chat;
use soso_llm_core::plan::MemoryPlanConfig;
use soso_llm_core::sample::Sampler;

use crate::{generar_tokens, preparar_sesion, Sesion};

const CONF: &str = "/etc/llm.conf";
const PROMPT: &str = "?> ";

/// Si `args` es `cmd` o empieza por `cmd `, devuelve el resto **literal** (solo
/// se recorta el espacio del final, que sobra siempre).
pub fn resto_tras<'a>(cmd: &str, args: &'a str) -> Option<&'a str> {
    let args = args.trim_start();
    if args == cmd {
        return Some("");
    }
    args.strip_prefix(cmd)
        .filter(|r| r.starts_with(' '))
        .map(|r| r[1..].trim_end())
}

pub struct Conf {
    pub modelo: String,
    pub max: usize,
    pub temp: f32,
    pub top_p: f32,
    pub seed: u64,
    /// Plantilla de chat, con tres valores posibles y por eso una sola clave:
    /// vacía = la que traiga el modelo (lo normal), `crudo` = ninguna, y
    /// cualquier otra cosa = ésa. Ver `plantilla_efectiva`.
    pub plantilla: String,
}

/// Valor de `plantilla=` que apaga la plantilla del modelo.
pub const PLANTILLA_CRUDA: &str = "crudo";

impl Default for Conf {
    fn default() -> Self {
        Conf {
            modelo: String::new(),
            max: 128,
            temp: 0.7,
            top_p: 0.9,
            seed: 42,
            plantilla: String::new(),
        }
    }
}

/// Qué plantilla usar de verdad, cruzando la conf con la del modelo.
///
/// Un modelo de chat al que se le manda el texto pelado no ve una conversación:
/// ve un fragmento de corpus y lo continúa (era la ensalada de palabras de
/// `ask hola` en la placa). Y al revés, meterle marcadores a un modelo que no los
/// vio al entrenar también es ruido — de ahí que esto pueda contestar «ninguna».
///
/// El byte-level de reserva no tiene tokens de verdad, así que ahí `<|user|>` son
/// nueve bytes de basura y la plantilla se ignora aunque esté puesta.
pub fn plantilla_efectiva<'a>(conf: &'a Conf, sesion: &'a Sesion) -> &'a str {
    if !sesion.bundle.tokenizer.tiene_vocabulario() {
        return "";
    }
    match conf.plantilla.as_str() {
        PLANTILLA_CRUDA => "",
        "" => sesion.bundle.rt.manifest.chat_template.as_str(),
        otra => otra,
    }
}

/// Lee `/etc/llm.conf`. Mismo formato que `wifi.conf`: `clave=valor`, una por
/// línea, saltando vacías y comentarios.
pub fn leer_conf() -> Conf {
    let mut c = Conf::default();
    let Some(texto) = leer_fichero(CONF) else {
        return c;
    };
    for linea in texto.lines() {
        let linea = linea.trim();
        if linea.is_empty() || linea.starts_with('#') {
            continue;
        }
        if let Some(v) = linea.strip_prefix("modelo=") {
            c.modelo = v.trim().to_string();
        } else if let Some(v) = linea.strip_prefix("max=") {
            if let Ok(n) = v.trim().parse() {
                c.max = n;
            }
        } else if let Some(v) = linea.strip_prefix("temp=") {
            if let Ok(n) = v.trim().parse() {
                c.temp = n;
            }
        } else if let Some(v) = linea.strip_prefix("top_p=") {
            if let Ok(n) = v.trim().parse() {
                c.top_p = n;
            }
        } else if let Some(v) = linea.strip_prefix("seed=") {
            if let Ok(n) = v.trim().parse() {
                c.seed = n;
            }
        } else if let Some(v) = linea.strip_prefix("plantilla=") {
            // Sin `trim` por la derecha: las plantillas acaban en `\n` a
            // propósito (el turno del asistente empieza en línea nueva) y
            // recortarlo cambiaría lo que ve el modelo.
            c.plantilla = chat::desescapar(v.trim_start());
        }
    }
    c
}

pub fn leer_fichero(path: &str) -> Option<String> {
    let fd = sys::open(path, abi::O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut out = String::new();
    let mut buf = [0u8; 512];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n <= 0 {
            break;
        }
        out.push_str(core::str::from_utf8(&buf[..n as usize]).unwrap_or(""));
    }
    sys::close(fd as u64);
    Some(out)
}

/// Nombres de `/models`, que es un directorio sintético sobre el catálogo de
/// sosomfs: una entrada por modelo montado.
pub fn modelos() -> Vec<String> {
    let mut out = Vec::new();
    let fd = sys::open("/models", abi::O_RDONLY);
    if fd < 0 {
        return out;
    }
    let mut ents = [abi::Dirent::default(); 16];
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n <= 0 {
            break;
        }
        for d in &ents[..n as usize / abi::DIRENT_SIZE] {
            if let Ok(nombre) = core::str::from_utf8(d.name_bytes()) {
                out.push(nombre.to_string());
            }
        }
    }
    sys::close(fd as u64);
    out
}

/// El modelo configurado si existe; si no, el primero que haya.
///
/// Sin `modelo=` en la config no hay aviso ninguno: es el modo por defecto, y
/// el primero de `/models` es el bueno — `mkfs_models_live` mete el modelo de
/// verdad delante de los sintéticos y `read_dir` conserva ese orden. El aviso
/// se reserva para cuando alguien SÍ pidió un modelo y no está, que eso sí es
/// una configuración equivocada.
pub fn modelo_efectivo(conf: &Conf) -> Option<String> {
    let disponibles = modelos();
    if !conf.modelo.is_empty() && disponibles.iter().any(|m| *m == conf.modelo) {
        return Some(conf.modelo.clone());
    }
    if !conf.modelo.is_empty() {
        println!(
            "ask: el modelo «{}» de {CONF} no está en /models; uso el primero",
            conf.modelo
        );
    }
    disponibles.into_iter().next()
}

pub fn run_ask(texto: &str) -> u8 {
    // `:eco` va lo PRIMERO: enseña el texto exactamente como llegó y no
    // necesita modelo ninguno. Estaba después de resolver la configuración, y
    // eso colaba por delante avisos como «el modelo X no está en /models» —
    // justo el ruido que este modo existe para no tener.
    if let Some(t) = resto_tras(":eco", texto) {
        println!("{t}");
        return 0;
    }

    let conf = leer_conf();
    let Some(modelo) = modelo_efectivo(&conf) else {
        println!("ask: no hay ningún modelo en /models");
        return 1;
    };

    // Una línea antes de cargar, siempre. `ask` calla el diagnóstico, pero
    // callarse también **durante la carga** es lo que hace que en un pendrive
    // lento parezca colgado: el modelo se lee del disco y eso puede tardar,
    // sin nada en pantalla desde que se pulsa Enter.
    println!("ask: cargando {modelo}…");
    let mut sesion = match preparar_sesion(&modelo, false, MemoryPlanConfig::default(), false) {
        Ok(s) => s,
        Err(c) => {
            println!("ask: no pude cargar «{modelo}» (código {c})");
            return c;
        }
    };
    if texto.is_empty() {
        repl(&mut sesion, &modelo, conf)
    } else {
        responder(&mut sesion, texto, &conf)
    }
}

fn responder(sesion: &mut Sesion, texto: &str, conf: &Conf) -> u8 {
    // Los tokens que se van a generar de verdad, plantilla incluida: el chequeo
    // de abajo tiene que mirar éstos y no el texto pelado.
    let plantilla = plantilla_efectiva(conf, sesion);
    let tokens = chat::render(plantilla, texto, &sesion.bundle.tokenizer);
    // Los modelos sintéticos pequeños tienen vocabularios de juguete
    // (`tiny-moe` son 64 tokens) y el tokenizador de reserva es byte-level:
    // cualquier letra normal se sale de rango y la inferencia muere con un
    // escueto «inferencia falló» que no dice de qué. Mejor avisar aquí.
    let vocab = sesion.bundle.rt.manifest.vocab_size;
    if let Some(t) = tokens.iter().find(|&&t| t >= vocab) {
        println!(
            "ask: este modelo sólo entiende {vocab} tokens y el texto usa el {t}; \
             prueba con otro modelo (ask-modelo)"
        );
        return 1;
    }
    let mut sampler = Sampler::new(conf.temp, conf.top_p, conf.seed);
    generar_tokens(sesion, &tokens, conf.max, &mut sampler, false, None)
}

/// Bucle de preguntas. Al leer de la tty directamente, el texto no pasa por
/// ningún parseo: aquí `|`, `>` y las comillas son caracteres y ya está.
fn repl(sesion: &mut Sesion, modelo: &str, mut conf: Conf) -> u8 {
    let mut modelo = modelo.to_string();
    println!("ask: modelo {modelo}, máx {} tokens", conf.max);
    println!("ask: escribe la pregunta; «salir» o Ctrl-D para terminar");
    println!("ask: :modelo <n>  :max <n>  :eco <texto>  :modelos");
    let mut lector = Lector::new();
    loop {
        libsoso::print!("{PROMPT}");
        let Some(linea) = lector.siguiente() else {
            return 0;
        };
        let texto = linea.trim();
        if texto.is_empty() {
            continue;
        }
        if texto == "salir" || texto == "exit" {
            return 0;
        }
        if let Some(t) = resto_tras(":eco", texto) {
            println!("{t}");
            continue;
        }
        if texto == ":modelos" {
            for m in modelos() {
                let marca = if m == modelo { '*' } else { ' ' };
                println!("{marca} {m}");
            }
            continue;
        }
        if let Some(n) = resto_tras(":modelo", texto) {
            let n = n.trim();
            if n.is_empty() {
                println!("ask: modelo actual: {modelo}");
                continue;
            }
            if !modelos().iter().any(|m| m == n) {
                println!("ask: no hay ningún modelo «{n}» en /models");
                continue;
            }
            println!("ask: cargando {n}…");
            match preparar_sesion(n, false, MemoryPlanConfig::default(), false) {
                Ok(nueva) => {
                    *sesion = nueva;
                    modelo = n.to_string();
                    println!("ask: modelo {modelo}");
                }
                Err(c) => println!("ask: no pude cargar «{n}» (código {c})"),
            }
            continue;
        }
        if let Some(n) = resto_tras(":max", texto) {
            match n.trim().parse() {
                Ok(v) => {
                    conf.max = v;
                    println!("ask: máx {v} tokens");
                }
                Err(_) => println!("ask: :max necesita un número"),
            }
            continue;
        }
        responder(sesion, texto, &conf);
    }
}
