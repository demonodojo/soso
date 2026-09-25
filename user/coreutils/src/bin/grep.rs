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
use libsoso::{abi, errno_str, println, sys};

libsoso::entry!(main);

/// Hubo al menos una coincidencia.
const SALIDA_HAY: u8 = 0;
/// No hubo ninguna. Es un resultado, no un fallo.
const SALIDA_NADA: u8 = 1;
/// No se pudo buscar: uso incorrecto, patrón no expresable o fichero ilegible.
const SALIDA_ERROR: u8 = 2;

struct Opciones {
    ignorar_caso: bool,
    numerar: bool,
    solo_nombres: bool,
    recursivo: bool,
    literal: bool,
    max: usize,
}

impl Default for Opciones {
    fn default() -> Self {
        Self {
            ignorar_caso: false,
            numerar: false,
            solo_nombres: false,
            recursivo: false,
            literal: false,
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
                otra => {
                    println!("grep: opción desconocida: -{otra}");
                    return SALIDA_ERROR;
                }
            }
        }
    }

    let Some(patron) = sueltos.first().copied() else {
        println!("uso: grep [-inlrF] [-m N] PATRON [FICHERO...]");
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
