//! ask-modelo [nombre]: qué modelo usa `ask`, y cambiarlo.
//!
//! `ask` no admite flags —todo lo que va detrás es la pregunta—, así que la
//! configuración vive fuera, en `/etc/llm.conf`. Esto es lo que la escribe sin
//! tener que acordarse del formato.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use libsoso::{abi, errno_str, println, sys};

libsoso::entry!(main);

const CONF: &str = "/etc/llm.conf";

fn main(args: &str) -> u8 {
    let arg = args.trim();
    if arg == "--help" || arg == "help" {
        println!("uso: ask-modelo            # lista los modelos y marca el actual");
        println!("     ask-modelo <nombre>   # fija el que usará `ask`");
        return 0;
    }

    let disponibles = modelos();
    if disponibles.is_empty() {
        println!("ask-modelo: no hay ningún modelo en /models");
        return 1;
    }

    if arg.is_empty() {
        let actual = modelo_configurado();
        for m in &disponibles {
            let marca = if Some(m.as_str()) == actual.as_deref() {
                '*'
            } else {
                ' '
            };
            println!("{marca} {m}");
        }
        match actual {
            Some(ref m) if disponibles.iter().any(|d| d == m) => {}
            Some(m) => println!("ask-modelo: {CONF} dice «{m}», que no está en /models"),
            None => println!("ask-modelo: sin {CONF}; `ask` usará {}", disponibles[0]),
        }
        return 0;
    }

    if !disponibles.iter().any(|m| m == arg) {
        println!("ask-modelo: no hay ningún modelo «{arg}» en /models");
        for m in &disponibles {
            println!("  {m}");
        }
        return 1;
    }

    // Se reescribe el fichero entero conservando las demás claves: `max`,
    // `temp` y compañía las lee `ask` de aquí y no son cosa nuestra.
    let mut lineas: Vec<String> = Vec::new();
    let mut puesto = false;
    if let Some(texto) = leer(CONF) {
        for l in texto.lines() {
            if l.trim_start().starts_with("modelo=") {
                lineas.push(format!("modelo={arg}"));
                puesto = true;
            } else {
                lineas.push(l.to_string());
            }
        }
    }
    if !puesto {
        if lineas.is_empty() {
            lineas.push("# Configuración de `ask`. Cámbiala con: ask-modelo <nombre>".to_string());
        }
        lineas.push(format!("modelo={arg}"));
    }
    let mut cuerpo = lineas.join("\n");
    cuerpo.push('\n');

    let fd = sys::open(CONF, abi::O_WRONLY | abi::O_CREAT);
    if fd < 0 {
        println!("ask-modelo: {CONF}: {}", errno_str(fd));
        return 1;
    }
    let escrito = sys::write_all(fd as u64, cuerpo.as_bytes());
    sys::close(fd as u64);
    if let Err(e) = escrito {
        println!("ask-modelo: escribiendo {CONF}: {}", errno_str(e));
        return 1;
    }
    println!("ask-modelo: {arg}");
    0
}

fn modelo_configurado() -> Option<String> {
    let texto = leer(CONF)?;
    for l in texto.lines() {
        if let Some(v) = l.trim().strip_prefix("modelo=") {
            return Some(v.trim().to_string());
        }
    }
    None
}

fn leer(path: &str) -> Option<String> {
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

fn modelos() -> Vec<String> {
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
