//! `grep [OPCIONES] PATRON [FICHERO...]` — líneas que contienen el patrón.
//!
//! **Busca subcadenas literales, no expresiones regulares.** Es la semántica
//! declarada de [N-009](../../../../docs/self-improvement/native/N-009.md), y
//! está escrita aquí porque un buscador cuya semántica hay que adivinar es
//! peor que uno limitado: quien pregunta «¿está este símbolo?» y recibe «no»
//! no vuelve a preguntar.
//!
//! De ahí las tres decisiones que no son de comodidad:
//!
//! - **El código de salida distingue «no hay» de «no pude»**: 0 hubo
//!   coincidencias, 1 no hubo ninguna, 2 no pude buscar. Antes siempre era 0
//!   salvo error de lectura, así que «no aparece» y «funcionó» eran el mismo
//!   valor.
//! - **Un patrón de expresión regular que no puede ser literal se rechaza**
//!   con 2, en vez de buscarse tal cual y devolver cero líneas. Cero líneas es
//!   una respuesta que *parece* un hecho sobre el código.
//! - **Se busca sobre bytes.** Antes se comparaba convirtiendo la línea a
//!   UTF-8 y **las líneas que no lo eran se saltaban en silencio**: un fichero
//!   con un byte suelto inválido escondía el resto de su contenido.
//!
//! Sin ficheros (o con `-`) lee **stdin**: `log | grep askd`. En la consola,
//! Ctrl-D cierra la entrada.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use coreutils::util::{err_path, leer_fichero};
use libsoso::glob;
use libsoso::{abi, errno_str, println, sys};

libsoso::entry!(main);

/// Hubo al menos una coincidencia.
const SALIDA_HAY: u8 = 0;
/// No hubo ninguna. Es un resultado, no un fallo.
const SALIDA_NADA: u8 = 1;
/// No se pudo buscar: uso incorrecto, patrón no expresable o fichero ilegible.
const SALIDA_ERROR: u8 = 2;

struct Opciones {
    /// Patrones de nombre que un fichero debe cumplir para mirarse.
    ///
    /// Vacío = todos. Es el hueco que [N-009] dejó declarado: «un agente los
    /// usa; hoy hay que filtrar después». Filtrar después significa leer y
    /// buscar en ficheros que no interesan, y en soso cada `open` cuesta
    /// (N-012).
    ///
    /// [N-009]: ../../../../docs/self-improvement/native/N-009.md
    incluir: Vec<String>,
    excluir: Vec<String>,
    ignorar_caso: bool,
    numerar: bool,
    solo_nombres: bool,
    recursivo: bool,
    literal: bool,
    /// Tratar como texto lo que parezca binario.
    ///
    /// Por defecto un fichero binario que coincide se anuncia en **una línea**
    /// en vez de volcar sus bytes. [N-009] lo dejó escrito como hueco: «se
    /// buscan como texto, sin detectar que lo son ni saltárselos, y puede
    /// escupir basura a la consola». Cuando quien lee es un agente, esa basura
    /// no es sólo fea: se lleva por delante su contexto.
    ///
    /// [N-009]: ../../../../docs/self-improvement/native/N-009.md
    texto: bool,
    max: usize,
}

impl Default for Opciones {
    fn default() -> Self {
        Self {
            incluir: Vec::new(),
            excluir: Vec::new(),
            ignorar_caso: false,
            numerar: false,
            solo_nombres: false,
            recursivo: false,
            literal: false,
            texto: false,
            max: usize::MAX,
        }
    }
}

/// Qué se puede prometer sobre un patrón.
enum Veredicto {
    /// Se busca tal cual y no hay nada que advertir.
    Literal,
    /// Se busca tal cual, pero lleva metacaracteres que **podrían** ser una
    /// intención de expresión regular. Se avisa y se busca igual: `foo.rs` y
    /// `data[0]` son patrones literales legítimos y rechazarlos molestaría más
    /// de lo que ayuda.
    LiteralConAviso(&'static str),
    /// No puede ser literal en ningún texto razonable: quien lo escribió
    /// quería una expresión regular. Se rechaza.
    NoExpresable(&'static str),
}

/// Clasifica el patrón. La frontera no es «¿tiene metacaracteres?» sino
/// «¿podría alguien querer esto **literalmente**?».
fn clasificar(patron: &str) -> Veredicto {
    let b = patron.as_bytes();
    // `\w`, `\d`, `\s`, `\b`… Una barra invertida seguida de letra no aparece
    // por accidente en una búsqueda literal de código.
    for i in 0..b.len().saturating_sub(1) {
        if b[i] == b'\\' && b[i + 1].is_ascii_alphabetic() {
            return Veredicto::NoExpresable("clases de expresión regular (\\w, \\d, \\s…)");
        }
    }
    // `.*`, `.+`, `.?`: el punto cuantificado. Un `.` suelto sí puede ser
    // literal —`main.rs`—, pero cuantificado no.
    if patron.contains(".*") || patron.contains(".+") || patron.contains(".?") {
        return Veredicto::NoExpresable("cuantificadores (.*, .+, .?)");
    }
    // Anclas en los extremos: `^fn ` o `;$`.
    if patron.starts_with('^') || patron.ends_with('$') {
        return Veredicto::NoExpresable("anclas (^ al principio, $ al final)");
    }
    if patron.contains('|') {
        return Veredicto::LiteralConAviso("la barra vertical no alterna: se busca tal cual");
    }
    if patron.contains('[') || patron.contains('(') || patron.contains('*') {
        return Veredicto::LiteralConAviso("los metacaracteres se buscan tal cual");
    }
    Veredicto::Literal
}

/// ¿Parece binario?
///
/// Un byte cero en los primeros 8 KiB, que es lo que usa `grep` de toda la
/// vida. No es una clasificación perfecta y no pretende serlo: es una heurística
/// **declarada**, y `-a` la desactiva.
fn es_binario(datos: &[u8]) -> bool {
    datos.iter().take(8192).any(|b| *b == 0)
}

/// El último componente de una ruta.
fn nombre_de(ruta: &str) -> &str {
    ruta.rsplit('/').next().unwrap_or(ruta)
}

/// ¿Hay que mirar este fichero?
///
/// `--exclude` gana sobre `--include`: quien excluye algo lo hace para no
/// verlo, y que un include lo devolviera sería lo contrario de lo que pidió.
fn se_mira(ruta: &str, o: &Opciones) -> bool {
    let nombre = nombre_de(ruta);
    if o.excluir.iter().any(|g| glob::casa(nombre, g)) {
        return false;
    }
    o.incluir.is_empty() || o.incluir.iter().any(|g| glob::casa(nombre, g))
}

/// Busca `aguja` en `pajar` por bytes. Sin asignar nada: los ficheros de
/// código son pequeños, pero un `to_lowercase` por línea no lo sería.
fn contiene(pajar: &[u8], aguja: &[u8], ignorar_caso: bool) -> bool {
    if aguja.is_empty() {
        return true;
    }
    if aguja.len() > pajar.len() {
        return false;
    }
    let igual = |a: u8, b: u8| {
        if ignorar_caso {
            a.eq_ignore_ascii_case(&b)
        } else {
            a == b
        }
    };
    for i in 0..=(pajar.len() - aguja.len()) {
        if (0..aguja.len()).all(|j| igual(pajar[i + j], aguja[j])) {
            return true;
        }
    }
    false
}

/// Lo que se lleva la búsqueda de un fichero.
struct Cuenta {
    coincidencias: usize,
}

fn emitir(linea: &[u8], nombre: Option<&str>, numero: Option<usize>) {
    if let Some(n) = nombre {
        let _ = sys::write_all(1, n.as_bytes());
        let _ = sys::write_all(1, b":");
    }
    if let Some(n) = numero {
        let _ = sys::write_all(1, format!("{n}:").as_bytes());
    }
    let _ = sys::write_all(1, linea);
    if !linea.ends_with(b"\n") {
        let _ = sys::write_all(1, b"\n");
    }
}

fn buscar_bytes(data: &[u8], patron: &[u8], nombre: Option<&str>, o: &Opciones) -> Cuenta {
    let mut cuenta = Cuenta { coincidencias: 0 };
    for (i, linea) in data.split(|b| *b == b'\n').enumerate() {
        // `split` da un último trozo vacío cuando el fichero acaba en salto;
        // no es una línea.
        if linea.is_empty() && i > 0 {
            continue;
        }
        if !contiene(linea, patron, o.ignorar_caso) {
            continue;
        }
        cuenta.coincidencias += 1;
        if o.solo_nombres {
            if let Some(n) = nombre {
                let _ = sys::write_all(1, n.as_bytes());
                let _ = sys::write_all(1, b"\n");
            }
            break;
        }
        emitir(linea, nombre, o.numerar.then_some(i + 1));
        if cuenta.coincidencias >= o.max {
            break;
        }
    }
    cuenta
}

/// Cuenta coincidencias sin imprimir nada.
///
/// Para los binarios: hace falta saber **si** coincide para el código de
/// salida, sin volcar los bytes.
fn contar_silencioso(data: &[u8], patron: &[u8], o: &Opciones) -> usize {
    data.split(|b| *b == b'\n')
        .filter(|l| contiene(l, patron, o.ignorar_caso))
        .count()
}

fn buscar_stdin(patron: &[u8], o: &Opciones) -> Result<usize, u8> {
    let mut data = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = sys::read(0, &mut buf);
        if n < 0 {
            println!("grep: stdin: {}", errno_str(n));
            return Err(SALIDA_ERROR);
        }
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    Ok(buscar_bytes(&data, patron, None, o).coincidencias)
}

fn es_dir(path: &str) -> bool {
    let mut st = abi::Stat::default();
    sys::stat(path, &mut st) == 0 && st.file_type == abi::FT_DIR
}

fn unir(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Recorre `dir` a lo ancho y devuelve los ficheros. Sin recursión de pila: un
/// árbol hondo no puede tumbar el proceso.
fn ficheros_de(dir: &str, fallo: &mut bool) -> Vec<String> {
    let mut pendientes = alloc::vec![String::from(dir)];
    let mut out = Vec::new();
    while let Some(d) = pendientes.pop() {
        let fd = sys::open(&d, abi::O_RDONLY);
        if fd < 0 {
            println!("grep: {d}: {}", errno_str(fd));
            *fallo = true;
            continue;
        }
        let mut ents = [abi::Dirent::default(); 16];
        loop {
            let n = sys::getdents(fd as u64, &mut ents);
            if n <= 0 {
                if n < 0 {
                    println!("grep: {d}: {}", errno_str(n));
                    *fallo = true;
                }
                break;
            }
            for e in &ents[..n as usize / abi::DIRENT_SIZE] {
                let Ok(name) = core::str::from_utf8(e.name_bytes()) else {
                    continue;
                };
                if name == "." || name == ".." {
                    continue;
                }
                let ruta = unir(&d, name);
                if es_dir(&ruta) {
                    pendientes.push(ruta);
                } else {
                    out.push(ruta);
                }
            }
        }
        sys::close(fd as u64);
    }
    out.sort();
    out
}

fn main(args: &[String]) -> u8 {
    let mut o = Opciones::default();
    let mut it = args.iter().map(|s| s.as_str()).peekable();
    let mut sueltos: Vec<&str> = Vec::new();
    let mut solo_sueltos = false;

    while let Some(a) = it.next() {
        if solo_sueltos || a == "-" || !a.starts_with('-') || a.len() < 2 {
            sueltos.push(a);
            continue;
        }
        if a == "--" {
            solo_sueltos = true;
            continue;
        }
        if let Some(g) = a.strip_prefix("--include=") {
            o.incluir.push(String::from(g));
            continue;
        }
        if let Some(g) = a.strip_prefix("--exclude=") {
            o.excluir.push(String::from(g));
            continue;
        }
        if a == "--include" || a == "--exclude" {
            let Some(g) = it.next() else {
                println!("grep: {a} necesita un patrón");
                return SALIDA_ERROR;
            };
            if a == "--include" {
                o.incluir.push(String::from(g));
            } else {
                o.excluir.push(String::from(g));
            }
            continue;
        }
        if a.starts_with("--") {
            println!("grep: opción desconocida: {a}");
            return SALIDA_ERROR;
        }
        if a == "-m" {
            let Some(v) = it.next().and_then(|v| v.parse::<usize>().ok()) else {
                println!("grep: -m necesita un número");
                return SALIDA_ERROR;
            };
            o.max = v;
            continue;
        }
        for c in a.chars().skip(1) {
            match c {
                'i' => o.ignorar_caso = true,
                'n' => o.numerar = true,
                'l' => o.solo_nombres = true,
                'r' => o.recursivo = true,
                'F' => o.literal = true,
                'a' => o.texto = true,
                otra => {
                    println!("grep: opción desconocida: -{otra}");
                    return SALIDA_ERROR;
                }
            }
        }
    }

    let Some(patron) = sueltos.first().copied() else {
        println!("uso: grep [-ainlrF] [-m N] [--include=GLOB] [--exclude=GLOB] PATRON [FICHERO...]");
        println!("  un binario que coincide se anuncia, no se vuelca; -a lo fuerza");
        println!("  globs: sólo * y ?, sobre el nombre del fichero (no la ruta)");
        println!("  busca subcadenas literales; no hay expresiones regulares");
        println!("  salida: 0 hubo coincidencias, 1 ninguna, 2 no se pudo buscar");
        return SALIDA_ERROR;
    };
    if !o.literal {
        match clasificar(patron) {
            Veredicto::Literal => {}
            Veredicto::LiteralConAviso(m) => println!("grep: aviso: {m}"),
            Veredicto::NoExpresable(m) => {
                println!("grep: este patrón usa {m}, y grep busca subcadenas literales");
                println!("grep: repítelo con -F si de verdad quieres buscarlo tal cual");
                return SALIDA_ERROR;
            }
        }
    }
    let patron = patron.as_bytes();

    let mut rutas: Vec<String> = sueltos[1..].iter().map(|s| String::from(*s)).collect();
    let mut fallo = false;
    if o.recursivo {
        let mut expandidas = Vec::new();
        for r in &rutas {
            if es_dir(r) {
                expandidas.extend(ficheros_de(r, &mut fallo));
            } else {
                expandidas.push(r.clone());
            }
        }
        rutas = expandidas;
    }
    // El filtro se aplica **a lo que se va a abrir**, no a la salida: abrir un
    // fichero que no interesa cuesta, y en soso cuesta bastante (N-012).
    //
    // Sobre rutas dadas a mano también: quien pone `--include=*.rs` y lista
    // ficheros espera que el filtro mande, no que sólo valga en `-r`.
    let dieron_rutas = !rutas.is_empty();
    if !o.incluir.is_empty() || !o.excluir.is_empty() {
        rutas.retain(|r| r == "-" || se_mira(r, &o));
    }
    // **Filtrar hasta dejar la lista vacía no es «no me dieron ficheros».**
    // Sin esto se caía en la rama de stdin y `grep` se quedaba esperando
    // entrada del terminal para siempre: un filtro que no casa con nada
    // colgaba el programa en vez de decir que no hay coincidencias. Lo cazó la
    // sonda, y sólo porque tenía un caso para el resultado vacío.
    if dieron_rutas && rutas.is_empty() {
        return SALIDA_NADA;
    }

    if rutas.is_empty() {
        return match buscar_stdin(patron, &o) {
            Err(e) => e,
            Ok(0) => SALIDA_NADA,
            Ok(_) => SALIDA_HAY,
        };
    }

    // El nombre delante se pone cuando hay más de un fichero, que es cuando
    // una línea suelta no diría de dónde sale.
    let varios = rutas.len() > 1;
    let mut total = 0usize;
    for path in &rutas {
        if path == "-" {
            match buscar_stdin(patron, &o) {
                Err(_) => fallo = true,
                Ok(n) => total += n,
            }
            continue;
        }
        match leer_fichero(path) {
            Ok(data) => {
                let nombre = if varios || o.solo_nombres { Some(path.as_str()) } else { None };
                // Un binario se busca igual —la aguja puede estar ahí— pero no
                // se vuelca. Sólo se anuncia, salvo `-a`.
                if !o.texto && !o.solo_nombres && es_binario(&data) {
                    let mut o_mudo = Opciones { solo_nombres: true, ..Opciones::default() };
                    o_mudo.ignorar_caso = o.ignorar_caso;
                    let n = contar_silencioso(&data, patron, &o_mudo);
                    if n > 0 {
                        println!("grep: {path}: binario coincide (usa -a para verlo)");
                        total += n;
                    }
                    continue;
                }
                total += buscar_bytes(&data, patron, nombre, &o).coincidencias;
            }
            Err(e) => {
                err_path(path, e);
                fallo = true;
            }
        }
    }
    if fallo {
        return SALIDA_ERROR;
    }
    if total == 0 { SALIDA_NADA } else { SALIDA_HAY }
}
