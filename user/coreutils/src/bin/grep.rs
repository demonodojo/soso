//! `grep PATRON [FICHERO...]` — líneas que contienen el patrón (subcadena).
//!
//! Sin ficheros (o con `-`) lee **stdin**: `log | grep askd`. En la consola,
//! Ctrl-D cierra la entrada.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use coreutils::util::{err_path, leer_fichero};
use libsoso::sys;

libsoso::entry!(main);

fn emitir(linea: &[u8], prefijo: Option<&str>) {
    if let Some(p) = prefijo {
        let _ = sys::write_all(1, p.as_bytes());
        let _ = sys::write_all(1, b":");
    }
    let _ = sys::write_all(1, linea);
}

fn contiene(linea: &[u8], patron: &str) -> bool {
    core::str::from_utf8(linea)
        .map(|s| s.contains(patron))
        .unwrap_or(false)
}

fn buscar_bytes(data: &[u8], patron: &str, prefijo: Option<&str>) {
    let mut start = 0usize;
    for (i, &b) in data.iter().enumerate() {
        if b != b'\n' {
            continue;
        }
        let linea = &data[start..=i];
        if contiene(linea, patron) {
            emitir(linea, prefijo);
        }
        start = i + 1;
    }
    if start < data.len() && contiene(&data[start..], patron) {
        emitir(&data[start..], prefijo);
        let _ = sys::write_all(1, b"\n");
    }
}

fn buscar_stdin(patron: &str, prefijo: Option<&str>) -> u8 {
    let mut data = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = sys::read(0, &mut buf);
        if n < 0 {
            return 1;
        }
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    buscar_bytes(&data, patron, prefijo);
    0
}

fn buscar_en(path: &str, patron: &str, prefijo: Option<&str>) -> u8 {
    if path == "-" {
        return buscar_stdin(patron, prefijo);
    }
    match leer_fichero(path) {
        Ok(data) => {
            buscar_bytes(&data, patron, prefijo);
            0
        }
        Err(e) => {
            err_path(path, e);
            1
        }
    }
}

fn main(args: &str) -> u8 {
    let mut it = args.split_whitespace();
    let Some(patron) = it.next() else {
        libsoso::println!("uso: grep PATRON [FICHERO...]");
        return 2;
    };
    let paths: Vec<&str> = it.collect();
    if paths.is_empty() {
        return buscar_stdin(patron, None);
    }
    let varios = paths.len() > 1;
    let mut fallo = 0u8;
    for path in &paths {
        let prefijo = if varios { Some(*path) } else { None };
        fallo |= buscar_en(path, patron, prefijo);
    }
    fallo
}
