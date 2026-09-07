//! `grep PATRON [FICHERO...]` — busca líneas que contienen el patrón.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use coreutils::util::{err_path, leer_fichero};
use libsoso::{abi, sys};

libsoso::entry!(main);

fn buscar_en(path: &str, patron: &str) -> u8 {
    let data = match leer_fichero(path) {
        Ok(d) => d,
        Err(e) => {
            err_path(path, e);
            return 1;
        }
    };
    let texto = core::str::from_utf8(&data).unwrap_or("");
    for linea in texto.split_inclusive('\n') {
        if linea.contains(patron) {
            if path == "-" {
                libsoso::print!("{linea}");
            } else {
                libsoso::print!("{path}:{linea}");
            }
        }
    }
    0
}

fn main(args: &str) -> u8 {
    let mut it = args.split_whitespace();
    let Some(patron) = it.next() else {
        libsoso::println!("uso: grep PATRON [FICHERO...]");
        return 2;
    };
    let mut alguno = false;
    let mut fallo = 0u8;
    for path in it {
        alguno = true;
        fallo |= buscar_en(path, patron);
    }
    if !alguno {
        let mut buf = [0u8; 1024];
        loop {
            let n = sys::read(0, &mut buf);
            if n <= 0 {
                break;
            }
            let chunk = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
            for linea in chunk.split_inclusive('\n') {
                if linea.contains(patron) {
                    libsoso::print!("{linea}");
                }
            }
        }
    }
    fallo
}
