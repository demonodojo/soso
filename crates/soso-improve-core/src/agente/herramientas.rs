//! Herramientas del agente nativo (T35, D10): leer, buscar, listar, editar y ejecutar.
//!
//! Todo entra por los traits de [`crate::entorno`], así que el mismo código corre en el host y
//! dentro de soso. El contrato es el de los sustitutos que ya existen (N-009 `grep`, N-010 `sosh`):
//! **ninguna herramienta calla**. Un agente no puede sospechar de una respuesta ni volver a mirar con
//! otros ojos; por eso «no hay» y «no pude» son respuestas distintas, una expresión regular se
//! rechaza en vez de buscarse literalmente, y un operador de shell se rechaza explicando qué hacer.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{json, Value};

use crate::entorno::{Entorno, Orden, Tipo};
use crate::{Error, Resultado};

/// Qué puede hacer el agente y con qué límites.
#[derive(Debug, Clone)]
pub struct Politica {
    /// Raíz del checkout. Ninguna herramienta sale de aquí.
    pub raiz: String,
    /// Prefijos de `argv` que la herramienta `bash` acepta (sin shell, sin operadores).
    pub ejecutables: Vec<Vec<String>>,
    /// Tope en bytes de lo que devuelve una herramienta al modelo.
    pub salida_max: usize,
    /// Segundos que puede tardar una orden externa.
    pub plazo_orden_s: u32,
    /// Líneas que devuelve `read` por defecto.
    pub lineas_por_defecto: usize,
    /// Coincidencias máximas de `grep` y entradas máximas de `glob`.
    pub resultados_max: usize,
    /// Variables que se **añaden** al entorno de las órdenes (p. ej. `CARGO_HOME`).
    pub entorno: Vec<(String, String)>,
    /// Build remoto (Forja): si está, el agente dispone de la herramienta `forja`.
    pub forja: Option<Forja>,
    /// Orden que comprueba la SINTAXIS de un archivo Rust por la entrada estándar (p. ej. `rustfmt --edition 2021 --check`). Se aplica al texto
    /// que quedaría ANTES de escribirlo: si da un `error:`, la edición se rechaza con el mensaje del compilador. `None` lo desactiva; si el
    /// programa no existe en el entorno, se ignora en silencio.
    pub comprobar_sintaxis: Option<Vec<String>>,
}

/// Cómo se llama a Forja. El **token no pasa por el modelo**: lo pone el adaptador al lanzar la orden.
#[derive(Debug, Clone)]
pub struct Forja {
    /// Ejecutable del cliente, p. ej. `["/bin/soso-forja"]`.
    pub argv: Vec<String>,
    pub host: String,
    pub token: String,
    pub plazo_s: u32,
}

impl Politica {
    pub fn nueva(raiz: &str) -> Self {
        Politica {
            raiz: raiz.trim_end_matches('/').to_string(),
            ejecutables: [
                &["cargo", "test"][..],
                &["cargo", "check"],
                &["cargo", "build"],
                &["cargo", "fmt"],
                &["cargo", "clippy"],
                &["git", "status"],
                &["git", "diff"],
                &["git", "log"],
            ]
            .iter()
            .map(|p| p.iter().map(|s| s.to_string()).collect())
            .collect(),
            salida_max: 12_000,
            plazo_orden_s: 600,
            lineas_por_defecto: 300,
            resultados_max: 100,
            entorno: Vec::new(),
            forja: None,
            comprobar_sintaxis: None,
        }
    }
}

/// Lo que se le devuelve al modelo. `error` sólo marca el tono para los registros: el texto es
/// siempre lo que el modelo lee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Respuesta {
    pub texto: String,
    pub error: bool,
}

impl Respuesta {
    fn ok(texto: String) -> Self {
        Respuesta { texto, error: false }
    }
    fn mal(texto: impl Into<String>) -> Self {
        Respuesta { texto: texto.into(), error: true }
    }
}

/// Definiciones en el formato de la API (`tools` de `/v1/chat/completions`).
pub fn definiciones(pol: &Politica) -> Value {
    let mut v = json!([
        {"type": "function", "function": {
            "name": "read",
            "description": "Lee un archivo (con líneas numeradas) o lista un directorio. Para archivos largos usa offset y limit.",
            "parameters": {"type": "object", "properties": {
                "filePath": {"type": "string", "description": "Ruta del archivo o directorio, relativa a la raíz del checkout"},
                "offset": {"type": "integer", "description": "Primera línea a leer (1 = la primera)"},
                "limit": {"type": "integer", "description": "Número de líneas a leer"},
                "simbolo": {"type": "string", "description": "En vez de offset/limit: nombre de una función, tipo o módulo de un archivo Rust; devuelve su código entero con números de línea"},
                "esquema": {"type": "boolean", "description": "Lista las funciones, tipos y módulos del archivo con su rango de líneas (útil antes de editar)"}},
                "required": ["filePath"]}}},
        {"type": "function", "function": {
            "name": "grep",
            "description": "Busca texto en los archivos de un directorio. Es LITERAL (no una expresión regular), sin importar los espacios entre palabras; admite `.*` entre trozos y `|` entre alternativas (`fn .*parse`, `leer|escribir`). Si no hay coincidencias exactas prueba ignorando mayúsculas y, si aun así no hay, sugiere nombres parecidos que sí existen. Devuelve archivo:línea: texto, con unas líneas de contexto (así normalmente no hace falta un `read` después); con un identificador, las definiciones salen primero.",
            "parameters": {"type": "object", "properties": {
                "pattern": {"type": "string", "description": "Texto literal a buscar, p. ej. el nombre de una función"},
                "path": {"type": "string", "description": "Directorio o archivo donde buscar (por defecto la raíz)"},
                "include": {"type": "string", "description": "Filtro de nombre de archivo, p. ej. *.rs"},
                "contexto": {"type": "integer", "description": "Líneas de contexto antes y después de cada coincidencia (por defecto 3 si hay pocas coincidencias)"},
                "ignorar_mayusculas": {"type": "boolean", "description": "Buscar sin distinguir mayúsculas de minúsculas"}},
                "required": ["pattern"]}}},
        {"type": "function", "function": {
            "name": "glob",
            "description": "Lista archivos cuyo nombre encaja con un patrón (* ? **), p. ej. crates/**/*.rs.",
            "parameters": {"type": "object", "properties": {
                "pattern": {"type": "string"},
                "path": {"type": "string", "description": "Directorio desde el que buscar (por defecto la raíz)"}},
                "required": ["pattern"]}}},
        {"type": "function", "function": {
            "name": "reemplazar_texto",
            "description": "Reemplaza un fragmento de código de un archivo por otro. `buscar` es el fragmento que quieres cambiar, copiado de `read` (puede ocupar varias líneas; no importan los espacios ni la sangría, sólo los caracteres) y debe aparecer una sola vez (si aparece varias, indica en `linea` cuál); `nuevo` es lo que lo sustituye. Usa un fragmento corto pero único, por ejemplo `k == key`. Para varios cambios en el mismo archivo usa `ediciones` (una lista de {buscar, nuevo}): se aplican todos o ninguno.",
            "parameters": {"type": "object", "properties": {
                "filePath": {"type": "string", "description": "Ruta del archivo, relativa a la raíz del checkout"},
                "buscar": {"type": "string", "description": "Fragmento a sustituir (sin importar los espacios)"},
                "nuevo": {"type": "string", "description": "Texto que lo sustituye"},
                "linea": {"type": "integer", "description": "Si `buscar` aparece varias veces: número de línea de la aparición que quieres cambiar"},
                "confirmar_borrado": {"type": "boolean", "description": "Sólo si la herramienta avisa de que la edición borraría código que `nuevo` no conserva y de verdad quieres borrarlo"},
                "ediciones": {"type": "array", "description": "Varios cambios en el mismo archivo en UNA llamada (todo o nada); en vez de buscar/nuevo", "items": {"type": "object", "properties": {"buscar": {"type": "string"}, "nuevo": {"type": "string"}, "linea": {"type": "integer"}}, "required": ["buscar", "nuevo"]}}},
                "required": ["filePath"]}}},
        {"type": "function", "function": {
            "name": "reemplazar_funcion",
            "description": "Sustituye una función, tipo o módulo de un archivo Rust ENTERO por `nuevo`, identificándolo por su nombre: no hay que dar números de línea ni copiar el código antiguo. `nuevo` debe ser la definición completa (firma, cuerpo y su última llave). Es la forma más segura de reescribir una función; usa `read` con `esquema` para ver los nombres.",
            "parameters": {"type": "object", "properties": {
                "filePath": {"type": "string", "description": "Ruta del archivo, relativa a la raíz del checkout"},
                "nombre": {"type": "string", "description": "Nombre de la función, tipo o módulo (p. ej. resolve)"},
                "nuevo": {"type": "string", "description": "La definición completa que lo sustituye, con su sangría"},
                "linea": {"type": "integer", "description": "Sólo si el nombre aparece varias veces: una línea dentro del que quieres cambiar"}},
                "required": ["filePath", "nombre", "nuevo"]}}},
        {"type": "function", "function": {
            "name": "reemplazar_lineas",
            "description": "Sustituye un RANGO de líneas (desde..hasta, ambas incluidas, según los números de `read`) por `nuevo`. No hay que copiar el texto antiguo, así que no puede fallar por un fragmento mal copiado: úsala para reescribir una función o un bloque entero, o cuando `reemplazar_texto` no encuentra el fragmento. `nuevo` vacío borra esas líneas. Devuelve cómo queda el tramo con la numeración NUEVA (los números de debajo cambian si metes más o menos líneas).",
            "parameters": {"type": "object", "properties": {
                "filePath": {"type": "string", "description": "Ruta del archivo, relativa a la raíz del checkout"},
                "desde": {"type": "integer", "description": "Primera línea a sustituir"},
                "hasta": {"type": "integer", "description": "Última línea a sustituir (incluida)"},
                "nuevo": {"type": "string", "description": "Texto que ocupa su lugar, con su sangría y sus llaves completas"},
                "confirmar_borrado": {"type": "boolean", "description": "Sólo si la herramienta avisa de que la edición borraría código que `nuevo` no conserva y de verdad quieres borrarlo"}},
                "required": ["filePath", "desde", "hasta", "nuevo"]}}},
        {"type": "function", "function": {
            "name": "insertar",
            "description": "Inserta texto NUEVO (por ejemplo una función entera) después de una línea. `linea` es el número de línea que te mostró `read` (0 = al principio del archivo; **-1 = al final del archivo**, lo más cómodo para añadir un módulo de pruebas o una función nueva). Úsala para AÑADIR código; para cambiar un fragmento existente usa reemplazar_texto. El texto debe traer sus llaves completas.",
            "parameters": {"type": "object", "properties": {
                "filePath": {"type": "string", "description": "Ruta del archivo, relativa a la raíz del checkout"},
                "linea": {"type": "integer", "description": "Se inserta después de esta línea (0 = al principio, -1 = al final del archivo)"},
                "despues_de": {"type": "string", "description": "En vez de `linea`: texto de la ÚNICA línea después de la cual insertar"},
                "antes_de": {"type": "string", "description": "En vez de `linea`: texto de la ÚNICA línea antes de la cual insertar"},
                "texto": {"type": "string", "description": "Texto a insertar, con su sangría"}},
                "required": ["filePath", "texto"]}}},
        {"type": "function", "function": {
            "name": "revisar",
            "description": "Te enseña TUS cambios (el diff de cada archivo que has editado respecto a como estaba al empezar) junto con la lista de lo que pide TASK.md. Úsala antes de dar el trabajo por terminado para comprobar, punto por punto, que no se te ha olvidado ningún caso. No lleva argumentos.",
            "parameters": {"type": "object", "properties": {}}}},
        {"type": "function", "function": {
            "name": "bash",
            "description": "Ejecuta UNA orden (sin shell: sin ; & > < $ ni comodines; se admite una sola tubería final `| head -N`, `| tail -N` o `| grep TEXTO`, y `2>&1`). Los argumentos de `cargo test` se reordenan solos: `--lib`/`--doc` antes de `--`, y el filtro de nombre va como `cargo test -p crate nombre`. Permitidas: cargo test, cargo check, cargo build, cargo fmt, cargo clippy (con `-- -W clippy::indexing_slicing -W clippy::arithmetic_side_effects` marca los accesos y cuentas que pueden entrar en pánico o desbordar), git status, git diff, git log. Se admite el prefijo `cd DIR && orden` (dentro del checkout), y `find`, `ls`, `cat`, `grep` se resuelven con las herramientas glob/read/grep (mejor úsalas directamente).",
            "parameters": {"type": "object", "properties": {
                "command": {"type": "string", "description": "La orden, p. ej. cargo test -p soso-http"}},
                "required": ["command"]}}}
    ]);
    if pol.forja.is_some() {
        if let Some(a) = v.as_array_mut() {
            a.push(json!({"type": "function", "function": {
                "name": "forja",
                "description": "Compila en el servidor de build remoto (Forja) las fuentes de /src/soso y aplica el binario resultante. `accion`: `all` = subir fuentes + compilar + aplicar; `sync` = sólo subir; `build` = compilar lo subido. Devuelve el diagnóstico del compilador: si falla, léelo y corrige el archivo.",
                "parameters": {"type": "object", "properties": {
                    "accion": {"type": "string", "enum": ["all", "sync", "build"]}},
                    "required": ["accion"]}}}));
        }
    }
    v
}

/// Las definiciones que puede usar un agente de sólo lectura (el planificador).
pub fn solo_lectura(defs: Value) -> Value {
    let v: Vec<Value> = defs.as_array().cloned().unwrap_or_default().into_iter().filter(|d| matches!(d.pointer("/function/name").and_then(|n| n.as_str()), Some("read" | "grep" | "glob"))).collect();
    Value::Array(v)
}

/// Las órdenes `cargo …` de la sección «Qué tiene que pasar» de `TASK.md` (entre comillas invertidas).
fn comandos_de_la_tarea<E: Entorno>(e: &mut E, pol: &Politica) -> Vec<String> {
    let Ok(datos) = e.leer(&format!("{}/TASK.md", pol.raiz.trim_end_matches('/'))) else { return Vec::new() };
    let texto = String::from_utf8_lossy(&datos);
    let mut dentro = false;
    let mut v: Vec<String> = Vec::new();
    for l in texto.lines() {
        if l.starts_with("## ") {
            dentro = l.to_lowercase().contains("qué tiene que pasar") || l.to_lowercase().contains("que tiene que pasar");
            continue;
        }
        if !dentro {
            continue;
        }
        let mut resto = l;
        while let Some(i) = resto.find('`') {
            let Some(j) = resto[i + 1..].find('`') else { break };
            let c = resto[i + 1..i + 1 + j].trim();
            if c.starts_with("cargo ") && !v.iter().any(|x| x == c) {
                v.push(c.to_string());
            }
            resto = &resto[i + 1 + j + 1..];
        }
    }
    v
}

/// Ejecuta las comprobaciones de la tarea (las de `TASK.md`) y devuelve el informe de la primera que falle, o `None` si todas pasan o la
/// tarea no declara ninguna.
pub fn comprobar_tarea<E: Entorno>(e: &mut E, pol: &Politica) -> Option<String> {
    for c in comandos_de_la_tarea(e, pol) {
        let r = ejecutar(e, pol, "bash", &json!({"command": c}));
        if r.error || !r.texto.contains("exit code: 0") {
            // Una prueba PROPIA del agente que falla rompe también la comprobación de la tarea (campaña 41, R40: su `pruebas_agente::test_…` roja).
            let aviso = if r.texto.contains("pruebas_agente::") {
                "\n\n(Entre lo que falla hay una prueba tuya de `pruebas_agente`: o la corrijes, si el código está mal, o, si contradice el enunciado o no consigues que pase, la ELIMINAS: una prueba tuya en rojo hace fallar la comprobación entera.)"
            } else {
                ""
            };
            return Some(format!("$ {c}\n{}{aviso}", r.texto));
        }
    }
    None
}

/// Texto de `TASK.md` (vacío si no existe).
fn leer_task<E: Entorno>(e: &mut E, pol: &Politica) -> String {
    e.leer(&format!("{}/TASK.md", pol.raiz.trim_end_matches('/'))).map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default()
}

/// Rutas de la sección «Qué puedes tocar» de `TASK.md` (`- \`ruta\``); vacío si no hay sección (sin restricción).
fn archivos_permitidos(task: &str) -> Vec<String> {
    let mut dentro = false;
    let mut v = Vec::new();
    for l in task.lines() {
        if l.starts_with("## ") {
            dentro = l.to_lowercase().contains("qué puedes tocar") || l.to_lowercase().contains("que puedes tocar");
            continue;
        }
        if dentro {
            if let Some(r) = l.strip_prefix("- `").and_then(|r| r.split('`').next()) {
                v.push(r.to_string());
            }
        }
    }
    v
}

/// Una edición fuera de lo que `TASK.md` deja tocar «Hecho» y luego se pierde: sólo se exportan esas rutas (campaña 51: R35 arregló `io.rs` en vez de
/// `lib.rs`, y R32 y R36 editaron `tests/roundtrip.rs`; tres intentos rechazados sin que el agente se enterara).
fn fuera_de_alcance<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Option<String> {
    let p = cadena(args, "filePath").ok()?;
    let (_, rel) = resolver(pol, p).ok()?;
    let permitidos = archivos_permitidos(&leer_task(e, pol));
    if permitidos.is_empty() || permitidos.iter().any(|a| a == &rel || (a.ends_with('/') && rel.starts_with(a.as_str()))) {
        return None;
    }
    let lista = permitidos.iter().map(|a| format!("`{a}`")).collect::<Vec<_>>().join(", ");
    Some(format!(
        "No he aplicado el cambio: `{rel}` está fuera de lo que esta tarea deja tocar. TASK.md («Qué puedes tocar») sólo permite {lista}; un cambio en cualquier otra ruta NO se exporta y se pierde. Haz el cambio dentro de {lista}: si la corrección vive en otro archivo, resuélvela desde el permitido (por ejemplo, con una función propia ahí) y no edites pruebas existentes."
    ))
}

/// Cabeceras (normalizadas) de los ítems `pub` de un fuente Rust: `pub fn f(a: u32) -> u32`, `pub struct S`, `pub const X: u32`…
fn cabeceras_publicas(texto: &str) -> Vec<String> {
    const INICIOS: [&str; 10] = ["pub fn ", "pub async fn ", "pub const fn ", "pub unsafe fn ", "pub struct ", "pub enum ", "pub trait ", "pub type ", "pub const ", "pub static "];
    let lineas: Vec<&str> = texto.lines().collect();
    let mut v = Vec::new();
    let mut i = 0;
    while i < lineas.len() {
        let t = lineas[i].trim_start();
        if INICIOS.iter().any(|p| t.starts_with(p)) {
            let mut cab = String::new();
            let mut j = i;
            while j < lineas.len() && j < i + 8 {
                cab.push(' ');
                cab.push_str(lineas[j].trim());
                if lineas[j].contains('{') || lineas[j].contains(';') {
                    break;
                }
                j += 1;
            }
            let fin = cab.find(|c| c == '{' || c == ';').unwrap_or(cab.len());
            let norm = cab[..fin].split_whitespace().collect::<Vec<_>>().join(" ").replace("( ", "(").replace(" )", ")").replace(",)", ")");
            v.push(norm);
        }
        i += 1;
    }
    v
}

/// Ítems `pub` que existían antes de la primera edición y ya no están (borrados) o tienen otra firma, por archivo. `None` si no falta ninguno.
pub fn api_publica_alterada<E: Entorno>(e: &mut E, originales: &alloc::collections::BTreeMap<String, Option<String>>) -> Option<String> {
    let mut lineas: Vec<String> = Vec::new();
    for (abs, antes) in originales {
        let Some(antes) = antes else { continue };
        if !abs.ends_with(".rs") {
            continue;
        }
        let Some(ahora) = e.leer(abs).ok().and_then(|d| String::from_utf8(d).ok()) else { continue };
        let nuevas = cabeceras_publicas(&ahora);
        let rel = abs.rsplit("/crates/").next().map(|r| format!("crates/{r}")).unwrap_or_else(|| abs.clone());
        for c in cabeceras_publicas(antes) {
            if !nuevas.contains(&c) {
                lineas.push(format!("  {rel}: `{c}`"));
            }
        }
    }
    if lineas.is_empty() {
        return None;
    }
    let mas = if lineas.len() > 12 { format!("\n  … y {} más", lineas.len() - 12) } else { String::new() };
    Some(format!("{}{mas}", lineas.iter().take(12).cloned().collect::<Vec<_>>().join("\n")))
}

/// ¿Pide `TASK.md` que no haya pánico o desbordamiento? (pública para el aviso del bucle)
pub fn pide_sin_panico<E: Entorno>(e: &mut E, pol: &Politica) -> bool {
    tarea_pide_sin_panico(&leer_task(e, pol))
}

/// ¿Pide la tarea que no haya pánico o desbordamiento? Entonces vale la pena pasar los lints de Clippy que los detectan.
fn tarea_pide_sin_panico(task: &str) -> bool {
    let t = task.to_lowercase();
    ["pánico", "panico", "panic", "desbord", "overflow", "sin entrar en pánico"].iter().any(|k| t.contains(k))
}

/// Orden de Clippy para la comprobación de la tarea: la primera `cargo test …` de «Qué tiene que pasar» convertida en `cargo clippy …` con los
/// lints que no vienen activados por defecto y que marcan cada acceso o aritmética que puede entrar en pánico o desbordar.
fn orden_clippy(cmd: &str) -> Option<String> {
    let t: Vec<&str> = cmd.split_whitespace().collect();
    if t.len() < 3 || t[0] != "cargo" || t[1] != "test" {
        return None;
    }
    let mut v: Vec<String> = alloc::vec!["cargo".into(), "clippy".into(), "--message-format".into(), "short".into()];
    let mut i = 2;
    while i < t.len() {
        match t[i] {
            "-q" | "--quiet" | "--lib" | "--doc" | "--tests" | "--no-run" => {}
            "-p" | "--package" | "--features" | "-F" => {
                v.push(t[i].to_string());
                if let Some(x) = t.get(i + 1) {
                    v.push((*x).to_string());
                }
                i += 1;
            }
            "--" => break,
            o if o.starts_with('-') => {}
            _ => {}
        }
        i += 1;
    }
    v.extend(["--".to_string(), "-W".into(), "clippy::indexing_slicing".into(), "-W".into(), "clippy::arithmetic_side_effects".into()]);
    Some(v.join(" "))
}

/// Avisos de Clippy (indexado y aritmética que pueden entrar en pánico o desbordar) que caen en líneas que el agente ha añadido o cambiado.
/// `None` si la tarea no habla de pánico o desbordamiento, si no hay orden de comprobación o si no hay nada en sus líneas.
pub fn alertas_de_panico<E: Entorno>(e: &mut E, pol: &Politica, anadidas: &alloc::collections::BTreeMap<String, alloc::collections::BTreeSet<usize>>) -> Option<String> {
    let task = leer_task(e, pol);
    if anadidas.is_empty() {
        return None;
    }
    // Si la tarea no habla de pánico, sólo se avisa de las sumas/restas/productos que pueden desbordar (en las pruebas, que van en `debug`, un desbordamiento
    // es un pánico: R33 de la campaña 54 promediaba estéreo sumando dos `i16` y fallaba `estereo_se_promedia` con `attempt to add with overflow`).
    let general = !tarea_pide_sin_panico(&task);
    let cmd = comandos_de_la_tarea(e, pol).into_iter().next()?;
    let orden = orden_clippy(&cmd)?;
    let argv: Vec<String> = orden.split_whitespace().map(|s| s.to_string()).collect();
    let o = Orden { argv, cwd: pol.raiz.clone(), stdin: Vec::new(), timeout_s: Some(pol.plazo_orden_s), entorno: pol.entorno.clone() };
    let s = e.ejecutar(&o).ok()?;
    s.codigo?;
    // Un acceso sin comprobar suele estar en una línea que el agente NO tocó pero que está en la función que sí tocó (R39 de la campaña 43: arregló el
    // producto `block * BLOCK_SIZE` y dejó sin comprobar la suma `off + BLOCK_SIZE`, dos líneas más abajo): los avisos valen para toda la función.
    let mut ampliadas = anadidas.clone();
    for (rel, set) in ampliadas.iter_mut() {
        let Ok(datos) = e.leer(&format!("{}/{}", pol.raiz.trim_end_matches('/'), rel)) else { continue };
        let texto = String::from_utf8_lossy(&datos).into_owned();
        let lineas: Vec<&str> = texto.lines().collect();
        let originales: alloc::collections::BTreeSet<usize> = set.clone();
        for x in elementos(&lineas).iter().filter(|x| x.clase == "fn") {
            if originales.iter().any(|l| x.ini <= *l && *l <= x.fin) {
                set.extend(x.ini..=x.fin);
            }
        }
    }
    let mut avisos: Vec<String> = Vec::new();
    for l in s.texto().lines() {
        // formato corto: `ruta:linea:col: warning: mensaje`
        let mut partes = l.splitn(4, ':');
        let (Some(ruta), Some(n), Some(_col), Some(resto)) = (partes.next(), partes.next(), partes.next(), partes.next()) else { continue };
        let Ok(n) = n.trim().parse::<usize>() else { continue };
        // Sólo los dos lints pedidos (no, por ejemplo, `result_unit_err` de una firma que el agente reformateó).
        if !resto.contains("warning") || !(resto.contains("side-effects") || (!general && resto.contains("may panic"))) {
            continue;
        }
        let ruta = ruta.trim_start_matches("./");
        let Some((rel, lineas)) = ampliadas.iter().find(|(r, _)| ruta.ends_with(r.as_str()) || r.ends_with(ruta)) else { continue };
        if lineas.contains(&n) {
            let msg = resto.trim().trim_start_matches("warning:").trim();
            avisos.push(format!("  {rel}:{n}: {msg}"));
        }
    }
    avisos.sort();
    avisos.dedup();
    if avisos.is_empty() {
        return None;
    }
    let mas = if avisos.len() > 12 { format!("\n  … y {} más", avisos.len() - 12) } else { String::new() };
    Some(format!("{}{mas}", avisos.iter().take(12).cloned().collect::<Vec<_>>().join("\n")))
}

/// Número de puntos («- …» sin sangría) de la sección «Qué se pide» de `TASK.md`.
pub fn puntos_de_la_tarea<E: Entorno>(e: &mut E, pol: &Politica) -> usize {
    let task = leer_task(e, pol);
    let mut dentro = false;
    let mut n = 0;
    for l in task.lines() {
        if l.starts_with("## ") {
            dentro = l.to_lowercase().contains("qué se pide") || l.to_lowercase().contains("que se pide");
            continue;
        }
        if dentro && l.starts_with("- ") {
            n += 1;
        }
    }
    n
}

/// Un cambio que es casi todo comentarios no implementa nada (R32 de la campaña 39: un monólogo de `// Wait, …` en lugar del código).
fn casi_solo_comentarios(rel: &str, nuevo: &str) -> Option<String> {
    if !rel.ends_with(".rs") {
        return None;
    }
    let utiles: Vec<&str> = nuevo.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    let coment = utiles.iter().filter(|l| l.starts_with("//")).count();
    if coment < 8 || coment * 2 <= utiles.len() {
        // Un monólogo de dudas dentro del código (R32 de la campaña 45: «Espera, si el FS no tiene `parent`, ¿cómo…?») es pensar en voz alta, no un comentario.
        let dudas = utiles.iter().filter(|l| l.starts_with("//") && (l.contains('?') || ["Espera", "Wait", "Hmm", "Releyendo", "Revisemos", "Quizás"].iter().any(|k| l.contains(k)))).count();
        if dudas >= 4 {
            return Some(format!(
                "No he escrito el cambio: {dudas} comentarios son dudas o monólogo («Espera…», «¿cómo…?») y no código. Resuelve las dudas ANTES de escribir, leyendo con `read`/`grep` lo que te falte (p. ej. cómo se guarda el padre de un directorio), y escribe sólo el código final con a lo sumo un comentario breve."
            ));
        }
        return None;
    }
    Some(format!(
        "No he escrito el cambio: {coment} de las {} líneas de `nuevo` son comentarios y casi no hay código, así que no implementa nada. Piensa en tu cabeza (o decide primero qué hace falta leyendo el código con `read`) y escribe sólo el CÓDIGO, con a lo sumo un comentario breve.",
        utiles.len()
    ))
}

/// Ubicaciones (`ruta`, línea) de los ERRORES de compilación (no de los avisos) de una salida de `cargo`.
fn errores_de_compilacion(texto: &str) -> Vec<(String, usize)> {
    let mut v = Vec::new();
    let mut en_error = false;
    for l in texto.lines() {
        let t = l.trim_start();
        if l.starts_with("error") {
            en_error = true;
        } else if l.starts_with("warning") {
            en_error = false;
        }
        if en_error {
            if let Some(r) = t.strip_prefix("--> ") {
                let mut partes = r.trim().rsplitn(3, ':');
                let (_col, lin, ruta) = (partes.next(), partes.next(), partes.next());
                if let (Some(lin), Some(ruta)) = (lin.and_then(|n| n.parse::<usize>().ok()), ruta) {
                    v.push((ruta.trim_start_matches("./").to_string(), lin));
                }
            }
        }
    }
    v
}

/// Ejecuta las comprobaciones de la tarea y elimina del código del agente las pruebas PROPIAS que rompen la comprobación: las funciones `#[test]`
/// de `mod pruebas_agente` que fallan, las que no compilan, y el módulo entero si el error de compilación cae dentro de él pero fuera de una prueba.
/// También las funciones `#[test]` que el agente añadió fuera del módulo cuando son las que no compilan. Nunca toca código de producción ni lo que
/// ya existía. Devuelve los nombres quitados.
pub fn podar_pruebas_rojas<E: Entorno>(
    e: &mut E,
    pol: &Politica,
    archivos: &[String],
    anadidas: &alloc::collections::BTreeMap<String, alloc::collections::BTreeSet<usize>>,
) -> Vec<String> {
    let mut quitadas: Vec<String> = Vec::new();
    for _ronda in 0..2 {
        let Some(rojo) = comprobar_tarea(e, pol) else { break };
        let nombres: Vec<String> = super::nombres_de_pruebas_rojas(&rojo)
            .into_iter()
            .filter(|n| n.contains("pruebas_agente::"))
            .map(|n| n.rsplit("::").next().unwrap_or(&n).to_string())
            .collect();
        let errores = errores_de_compilacion(&rojo);
        if nombres.is_empty() && errores.is_empty() {
            break;
        }
        let mut hubo = false;
        for abs in archivos {
            if !abs.ends_with(".rs") {
                continue;
            }
            let rel = abs.strip_prefix(pol.raiz.trim_end_matches('/')).unwrap_or(abs).trim_start_matches('/').to_string();
            let Ok(datos) = e.leer(abs) else { continue };
            let Ok(texto) = String::from_utf8(datos) else { continue };
            let lineas: Vec<&str> = texto.lines().collect();
            let els = elementos(&lineas);
            let errores_aqui: Vec<usize> = errores.iter().filter(|(r, _)| r.ends_with(&rel) || rel.ends_with(r.as_str())).map(|(_, l)| *l).collect();
            let modulos: Vec<(usize, usize)> = els.iter().filter(|x| x.clase == "mod" && x.nombre == "pruebas_agente").map(|x| (x.ini, x.fin)).collect();
            let nuevas_aqui = anadidas.get(&rel);
            let es_test = |x: &Elemento| x.clase == "fn" && x.ini > 1 && (x.ini.saturating_sub(3)..x.ini).any(|k| k >= 1 && lineas[k - 1].trim() == "#[test]");
            let sin_atributos = |ini: usize| {
                let mut i = ini;
                while i > 1 && lineas[i - 2].trim_start().starts_with("#[") {
                    i -= 1;
                }
                i
            };
            let mut rangos: Vec<(usize, usize, String)> = Vec::new();
            // Un error de compilación dentro del módulo de pruebas que no esté en una prueba concreta se lleva el módulo entero.
            let mut quitar_modulo = false;
            for &l in &errores_aqui {
                if let Some((mi, mf)) = modulos.iter().find(|(a, b)| *a <= l && l <= *b) {
                    let dentro_de_prueba = els.iter().any(|x| es_test(x) && x.ini <= l && l <= x.fin && *mi < x.ini && x.fin <= *mf);
                    if !dentro_de_prueba {
                        quitar_modulo = true;
                    }
                }
            }
            if quitar_modulo {
                for (mi, mf) in &modulos {
                    rangos.push((sin_atributos(*mi), *mf, "pruebas_agente".to_string()));
                }
            } else {
                for x in els.iter().filter(|x| es_test(x)) {
                    let en_modulo = modulos.iter().any(|(a, b)| *a < x.ini && x.fin <= *b);
                    let rota_en_ejecucion = en_modulo && nombres.contains(&x.nombre);
                    let rota_al_compilar = errores_aqui.iter().any(|l| x.ini <= *l && *l <= x.fin)
                        && (en_modulo || nuevas_aqui.is_some_and(|n| (x.ini..=x.fin).all(|k| n.contains(&k))));
                    if rota_en_ejecucion || rota_al_compilar {
                        rangos.push((sin_atributos(x.ini), x.fin, x.nombre.clone()));
                    }
                }
            }
            if rangos.is_empty() {
                continue;
            }
            rangos.sort_by(|a, b| b.0.cmp(&a.0));
            let mut nuevo: Vec<&str> = lineas.clone();
            for (ini, fin, _) in &rangos {
                nuevo.drain(ini - 1..*fin);
            }
            let mut nuevo_texto = nuevo.join("\n");
            if texto.ends_with('\n') {
                nuevo_texto.push('\n');
            }
            if desequilibrio(abs, &texto, &nuevo_texto).is_some() {
                continue;
            }
            let modo = e.metadatos(abs).map(|m| m.modo).unwrap_or(0o644);
            if e.escribir(abs, nuevo_texto.as_bytes(), modo).is_ok() {
                quitadas.extend(rangos.into_iter().map(|(_, _, n)| n));
                hubo = true;
            }
        }
        if !hubo {
            break;
        }
    }
    quitadas
}

/// Ejecuta una herramienta por nombre. Un nombre desconocido o argumentos mal formados son una
/// respuesta para el modelo, no un error del agente.
pub fn ejecutar<E: Entorno>(e: &mut E, pol: &Politica, nombre: &str, args: &Value) -> Respuesta {
    if matches!(nombre, "reemplazar_texto" | "insertar" | "reemplazar_lineas" | "reemplazar_funcion") {
        if let Some(m) = fuera_de_alcance(e, pol, args) {
            return Respuesta::mal(m);
        }
    }
    let r = match nombre {
        "read" => leer(e, pol, args),
        "grep" => buscar(e, pol, args),
        "glob" => listar_glob(e, pol, args),
        "reemplazar_texto" => reemplazar(e, pol, args),
        "insertar" => insertar(e, pol, args),
        "reemplazar_lineas" => reemplazar_lineas(e, pol, args),
        "reemplazar_funcion" => reemplazar_funcion(e, pol, args),
        "bash" => orden(e, pol, args),
        "forja" if pol.forja.is_some() => forja(e, pol, args),
        otro => Ok(Respuesta::mal(format!(
            "La herramienta «{otro}» no existe. Las disponibles son: read, grep, glob, reemplazar_texto, reemplazar_lineas, reemplazar_funcion, insertar, revisar, bash{}.", if pol.forja.is_some() { " y forja" } else { "" }
        ))),
    };
    match r {
        Ok(mut r) => {
            r.texto = recortar(r.texto, pol.salida_max);
            r
        }
        Err(err) => Respuesta::mal(recortar(format!("Error: {err}"), pol.salida_max)),
    }
}

/// Nombres alternativos con los que un modelo suele llamar a los argumentos (`text` en vez de `texto`, `path` en vez de `filePath`…).
fn alias(clave: &str) -> &'static [&'static str] {
    match clave {
        "filePath" => &["file_path", "filepath", "file", "path", "archivo", "ruta"],
        "buscar" => &["search", "old", "old_string", "old_text", "find"],
        "nuevo" => &["new", "new_string", "new_text", "replace", "replacement"],
        "texto" => &["text", "code", "contenido", "content", "insert", "new_text"],
        "command" => &["cmd", "comando"],
        "linea" => &["line", "línea", "line_number"],
        "nombre" => &["name", "funcion", "función", "function", "simbolo", "symbol"],
        "desde" => &["from", "start", "start_line", "inicio", "primera", "desde_linea"],
        "hasta" => &["to", "end", "end_line", "fin", "ultima", "última", "hasta_linea"],
        _ => &[],
    }
}

fn valor<'a>(args: &'a Value, clave: &str) -> Option<&'a Value> {
    args.get(clave).or_else(|| alias(clave).iter().find_map(|a| args.get(*a)))
}

fn cadena<'a>(args: &'a Value, clave: &str) -> Resultado<&'a str> {
    valor(args, clave)
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::uso(format!("falta el argumento «{clave}» (texto)")))
}

fn entero(args: &Value, clave: &str) -> Option<usize> {
    let v = valor(args, clave)?;
    v.as_u64().map(|n| n as usize).or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Recorta por el centro (se conserva el principio y el final, que es donde suelen estar la causa y
/// el resumen) respetando los límites de carácter.
pub fn recortar(texto: String, max: usize) -> String {
    if texto.len() <= max {
        return texto;
    }
    let cabeza = max * 2 / 3;
    let cola = max - cabeza;
    let mut a = cabeza;
    while !texto.is_char_boundary(a) {
        a -= 1;
    }
    let mut b = texto.len() - cola;
    while !texto.is_char_boundary(b) {
        b += 1;
    }
    format!(
        "{}\n… (recortado: {} bytes de {} omitidos) …\n{}",
        &texto[..a],
        b - a,
        texto.len(),
        &texto[b..]
    )
}

/// Resuelve una ruta del modelo dentro de la raíz: devuelve (absoluta, relativa).
pub fn resolver(pol: &Politica, p: &str) -> Resultado<(String, String)> {
    let raiz = pol.raiz.as_str();
    let rel_in = if let Some(resto) = p.strip_prefix(raiz) {
        if resto.is_empty() || resto.starts_with('/') {
            resto
        } else if p.starts_with('/') {
            return Err(Error::uso(format!("«{p}» está fuera del checkout ({raiz})")));
        } else {
            p
        }
    } else if p.starts_with('/') {
        return Err(Error::uso(format!("«{p}» está fuera del checkout ({raiz})")));
    } else {
        p
    };
    let mut partes: Vec<&str> = Vec::new();
    for seg in rel_in.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if partes.pop().is_none() {
                    return Err(Error::uso(format!("«{p}» sale del checkout ({raiz})")));
                }
            }
            s => partes.push(s),
        }
    }
    let rel = partes.join("/");
    let abs = if rel.is_empty() { raiz.to_string() } else { format!("{raiz}/{rel}") };
    Ok((abs, rel))
}

fn mostrar(rel: &str) -> &str {
    if rel.is_empty() {
        "."
    } else {
        rel
    }
}

// ---------------------------------------------------------------- «no existe» que ayuda

/// «No existe», pero diciendo qué sí hay: el ancestro más cercano que existe y sus entradas. Un
/// modelo que recibe sólo «no existe» adivina otra ruta o se rinde; con la lista, corrige.
fn no_existe<E: Entorno>(e: &E, pol: &Politica, rel: &str) -> Error {
    let mut partes: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
    loop {
        let r = partes.join("/");
        let abs = if r.is_empty() { pol.raiz.clone() } else { format!("{}/{r}", pol.raiz) };
        if let Ok(m) = e.metadatos(&abs) {
            if m.tipo == Tipo::Directorio {
                if let Ok(mut v) = e.listar(&abs) {
                    v.sort_by(|a, b| a.ruta.cmp(&b.ruta));
                    let total = v.len();
                    let nombres: Vec<String> = v.into_iter().take(40).map(|x| if x.tipo == Tipo::Directorio { format!("{}/", x.ruta) } else { x.ruta }).collect();
                    return Error::uso(format!(
                        "«{}» no existe. Las rutas son relativas a la raíz del checkout. En «{}» hay{}: {}",
                        mostrar(rel),
                        mostrar(&r),
                        if total > 40 { format!(" {total} entradas, las primeras 40") } else { String::new() },
                        nombres.join(", ")
                    ));
                }
            }
        }
        if partes.pop().is_none() {
            return Error::uso(format!("«{}» no existe", mostrar(rel)));
        }
    }
}

// ---------------------------------------------------------------- read

const LINEA_MAX: usize = 2000;
/// Un `read` con `limit` menor se amplía a esto: los modelos leían ventanas de 10 líneas y encadenaban hasta 9 lecturas del mismo
/// archivo (37 casos en las campañas 8–22); una ventana mayor cuesta un poco más de contexto y ahorra llamadas.
const LECTURA_MIN: usize = 80;
const ARCHIVO_MAX: u64 = 4 * 1024 * 1024;

fn leer<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
    if let Some(sim) = args.get("simbolo").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
        return leer_simbolo(e, pol, args, sim);
    }
    if valor(args, "esquema").and_then(|v| v.as_bool()).unwrap_or(false) {
        let (abs, rel) = resolver(pol, cadena(args, "filePath")?)?;
        let datos = e.leer(&abs).map_err(|_| no_existe(e, pol, &rel))?;
        let texto = String::from_utf8_lossy(&datos).into_owned();
        let lineas: Vec<&str> = texto.lines().collect();
        return Ok(Respuesta::ok(esquema_texto(&rel, &lineas)));
    }
    let (abs, rel) = resolver(pol, cadena(args, "filePath")?)?;
    let meta = e.metadatos(&abs).map_err(|_| no_existe(e, pol, &rel))?;
    if meta.tipo == Tipo::Directorio {
        let mut nombres: Vec<String> = e
            .listar(&abs)?
            .into_iter()
            .map(|x| if x.tipo == Tipo::Directorio { format!("{}/", x.ruta) } else { x.ruta })
            .collect();
        nombres.sort();
        return Ok(Respuesta::ok(format!(
            "<path>{}</path>\n<type>directory</type>\n<entries>\n{}\n</entries>",
            mostrar(&rel),
            nombres.join("\n")
        )));
    }
    if meta.bytes > ARCHIVO_MAX {
        return Err(Error::uso(format!(
            "«{}» pesa {} bytes (tope {ARCHIVO_MAX}); no se lee entero",
            mostrar(&rel),
            meta.bytes
        )));
    }
    let datos = e.leer(&abs)?;
    if datos.contains(&0) {
        return Err(Error::uso(format!("«{}» es un archivo binario", mostrar(&rel))));
    }
    let texto = String::from_utf8_lossy(&datos);
    let lineas: Vec<&str> = {
        let mut v: Vec<&str> = texto.split('\n').collect();
        if v.last() == Some(&"") {
            v.pop();
        }
        v
    };
    let total = lineas.len();
    let desde = entero(args, "offset").unwrap_or(1).max(1);
    let pedido = entero(args, "limit");
    let n = pedido.unwrap_or(pol.lineas_por_defecto).max(LECTURA_MIN);
    let ampliada = pedido.is_some_and(|p| p < LECTURA_MIN);
    if total > 0 && desde > total {
        return Err(Error::uso(format!(
            "offset {desde} pasa del final: «{}» tiene {total} línea(s)",
            mostrar(&rel)
        )));
    }
    let hasta = (desde - 1 + n).min(total);
    let mut cuerpo = String::new();
    for (i, l) in lineas.iter().enumerate().take(hasta).skip(desde - 1) {
        let l = l.strip_suffix('\r').unwrap_or(l);
        if l.len() > LINEA_MAX {
            let mut c = LINEA_MAX;
            while !l.is_char_boundary(c) {
                c -= 1;
            }
            cuerpo.push_str(&format!("{}: {}… (línea recortada)\n", i + 1, &l[..c]));
        } else {
            cuerpo.push_str(&format!("{}: {}\n", i + 1, l));
        }
    }
    let pie = if hasta < total {
        let aviso = if total > 400 {
            " Archivo largo: no lo recorras por tramos. Llama a `grep` con el nombre de la función o el texto que buscas y lee sólo ese tramo con offset y limit."
        } else {
            ""
        };
        let amp = if ampliada { format!(" He ampliado la ventana a {LECTURA_MIN} líneas para ahorrarte lecturas.") } else { String::new() };
        format!("(Líneas {desde}-{hasta} de {total}. Usa offset={} para seguir.{amp}{aviso})", hasta + 1)
    } else {
        format!("(Fin del archivo: {total} línea(s).)")
    };
    Ok(Respuesta::ok(format!(
        "<path>{}</path>\n<type>file</type>\n<content>\n{cuerpo}\n{pie}\n</content>",
        mostrar(&rel)
    )))
}

// ---------------------------------------------------------------- recorrido

const PROFUNDIDAD_MAX: usize = 24;
const IGNORADOS: [&str; 3] = [".git", "target", "node_modules"];

/// Recorre un árbol en orden estable. `visita(abs, rel)` se llama por cada archivo regular.
fn recorrer<E: Entorno>(
    e: &E,
    abs: &str,
    rel: &str,
    profundidad: usize,
    visita: &mut dyn FnMut(&str, &str),
) -> Resultado<()> {
    if profundidad > PROFUNDIDAD_MAX {
        return Ok(());
    }
    let mut entradas = e.listar(abs)?;
    entradas.sort_by(|a, b| a.ruta.cmp(&b.ruta));
    for x in entradas {
        let (a, r) = (format!("{abs}/{}", x.ruta), if rel.is_empty() { x.ruta.clone() } else { format!("{rel}/{}", x.ruta) });
        match x.tipo {
            Tipo::Directorio => {
                if !IGNORADOS.contains(&x.ruta.as_str()) {
                    let _ = recorrer(e, &a, &r, profundidad + 1, visita);
                }
            }
            Tipo::Archivo => visita(&a, &r),
            Tipo::Otro => {}
        }
    }
    Ok(())
}

/// `*` y `?` dentro de un segmento (sin `/`).
fn glob_segmento(pat: &[u8], texto: &[u8]) -> bool {
    match pat.split_first() {
        None => texto.is_empty(),
        Some((b'*', resto)) => (0..=texto.len()).any(|i| glob_segmento(resto, &texto[i..])),
        Some((b'?', resto)) => !texto.is_empty() && glob_segmento(resto, &texto[1..]),
        Some((c, resto)) => texto.first() == Some(c) && glob_segmento(resto, &texto[1..]),
    }
}

/// Patrón con `/` y `**` frente a una ruta relativa.
fn glob_ruta(pat: &[&str], ruta: &[&str]) -> bool {
    match pat.split_first() {
        None => ruta.is_empty(),
        Some((&"**", resto)) => (0..=ruta.len()).any(|i| glob_ruta(resto, &ruta[i..])),
        Some((p, resto)) => match ruta.split_first() {
            Some((r, rr)) => glob_segmento(p.as_bytes(), r.as_bytes()) && glob_ruta(resto, rr),
            None => false,
        },
    }
}

fn listar_glob<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
    let patron = cadena(args, "pattern")?;
    let (abs, rel) = resolver(pol, args.get("path").and_then(|v| v.as_str()).unwrap_or("."))?;
    // Sin `/` en el patrón se busca por nombre en cualquier profundidad (como hace el modelo al
    // escribir `*.rs`).
    let segmentos: Vec<&str> = patron.split('/').filter(|s| !s.is_empty()).collect();
    let por_nombre = segmentos.len() == 1;
    let mut halladas: Vec<String> = Vec::new();
    let mut total = 0usize;
    recorrer(e, &abs, &rel, 0, &mut |_a, r| {
        let partes: Vec<&str> = r.split('/').collect();
        let encaja = if por_nombre {
            glob_segmento(segmentos[0].as_bytes(), partes.last().copied().unwrap_or("").as_bytes())
        } else {
            glob_ruta(&segmentos, &partes)
        };
        if encaja {
            total += 1;
            if halladas.len() < pol.resultados_max {
                halladas.push(r.to_string());
            }
        }
    })?;
    if total == 0 {
        return Ok(Respuesta::ok(format!("Ningún archivo encaja con «{patron}» bajo {}.", mostrar(&rel))));
    }
    let mut t = halladas.join("\n");
    if total > halladas.len() {
        t.push_str(&format!("\n(… y {} más; afina el patrón)", total - halladas.len()));
    }
    Ok(Respuesta::ok(t))
}

// ---------------------------------------------------------------- grep

/// ¿Usa una construcción de expresión regular que NO se soporta (clases `\w \d \s \b`, cuantificadores `.+ .?`)? Esto sí se rechaza:
/// buscarlo tal cual daría «no hay» sin ser cierto. Lo que sí se soporta —`.*`, `|`, `^`, `$`— se interpreta como comodines.
fn parece_regex(p: &str) -> bool {
    let b = p.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'\\' && b[i + 1].is_ascii_alphabetic() {
            return true;
        }
        if b[i] == b'.' && matches!(b[i + 1], b'+' | b'?') {
            return true;
        }
        i += 1;
    }
    false
}

/// Cómo se emparejó una búsqueda: tal cual, ignorando mayúsculas, o como patrón con comodines (`.*`) y alternativas (`|`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModoGrep {
    Exacto,
    SinMayusculas,
    Comodines,
}

/// El patrón del modelo, preparado para los tres modos.
struct Consulta {
    patron: String,
    bajo: String,
    espacios: bool,
    /// Alternativas (`a|b`), cada una una lista de fragmentos separados por `.*` que deben aparecer en orden (en minúsculas).
    alternativas: Vec<Vec<String>>,
    ancla_ini: bool,
    ancla_fin: bool,
}

fn normalizar_espacios(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Consulta {
    fn nueva(patron: &str) -> Self {
        let ancla_ini = patron.starts_with('^');
        let ancla_fin = patron.ends_with('$') && !patron.ends_with("\\$");
        let cuerpo = patron.trim_start_matches('^');
        let cuerpo = if ancla_fin { &cuerpo[..cuerpo.len() - 1] } else { cuerpo };
        let alternativas = cuerpo
            .split('|')
            .map(|alt| {
                alt.replace("\\.", ".")
                    .replace("\\(", "(")
                    .replace("\\)", ")")
                    .replace("\\[", "[")
                    .replace("\\]", "]")
                    .replace("\\*", "\u{1}")
                    .split(".*")
                    .map(|f| f.replace('\u{1}', "*").to_lowercase())
                    .filter(|f| !f.is_empty())
                    .collect::<Vec<String>>()
            })
            .filter(|v| !v.is_empty())
            .collect();
        Consulta { patron: patron.to_string(), bajo: patron.to_lowercase(), espacios: patron.contains(char::is_whitespace), alternativas, ancla_ini, ancla_fin }
    }

    fn casa(&self, modo: ModoGrep, linea: &str) -> bool {
        match modo {
            ModoGrep::Exacto => {
                linea.contains(&self.patron) || (self.espacios && normalizar_espacios(linea).contains(&normalizar_espacios(&self.patron)))
            }
            ModoGrep::SinMayusculas => {
                let l = linea.to_lowercase();
                l.contains(&self.bajo) || (self.espacios && normalizar_espacios(&l).contains(&normalizar_espacios(&self.bajo)))
            }
            ModoGrep::Comodines => {
                let l = linea.to_lowercase();
                self.alternativas.iter().any(|frags| {
                    let (mut pos, mut primero) = (0usize, true);
                    let mut fin = 0usize;
                    for f in frags {
                        let Some(i) = l[pos..].find(f.as_str()) else { return false };
                        if primero && self.ancla_ini && !l[..pos + i].trim().is_empty() {
                            return false;
                        }
                        primero = false;
                        pos += i + f.len();
                        fin = pos;
                    }
                    !self.ancla_fin || l[fin..].trim().is_empty()
                })
            }
        }
    }
}

/// Coincidencias de un archivo: ruta, texto de cada línea (recortado a 300) y los índices de las que casan.
type Hallazgo = (String, Vec<String>, Vec<usize>);

struct Escaneo {
    pendientes: Vec<Hallazgo>,
    coincidencias: usize,
    con_hallazgo: usize,
    sin_leer: usize,
    binarios: usize,
}

fn escanear<E: Entorno>(e: &mut E, archivos: &[(String, String)], modo: ModoGrep, q: &Consulta) -> Escaneo {
    let mut r = Escaneo { pendientes: Vec::new(), coincidencias: 0, con_hallazgo: 0, sin_leer: 0, binarios: 0 };
    for (a, rel) in archivos {
        let datos = match e.leer(a) {
            Ok(d) => d,
            Err(_) => {
                r.sin_leer += 1;
                continue;
            }
        };
        if datos.contains(&0) {
            r.binarios += 1;
            continue;
        }
        let texto = String::from_utf8_lossy(&datos);
        let mut idx: Vec<usize> = Vec::new();
        let mut lineas: Vec<String> = Vec::new();
        for (n, l) in texto.split('\n').enumerate() {
            let l = l.trim_end_matches('\r');
            if q.casa(modo, l) {
                idx.push(n);
            }
            lineas.push(l.to_string());
        }
        if !idx.is_empty() {
            r.coincidencias += idx.len();
            r.con_hallazgo += 1;
            let lineas = lineas
                .into_iter()
                .map(|t| if t.len() > 300 { t[..t.char_indices().take_while(|(i, _)| *i < 300).last().map_or(0, |(i, c)| i + c.len_utf8())].to_string() } else { t })
                .collect();
            r.pendientes.push((rel.clone(), lineas, idx));
        }
    }
    r
}

/// ¿Esta línea DEFINE el identificador (`fn x`, `struct x`, `enum x`, `trait x`, `type x`, `const x`, `static x`, `mod x`, `impl … x`)?
fn es_definicion(linea: &str, ident: &str) -> bool {
    let t = linea.trim_start();
    let palabra_en = |trozo: &str| {
        trozo.match_indices(ident).any(|(i, _)| {
            let antes = trozo[..i].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
            let despues = trozo[i + ident.len()..].chars().next().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
            antes && despues
        })
    };
    if t.starts_with("impl") && palabra_en(t) {
        return true;
    }
    if t.starts_with("macro_rules!") && palabra_en(t) {
        return true;
    }
    ["fn ", "struct ", "enum ", "trait ", "type ", "const ", "static ", "mod ", "union "].iter().any(|kw| {
        t.match_indices(kw).any(|(i, _)| {
            let previo = t[..i].chars().next_back().is_none_or(|c| c == ' ' || c == ')');
            previo && t[i + kw.len()..].trim_start().starts_with(ident) && palabra_en(&t[i + kw.len()..])
        })
    })
}

fn es_identificador(p: &str) -> bool {
    !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || c == '_') && !p.chars().next().unwrap().is_ascii_digit()
}

/// Trozos de un identificador (`read_tensor_dims` → read, tensor, dims; `TensorDims` → tensor, dims), en minúsculas.
fn subtrozos(id: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let mut actual = String::new();
    let mut prev_min = false;
    for c in id.chars() {
        if c == '_' {
            if !actual.is_empty() {
                v.push(core::mem::take(&mut actual));
            }
            prev_min = false;
        } else {
            if c.is_uppercase() && prev_min && !actual.is_empty() {
                v.push(core::mem::take(&mut actual));
            }
            actual.extend(c.to_lowercase());
            prev_min = c.is_lowercase() || c.is_ascii_digit();
        }
    }
    if !actual.is_empty() {
        v.push(actual);
    }
    v
}

fn distancia(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut fila: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut previo = fila[0];
        fila[0] = i;
        for j in 1..=b.len() {
            let tmp = fila[j];
            fila[j] = (fila[j] + 1).min(fila[j - 1] + 1).min(previo + usize::from(a[i - 1] != b[j - 1]));
            previo = tmp;
        }
    }
    fila[b.len()]
}

/// Cuando una búsqueda no encuentra nada: identificadores que SÍ existen y se parecen a lo pedido (el modelo suele buscar un nombre
/// que no es exactamente el del código: `tensor_dims` cuando es `read_tensor_shape`). Devuelve hasta seis, con dónde aparecen.
fn sugerir<E: Entorno>(e: &mut E, archivos: &[(String, String)], patron: &str) -> Vec<String> {
    let ident = patron
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| w.len() >= 4)
        .max_by_key(|w| w.len())
        .map(|w| w.to_string());
    let Some(ident) = ident else { return Vec::new() };
    let q = ident.to_lowercase();
    let qs = subtrozos(&ident);
    let mut vistos: alloc::collections::BTreeMap<String, (usize, String)> = alloc::collections::BTreeMap::new();
    for (a, rel) in archivos {
        let Ok(datos) = e.leer(a) else { continue };
        if datos.contains(&0) {
            continue;
        }
        let texto = String::from_utf8_lossy(&datos);
        for (n, l) in texto.lines().enumerate() {
            for w in l.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| w.len() >= 4 && w.len() <= 48) {
                let ent = vistos.entry(w.to_string()).or_insert_with(|| (0, format!("{rel}:{}", n + 1)));
                ent.0 += 1;
            }
        }
    }
    let mut puntuados: Vec<(u32, usize, String, String)> = Vec::new();
    for (w, (veces, donde)) in &vistos {
        if *w == ident {
            continue;
        }
        let wl = w.to_lowercase();
        let ws = subtrozos(w);
        let comunes = qs.iter().filter(|x| x.len() >= 3 && ws.contains(x)).count();
        let sim = if qs.is_empty() { 0 } else { (100 * comunes / qs.len()) as u32 };
        let puntos = if wl.contains(&q) {
            100 + sim
        } else if q.contains(&wl) && wl.len() >= 5 {
            90
        } else if q.len() >= 6 && distancia(&q, &wl) <= 2 {
            85
        } else if sim >= 50 && comunes >= 1 {
            sim
        } else {
            0
        };
        if puntos > 0 {
            puntuados.push((puntos, *veces, w.clone(), donde.clone()));
        }
    }
    puntuados.sort_by(|x, y| y.0.cmp(&x.0).then(y.1.cmp(&x.1)).then(x.2.cmp(&y.2)));
    puntuados.into_iter().take(6).map(|(_, veces, w, donde)| format!("`{w}` ({veces} aparición(es); p. ej. {donde})")).collect()
}

fn buscar<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
    let patron = cadena(args, "pattern")?;
    if patron.is_empty() {
        return Err(Error::uso("el patrón está vacío"));
    }
    if parece_regex(patron) {
        return Ok(Respuesta::mal(format!(
            "El patrón «{patron}» parece una expresión regular con clases o cuantificadores (`\\w`, `\\d`, `.+`…), que esta herramienta no \
             interpreta: busca texto LITERAL, y admite `.*` entre trozos y `|` entre alternativas (`fn .*parse` , `leer|escribir`). \
             Busca un fragmento literal del código, por ejemplo el nombre de la función o una cadena que aparezca en él."
        )));
    }
    let (abs, rel) = resolver(pol, args.get("path").and_then(|v| v.as_str()).unwrap_or("."))?;
    let filtro = args.get("include").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
    let mut archivos: Vec<(String, String)> = Vec::new();
    let meta = e.metadatos(&abs).map_err(|_| no_existe(e, pol, &rel))?;
    if meta.tipo == Tipo::Directorio {
        recorrer(e, &abs, &rel, 0, &mut |a, r| {
            let nombre = r.rsplit('/').next().unwrap_or(r);
            if filtro.map_or(true, |f| glob_segmento(f.as_bytes(), nombre.as_bytes())) {
                archivos.push((a.to_string(), r.to_string()));
            }
        })?;
    } else {
        archivos.push((abs.clone(), rel.clone()));
    }

    let q = Consulta::nueva(patron);
    let con_comodines = patron.contains(".*") || patron.contains('|') || patron.starts_with('^') || patron.ends_with('$');
    let ignorar = valor(args, "ignorar_mayusculas").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut orden: Vec<ModoGrep> = Vec::new();
    if con_comodines {
        orden.push(ModoGrep::Comodines); // lo que el patrón dice explícitamente; si no, lo literal no tendría sentido
    }
    orden.push(if ignorar { ModoGrep::SinMayusculas } else { ModoGrep::Exacto });
    if !ignorar {
        orden.push(ModoGrep::SinMayusculas);
    }
    let mut r = Escaneo { pendientes: Vec::new(), coincidencias: 0, con_hallazgo: 0, sin_leer: 0, binarios: 0 };
    let mut modo_usado = orden[0];
    for m in &orden {
        r = escanear(e, &archivos, *m, &q);
        modo_usado = *m;
        if r.coincidencias > 0 {
            break;
        }
    }
    let revisados = archivos.len() - r.sin_leer - r.binarios;
    let nota_fallos = if r.sin_leer + r.binarios > 0 { format!(" ({} no se pudieron leer, {} binarios omitidos)", r.sin_leer, r.binarios) } else { String::new() };
    if r.coincidencias == 0 {
        // «No hay» sólo se dice si de verdad se pudo mirar; si no, es «no pude».
        let veredicto = if r.sin_leer > 0 && revisados == 0 { "No pude buscar" } else { "No hay coincidencias" };
        let mut t = format!("{veredicto} de «{patron}» en {revisados} archivo(s) revisado(s){nota_fallos}.");
        // Un patrón de varias palabras que no aparece: quizá sí aparezca quitando las primeras (`pub fn leer_x` → `leer_x`).
        let palabras: Vec<&str> = patron.split_whitespace().collect();
        if palabras.len() > 1 {
            for k in 1..palabras.len() {
                let resto = palabras[k..].join(" ");
                if resto.len() < 4 {
                    break;
                }
                let sub = escanear(e, &archivos, ModoGrep::Exacto, &Consulta::nueva(&resto));
                if sub.coincidencias > 0 {
                    let primeras: Vec<String> = sub.pendientes.iter().take(3).map(|(rr, ls, ix)| format!("  {rr}:{}: {}", ix[0] + 1, ls[ix[0]].trim())).collect();
                    t.push_str(&format!("\nPero «{resto}» sí aparece ({} coincidencia(s)):\n{}", sub.coincidencias, primeras.join("\n")));
                    return Ok(Respuesta::ok(t));
                }
            }
        }
        let sugerencias = sugerir(e, &archivos, patron);
        if !sugerencias.is_empty() {
            t.push_str(&format!("\n¿Quizá buscabas alguno de estos nombres, que sí existen?\n  {}", sugerencias.join("\n  ")));
        } else if con_comodines {
            t.push_str("\nSe interpretó como trozos en orden (`.*`) y alternativas (`|`), sin distinguir mayúsculas.");
        }
        return Ok(Respuesta::ok(t));
    }
    let aviso = match modo_usado {
        ModoGrep::Exacto => String::new(),
        ModoGrep::SinMayusculas => format!("Sin coincidencias exactas de «{patron}»; estas ignoran mayúsculas y minúsculas:\n"),
        ModoGrep::Comodines => format!("«{patron}» se ha interpretado como trozos en orden (`.*`) y alternativas (`|`), sin distinguir mayúsculas:\n"),
    };
    // Los archivos con la DEFINICIÓN del identificador buscado van primero y sus líneas se marcan.
    let ident_def = if es_identificador(patron) && r.coincidencias > 1 { Some(patron) } else { None };
    let mut pendientes = r.pendientes;
    if let Some(id) = ident_def {
        pendientes.sort_by_key(|(_, ls, ix)| core::cmp::Reverse(ix.iter().any(|&i| es_definicion(&ls[i], id))));
    }
    let explicito = entero(args, "contexto");
    let ctx = explicito.unwrap_or(if r.coincidencias <= 6 { 3 } else { 0 }).min(10);
    let mut salida = String::new();
    if r.coincidencias > 30 && explicito.is_none() {
        // Muchas coincidencias: un resumen por archivo (cuántas y dónde) en vez de volcarlas todas.
        let mut por_archivo: Vec<&Hallazgo> = pendientes.iter().collect();
        por_archivo.sort_by_key(|h| core::cmp::Reverse(h.2.len()));
        for (rr, ls, ix) in por_archivo.iter().take(15) {
            let lista: Vec<String> = ix.iter().take(6).map(|i| (i + 1).to_string()).collect();
            salida.push_str(&format!("{rr}: {} coincidencia(s) (líneas {}{})\n", ix.len(), lista.join(", "), if ix.len() > 6 { ", …" } else { "" }));
            for i in ix.iter().take(2) {
                let mut t = ls[*i].trim().to_string();
                if ident_def.is_some_and(|id| es_definicion(&ls[*i], id)) {
                    t.push_str("   ← definición");
                }
                salida.push_str(&format!("    {}: {t}\n", i + 1));
            }
        }
        if por_archivo.len() > 15 {
            salida.push_str(&format!("(… y {} archivos más)\n", por_archivo.len() - 15));
        }
        salida.push_str("Son muchas: afina con `path`, `include` o un texto más largo, o pide `contexto` para ver las líneas.\n");
    } else {
        let mut emitidas = 0usize;
        let mut lineas_salida = 0usize;
        'archivos: for (rr, ls, ix) in &pendientes {
            let mut ultimo: Option<usize> = None;
            for &h in ix {
                emitidas += 1;
                if emitidas > pol.resultados_max || lineas_salida > 250 {
                    break 'archivos;
                }
                let ini = h.saturating_sub(ctx).max(ultimo.map_or(0, |u| u + 1));
                if ctx > 0 && ultimo.is_some_and(|u| ini > u + 1) {
                    salida.push_str("--\n");
                }
                for i in ini..=(h + ctx).min(ls.len().saturating_sub(1)) {
                    if ix.contains(&i) {
                        let marca = if ident_def.is_some_and(|id| es_definicion(&ls[i], id)) { "   ← definición" } else { "" };
                        salida.push_str(&format!("{rr}:{}: {}{marca}\n", i + 1, ls[i].trim()));
                    } else {
                        salida.push_str(&format!("{rr}-{}- {}\n", i + 1, ls[i].trim_end()));
                    }
                    lineas_salida += 1;
                }
                ultimo = Some((h + ctx).min(ls.len().saturating_sub(1)));
            }
            if ctx > 0 {
                salida.push_str("--\n");
            }
        }
        if r.coincidencias > pol.resultados_max {
            salida.push_str(&format!("(… {} coincidencias más; afina la búsqueda)\n", r.coincidencias - pol.resultados_max));
        }
    }
    Ok(Respuesta::ok(format!("{aviso}{salida}{} coincidencia(s) en {} archivo(s){nota_fallos}.", r.coincidencias, r.con_hallazgo)))
}

// ---------------------------------------------------------------- reemplazar_texto

/// Resultado de localizar y sustituir un fragmento.
struct Cambio {
    nuevo_texto: String,
    linea: usize,
    antes: String,
}

/// Localiza `buscar` en `texto` (sin importar espacios ni sangría) y lo sustituye por `nuevo`. El `Err` exterior es un error de uso;
/// el interior es el mensaje para el modelo cuando no se puede aplicar. `linea_hint` desambigua si el fragmento aparece varias veces.
fn aplicar_edicion(rel: &str, texto: &str, buscar: &str, nuevo: &str, linea_hint: Option<usize>) -> Resultado<core::result::Result<Cambio, String>> {
    // El texto sin espacios y la posición (en bytes) de cada carácter que se conserva: así el
    // fragmento se localiza aunque el modelo haya comprimido o cambiado la sangría.
    let mut plano = String::new();
    let mut pos: Vec<(usize, usize)> = Vec::new(); // (inicio, fin) de cada carácter visible
    for (i, c) in texto.char_indices() {
        if !c.is_whitespace() {
            plano.push(c);
            pos.push((i, i + c.len_utf8()));
        }
    }
    let mut aguja: String = buscar.chars().filter(|c| !c.is_whitespace()).collect();
    if aguja.is_empty() {
        return Err(Error::uso("`buscar` está vacío"));
    }
    // Si no está tal cual, quizá el modelo copió los números de línea que muestra `read` (`33: …`): se quitan y se reintenta.
    if !plano.contains(&aguja) {
        let sin_numeros: String = buscar
            .lines()
            .map(|l| {
                let t = l.trim_start();
                let d = t.trim_start_matches(|c: char| c.is_ascii_digit());
                if d.len() < t.len() && (d.starts_with(':') || d.starts_with('|') || d.starts_with('\t')) { d[1..].to_string() } else { l.to_string() }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let alterna: String = sin_numeros.chars().filter(|c| !c.is_whitespace()).collect();
        if !alterna.is_empty() && plano.contains(&alterna) {
            aguja = alterna;
        }
    }
    let n_aguja = aguja.chars().count();
    // Índice de carácter de cada coincidencia en `plano`.
    let en_chars = |byte: usize| plano[..byte].chars().count();
    let Some(primero_b) = plano.find(&aguja) else {
        // ¿Ya está hecho? Tras una edición correcta el modelo suele repetirla con el texto ANTIGUO en `buscar`, que ya no existe: lo
        // que hace falta decirle no es «copia un trozo más corto» sino «esto ya está aplicado, sigue». Se le enseña cómo queda ahora.
        let nuevo_plano: String = nuevo.chars().filter(|c| !c.is_whitespace()).collect();
        if nuevo_plano.chars().count() >= 8 {
            if let Some(b) = plano.find(&nuevo_plano) {
                let ini = pos[en_chars(b)].0;
                let linea = texto[..ini].matches('\n').count() + 1;
                let ventana: Vec<String> =
                    texto.lines().enumerate().skip(linea - 1).take(8).map(|(i, l)| format!("{}: {l}", i + 1)).collect();
                return Ok(Err(format!(
                    "Ese cambio YA está aplicado: el texto de `nuevo` aparece en {} desde la línea {linea}. Así está ahora:\n{}\nNo repitas la edición. Pasa al siguiente paso: ejecuta las comprobaciones con `bash`, o edita otra parte del archivo.",
                    mostrar(rel),
                    ventana.join("\n")
                )));
            }
        }
        // Un bloque de varias líneas que no aparece suele ser un bloque que el propio modelo ya cambió: se le enseña el tramo
        // ACTUAL entero (no sólo cinco líneas) para que lo copie tal cual.
        let pista = if buscar.lines().filter(|l| !l.trim().is_empty()).count() >= 3 {
            tramo_parecido(&texto, buscar)
                .map(|(a, b, t)| format!("\nEl archivo ha cambiado desde que lo leíste (tus ediciones anteriores lo modificaron). Así está AHORA el tramo más parecido (líneas {a}–{b}):\n{t}\nCopia `buscar` de AHÍ (una o dos líneas bastan), o cambia ese tramo por números de línea con `reemplazar_lineas` (desde, hasta, nuevo), que no necesita copiar nada."))
                .unwrap_or_default()
        } else {
            linea_parecida(&texto, buscar)
                .map(|p| format!("\nLa línea más parecida del archivo es:\n{p}\nCopia de ahí UNA línea completa y corta, o usa `reemplazar_lineas` con el rango de líneas."))
                .unwrap_or_default()
        };
        return Ok(Err(format!(
            "No encuentro ese fragmento en {}. Copia un trozo más corto de lo que te mostró `read`, sin los números de línea.{pista}",
            mostrar(rel)
        )));
    };
    // Todas las apariciones (posición en `plano`, en bytes), para elegir la que pide `linea` o enumerarlas si hay duda.
    let paso = |b: usize| plano[b..].chars().next().map_or(1, |c| c.len_utf8());
    let mut apariciones: Vec<usize> = alloc::vec![primero_b];
    let mut desde = primero_b + paso(primero_b);
    while let Some(sig) = plano[desde..].find(&aguja) {
        apariciones.push(desde + sig);
        desde += sig + paso(desde + sig);
    }
    let linea_de = |b: usize| texto[..pos[en_chars(b)].0].matches('\n').count() + 1;
    let elegido = if apariciones.len() == 1 {
        // `linea` lejos de la única aparición: el modelo cree que edita OTRA línea. Aplicarla en silencio cambiaba una que no quería
        // (campaña 27: pedía la 143 y se cambiaba la 107, y el modelo alternaba entre las dos sin entender qué pasaba).
        let donde = linea_de(primero_b);
        if let Some(h) = linea_hint {
            if h.abs_diff(donde) > 10 + buscar.lines().count() {
                return Ok(Err(format!(
                    "`buscar` sólo aparece en la línea {donde} de {}, no cerca de la {h} que indicas ({}). No he cambiado nada. Si quieres esa línea {donde}, repite con `linea`: {donde}; si querías otra, lee ese tramo con `read` y copia su texto.",
                    mostrar(rel),
                    texto.lines().nth(donde - 1).unwrap_or("").trim()
                )));
            }
        }
        primero_b
    } else if let Some(h) = linea_hint {
        *apariciones.iter().min_by_key(|b| linea_de(**b).abs_diff(h)).unwrap()
    } else {
        let lista: Vec<String> = apariciones
            .iter()
            .take(8)
            .map(|b| {
                let l = linea_de(*b);
                format!("  línea {l}: {}", texto.lines().nth(l - 1).unwrap_or("").trim())
            })
            .collect();
        return Ok(Err(format!(
            "El fragmento aparece {} veces en {}:\n{}\nAñade más contexto a `buscar`, o indica en `linea` el número de línea de la aparición que quieres cambiar.",
            apariciones.len(),
            mostrar(rel),
            lista.join("\n")
        )));
    };
    // Edición «añadir tras/antes de buscar» ya aplicada: `nuevo` contiene `buscar` más líneas extra y el archivo YA tiene exactamente eso
    // en esa posición. Repetirla duplicaría lo añadido cada vez, y devolvía «Hecho» (campaña 28: 30 veces el mismo guardia en R31).
    let nuevo_plano: String = nuevo.chars().filter(|c| !c.is_whitespace()).collect();
    // Lo añadido ha de ser sustancioso (≥ 12 caracteres sin espacios): un `}` suelto que casualmente sigue en el archivo no cuenta.
    if nuevo_plano.len() >= aguja.len() + 12 {
        let ya_despues = plano[elegido..].starts_with(&nuevo_plano);
        let ya_antes = plano[..elegido + aguja.len()].ends_with(&nuevo_plano);
        if ya_despues || ya_antes {
            return Ok(Err(format!(
                "Ese cambio YA está aplicado en la línea {}: {} ya contiene `buscar` junto con lo que añades. Repetirlo lo DUPLICARÍA, así que no he cambiado nada. Pasa al siguiente paso: ejecuta las comprobaciones con `bash`, o edita otra parte.",
                linea_de(elegido),
                mostrar(rel)
            )));
        }
    }
    let i0 = en_chars(elegido);
    let (ini, fin) = (pos[i0].0, pos[i0 + n_aguja - 1].1);
    // El fragmento se localiza sin espacios, así que `ini` cae en el primer carácter visible. Si `nuevo` trae su propia sangría
    // (`    fn x`) y se pegara ahí, la línea quedaría con la sangría doble: se retrocede al principio de la línea y se sustituye también.
    let inicio_linea = texto[..ini].rfind('\n').map_or(0, |p| p + 1);
    let ini = if inicio_linea < ini && texto[inicio_linea..ini].chars().all(|c| c == ' ' || c == '\t') && nuevo.starts_with([' ', '\t']) { inicio_linea } else { ini };
    let antes = texto[ini..fin].to_string();
    let linea = texto[..ini].matches('\n').count() + 1;
    // Un atributo o comentario de documentación que `nuevo` repite y que ya está justo encima se descarta (`#[derive(..)]` duplicado).
    let nuevo = {
        let previa = texto[..inicio_linea].trim_end_matches('\n').rsplit('\n').next().unwrap_or("").trim();
        let (primera, resto) = nuevo.split_once('\n').unwrap_or((nuevo, ""));
        if !resto.is_empty() && !previa.is_empty() && primera.trim() == previa && (previa.starts_with("#[") || previa.starts_with("///")) { resto } else { nuevo }
    };
    let viejas_l: Vec<&str> = antes.split('\n').collect();
    let nuevas_l: Vec<&str> = nuevo.split('\n').collect();
    let nuevo = restaurar_espacios(&viejas_l, &nuevas_l).join("\n");
    let mut nuevo_texto = String::with_capacity(texto.len() + nuevo.len());
    nuevo_texto.push_str(&texto[..ini]);
    nuevo_texto.push_str(&nuevo);
    nuevo_texto.push_str(&texto[fin..]);
    Ok(Ok(Cambio { nuevo_texto, linea, antes }))
}

fn reemplazar<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
    let (abs, rel) = resolver(pol, cadena(args, "filePath")?)?;
    // Una edición (`buscar`, `nuevo`) o varias a la vez (`ediciones`: lista de {buscar, nuevo, linea}), todo o nada.
    let ediciones: Vec<(String, String, Option<usize>)> = match args.get("ediciones").and_then(|v| v.as_array()) {
        Some(lista) if !lista.is_empty() => {
            let mut v = Vec::new();
            for (i, x) in lista.iter().enumerate() {
                let b = cadena(x, "buscar").map_err(|_| Error::uso(format!("ediciones[{i}]: falta «buscar»")))?;
                let n = cadena(x, "nuevo").map_err(|_| Error::uso(format!("ediciones[{i}]: falta «nuevo»")))?;
                v.push((b.to_string(), n.to_string(), entero(x, "linea")));
            }
            v
        }
        _ => alloc::vec![(cadena(args, "buscar")?.to_string(), cadena(args, "nuevo")?.to_string(), entero(args, "linea"))],
    };
    let datos = e.leer(&abs).map_err(|_| no_existe(e, pol, &rel))?;
    let texto = String::from_utf8(datos).map_err(|_| Error::uso(format!("«{}» no es UTF-8", mostrar(&rel))))?;
    let total = ediciones.len();
    let mut actual = texto.clone();
    let mut hechas: Vec<(usize, String, String)> = Vec::new();
    for (k, (b, n, l)) in ediciones.iter().enumerate() {
        match aplicar_edicion(&rel, &actual, b, n, *l)? {
            Ok(c) => {
                let viejas: Vec<&str> = c.antes.lines().collect();
                let nuevas_l: Vec<&str> = n.lines().collect();
                if let Some(m) = borrado_colateral(&rel, &viejas, &nuevas_l, args.get("confirmar_borrado").and_then(|v| v.as_bool()).unwrap_or(false)) {
                    return Ok(Respuesta::mal(m));
                }
                hechas.push((c.linea, c.antes, n.clone()));
                actual = c.nuevo_texto;
            }
            Err(m) => {
                let pref = if total > 1 { format!("La edición {} de {total} no se pudo aplicar, así que NO se ha escrito ningún cambio. ", k + 1) } else { String::new() };
                return Ok(Respuesta::mal(format!("{pref}{m}")));
            }
        }
    }
    if actual == texto {
        // Una edición que deja el archivo idéntico NO es «Hecho»: el modelo la repetía sin parar (campaña 27: 27 veces seguidas la misma
        // línea) creyendo que avanzaba, y cada «Hecho» reiniciaba las guardas de repetición.
        return Ok(Respuesta::mal(format!(
            "Esa edición no cambia nada: {} ya contiene exactamente ese texto (línea {}). No la repitas. Si querías otra cosa, mira el estado real con `read` (con `offset` y `limit`); si el cambio ya está hecho, ejecuta las comprobaciones con `bash`.",
            mostrar(&rel),
            hechas.first().map_or(0, |h| h.0)
        )));
    }
    if let Some(m) = desequilibrio(&rel, &texto, &actual) {
        return Ok(Respuesta::mal(m));
    }
    if let Some(m) = casi_solo_comentarios(&rel, &ediciones.iter().map(|e| e.1.as_str()).collect::<Vec<_>>().join("\n")) {
        return Ok(Respuesta::mal(m));
    }
    if let Some(m) = modulo_de_pruebas_duplicado(&rel, &texto, &actual) {
        return Ok(Respuesta::mal(m));
    }
    if let Some(m) = error_de_sintaxis(e, pol, &rel, &texto, &actual) {
        return Ok(Respuesta::mal(m));
    }
    let modo = e.metadatos(&abs).map(|m| m.modo).unwrap_or(0o644);
    e.escribir(&abs, actual.as_bytes(), modo)?;
    if total == 1 {
        let (linea, antes, nuevo) = &hechas[0];
        let n_nuevo = nuevo.lines().count().max(1);
        let ventana = ventana_numerada(&actual, linea.saturating_sub(2).max(1), linea + n_nuevo + 2);
        return Ok(Respuesta::ok(format!("Hecho en {}, línea {linea}.\nAntes:\n{antes}\nDespués:\n{nuevo}\nAsí queda ahora (numeración nueva):\n{ventana}", mostrar(&rel))));
    }
    let resumen: Vec<String> = hechas
        .iter()
        .enumerate()
        .map(|(i, (l, a, n))| {
            let corto = |t: &str| {
                let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
                if t.chars().count() > 90 { format!("{}…", t.chars().take(90).collect::<String>()) } else { t }
            };
            format!("{}) línea {l}: «{}» → «{}»", i + 1, corto(a), corto(n))
        })
        .collect();
    Ok(Respuesta::ok(format!("Hecho: {total} ediciones en {}.\n{}", mostrar(&rel), resumen.join("\n"))))
}

/// Última línea (1-based) del bloque que empieza en `desde`: la primera donde, contando desde `desde`, las llaves, paréntesis y
/// corchetes vuelven a quedar equilibrados tras haber abierto alguno. Sirve de pista cuando `reemplazar_lineas` se queda corto o se
/// pasa del final de una función (campaña 33, R32: 13 intentos con `hasta` = 222 cuando la función acababa en la 224).
fn fin_de_bloque(lineas: &[&str], desde: usize) -> Option<usize> {
    let (mut acc, mut abierto) = ([0i64; 3], false);
    for (i, l) in lineas.iter().enumerate().skip(desde.saturating_sub(1)) {
        let d = equilibrio_rs(l);
        for k in 0..3 {
            acc[k] += d[k];
        }
        abierto |= d.iter().any(|x| *x > 0);
        if abierto && acc == [0, 0, 0] {
            return Some(i + 1);
        }
    }
    None
}

/// Líneas «de código» (no sólo llaves, paréntesis o comentarios) de `viejas` que ya no aparecen en `nuevas` (comparación sin sangría).
fn lineas_perdidas(viejas: &[&str], nuevas: &[&str]) -> Vec<String> {
    let conservadas: alloc::collections::BTreeSet<&str> = nuevas.iter().map(|l| l.trim()).collect();
    viejas
        .iter()
        .map(|l| l.trim())
        .filter(|t| !t.is_empty() && !t.starts_with("//") && t.chars().any(|c| c.is_alphanumeric()) && !conservadas.contains(t))
        .map(|t| t.to_string())
        .collect()
}

/// Edición que BORRA bastante código que `nuevo` no conserva (R35 de la campaña 39: al añadir una comprobación se llevó por delante el cálculo
/// del alineamiento de un parser y no se notó). Se pide confirmación una vez con `confirmar_borrado: true`; si el borrado es intencionado, se repite.
fn borrado_colateral(archivo: &str, viejas: &[&str], nuevas: &[&str], confirmado: bool) -> Option<String> {
    if confirmado || viejas.len() < nuevas.len() + 4 {
        return None;
    }
    let perdidas = lineas_perdidas(viejas, nuevas);
    if perdidas.len() < 4 {
        return None;
    }
    let ejemplos: Vec<String> = perdidas.iter().take(6).map(|l| format!("  {}", if l.chars().count() > 100 { format!("{}…", l.chars().take(100).collect::<String>()) } else { l.clone() })).collect();
    Some(format!(
        "No he escrito el cambio: BORRARÍA {} líneas de código de {} que `nuevo` no conserva ({} reemplazadas por {}), por ejemplo:\n{}\nSi las querías borrar, repite la misma llamada con `confirmar_borrado`: true. Si sólo querías cambiar o añadir algo, incluye en `nuevo` las líneas que deben seguir ahí (o reduce el rango).",
        perdidas.len(),
        mostrar(archivo),
        viejas.len(),
        nuevas.len(),
        ejemplos.join("\n")
    ))
}

/// Los modelos reescriben las líneas que no cambian con los espacios comprimidos (`data:&[u8]`, `Qwen3.8:hidden`, hasta dentro de una cadena
/// `"hola mundo"` → `"holamundo"`): la herramienta es insensible a los espacios al BUSCAR y, al sustituir, los dejaba así. Aquí se alinean las
/// líneas nuevas con las viejas ignorando TODOS los espacios y, donde coinciden y tienen la misma sangría, se conserva el texto ORIGINAL.
fn restaurar_espacios(viejas: &[&str], nuevas: &[&str]) -> Vec<String> {
    let clave = |l: &str| -> String { l.chars().filter(|c| !c.is_whitespace()).collect() };
    let sangria = |l: &str| l.len() - l.trim_start().len();
    let (kv, kn): (Vec<String>, Vec<String>) = (viejas.iter().map(|l| clave(l)).collect(), nuevas.iter().map(|l| clave(l)).collect());
    let (n, m) = (kv.len(), kn.len());
    if n == 0 || m == 0 || n.saturating_mul(m) > 4_000_000 {
        return nuevas.iter().map(|l| l.to_string()).collect();
    }
    let w = m + 1;
    let mut t: Vec<u16> = alloc::vec![0; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i * w + j] = if kv[i] == kn[j] { t[(i + 1) * w + j + 1].saturating_add(1) } else { t[(i + 1) * w + j].max(t[i * w + j + 1]) };
        }
    }
    let mut sal: Vec<String> = Vec::with_capacity(m);
    let (mut i, mut j) = (0, 0);
    while j < m {
        if i < n && kv[i] == kn[j] && !kn[j].is_empty() {
            let misma_sangria = sangria(viejas[i]) == sangria(nuevas[j]) && viejas[i].chars().take(sangria(viejas[i])).eq(nuevas[j].chars().take(sangria(nuevas[j])));
            sal.push(if misma_sangria { viejas[i].to_string() } else { nuevas[j].to_string() });
            i += 1;
            j += 1;
        } else if i < n && t[(i + 1) * w + j] >= t[i * w + j + 1] {
            i += 1;
        } else {
            sal.push(nuevas[j].to_string());
            j += 1;
        }
    }
    sal
}

/// Primera línea (1-based) del bloque cuyo cierre está en `hasta`: hacia atrás, la primera `desde` tal que las líneas `desde..=hasta`
/// quedan equilibradas y la línea `desde` abre algo. Para cuando el modelo empieza el rango DENTRO de un bloque.
fn inicio_de_bloque(lineas: &[&str], hasta: usize) -> Option<usize> {
    let mut acc = [0i64; 3];
    let mut i = hasta;
    let limite = hasta.saturating_sub(400);
    while i > limite && i >= 1 {
        let d = equilibrio_rs(lineas[i - 1]);
        for k in 0..3 {
            acc[k] += d[k];
        }
        if acc == [0, 0, 0] && d.iter().any(|x| *x > 0) {
            return Some(i);
        }
        i -= 1;
    }
    None
}

fn unir_lineas(lineas: &[&str], desde: usize, hasta: usize, nuevas: &[&str], con_salto: bool) -> String {
    let restauradas = restaurar_espacios(&lineas[desde - 1..hasta], nuevas);
    let mut r: Vec<&str> = Vec::with_capacity(lineas.len() + nuevas.len());
    r.extend_from_slice(&lineas[..desde - 1]);
    r.extend(restauradas.iter().map(|l| l.as_str()));
    r.extend_from_slice(&lineas[hasta..]);
    let mut t = r.join("\n");
    if con_salto {
        t.push('\n');
    }
    t
}

fn reemplazar_lineas<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
    let (abs, rel) = resolver(pol, cadena(args, "filePath")?)?;
    let desde = entero(args, "desde").ok_or_else(|| Error::uso("falta «desde» (entero: primera línea a sustituir)"))?;
    let hasta = entero(args, "hasta").ok_or_else(|| Error::uso("falta «hasta» (entero: última línea a sustituir, incluida)"))?;
    let nuevo = cadena(args, "nuevo")?;
    let datos = e.leer(&abs).map_err(|_| no_existe(e, pol, &rel))?;
    let texto = String::from_utf8(datos).map_err(|_| Error::uso(format!("«{}» no es UTF-8", mostrar(&rel))))?;
    let mut lineas: Vec<&str> = texto.split('\n').collect();
    let terminaba_en_salto = lineas.last() == Some(&"");
    if terminaba_en_salto {
        lineas.pop();
    }
    let total = lineas.len();
    if desde == 0 || desde > hasta || hasta > total {
        return Ok(Respuesta::mal(format!(
            "Rango inválido {desde}–{hasta}: {} tiene {total} línea(s) y hace falta 1 ≤ desde ≤ hasta ≤ {total}. Mira los números con `read` (con `offset` y `limit`).",
            mostrar(&rel)
        )));
    }
    let nuevas: Vec<&str> = if nuevo.is_empty() { Vec::new() } else { nuevo.trim_end_matches('\n').split('\n').collect() };
    let (mut desde, mut hasta) = (desde, hasta);
    let mut nuevo_texto = unir_lineas(&lineas, desde, hasta, &nuevas, terminaba_en_salto);
    if nuevo_texto == texto {
        return Ok(Respuesta::mal(format!(
            "Esa sustitución no cambia nada: las líneas {desde}–{hasta} de {} ya son exactamente ese texto. No la repitas; comprueba con `bash` o edita otra parte.",
            mostrar(&rel)
        )));
    }
    let mut ajuste = String::new();
    if let Some(mut m) = desequilibrio(&rel, &texto, &nuevo_texto) {
        // El 85 % de estos rechazos (campañas 28–38: 79 de 93) traían un `nuevo` equilibrado y un RANGO que corta un bloque: se queda
        // corto (borra aperturas y deja sus cierres) o empieza dentro. Si ampliar el rango al bloque entero deja el archivo equilibrado,
        // se aplica así y se le dice, en vez de devolverle el error veinte veces.
        // Sólo si `nuevo` es un bloque completo (equilibrado y con alguna apertura): borrar o recortar líneas sueltas es otra intención y
        // ampliar el rango se llevaría por delante código que el modelo no quería tocar.
        let es_bloque = {
            let b = equilibrio_rs(&nuevas.join("\n"));
            b == [0, 0, 0] && nuevas.iter().any(|l| l.contains('{') || l.contains('('))
        };
        let mut candidatos: Vec<(usize, usize)> = Vec::new();
        if !es_bloque {
            // nada que ampliar
        } else if let Some(fin) = fin_de_bloque(&lineas, desde) {
            if fin > hasta {
                candidatos.push((desde, fin));
            }
        }
        if let Some(ini) = inicio_de_bloque(&lineas, hasta).filter(|_| es_bloque) {
            if ini < desde {
                candidatos.push((ini, hasta));
            }
        }
        let mut resuelto = false;
        for (d2, h2) in candidatos {
            if (h2 - d2) > (hasta - desde) + 80 {
                continue;
            }
            let t2 = unir_lineas(&lineas, d2, h2, &nuevas, terminaba_en_salto);
            if desequilibrio(&rel, &texto, &t2).is_none() && t2 != texto {
                ajuste = format!("AJUSTE: tu rango {desde}–{hasta} cortaba un bloque y dejaba las llaves desequilibradas; lo he ampliado a {d2}–{h2} (el bloque entero) para que `nuevo` lo sustituya completo. Si no querías tocar esas líneas, vuelve a leer el tramo y repite con otro rango.\n");
                desde = d2;
                hasta = h2;
                nuevo_texto = t2;
                resuelto = true;
                break;
            }
        }
        if !resuelto {
            // El rango ya abarca un bloque entero (o no hay forma de cuadrarlo): el desequilibrio viene de `nuevo`. Se le enseña el balance de cada parte.
            let b_old = equilibrio_rs(&lineas[desde - 1..hasta].join("\n"));
            let b_new = equilibrio_rs(&nuevas.join("\n"));
            m.push_str(&format!(
                " Balance de las líneas que reemplazas ({desde}–{hasta}): llaves {:+}, paréntesis {:+}, corchetes {:+}; balance de tu `nuevo`: llaves {:+}, paréntesis {:+}, corchetes {:+}. Para que quede equilibrado deben ser iguales.",
                b_old[2], b_old[0], b_old[1], b_new[2], b_new[0], b_new[1]
            ));
            if let Some(fin) = fin_de_bloque(&lineas, desde) {
                if fin != hasta {
                    m.push_str(&format!(" Pista: el bloque que empieza en la línea {desde} termina en la {fin}."));
                }
            }
            return Ok(Respuesta::mal(m));
        }
    }
    // Con un AJUSTE de rango `nuevo` ya es un bloque completo que sustituye al bloque entero: reescribir una función no es un borrado colateral.
    let confirmado = ajuste.contains("AJUSTE") || args.get("confirmar_borrado").and_then(|v| v.as_bool()).unwrap_or(false);
    if let Some(m) = borrado_colateral(&rel, &lineas[desde - 1..hasta], &nuevas, confirmado) {
        return Ok(Respuesta::mal(m));
    }
    if let Some(m) = casi_solo_comentarios(&rel, nuevo) {
        return Ok(Respuesta::mal(m));
    }
    if let Some(m) = modulo_de_pruebas_duplicado(&rel, &texto, &nuevo_texto) {
        return Ok(Respuesta::mal(m));
    }
    if let Some(m) = error_de_sintaxis(e, pol, &rel, &texto, &nuevo_texto) {
        return Ok(Respuesta::mal(m));
    }
    let sustituidas: Vec<&str> = lineas[desde - 1..hasta].to_vec();
    let modo = e.metadatos(&abs).map(|m| m.modo).unwrap_or(0o644);
    e.escribir(&abs, nuevo_texto.as_bytes(), modo)?;
    // Lo que se ha quitado (para que un rango mal elegido se note) y cómo queda, con la numeración nueva.
    let corto = |l: &str| if l.chars().count() > 100 { format!("{}…", l.chars().take(100).collect::<String>()) } else { l.to_string() };
    let mut quitado: Vec<String> = sustituidas.iter().take(3).map(|l| format!("  {}", corto(l))).collect();
    if sustituidas.len() > 3 {
        quitado.push(format!("  … ({} líneas más)", sustituidas.len() - 3));
    }
    let resultado_n = nuevo_texto.lines().count();
    let ventana = ventana_numerada(&nuevo_texto, desde.saturating_sub(3).max(1), (desde + nuevas.len() + 2).min(resultado_n));
    Ok(Respuesta::ok(format!(
        "Hecho en {}: líneas {desde}–{hasta} ({} línea(s)) sustituidas por {} línea(s).\n{ajuste}Se quitó:\n{}\nAsí queda ahora (numeración nueva):\n{}",
        mostrar(&rel),
        sustituidas.len(),
        nuevas.len(),
        quitado.join("\n"),
        ventana
    )))
}

/// Las líneas `desde..=hasta` (1-based) de `texto`, numeradas como `read`; hasta 40.
fn ventana_numerada(texto: &str, desde: usize, hasta: usize) -> String {
    let ls: Vec<&str> = texto.lines().collect();
    let hasta = hasta.min(ls.len()).min(desde + 40);
    (desde..=hasta).filter(|i| *i >= 1 && *i <= ls.len()).map(|i| format!("{i}: {}", ls[i - 1])).collect::<Vec<_>>().join("\n")
}

// ---------------------------------------------------------------- estructura de un archivo Rust (esquema, símbolo, reemplazar_funcion)

/// Un elemento de primer nivel o anidado (`fn`, `struct`, `enum`, `trait`, `impl`, `mod`, `type`, `const`, `static`) con su rango de líneas.
struct Elemento {
    ini: usize,
    fin: usize,
    clase: &'static str,
    nombre: String,
    firma: String,
}

const CLASES: [&str; 9] = ["fn", "struct", "enum", "trait", "impl", "mod", "type", "const", "static"];

/// ¿Empieza aquí un elemento? Devuelve su clase, su nombre (para `impl`, la cabecera sin `impl`) y la línea sin comentarios.
fn cabecera_de_elemento(linea: &str) -> Option<(&'static str, String)> {
    let mut t = linea.trim_start();
    if t.starts_with("//") || t.starts_with('#') {
        return None;
    }
    // Modificadores que preceden a la clase.
    loop {
        let antes = t;
        for m in ["pub(crate) ", "pub(super) ", "pub ", "async ", "unsafe ", "default ", "extern \"C\" ", "extern "] {
            if let Some(r) = t.strip_prefix(m) {
                t = r;
            }
        }
        if t.starts_with("pub(") {
            if let Some(i) = t.find(')') {
                t = t[i + 1..].trim_start();
            }
        }
        if t == antes {
            break;
        }
    }
    // `const fn` y `unsafe impl`: `const` seguido de `fn` es una función.
    let t = if let Some(r) = t.strip_prefix("const ") { if r.starts_with("fn ") { r } else { t } } else { t };
    for clase in CLASES {
        if let Some(r) = t.strip_prefix(clase) {
            if !r.starts_with(' ') && !r.starts_with('<') {
                continue;
            }
            let r = r.trim_start();
            let nombre: String = if clase == "impl" {
                r.split('{').next().unwrap_or("").trim().to_string()
            } else {
                r.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect()
            };
            if nombre.is_empty() {
                return None;
            }
            return Some((clase, nombre));
        }
    }
    None
}

/// Última línea (1-based) del elemento que empieza en `ini`: la que cierra su `{ }`, o la que acaba en `;` si no tiene cuerpo.
fn fin_de_elemento(lineas: &[&str], ini: usize) -> Option<usize> {
    let (mut acc, mut llave) = ([0i64; 3], false);
    for (i, l) in lineas.iter().enumerate().skip(ini - 1) {
        let d = equilibrio_rs(l);
        for k in 0..3 {
            acc[k] += d[k];
        }
        // `fn f() {}` en una sola línea tiene balance 0 pero SÍ tiene cuerpo: basta con que la línea abra una llave.
        if l.split("//").next().unwrap_or("").contains('{') {
            llave = true;
        }
        if acc == [0, 0, 0] {
            if llave {
                return Some(i + 1);
            }
            let codigo = l.split("//").next().unwrap_or("").trim_end();
            if codigo.ends_with(';') {
                return Some(i + 1);
            }
        }
        if i + 1 - ini > 1500 {
            break;
        }
    }
    None
}

fn elementos(lineas: &[&str]) -> Vec<Elemento> {
    let mut v = Vec::new();
    for (i, l) in lineas.iter().enumerate() {
        if let Some((clase, nombre)) = cabecera_de_elemento(l) {
            if let Some(fin) = fin_de_elemento(lineas, i + 1) {
                let firma: String = l.trim().chars().take(110).collect();
                v.push(Elemento { ini: i + 1, fin, clase, nombre, firma });
            }
        }
    }
    v
}

fn esquema_texto(rel: &str, lineas: &[&str]) -> String {
    let els = elementos(lineas);
    if els.is_empty() {
        return format!("<path>{}</path>\n<type>schema</type>\n(no he encontrado funciones, tipos ni módulos en este archivo)", mostrar(rel));
    }
    let mut t = format!("<path>{}</path>\n<type>schema</type>\n<items>\n", mostrar(rel));
    for x in els.iter().take(200) {
        let sangria = lineas[x.ini - 1].len() - lineas[x.ini - 1].trim_start().len();
        t.push_str(&format!("{}{}–{}  {}\n", " ".repeat(sangria.min(12)), x.ini, x.fin, x.firma));
    }
    if els.len() > 200 {
        t.push_str(&format!("(… y {} más)\n", els.len() - 200));
    }
    t.push_str("</items>\nCon `read` y `simbolo` lees uno entero; con `reemplazar_funcion` sustituyes uno por su nombre, sin números de línea.");
    t
}

/// Elementos de `lineas` que se llaman `nombre` (o, para `impl`, cuya cabecera lo contiene).
fn buscar_elementos<'a>(els: &'a [Elemento], nombre: &str) -> Vec<&'a Elemento> {
    let n = nombre.trim().trim_start_matches("fn ").trim();
    // `Tipo::método` (como lo escribe un modelo): el método que está DENTRO de un `impl` de ese tipo.
    if let Some((tipo, metodo)) = n.rsplit_once("::") {
        let tipo = tipo.rsplit("::").next().unwrap_or(tipo);
        let dentro: Vec<&Elemento> = els
            .iter()
            .filter(|x| x.nombre == metodo && x.clase == "fn")
            .filter(|f| els.iter().any(|i| i.clase == "impl" && i.nombre.split_whitespace().last().is_some_and(|u| u.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_') == tipo) && i.ini < f.ini && f.fin <= i.fin))
            .collect();
        if !dentro.is_empty() {
            return dentro;
        }
        return els.iter().filter(|x| x.nombre == metodo).collect();
    }
    els.iter().filter(|x| x.nombre == n || (x.clase == "impl" && x.nombre.contains(n))).collect()
}

fn parecidos(els: &[Elemento], nombre: &str) -> Vec<String> {
    let q = nombre.to_lowercase();
    let mut c: Vec<(usize, String)> = els
        .iter()
        .filter(|x| x.clase != "impl")
        .map(|x| (distancia(&q, &x.nombre.to_lowercase()) + if x.nombre.to_lowercase().contains(&q) { 0 } else { 3 }, format!("`{}` ({} {}–{})", x.nombre, x.clase, x.ini, x.fin)))
        .collect();
    c.sort();
    c.dedup_by(|a, b| a.1 == b.1);
    c.into_iter().take(5).map(|(_, t)| t).collect()
}

fn leer_simbolo<E: Entorno>(e: &mut E, pol: &Politica, args: &Value, simbolo: &str) -> Resultado<Respuesta> {
    let (abs, rel) = resolver(pol, cadena(args, "filePath")?)?;
    let datos = e.leer(&abs).map_err(|_| no_existe(e, pol, &rel))?;
    let texto = String::from_utf8_lossy(&datos).into_owned();
    let lineas: Vec<&str> = texto.lines().collect();
    let els = elementos(&lineas);
    let hallados = buscar_elementos(&els, simbolo);
    match hallados.as_slice() {
        [] => {
            let p = parecidos(&els, simbolo);
            let ayuda = if p.is_empty() { String::new() } else { format!(" Nombres parecidos: {}.", p.join(", ")) };
            Ok(Respuesta::mal(format!("No hay ninguna función, tipo ni módulo llamado «{simbolo}» en {}.{ayuda} `read` con `esquema` los lista todos.", mostrar(&rel))))
        }
        [x] => {
            let fin = x.fin.min(x.ini + 250);
            let cuerpo = ventana_numerada(&texto, x.ini, fin);
            let mas = if x.fin > fin { format!("\n(… sigue hasta la línea {}; usa `offset` para el resto)", x.fin) } else { String::new() };
            Ok(Respuesta::ok(format!("<path>{}</path>\n<symbol>{} {}</symbol> (líneas {}–{})\n<content>\n{cuerpo}{mas}\n</content>", mostrar(&rel), x.clase, x.nombre, x.ini, x.fin)))
        }
        varios => {
            let lista: Vec<String> = varios.iter().map(|x| format!("  {} {}–{}: {}   → read offset={} limit={}", x.clase, x.ini, x.fin, x.firma, x.ini, (x.fin - x.ini + 1).min(250))).collect();
            Ok(Respuesta::mal(format!("«{simbolo}» aparece {} veces en {}:\n{}\nElige una y llama a `read` con ese `offset` y `limit` (los de arriba). Para un método concreto de un tipo escribe `Tipo::método`.", varios.len(), mostrar(&rel), lista.join("\n"))))
        }
    }
}

/// Sustituye un elemento (función, tipo, módulo) por su nombre: el rango lo calcula la herramienta, no el modelo.
fn reemplazar_funcion<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
    let (abs, rel) = resolver(pol, cadena(args, "filePath")?)?;
    let nombre = cadena(args, "nombre")?;
    let nuevo = cadena(args, "nuevo")?;
    let linea = entero(args, "linea");
    let datos = e.leer(&abs).map_err(|_| no_existe(e, pol, &rel))?;
    let texto = String::from_utf8(datos).map_err(|_| Error::uso(format!("«{}» no es UTF-8", mostrar(&rel))))?;
    let lineas: Vec<&str> = texto.lines().collect();
    let els = elementos(&lineas);
    let hallados = buscar_elementos(&els, nombre);
    let x = match (hallados.as_slice(), linea) {
        ([], _) => {
            let p = parecidos(&els, nombre);
            let ayuda = if p.is_empty() { String::new() } else { format!(" Nombres parecidos: {}.", p.join(", ")) };
            return Ok(Respuesta::mal(format!("No hay ninguna función, tipo ni módulo llamado «{nombre}» en {}.{ayuda} Si es código nuevo, usa `insertar`.", mostrar(&rel))));
        }
        ([x], _) => *x,
        (varios, Some(l)) => match varios.iter().find(|x| x.ini <= l && l <= x.fin) {
            Some(x) => *x,
            None => return Ok(Respuesta::mal(format!("Ningún «{nombre}» contiene la línea {l}. Hay: {}.", varios.iter().map(|x| format!("{}–{}", x.ini, x.fin)).collect::<Vec<_>>().join(", ")))),
        },
        (varios, None) => {
            let lista: Vec<String> = varios.iter().map(|x| format!("  {} {}–{}: {}", x.clase, x.ini, x.fin, x.firma)).collect();
            return Ok(Respuesta::mal(format!("«{nombre}» aparece {} veces en {}:\n{}\nIndica en `linea` una línea de la que quieres cambiar.", varios.len(), mostrar(&rel), lista.join("\n"))));
        }
    };
    // Si `nuevo` repite los atributos o la documentación que ya están justo encima del elemento, se descartan (quedarían duplicados).
    let mut encima: Vec<&str> = Vec::new();
    let mut k = x.ini - 1;
    while k >= 1 {
        let t = lineas[k - 1].trim_start();
        if t.starts_with("#[") || t.starts_with("///") {
            encima.insert(0, lineas[k - 1].trim());
            k -= 1;
        } else {
            break;
        }
    }
    let mut nuevo_ls: Vec<&str> = nuevo.trim_end_matches('\n').split('\n').collect();
    let mut quitadas = 0;
    while quitadas < encima.len() && nuevo_ls.len() > 1 && nuevo_ls[0].trim() == encima[quitadas] {
        nuevo_ls.remove(0);
        quitadas += 1;
    }
    let args2 = json!({"filePath": rel, "desde": x.ini, "hasta": x.fin, "nuevo": nuevo_ls.join("\n"), "confirmar_borrado": true});
    let mut r = reemplazar_lineas(e, pol, &args2)?;
    if !r.error {
        r.texto = format!("{}\n(Era {} «{}», líneas {}–{}: se ha sustituido por completo.)", r.texto, x.clase, x.nombre, x.ini, x.fin);
    }
    Ok(r)
}

// ---------------------------------------------------------------- sintaxis (rustfmt por stdin)

/// Orden por defecto del comprobador de sintaxis del host.
pub fn orden_sintaxis_rustfmt() -> Vec<String> {
    ["rustfmt", "--edition", "2021", "--check"].iter().map(|s| s.to_string()).collect()
}

/// De la salida de `rustfmt --check`: las líneas de un error de sintaxis (`error: expected …` con su `-->` y el recorte de código), o `None`
/// si no hay ninguno. Un archivo que sólo está mal formateado da un `Diff in …`, no un `error:`, y no cuenta.
fn error_de_rustfmt(salida: &str) -> Option<String> {
    let ignorables = ["left behind trailing whitespace", "exceeded maximum width", "line formatted", "internal error"];
    let lineas: Vec<&str> = salida.lines().collect();
    let ini = lineas.iter().position(|l| (l.starts_with("error:") || l.starts_with("error[")) && !ignorables.iter().any(|i| l.contains(i)))?;
    let mut t: Vec<String> = Vec::new();
    for l in &lineas[ini..] {
        if l.starts_with("Diff in") {
            break;
        }
        t.push((*l).to_string());
        if t.len() >= 14 {
            break;
        }
    }
    Some(t.join("\n"))
}

fn ejecutar_comprobador<E: Entorno>(e: &mut E, pol: &Politica, argv: &[String], texto: &str) -> Option<String> {
    let o = Orden { argv: argv.to_vec(), cwd: pol.raiz.clone(), stdin: texto.as_bytes().to_vec(), timeout_s: Some(30), entorno: pol.entorno.clone() };
    let s = e.ejecutar(&o).ok()?;
    if s.codigo.is_none() {
        return None; // no terminó (¿no existe el programa?): sin comprobación
    }
    error_de_rustfmt(&s.texto())
}

/// Dos `mod pruebas_agente` en el mismo archivo no compilan (E0428): el modelo añadía su módulo de pruebas al final sin ver que ya lo tenía.
fn modulo_de_pruebas_duplicado(rel: &str, antes: &str, despues: &str) -> Option<String> {
    let cuenta = |t: &str| t.matches("mod pruebas_agente").count();
    if !rel.ends_with(".rs") || cuenta(despues) < 2 || cuenta(antes) >= cuenta(despues) {
        return None;
    }
    Some(format!(
        "No he escrito el cambio: dejaría DOS `mod pruebas_agente` en {} y no compilaría. Ya tienes uno: añade tus funciones `#[test]` DENTRO de él (`read` con `simbolo`: pruebas_agente te lo enseña; inserta antes de su llave final) o sustitúyelo entero con `reemplazar_funcion` (`nombre`: pruebas_agente).",
        mostrar(rel)
    ))
}

/// ¿Dejaría `despues` el archivo con un error de sintaxis? Sólo `.rs` y sólo si la política trae un comprobador. Si el original ya estaba
/// roto no se le echa la culpa a esta edición. Devuelve el mensaje para el modelo.
fn error_de_sintaxis<E: Entorno>(e: &mut E, pol: &Politica, rel: &str, antes: &str, despues: &str) -> Option<String> {
    let argv = pol.comprobar_sintaxis.clone()?;
    if !rel.ends_with(".rs") {
        return None;
    }
    let err = ejecutar_comprobador(e, pol, &argv, despues)?;
    if ejecutar_comprobador(e, pol, &argv, antes).is_some() {
        return None;
    }
    // El modelo no sabe leer «línea 60 de <stdin>»: se le enseña el archivo tal como quedaría alrededor de esa línea (campaña 41, R31: repetía el mismo
    // `reemplazar_lineas` roto una docena de veces sin ver que, tras su texto, seguía el código original del bloque que reescribía).
    let ventana = err
        .split("<stdin>:")
        .nth(1)
        .and_then(|r| r.split(':').next())
        .and_then(|n| n.trim().parse::<usize>().ok())
        .map(|n| format!("\nEl archivo tal como quedaría, alrededor de la línea {n}:\n{}\n(Si tras tu texto sigue código ANTIGUO del mismo bloque, tu rango se quedó corto: amplía `hasta` hasta el final de lo que reescribes, o usa `reemplazar_funcion`.)", ventana_numerada(despues, n.saturating_sub(5).max(1), n + 4)))
        .unwrap_or_default();
    Some(format!(
        "No he escrito el cambio: con él {} tendría un error de SINTAXIS (lo dice rustfmt; los números de línea son los del archivo tal como quedaría):\n{err}{ventana}\nCorrige el texto de tu cambio (¿falta un `;`, una coma, un paréntesis o una palabra?) y repite.",
        mostrar(rel)
    ))
}

// ---------------------------------------------------------------- insertar

fn insertar<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
    let (abs, rel) = resolver(pol, cadena(args, "filePath")?)?;
    let nuevo = cadena(args, "texto")?;
    // `linea`: 0 = al principio; N = después de la línea N de `read`; -1 (o «final») = al final del archivo.
    let al_final = valor(args, "linea").is_some_and(|v| {
        v.as_i64().is_some_and(|n| n < 0) || v.as_str().is_some_and(|s| matches!(s.trim().to_lowercase().as_str(), "final" | "fin" | "end" | "eof" | "-1"))
    });
    let ancla_despues = args.get("despues_de").and_then(|v| v.as_str()).filter(|a| !a.trim().is_empty());
    let ancla_antes = args.get("antes_de").and_then(|v| v.as_str()).filter(|a| !a.trim().is_empty());
    let linea_pedida = if al_final { Some(usize::MAX) } else { entero(args, "linea") };
    if nuevo.trim().is_empty() {
        return Err(Error::uso("`texto` está vacío"));
    }
    let datos = e.leer(&abs).map_err(|_| no_existe(e, pol, &rel))?;
    let texto = String::from_utf8(datos).map_err(|_| Error::uso(format!("«{}» no es UTF-8", mostrar(&rel))))?;
    // Ancla de texto en vez de número de línea: se inserta después de (o antes de) la ÚNICA línea que contiene ese texto.
    let linea_pedida = match (ancla_despues.or(ancla_antes), linea_pedida) {
        (Some(ancla), _) => {
            let buscada: String = ancla.chars().filter(|c| !c.is_whitespace()).collect();
            let hallazgos: Vec<usize> = texto
                .lines()
                .enumerate()
                .filter(|(_, l)| l.chars().filter(|c| !c.is_whitespace()).collect::<String>().contains(&buscada))
                .map(|(i, _)| i + 1)
                .collect();
            match hallazgos.as_slice() {
                [] => return Ok(Respuesta::mal(format!("No hay ninguna línea con «{ancla}» en {}. Copia un trozo de una línea que aparezca en `read`.", mostrar(&rel)))),
                [l] => Some(if ancla_despues.is_some() { *l } else { l - 1 }),
                varias => {
                    let lista: Vec<String> = varias.iter().take(8).map(|l| format!("  línea {l}: {}", texto.lines().nth(l - 1).unwrap_or("").trim())).collect();
                    return Ok(Respuesta::mal(format!("«{ancla}» aparece en {} líneas de {}:\n{}\nUsa un texto más concreto o el número de `linea`.", varias.len(), mostrar(&rel), lista.join("\n"))));
                }
            }
        }
        (None, l) => l,
    };
    let linea_pedida = linea_pedida.ok_or_else(|| Error::uso("falta el argumento «linea» (entero: 0 = al principio, N = después de la línea N de `read`, -1 = al final del archivo) o un ancla `despues_de` / `antes_de` con el texto de una línea"))?;
    let mut lineas: Vec<&str> = texto.split('\n').collect();
    let terminaba_en_salto = lineas.last() == Some(&"");
    if terminaba_en_salto {
        lineas.pop();
    }
    let linea = if linea_pedida == usize::MAX { lineas.len() } else { linea_pedida };
    if linea > lineas.len() {
        return Err(Error::uso(format!("«{}» tiene {} línea(s): no se puede insertar después de la {linea}. Usa `linea` = -1 para añadir al final del archivo", mostrar(&rel), lineas.len())));
    }
    let mut nuevo_texto = String::new();
    for (i, l) in lineas.iter().enumerate() {
        if i == linea {
            nuevo_texto.push_str(nuevo.trim_end_matches('\n'));
            nuevo_texto.push('\n');
        }
        nuevo_texto.push_str(l);
        nuevo_texto.push('\n');
    }
    if linea == lineas.len() {
        nuevo_texto.push_str(nuevo.trim_end_matches('\n'));
        nuevo_texto.push('\n');
    }
    if !terminaba_en_salto {
        nuevo_texto.pop();
    }
    if let Some(m) = desequilibrio(&rel, &texto, &nuevo_texto) {
        return Ok(Respuesta::mal(m));
    }
    if let Some(m) = casi_solo_comentarios(&rel, nuevo) {
        return Ok(Respuesta::mal(m));
    }
    if let Some(m) = modulo_de_pruebas_duplicado(&rel, &texto, &nuevo_texto) {
        return Ok(Respuesta::mal(m));
    }
    if let Some(m) = error_de_sintaxis(e, pol, &rel, &texto, &nuevo_texto) {
        return Ok(Respuesta::mal(m));
    }
    let modo = e.metadatos(&abs).map(|m| m.modo).unwrap_or(0o644);
    e.escribir(&abs, nuevo_texto.as_bytes(), modo)?;
    let n_ins = nuevo.trim_end_matches('\n').matches('\n').count() + 1;
    let ventana = ventana_numerada(&nuevo_texto, linea.saturating_sub(1).max(1), linea + n_ins + 2);
    Ok(Respuesta::ok(format!(
        "Hecho: {n_ins} línea(s) insertadas en {} después de la línea {linea}.\nInsertado:\n{nuevo}\nAsí queda ahora (numeración nueva):\n{ventana}",
        mostrar(&rel)
    )))
}

// ---------------------------------------------------------------- equilibrio de delimitadores

/// Diferencia (aperturas − cierres) de `()`, `[]` y `{}` en código Rust, ignorando comentarios, cadenas,
/// cadenas crudas y caracteres. No es un parser: sólo lo bastante léxico para no contar los `{` de un
/// literal de texto.
fn equilibrio_rs(t: &str) -> [i64; 3] {
    let b: Vec<char> = t.chars().collect();
    let mut n = [0i64; 3];
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let sig = b.get(i + 1).copied();
        match c {
            '/' if sig == Some('/') => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if sig == Some('*') => {
                let mut prof = 1;
                i += 2;
                while i < b.len() && prof > 0 {
                    if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                        prof += 1;
                        i += 2;
                    } else if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                        prof -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            'r' if matches!(sig, Some('"') | Some('#')) => {
                let mut j = i + 1;
                let mut almohadillas = 0;
                while b.get(j) == Some(&'#') {
                    almohadillas += 1;
                    j += 1;
                }
                if b.get(j) == Some(&'"') {
                    j += 1;
                    'crudo: while j < b.len() {
                        if b[j] == '"' {
                            let mut k = 0;
                            while k < almohadillas && b.get(j + 1 + k) == Some(&'#') {
                                k += 1;
                            }
                            if k == almohadillas {
                                j += 1 + almohadillas;
                                break 'crudo;
                            }
                        }
                        j += 1;
                    }
                    i = j;
                    continue;
                }
            }
            '"' => {
                i += 1;
                while i < b.len() && b[i] != '"' {
                    if b[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
                continue;
            }
            '\'' => {
                // Carácter (`'a'`, `'\n'`) o vida útil (`'a`): sólo el primero se salta.
                if sig == Some('\\') {
                    i += 2;
                    while i < b.len() && b[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                    continue;
                } else if b.get(i + 2) == Some(&'\'') {
                    i += 3;
                    continue;
                }
            }
            '(' => n[0] += 1,
            ')' => n[0] -= 1,
            '[' => n[1] += 1,
            ']' => n[1] -= 1,
            '{' => n[2] += 1,
            '}' => n[2] -= 1,
            _ => {}
        }
        i += 1;
    }
    n
}

/// Si el archivo es Rust y estaba equilibrado, una edición que lo desequilibra **no se escribe**: lo más
/// frecuente en un modelo pequeño es cortar una cabecera `fn …{` o un `match` por la mitad. El mensaje
/// dice qué delimitador y cuánto, que es lo que necesita para corregirlo.
fn desequilibrio(rel: &str, antes: &str, despues: &str) -> Option<String> {
    if !rel.ends_with(".rs") {
        return None;
    }
    let a = equilibrio_rs(antes);
    if a != [0, 0, 0] {
        return None;
    }
    let d = equilibrio_rs(despues);
    if d == [0, 0, 0] {
        return None;
    }
    let nombres = ["paréntesis ( )", "corchetes [ ]", "llaves { }"];
    let mut m = format!("No he escrito el cambio: dejaría {} con los delimitadores desequilibrados.", mostrar(rel));
    for (i, x) in d.iter().enumerate() {
        if *x != 0 {
            m.push_str(&format!(
                " En {}: {} {} de más.",
                nombres[i],
                x.abs(),
                if *x > 0 { "apertura(s)" } else { "cierre(s)" }
            ));
        }
    }
    m.push_str(" Cambia **bloques completos**: si añades una función o una rama, incluye su `{` y su `}`; si sustituyes sólo una línea, no toques las llaves de alrededor. Para añadir código nuevo usa `insertar`.");
    Some(m)
}

// ---------------------------------------------------------------- bash

/// Trocea una línea en argumentos. Sólo entiende comillas simples y dobles; **rechaza** todo lo que
/// sea shell (N-010: un operador que se ignora en silencio hace que el modelo vea una respuesta
/// plausible de media orden).
fn trocear(linea: &str) -> core::result::Result<Vec<String>, String> {
    trocear_con(linea, false)
}

/// Igual que `trocear`; con `comodines` admite `* ? ( )` sin comillas (los de `find -name *.rs`, que aquí no se expanden).
fn trocear_con(linea: &str, comodines: bool) -> core::result::Result<Vec<String>, String> {
    let mut args: Vec<String> = Vec::new();
    let mut actual = String::new();
    let mut hay = false;
    let mut comilla: Option<char> = None;
    for c in linea.chars() {
        match comilla {
            Some(q) => {
                if c == q {
                    comilla = None;
                } else {
                    actual.push(c);
                }
            }
            None => match c {
                '\'' | '"' => {
                    comilla = Some(c);
                    hay = true;
                }
                c if c.is_whitespace() => {
                    if hay {
                        args.push(core::mem::take(&mut actual));
                        hay = false;
                    }
                }
                '*' | '?' | '(' | ')' if comodines => {
                    actual.push(c);
                    hay = true;
                }
                ';' | '&' | '|' | '<' | '>' | '$' | '`' | '(' | ')' | '*' | '?' => {
                    return Err(format!(
                        "«{c}» no está disponible: la herramienta ejecuta UNA orden sin shell (sin encadenar con ; && ||, \
                         sin tuberías ni redirecciones, sin variables ni comodines). Haz una llamada por orden."
                    ));
                }
                c => {
                    actual.push(c);
                    hay = true;
                }
            },
        }
    }
    if comilla.is_some() {
        return Err("hay una comilla sin cerrar".to_string());
    }
    if hay {
        args.push(actual);
    }
    if args.first().map_or(false, |a| a.contains('=')) {
        return Err("no se pueden fijar variables de entorno en la orden".to_string());
    }
    Ok(args)
}

/// Opciones que son de `cargo` (van ANTES de `--`) y que los modelos escriben detrás.
const OPCIONES_CARGO: [&str; 8] = ["--lib", "--doc", "--bins", "--tests", "--examples", "--all-targets", "--release", "--no-run"];
/// Opciones de libtest (van DESPUÉS de `--`) y que los modelos escriben delante.
const OPCIONES_LIBTEST: [&str; 5] = ["--nocapture", "--show-output", "--exact", "--ignored", "--include-ignored"];

/// Recoloca los argumentos de `cargo test` que están en el lado equivocado de `--`: `-- --lib` / `-- --doc` (error «Unrecognized
/// option») y `--nocapture` sin `--` (error «unexpected argument») eran el 43 % de los `bash` fallidos de las campañas. Un
/// `--test=NOMBRE` (o `--filter`, `--scope`) detrás de `--` se interpreta como filtro de nombre. Devuelve la orden corregida y
/// una nota para el modelo, o `None` si no hay nada que corregir.
fn normalizar_cargo_test(argv: &[String]) -> Option<(Vec<String>, String)> {
    if argv.len() < 2 || argv[0] != "cargo" || argv[1] != "test" {
        return None;
    }
    let corte = argv.iter().position(|a| a == "--").unwrap_or(argv.len());
    let mut antes: Vec<String> = Vec::new();
    let mut despues: Vec<String> = Vec::new();
    let mut notas: Vec<String> = Vec::new();
    let mut i = 0;
    while i < corte {
        let a = &argv[i];
        if i >= 2 && OPCIONES_LIBTEST.contains(&a.as_str()) {
            despues.push(a.clone());
            notas.push(format!("`{a}` es de libtest: lo he puesto detrás de `--`"));
        } else if i >= 2 && (a == "--test-threads" || a.starts_with("--test-threads=")) {
            despues.push(a.clone());
            if a == "--test-threads" && i + 1 < corte {
                despues.push(argv[i + 1].clone());
                i += 1;
            }
            notas.push("`--test-threads` es de libtest: lo he puesto detrás de `--`".to_string());
        } else {
            antes.push(a.clone());
        }
        i += 1;
    }
    let resto: Vec<&String> = argv[corte..].iter().filter(|a| a.as_str() != "--").collect();
    let mut j = 0;
    while j < resto.len() {
        let a = resto[j].as_str();
        let (clave, valor) = match a.split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (a, None),
        };
        if OPCIONES_CARGO.contains(&a) {
            antes.push(a.to_string());
            notas.push(format!("`{a}` es de cargo: lo he puesto antes de `--`"));
        } else if matches!(clave, "--test" | "--filter" | "--scope" | "--test-filter" | "--test_filter" | "--name" | "--args" | "--test-name") {
            let nombre = valor.or_else(|| resto.get(j + 1).filter(|n| !n.starts_with('-')).map(|n| (*n).clone()).inspect(|_| j += 1));
            match nombre {
                Some(n) => {
                    notas.push(format!("`{clave}` no filtra por nombre en libtest: he usado «{n}» como filtro"));
                    despues.push(n);
                }
                None => despues.push(a.to_string()),
            }
        } else {
            despues.push(a.to_string());
        }
        j += 1;
    }
    if notas.is_empty() {
        return None;
    }
    let mut nueva = antes;
    if !despues.is_empty() {
        nueva.push("--".to_string());
        nueva.extend(despues);
    }
    Some((nueva, notas.join("; ")))
}

/// Separa `orden | filtro` cuando hay UNA tubería final fuera de comillas, y quita `2>&1` (la salida ya junta stdout y stderr).
fn separar_filtro(linea: &str) -> (String, Option<String>) {
    let limpia = linea.replace(" 2>&1", "").replace("2>&1 ", "");
    let mut comilla: Option<char> = None;
    let mut cortes: Vec<usize> = Vec::new();
    let cs: Vec<(usize, char)> = limpia.char_indices().collect();
    for (k, (i, c)) in cs.iter().enumerate() {
        match comilla {
            Some(q) if *c == q => comilla = None,
            Some(_) => {}
            None => match c {
                '\'' | '"' => comilla = Some(*c),
                '|' => {
                    let vecino = |d: isize| cs.get((k as isize + d) as usize).map(|(_, x)| *x);
                    if vecino(-1) != Some('|') && vecino(1) != Some('|') {
                        cortes.push(*i);
                    }
                }
                _ => {}
            },
        }
    }
    if cortes.len() == 1 {
        let (a, b) = limpia.split_at(cortes[0]);
        (a.trim().to_string(), Some(b[1..].trim().to_string()))
    } else {
        (limpia, None)
    }
}

/// Aplica a la salida un filtro `head`, `tail` o `grep` (texto literal) de los que un modelo encadena con `|`.
fn aplicar_filtro(filtro: &str, texto: &str) -> core::result::Result<String, String> {
    let argv = trocear(filtro).map_err(|m| format!("filtro no válido: {m}"))?;
    let lineas: Vec<&str> = texto.lines().collect();
    let numero = |v: &str| v.trim_start_matches('-').trim_start_matches('n').parse::<usize>().ok();
    let cuantas = |args: &[String]| -> Option<usize> {
        let mut it = args.iter().skip(1);
        let mut n = 10usize;
        while let Some(a) = it.next() {
            if a == "-n" {
                n = numero(it.next()?)?;
            } else if a.starts_with('-') && a.len() > 1 {
                n = numero(a)?;
            }
        }
        Some(n)
    };
    match argv.first().map(|s| s.as_str()) {
        Some("head") => {
            let n = cuantas(&argv).ok_or("uso: `| head -N`")?;
            Ok(lineas.iter().take(n).copied().collect::<Vec<_>>().join("\n"))
        }
        Some("tail") => {
            let n = cuantas(&argv).ok_or("uso: `| tail -N`")?;
            Ok(lineas[lineas.len().saturating_sub(n)..].join("\n"))
        }
        Some("grep") => {
            let (mut inv, mut ins, mut cuenta) = (false, false, false);
            let (mut antes, mut despues) = (0usize, 0usize);
            let mut patron: Option<String> = None;
            let mut it = argv.iter().skip(1);
            while let Some(a) = it.next() {
                match a.as_str() {
                    "-v" => inv = true,
                    "-i" => ins = true,
                    "-c" => cuenta = true,
                    "-n" | "-E" | "-F" | "-e" | "-H" | "-h" | "--color=never" => {}
                    "-A" => despues = it.next().and_then(|v| v.parse().ok()).ok_or("uso: `| grep -A N TEXTO`")?,
                    "-B" => antes = it.next().and_then(|v| v.parse().ok()).ok_or("uso: `| grep -B N TEXTO`")?,
                    "-C" => {
                        let n = it.next().and_then(|v| v.parse().ok()).ok_or("uso: `| grep -C N TEXTO`")?;
                        (antes, despues) = (n, n);
                    }
                    o if o.starts_with("-A") || o.starts_with("-B") || o.starts_with("-C") => {
                        let n: usize = o[2..].parse().map_err(|_| "uso: `| grep -A N TEXTO`".to_string())?;
                        match &o[..2] {
                            "-A" => despues = n,
                            "-B" => antes = n,
                            _ => (antes, despues) = (n, n),
                        }
                    }
                    o if o.starts_with('-') && o.len() > 1 => return Err(format!("`| grep {o}` no está disponible: se admiten -i -v -c -n -A -B -C")),
                    o => patron = Some(o.to_string()),
                }
            }
            let patron = patron.ok_or("uso: `| grep TEXTO`")?;
            let pat = if ins { patron.to_lowercase() } else { patron.clone() };
            let casa = |l: &str| (if ins { l.to_lowercase() } else { l.to_string() }).contains(&pat) != inv;
            if cuenta {
                return Ok(lineas.iter().filter(|l| casa(l)).count().to_string());
            }
            let mut sal: Vec<String> = Vec::new();
            let mut ultimo: Option<usize> = None;
            for (i, l) in lineas.iter().enumerate() {
                if casa(l) {
                    let ini = i.saturating_sub(antes).max(ultimo.map_or(0, |u| u + 1));
                    if ultimo.is_some_and(|u| ini > u + 1) && (antes > 0 || despues > 0) {
                        sal.push("--".to_string());
                    }
                    for k in ini..=(i + despues).min(lineas.len() - 1) {
                        sal.push(lineas[k].to_string());
                    }
                    ultimo = Some((i + despues).min(lineas.len() - 1));
                }
            }
            Ok(sal.join("\n"))
        }
        otro => Err(format!("`| {}` no está disponible: sólo se admiten `| head -N`, `| tail -N` y `| grep TEXTO`", otro.unwrap_or(""))),
    }
}

/// Qué herramienta usar en lugar de una orden de shell que no está permitida.
fn alternativa(orden: &str) -> &'static str {
    match orden {
        "ls" | "find" | "tree" | "fd" => " Para listar o buscar archivos usa la herramienta `glob`.",
        "cat" | "head" | "tail" | "sed" | "awk" | "less" | "more" => " Para leer un archivo usa la herramienta `read` (con `offset` y `limit` para un tramo).",
        "grep" | "rg" | "ack" => " Para buscar texto en el código usa la herramienta `grep`.",
        "cd" => " No hace falta `cd`: la orden se ejecuta en la raíz del checkout; para un crate usa `-p NOMBRE`.",
        "rm" | "mv" | "cp" | "touch" | "mkdir" | "echo" | "tee" => " Para cambiar archivos usa `reemplazar_texto` o `insertar`.",
        _ => "",
    }
}

/// Quita de la salida de `cargo` lo que no ayuda al modelo: las líneas de progreso (`Compiling`, `Finished`…), los bloques `warning:` y
/// las pruebas que pasan (`test x ... ok`). Los errores, las pruebas que fallan y el resumen se conservan enteros. En las campañas el
/// 48 % de las líneas de salida de `bash` era este ruido, que además escondía el error útil. Salidas cortas no se tocan.
fn compactar_cargo(texto: &str) -> String {
    if texto.len() < 1200 {
        return texto.to_string();
    }
    let (mut avisos, mut oks, mut progreso) = (0usize, 0usize, 0usize);
    let mut sal: Vec<&str> = Vec::new();
    let mut en_aviso = false;
    for l in texto.lines() {
        let t = l.trim_start();
        if en_aviso {
            // El bloque de un aviso sigue mientras las líneas estén sangradas o sean `123 | …`; acaba en una línea vacía o en una nueva.
            let sangrada = l.starts_with(' ') || l.starts_with('\t') || l.starts_with('|');
            if l.trim().is_empty() {
                en_aviso = false;
                continue;
            }
            if sangrada {
                continue;
            }
            en_aviso = false;
        }
        if ["Compiling ", "Finished ", "Fresh ", "Downloaded ", "Updating ", "Blocking ", "Locking ", "Adding "].iter().any(|p| t.starts_with(p)) {
            progreso += 1;
            continue;
        }
        if t.starts_with("warning:") {
            avisos += 1;
            en_aviso = true;
            continue;
        }
        if l.starts_with("test ") && l.ends_with(" ... ok") {
            oks += 1;
            continue;
        }
        sal.push(l);
    }
    if avisos + oks + progreso == 0 {
        return texto.to_string();
    }
    let mut r = String::new();
    let mut vacia = false;
    for l in sal {
        if l.trim().is_empty() {
            if vacia {
                continue;
            }
            vacia = true;
        } else {
            vacia = false;
        }
        r.push_str(l);
        r.push('\n');
    }
    let mut omitido: Vec<String> = Vec::new();
    if avisos > 0 {
        omitido.push(format!("{avisos} aviso(s) del compilador"));
    }
    if oks > 0 {
        omitido.push(format!("{oks} prueba(s) que pasan"));
    }
    if progreso > 0 {
        omitido.push(format!("{progreso} línea(s) de progreso"));
    }
    r.push_str(&format!("[salida compactada: se omiten {}]", omitido.join(", ")));
    r
}

/// `cd DIR && orden`: el modelo se acostumbra a entrar en el directorio del crate. Se ejecuta `orden` con ese directorio de trabajo
/// (dentro del checkout) en vez de rechazarla. Devuelve la orden sin el `cd`, el directorio absoluto y el relativo.
fn quitar_cd<E: Entorno>(e: &E, pol: &Politica, linea: &str) -> Resultado<Option<(String, String, String)>> {
    let t = linea.trim_start();
    let Some(resto) = t.strip_prefix("cd ") else { return Ok(None) };
    let Some((dir, orden)) = resto.split_once("&&") else { return Ok(None) };
    let dir = dir.trim().trim_matches(|c| c == '"' || c == '\'');
    if dir.is_empty() || orden.trim().is_empty() {
        return Ok(None);
    }
    let (abs, rel) = resolver(pol, dir)?;
    match e.metadatos(&abs) {
        Ok(m) if m.tipo == Tipo::Directorio => Ok(Some((orden.trim().to_string(), abs, rel))),
        _ => Err(no_existe(e, pol, &rel)),
    }
}

fn unir_ruta(base: &str, p: &str) -> String {
    let p = p.trim_start_matches("./");
    if p.starts_with('/') || base.is_empty() {
        return if p.is_empty() { ".".to_string() } else { p.to_string() };
    }
    if p.is_empty() || p == "." { base.to_string() } else { format!("{base}/{p}") }
}

/// `*` y `?` frente a una ruta completa (el `*` cruza `/`, como en `find -path`).
fn comodin(pat: &[u8], texto: &[u8]) -> bool {
    match pat.split_first() {
        None => texto.is_empty(),
        Some((b'*', resto)) => (0..=texto.len()).any(|i| comodin(resto, &texto[i..])),
        Some((b'?', resto)) => !texto.is_empty() && comodin(resto, &texto[1..]),
        Some((c, resto)) => texto.first() == Some(c) && comodin(resto, &texto[1..]),
    }
}

/// Órdenes de shell de lectura (`find`, `ls`, `cat`, `grep`, `rg`) que un modelo escribe por costumbre: en vez de rechazarlas se
/// resuelven con las herramientas propias (`glob`, `read`, `grep`). Lo que no se sabe traducir se explica con la herramienta que sirve.
fn emular<E: Entorno>(e: &mut E, pol: &Politica, principal: &str, base: &str) -> Resultado<Option<Respuesta>> {
    let laxa = principal.replace("\\;", " __FIN__ ").replace(" {} +", " {} __FIN__ ");
    let argv = match trocear_con(&laxa, true) {
        Ok(a) if !a.is_empty() => a,
        _ => return Ok(None),
    };
    let nota = |o: &str, h: &str| format!("Nota: `{o}` no existe aquí (no hay shell); lo he resuelto con {h}.\n");
    match argv[0].as_str() {
        "ls" => {
            let ruta = argv[1..].iter().find(|a| !a.starts_with('-')).map_or(".", |s| s.as_str());
            let r = leer(e, pol, &json!({"filePath": unir_ruta(base, ruta)}))?;
            Ok(Some(Respuesta { texto: format!("{}{}", nota("ls", "`read` (un directorio se lista)"), r.texto), error: r.error }))
        }
        "cat" => {
            let rutas: Vec<&String> = argv[1..].iter().filter(|a| !a.starts_with('-')).collect();
            if rutas.len() != 1 {
                return Ok(Some(Respuesta::mal("`cat` lee un archivo por llamada: usa `read` con `filePath` (y `offset`/`limit` para un tramo).".to_string())));
            }
            let r = leer(e, pol, &json!({"filePath": unir_ruta(base, rutas[0])}))?;
            Ok(Some(Respuesta { texto: format!("{}{}", nota("cat", "`read`"), r.texto), error: r.error }))
        }
        "grep" | "rg" | "ack" => {
            let (mut ins, mut solo_archivos) = (false, false);
            let mut ctx: Option<usize> = None;
            let (mut include, mut patron): (Option<String>, Option<String>) = (None, None);
            let mut rutas: Vec<String> = Vec::new();
            let mut it = argv[1..].iter().peekable();
            while let Some(a) = it.next() {
                if let Some(v) = a.strip_prefix("--include=") {
                    include = Some(v.to_string());
                } else if a == "--include" || a == "-g" || a == "--glob" {
                    include = it.next().cloned();
                } else if a.starts_with("--") {
                    // --color, --exclude…, --line-number: sin efecto aquí
                } else if a == "-e" {
                    patron = it.next().cloned();
                } else if matches!(a.as_str(), "-A" | "-B" | "-C" | "-m") {
                    let n = it.next().and_then(|v| v.parse::<usize>().ok());
                    if a != "-m" {
                        ctx = n;
                    }
                } else if a.len() > 1 && a.starts_with('-') && a[1..].chars().all(|c| c.is_ascii_alphabetic()) {
                    for c in a[1..].chars() {
                        match c {
                            'i' => ins = true,
                            'l' => solo_archivos = true,
                            _ => {}
                        }
                    }
                } else if a.len() > 2 && (a.starts_with("-A") || a.starts_with("-B") || a.starts_with("-C")) && a[2..].chars().all(|c| c.is_ascii_digit()) {
                    ctx = a[2..].parse().ok();
                } else if patron.is_none() {
                    patron = Some(a.clone());
                } else {
                    rutas.push(a.clone());
                }
            }
            let Some(patron) = patron else { return Ok(Some(Respuesta::mal("falta el texto a buscar: usa la herramienta `grep` con `pattern`.".to_string()))) };
            let patron = patron.replace("\\|", "|");
            let mut a = json!({"pattern": patron, "path": unir_ruta(base, rutas.first().map_or(".", |s| s.as_str())), "ignorar_mayusculas": ins});
            if let Some(i) = include {
                a["include"] = json!(i);
            }
            if let Some(c) = ctx {
                a["contexto"] = json!(c);
            }
            let r = buscar(e, pol, &a)?;
            let texto = if solo_archivos && !r.error {
                let mut f: Vec<&str> = Vec::new();
                for l in r.texto.lines() {
                    if let Some((ruta, resto)) = l.split_once(':') {
                        if resto.split(':').next().is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) && !f.contains(&ruta) {
                            f.push(ruta);
                        }
                    }
                }
                if f.is_empty() { r.texto.clone() } else { f.join("\n") }
            } else {
                r.texto
            };
            let extra = if rutas.len() > 1 { " (sólo he buscado en la primera ruta; usa `path` por llamada)" } else { "" };
            Ok(Some(Respuesta { texto: format!("{}{texto}", nota(&argv[0], &format!("la herramienta `grep`{extra}"))), error: r.error }))
        }
        "find" => {
            let mut i = 1;
            let mut ruta = ".";
            if argv.get(1).is_some_and(|a| !a.starts_with('-') && a != "(") {
                ruta = argv[1].as_str();
                i = 2;
            }
            // Predicados en grupos: `-o` separa alternativas; dentro de un grupo, todos deben cumplirse (el AND implícito de `find`).
            let mut grupos: Vec<Vec<(bool, String, bool)>> = alloc::vec![Vec::new()]; // (es_ruta, patrón, ignora mayúsculas)
            let mut exec: Option<Vec<String>> = None;
            while i < argv.len() {
                let a = argv[i].as_str();
                match a {
                    "-name" | "-iname" | "-path" | "-ipath" | "-wholename" => {
                        let Some(v) = argv.get(i + 1) else { return Ok(None) };
                        grupos.last_mut().unwrap().push((!a.ends_with("name"), v.clone(), a.starts_with("-i")));
                        i += 1;
                    }
                    "-type" => {
                        if argv.get(i + 1).map(|s| s.as_str()) != Some("f") {
                            return Ok(Some(Respuesta::mal("`find -type d` no está disponible: para ver un directorio usa `read` con su ruta, o `glob`.".to_string())));
                        }
                        i += 1;
                    }
                    "-maxdepth" | "-mindepth" => i += 1,
                    "-o" | "-or" => grupos.push(Vec::new()),
                    "-a" | "-and" | "-print" | "(" | ")" | "\\(" | "\\)" => {}
                    "-exec" => {
                        let fin = argv[i + 1..].iter().position(|x| x == "__FIN__").map_or(argv.len(), |p| i + 1 + p);
                        exec = Some(argv[i + 1..fin].to_vec());
                        i = fin;
                    }
                    otro => {
                        return Ok(Some(Respuesta::mal(format!(
                            "`find {otro}` no está disponible. Para listar archivos usa `glob` (p. ej. `crates/**/*.rs`); para buscar texto, `grep`."
                        ))))
                    }
                }
                i += 1;
            }
            // `-exec grep -l TEXTO {} \;`: sólo los archivos que contienen el texto.
            let mut contenido: Option<(Vec<String>, bool)> = None;
            if let Some(x) = &exec {
                if x.first().map(|s| s.as_str()) != Some("grep") {
                    return Ok(Some(Respuesta::mal("`find -exec` sólo se admite con `grep -l TEXTO {} \\;`. Para otra cosa usa `glob`, `grep` o `read`.".to_string())));
                }
                let (mut ins, mut pat) = (false, None);
                for t in &x[1..] {
                    if t == "{}" {
                        continue;
                    }
                    if t.len() > 1 && t.starts_with('-') {
                        ins |= t.contains('i');
                    } else if pat.is_none() {
                        pat = Some(t.replace("\\|", "|"));
                    }
                }
                let Some(p) = pat else { return Ok(None) };
                contenido = Some((p.split('|').map(|a| if ins { a.to_lowercase() } else { a.to_string() }).collect(), ins));
            }
            let (abs, rel) = resolver(pol, &unir_ruta(base, ruta))?;
            let meta = e.metadatos(&abs).map_err(|_| no_existe(e, pol, &rel))?;
            let mut todos: Vec<(String, String)> = Vec::new();
            if meta.tipo == Tipo::Directorio {
                recorrer(e, &abs, &rel, 0, &mut |a, r| todos.push((a.to_string(), r.to_string())))?;
            } else {
                todos.push((abs.clone(), rel.clone()));
            }
            let mut halladas: Vec<String> = Vec::new();
            let mut total = 0usize;
            for (a, r) in todos {
                let nombre = r.rsplit('/').next().unwrap_or(&r).to_string();
                let casa = |(es_ruta, p, i): &(bool, String, bool)| {
                    let (p, n, c, c2) = if *i { (p.to_lowercase(), nombre.to_lowercase(), r.to_lowercase(), format!("./{}", r.to_lowercase())) } else { (p.clone(), nombre.clone(), r.clone(), format!("./{r}")) };
                    if *es_ruta { comodin(p.as_bytes(), c.as_bytes()) || comodin(p.as_bytes(), c2.as_bytes()) } else { glob_segmento(p.as_bytes(), n.as_bytes()) }
                };
                if grupos.iter().any(|g| !g.is_empty()) && !grupos.iter().any(|g| !g.is_empty() && g.iter().all(casa)) {
                    continue;
                }
                if let Some((alts, ins)) = &contenido {
                    let Ok(d) = e.leer(&a) else { continue };
                    let t = String::from_utf8_lossy(&d);
                    let t = if *ins { t.to_lowercase() } else { t.into_owned() };
                    if !alts.iter().any(|x| t.contains(x.as_str())) {
                        continue;
                    }
                }
                total += 1;
                if halladas.len() < pol.resultados_max {
                    halladas.push(r);
                }
            }
            let mut t = nota("find", "`glob`/`grep`");
            if halladas.is_empty() {
                t.push_str("(sin resultados)");
            } else {
                t.push_str(&halladas.join("\n"));
                if total > halladas.len() {
                    t.push_str(&format!("\n(… {} más)", total - halladas.len()));
                }
            }
            Ok(Some(Respuesta::ok(t)))
        }
        _ => Ok(None),
    }
}

fn orden<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
    let linea = cadena(args, "command")?;
    let (linea, cwd, base, nota_cd) = match quitar_cd(e, pol, linea)? {
        Some((resto, abs, rel)) => (resto, abs, rel.clone(), format!("Nota: he ejecutado la orden dentro de {} (`cd` sólo vale como prefijo `cd DIR && orden`).\n", mostrar(&rel))),
        None => (linea.to_string(), pol.raiz.clone(), String::new(), String::new()),
    };
    let (principal, filtro) = separar_filtro(&linea);
    if let Some(r) = emular(e, pol, &principal, &base)? {
        let texto = match (&filtro, r.error) {
            (Some(f), false) => match aplicar_filtro(f, &r.texto) {
                Ok(t) => t,
                Err(m) => return Ok(Respuesta::mal(m)),
            },
            _ => r.texto,
        };
        return Ok(Respuesta { texto: format!("{nota_cd}{texto}"), error: r.error });
    }
    let mut argv = match trocear(&principal) {
        Ok(a) if !a.is_empty() => a,
        Ok(_) => return Err(Error::uso("la orden está vacía")),
        Err(m) => return Ok(Respuesta::mal(m)),
    };
    let mut nota = String::new();
    if let Some((nueva, n)) = normalizar_cargo_test(&argv) {
        argv = nueva;
        nota = format!("Nota: he reordenado los argumentos de cargo ({n}). Orden ejecutada: {}\n", argv.join(" "));
    }
    let permitida = pol.ejecutables.iter().any(|p| argv.len() >= p.len() && argv[..p.len()] == p[..]);
    if !permitida {
        let lista: Vec<String> = pol.ejecutables.iter().map(|p| p.join(" ")).collect();
        return Ok(Respuesta::mal(format!(
            "«{}» no está permitida. Las órdenes permitidas son: {}.{}",
            argv.join(" "),
            lista.join(", "),
            alternativa(argv[0].as_str())
        )));
    }
    // El filtro se valida ANTES de ejecutar: una orden con un filtro que no existe no debe llegar a lanzarse.
    if let Some(f) = &filtro {
        if let Err(m) = aplicar_filtro(f, "") {
            return Ok(Respuesta::mal(m));
        }
    }
    let mut o = Orden { argv, cwd, stdin: Vec::new(), timeout_s: Some(pol.plazo_orden_s), entorno: pol.entorno.clone() };
    o.entorno.sort();
    let s = e.ejecutar(&o)?;
    let cabecera = match (s.codigo, &s.motivo) {
        (Some(c), _) => format!("exit code: {c}"),
        (None, Some(m)) => format!("no terminó: {m}"),
        (None, None) => "no terminó".to_string(),
    };
    let con_filtro = filtro.is_some();
    let cuerpo = match filtro {
        Some(f) => match aplicar_filtro(&f, &s.texto()) {
            Ok(t) => format!("{t}\n[filtrado con `| {f}`; el exit code es el de la orden, no el del filtro]"),
            Err(m) => return Ok(Respuesta::mal(m)),
        },
        None if o.argv.first().is_some_and(|a| a == "cargo") => compactar_cargo(&s.texto()),
        None => s.texto(),
    };
    // `cargo test` que sale con 0 sin haber ejecutado NI UNA prueba (filtro que no encaja, o crate sin pruebas con esas features): el
    // «exit code: 0» engaña, el modelo lo lee como «mis pruebas pasan». En las campañas fue el 15 % de los `cargo test`.
    let aviso_cero = if o.argv.len() >= 2 && o.argv[0] == "cargo" && o.argv[1] == "test" && s.codigo == Some(0) && !con_filtro {
        sin_pruebas_ejecutadas(&s.texto())
    } else {
        None
    };
    let cuerpo = match aviso_cero {
        Some(a) => format!("{cuerpo}\n{a}"),
        None => cuerpo,
    };
    let cuerpo = match pista_no_std(&s.texto()) {
        Some(p) => format!("{cuerpo}\n{p}"),
        None => cuerpo,
    };
    Ok(Respuesta { texto: format!("{nota_cd}{nota}{cabecera}\n{cuerpo}"), error: s.codigo != Some(0) })
}

/// Un crate `no_std` no trae `vec!`, `Vec`, `String` ni `format!` en el preludio: el módulo de pruebas del agente se quedaba con errores de compilación en
/// ellos (R36 de la campaña 48: todo el intento peleando con `cannot find macro vec`, sin que sus pruebas llegaran a ejecutarse).
fn pista_no_std(texto: &str) -> Option<String> {
    let falta = ["cannot find macro `vec`", "cannot find macro `format`", "cannot find type `Vec`", "cannot find type `String`", "cannot find type `Box`", "failed to resolve: use of undeclared type `Vec`", "failed to resolve: use of undeclared type `String`"];
    falta.iter().any(|f| texto.contains(f)).then(|| "Pista: este crate es `no_std` (sin `vec!`, `Vec`, `String`, `format!` en el preludio). Dentro de tu `mod pruebas_agente` añade `extern crate alloc; use alloc::{vec, vec::Vec, string::String, format};` o prueba con arrays y `&[..]`; y ejecuta las pruebas con `--features std` si el crate lo tiene.".to_string())
}

/// Si la salida de `cargo test` sólo contiene bloques `running 0 tests` (y al menos uno), el aviso para el modelo.
fn sin_pruebas_ejecutadas(texto: &str) -> Option<String> {
    let mut bloques = 0usize;
    for l in texto.lines() {
        let Some(r) = l.trim().strip_prefix("running ") else { continue };
        let n: usize = r.split_whitespace().next()?.parse().ok()?;
        if n > 0 {
            return None;
        }
        bloques += 1;
    }
    (bloques > 0).then(|| "OJO: no se ha ejecutado NINGUNA prueba (0 tests), así que este «exit code: 0» no demuestra nada. O el filtro de nombre no coincide con ninguna prueba, o ese crate no tiene pruebas con esas opciones (¿falta `--features std`?). Comprueba los nombres con `cargo test -p CRATE --features std -- --list`.".to_string())
}

// ---------------------------------------------------------------- forja

fn forja<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
    let f = pol.forja.as_ref().ok_or_else(|| Error::uso("Forja no está configurada"))?;
    let accion = cadena(args, "accion")?;
    if !["all", "sync", "build"].contains(&accion) {
        return Ok(Respuesta::mal(format!("la acción «{accion}» no existe: usa all, sync o build")));
    }
    let mut argv = f.argv.clone();
    argv.extend([accion.to_string(), "--host".to_string(), f.host.clone(), "--token".to_string(), f.token.clone()]);
    let o = Orden { argv, cwd: pol.raiz.clone(), stdin: Vec::new(), timeout_s: Some(f.plazo_s), entorno: pol.entorno.clone() };
    let s = e.ejecutar(&o)?;
    let cabecera = match (s.codigo, &s.motivo) {
        (Some(c), _) => format!("exit code: {c}"),
        (None, Some(m)) => format!("no terminó: {m}"),
        (None, None) => "no terminó".to_string(),
    };
    // El token no debe volver al modelo aunque la salida lo repita.
    let texto = s.texto().replace(f.token.as_str(), "***");
    Ok(Respuesta { texto: format!("{cabecera}\n{texto}"), error: s.codigo != Some(0) })
}


/// El tramo del archivo que más se parece a un bloque `buscar` de varias líneas (por palabras en común), con sus números de
/// línea. Devuelve `(primera, última, texto)` de un tramo de tantas líneas como `buscar` más dos, con tope de 40.
fn tramo_parecido(texto: &str, buscar: &str) -> Option<(usize, usize, String)> {
    fn palabras(s: &str) -> alloc::collections::BTreeSet<String> {
        s.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|w| w.len() > 1).map(|w| w.to_string()).collect()
    }
    let objetivo = palabras(buscar);
    if objetivo.len() < 3 {
        return None;
    }
    let lineas: Vec<&str> = texto.lines().collect();
    if lineas.is_empty() {
        return None;
    }
    let n = (buscar.lines().count() + 2).clamp(3, 40).min(lineas.len());
    let (mut mejor, mut punt) = (0usize, 0usize);
    for i in 0..=lineas.len() - n {
        let ventana = palabras(&lineas[i..i + n].join(" "));
        let comunes = objetivo.iter().filter(|w| ventana.contains(*w)).count();
        if comunes > punt {
            (mejor, punt) = (i, comunes);
        }
    }
    if punt * 2 < objetivo.len() {
        return None;
    }
    let t = (mejor..mejor + n).map(|i| format!("{}: {}", i + 1, lineas[i])).collect::<Vec<_>>().join("\n");
    Some((mejor + 1, mejor + n, t))
}

/// La línea del archivo que más se parece a la primera línea no vacía de `buscar` (por palabras en común), con dos
/// líneas de contexto a cada lado y su número, para que el modelo corrija en vez de repetir la misma búsqueda.
fn linea_parecida(texto: &str, buscar: &str) -> Option<String> {
    fn palabras(s: &str) -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|w| w.len() > 1).map(|w| w.to_string()).collect()
    }
    let objetivo = palabras(buscar.lines().map(|l| l.trim()).find(|l| !l.is_empty())?);
    if objetivo.is_empty() {
        return None;
    }
    let lineas: Vec<&str> = texto.lines().collect();
    let (mut mejor, mut punt) = (0usize, 0usize);
    for (i, l) in lineas.iter().enumerate() {
        let pl = palabras(l);
        let comunes = objetivo.iter().filter(|w| pl.contains(w)).count();
        if comunes > punt {
            (mejor, punt) = (i, comunes);
        }
    }
    if punt == 0 || punt * 2 < objetivo.len() {
        return None;
    }
    let (a, b) = (mejor.saturating_sub(2), (mejor + 3).min(lineas.len()));
    Some((a..b).map(|i| format!("{}: {}", i + 1, lineas[i])).collect::<Vec<_>>().join("\n"))
}
