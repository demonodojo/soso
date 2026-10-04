//! rm [-r] <ruta>...: borra ficheros y directorios vacíos.
//! Con -r (o -R), vacía el árbol y borra el directorio.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libsoso::{abi, errno_str, println, sys};

libsoso::entry!(main);

fn main(args: &[String]) -> u8 {
    let mut recursivo = false;
    let mut rutas: Vec<&str> = Vec::new();
    let mut solo_rutas = false;
    for a in args {
        let a = a.as_str();
        if solo_rutas || a == "-" || !a.starts_with('-') {
            rutas.push(a);
            continue;
        }
        if a == "--" {
            solo_rutas = true;
            continue;
        }
        if a == "--recursive" {
            recursivo = true;
            continue;
        }
        if a.starts_with("--") {
            println!("rm: opción desconocida: {a}");
            return 2;
        }
        for c in a.chars().skip(1) {
            match c {
                'r' | 'R' => recursivo = true,
                otra => {
                    println!("rm: opción desconocida: -{otra}");
                    return 2;
                }
            }
        }
    }
    if rutas.is_empty() {
        println!("uso: rm [-r] <ruta>...");
        return 2;
    }
    let raiz = ino_raiz();
    let mut fallo = 0u8;
    for path in rutas {
        fallo |= borrar(path, recursivo, raiz);
    }
    fallo
}

fn ino_raiz() -> Option<u64> {
    let mut st = abi::Stat::default();
    if sys::stat("/", &mut st) < 0 {
        None
    } else {
        Some(st.ino)
    }
}

/// `..` desde cualquier sitio se normaliza a `/` en el kernel. Comparar el
/// inodo evita borrar el árbol entero con `rm -r ..`.
fn es_raiz(path: &str, raiz: Option<u64>) -> bool {
    let Some(raiz) = raiz else {
        return false;
    };
    let mut st = abi::Stat::default();
    sys::stat(path, &mut st) == 0 && st.file_type == abi::FT_DIR && st.ino == raiz
}

fn borrar(path: &str, recursivo: bool, raiz: Option<u64>) -> u8 {
    if es_raiz(path, raiz) {
        println!("rm: {path}: no borro /");
        return 1;
    }
    if recursivo {
        let mut st = abi::Stat::default();
        let r = sys::stat(path, &mut st);
        if r < 0 {
            println!("rm: {path}: {}", errno_str(r));
            return 1;
        }
        if st.file_type == abi::FT_DIR && vaciar(path) != 0 {
            return 1;
        }
    }
    let r = sys::unlink(path);
    if r < 0 {
        println!("rm: {path}: {}", errno_str(r));
        return 1;
    }
    0
}

/// Borra el contenido de `dir`. El directorio en sí lo quita el llamante.
fn vaciar(dir: &str) -> u8 {
    let fd = sys::open(dir, abi::O_RDONLY);
    if fd < 0 {
        println!("rm: {dir}: {}", errno_str(fd));
        return 1;
    }
    let mut hijos: Vec<(String, bool)> = Vec::new();
    let mut ents = [abi::Dirent::default(); 16];
    let mut fallo = 0u8;
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n < 0 {
            println!("rm: {dir}: {}", errno_str(n));
            fallo = 1;
            break;
        }
        if n == 0 {
            break;
        }
        for d in &ents[..n as usize / abi::DIRENT_SIZE] {
            let Ok(name) = core::str::from_utf8(d.name_bytes()) else {
                println!("rm: {dir}: nombre no es utf-8");
                fallo = 1;
                continue;
            };
            if name.is_empty() || name == "." || name == ".." {
                continue;
            }
            let base = dir.trim_end_matches('/');
            let ruta = if base.is_empty() {
                format!("/{name}")
            } else {
                format!("{base}/{name}")
            };
            hijos.push((ruta, d.file_type == abi::FT_DIR));
        }
    }
    sys::close(fd as u64);
    for (ruta, es_dir) in hijos {
        if es_dir && vaciar(&ruta) != 0 {
            fallo = 1;
            continue;
        }
        let r = sys::unlink(&ruta);
        if r < 0 {
            println!("rm: {ruta}: {}", errno_str(r));
            fallo = 1;
        }
    }
    fallo
}
