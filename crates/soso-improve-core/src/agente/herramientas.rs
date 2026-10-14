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
                "limit": {"type": "integer", "description": "Número de líneas a leer"}},
                "required": ["filePath"]}}},
        {"type": "function", "function": {
            "name": "grep",
            "description": "Busca un texto LITERAL (no una expresión regular) en los archivos de un directorio. Devuelve archivo:línea: texto.",
            "parameters": {"type": "object", "properties": {
                "pattern": {"type": "string", "description": "Texto literal a buscar, p. ej. el nombre de una función"},
                "path": {"type": "string", "description": "Directorio o archivo donde buscar (por defecto la raíz)"},
                "include": {"type": "string", "description": "Filtro de nombre de archivo, p. ej. *.rs"}},
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
            "description": "Reemplaza un fragmento de código de un archivo por otro. `buscar` es el fragmento que quieres cambiar, copiado de `read` (puede ocupar varias líneas; no importan los espacios ni la sangría, sólo los caracteres) y debe aparecer una sola vez; `nuevo` es lo que lo sustituye. Usa un fragmento corto pero único, por ejemplo `k == key`.",
            "parameters": {"type": "object", "properties": {
                "filePath": {"type": "string", "description": "Ruta del archivo, relativa a la raíz del checkout"},
                "buscar": {"type": "string", "description": "Fragmento a sustituir (sin importar los espacios)"},
                "nuevo": {"type": "string", "description": "Texto que lo sustituye"}},
                "required": ["filePath", "buscar", "nuevo"]}}},
        {"type": "function", "function": {
            "name": "insertar",
            "description": "Inserta texto NUEVO (por ejemplo una función entera) después de una línea. `linea` es el número de línea que te mostró `read` (0 = al principio del archivo). Úsala para AÑADIR código; para cambiar un fragmento existente usa reemplazar_texto. El texto debe traer sus llaves completas.",
            "parameters": {"type": "object", "properties": {
                "filePath": {"type": "string", "description": "Ruta del archivo, relativa a la raíz del checkout"},
                "linea": {"type": "integer", "description": "Se inserta después de esta línea (0 = al principio)"},
                "texto": {"type": "string", "description": "Texto a insertar, con su sangría"}},
                "required": ["filePath", "linea", "texto"]}}},
        {"type": "function", "function": {
            "name": "bash",
            "description": "Ejecuta UNA orden (sin shell: sin ; & | > < $ ni comodines). Permitidas: cargo test, cargo check, cargo build, cargo fmt, git status, git diff, git log.",
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

/// Ejecuta una herramienta por nombre. Un nombre desconocido o argumentos mal formados son una
/// respuesta para el modelo, no un error del agente.
pub fn ejecutar<E: Entorno>(e: &mut E, pol: &Politica, nombre: &str, args: &Value) -> Respuesta {
    let r = match nombre {
        "read" => leer(e, pol, args),
        "grep" => buscar(e, pol, args),
        "glob" => listar_glob(e, pol, args),
        "reemplazar_texto" => reemplazar(e, pol, args),
        "insertar" => insertar(e, pol, args),
        "bash" => orden(e, pol, args),
        "forja" if pol.forja.is_some() => forja(e, pol, args),
        otro => Ok(Respuesta::mal(format!(
            "La herramienta «{otro}» no existe. Las disponibles son: read, grep, glob, reemplazar_texto, insertar, bash{}.", if pol.forja.is_some() { " y forja" } else { "" }
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

fn cadena<'a>(args: &'a Value, clave: &str) -> Resultado<&'a str> {
    args.get(clave)
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::uso(format!("falta el argumento «{clave}» (texto)")))
}

fn entero(args: &Value, clave: &str) -> Option<usize> {
    let v = args.get(clave)?;
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
const ARCHIVO_MAX: u64 = 4 * 1024 * 1024;

fn leer<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
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
    let n = entero(args, "limit").unwrap_or(pol.lineas_por_defecto).max(1);
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
        format!("(Líneas {desde}-{hasta} de {total}. Usa offset={} para seguir.{aviso})", hasta + 1)
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

/// ¿Parece una expresión regular? El rechazo sólo cubre lo que nadie escribe por accidente en un
/// literal (N-009): barra invertida con letra, cuantificadores tras `.`, y anclas en los extremos.
fn parece_regex(p: &str) -> bool {
    let b = p.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'\\' && b[i + 1].is_ascii_alphabetic() {
            return true;
        }
        if b[i] == b'.' && matches!(b[i + 1], b'*' | b'+' | b'?') {
            return true;
        }
        i += 1;
    }
    p.starts_with('^') || p.ends_with('$')
}

fn contiene(linea: &[u8], aguja: &[u8]) -> bool {
    aguja.is_empty() || linea.windows(aguja.len()).any(|w| w == aguja)
}

fn buscar<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
    let patron = cadena(args, "pattern")?;
    if patron.is_empty() {
        return Err(Error::uso("el patrón está vacío"));
    }
    if parece_regex(patron) {
        return Ok(Respuesta::mal(format!(
            "El patrón «{patron}» parece una expresión regular y esta herramienta busca texto LITERAL (como grep -F); \
             buscarlo tal cual daría «no hay» sin que sea cierto. Busca un fragmento literal del código, por ejemplo \
             el nombre de la función o una cadena que aparezca en él."
        )));
    }
    let (abs, rel) = resolver(pol, args.get("path").and_then(|v| v.as_str()).unwrap_or("."))?;
    let filtro = args.get("include").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
    let aviso = if patron.contains(['|', '[', '(', '*']) {
        format!("Aviso: «{patron}» se busca tal cual (literal), sin interpretar |, [, ( ni *.\n")
    } else {
        String::new()
    };
    let aguja = patron.as_bytes();

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

    let (mut coincidencias, mut con_hallazgo, mut sin_leer, mut binarios) = (0usize, 0usize, 0usize, 0usize);
    let mut salida = String::new();
    for (a, r) in &archivos {
        let datos = match e.leer(a) {
            Ok(d) => d,
            Err(_) => {
                sin_leer += 1;
                continue;
            }
        };
        if datos.contains(&0) {
            binarios += 1;
            continue;
        }
        let mut en_archivo = false;
        for (n, linea) in datos.split(|&c| c == b'\n').enumerate() {
            if contiene(linea, aguja) {
                coincidencias += 1;
                en_archivo = true;
                if coincidencias <= pol.resultados_max {
                    let t = String::from_utf8_lossy(linea);
                    let t = t.trim_end_matches('\r');
                    let t = if t.len() > 300 { &t[..t.char_indices().take_while(|(i, _)| *i < 300).last().map_or(0, |(i, c)| i + c.len_utf8())] } else { t };
                    salida.push_str(&format!("{r}:{}: {}\n", n + 1, t.trim()));
                }
            }
        }
        if en_archivo {
            con_hallazgo += 1;
        }
    }
    let revisados = archivos.len() - sin_leer - binarios;
    let nota_fallos = if sin_leer + binarios > 0 {
        format!(" ({sin_leer} no se pudieron leer, {binarios} binarios omitidos)")
    } else {
        String::new()
    };
    if coincidencias == 0 {
        // «No hay» sólo se dice si de verdad se pudo mirar; si no, es «no pude».
        let veredicto = if sin_leer > 0 && revisados == 0 { "No pude buscar" } else { "No hay coincidencias" };
        return Ok(Respuesta::ok(format!(
            "{aviso}{veredicto} de «{patron}» en {revisados} archivo(s) revisado(s){nota_fallos}."
        )));
    }
    let mut t = format!("{aviso}{salida}");
    if coincidencias > pol.resultados_max {
        t.push_str(&format!("(… {} coincidencias más; afina la búsqueda)\n", coincidencias - pol.resultados_max));
    }
    t.push_str(&format!("{coincidencias} coincidencia(s) en {con_hallazgo} archivo(s){nota_fallos}."));
    Ok(Respuesta::ok(t))
}

// ---------------------------------------------------------------- reemplazar_texto

fn reemplazar<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
    let (abs, rel) = resolver(pol, cadena(args, "filePath")?)?;
    let buscar = cadena(args, "buscar")?;
    let nuevo = cadena(args, "nuevo")?;
    let datos = e.leer(&abs).map_err(|_| no_existe(e, pol, &rel))?;
    let texto = String::from_utf8(datos).map_err(|_| Error::uso(format!("«{}» no es UTF-8", mostrar(&rel))))?;
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
        let pista = linea_parecida(&texto, buscar)
            .map(|p| format!("\nLa línea más parecida del archivo es:\n{p}\nCopia de ahí UNA línea completa y corta."))
            .unwrap_or_default();
        return Ok(Respuesta::mal(format!(
            "No encuentro ese fragmento en {}. Copia un trozo más corto de lo que te mostró `read`, sin los números de línea.{pista}",
            mostrar(&rel)
        )));
    };
    if let Some(sig) = plano[primero_b + plano[primero_b..].chars().next().map_or(1, |c| c.len_utf8())..].find(&aguja) {
        let segundo_b = primero_b + plano[primero_b..].chars().next().map_or(1, |c| c.len_utf8()) + sig;
        let linea = |b: usize| texto[..pos[en_chars(b)].0].matches('\n').count() + 1;
        return Ok(Respuesta::mal(format!(
            "El fragmento aparece más de una vez (líneas {} y {}). Añade más contexto.",
            linea(primero_b),
            linea(segundo_b)
        )));
    }
    let i0 = en_chars(primero_b);
    let (ini, fin) = (pos[i0].0, pos[i0 + n_aguja - 1].1);
    let antes = &texto[ini..fin];
    let linea = texto[..ini].matches('\n').count() + 1;
    let mut nuevo_texto = String::with_capacity(texto.len() + nuevo.len());
    nuevo_texto.push_str(&texto[..ini]);
    nuevo_texto.push_str(nuevo);
    nuevo_texto.push_str(&texto[fin..]);
    if let Some(m) = desequilibrio(&rel, &texto, &nuevo_texto) {
        return Ok(Respuesta::mal(m));
    }
    let modo = e.metadatos(&abs).map(|m| m.modo).unwrap_or(0o644);
    e.escribir(&abs, nuevo_texto.as_bytes(), modo)?;
    Ok(Respuesta::ok(format!(
        "Hecho en {}, línea {linea}.\nAntes:\n{antes}\nDespués:\n{nuevo}",
        mostrar(&rel)
    )))
}

// ---------------------------------------------------------------- insertar

fn insertar<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
    let (abs, rel) = resolver(pol, cadena(args, "filePath")?)?;
    let nuevo = cadena(args, "texto")?;
    let linea = entero(args, "linea").ok_or_else(|| Error::uso("falta el argumento «linea» (entero: 0 = al principio, N = después de la línea N de `read`)"))?;
    if nuevo.trim().is_empty() {
        return Err(Error::uso("`texto` está vacío"));
    }
    let datos = e.leer(&abs).map_err(|_| no_existe(e, pol, &rel))?;
    let texto = String::from_utf8(datos).map_err(|_| Error::uso(format!("«{}» no es UTF-8", mostrar(&rel))))?;
    let mut lineas: Vec<&str> = texto.split('\n').collect();
    let terminaba_en_salto = lineas.last() == Some(&"");
    if terminaba_en_salto {
        lineas.pop();
    }
    if linea > lineas.len() {
        return Err(Error::uso(format!("«{}» tiene {} línea(s): no se puede insertar después de la {linea}", mostrar(&rel), lineas.len())));
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
    let modo = e.metadatos(&abs).map(|m| m.modo).unwrap_or(0o644);
    e.escribir(&abs, nuevo_texto.as_bytes(), modo)?;
    Ok(Respuesta::ok(format!(
        "Hecho: {} línea(s) insertadas en {} después de la línea {linea}.\nInsertado:\n{nuevo}",
        nuevo.trim_end_matches('\n').matches('\n').count() + 1,
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

fn orden<E: Entorno>(e: &mut E, pol: &Politica, args: &Value) -> Resultado<Respuesta> {
    let linea = cadena(args, "command")?;
    let argv = match trocear(linea) {
        Ok(a) if !a.is_empty() => a,
        Ok(_) => return Err(Error::uso("la orden está vacía")),
        Err(m) => return Ok(Respuesta::mal(m)),
    };
    let permitida = pol.ejecutables.iter().any(|p| argv.len() >= p.len() && argv[..p.len()] == p[..]);
    if !permitida {
        let lista: Vec<String> = pol.ejecutables.iter().map(|p| p.join(" ")).collect();
        return Ok(Respuesta::mal(format!(
            "«{}» no está permitida. Las órdenes permitidas son: {}.",
            argv.join(" "),
            lista.join(", ")
        )));
    }
    let mut o = Orden { argv, cwd: pol.raiz.clone(), stdin: Vec::new(), timeout_s: Some(pol.plazo_orden_s), entorno: pol.entorno.clone() };
    o.entorno.sort();
    let s = e.ejecutar(&o)?;
    let cabecera = match (s.codigo, &s.motivo) {
        (Some(c), _) => format!("exit code: {c}"),
        (None, Some(m)) => format!("no terminó: {m}"),
        (None, None) => "no terminó".to_string(),
    };
    Ok(Respuesta { texto: format!("{cabecera}\n{}", s.texto()), error: s.codigo != Some(0) })
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
