//! `cp ORIGEN DESTINO` — copia un fichero.

#![no_std]
#![no_main]

extern crate alloc;

use coreutils::util::{err_path, escribir_fichero, leer_fichero};
use libsoso::{abi, sys};

libsoso::entry!(main);

fn main(args: &str) -> u8 {
    let mut it = args.split_whitespace();
    let Some(src) = it.next() else {
        libsoso::println!("uso: cp ORIGEN DESTINO");
        return 2;
    };
    let Some(dst) = it.next() else {
        libsoso::println!("uso: cp ORIGEN DESTINO");
        return 2;
    };
    let data = match leer_fichero(src) {
        Ok(d) => d,
        Err(e) => {
            err_path(src, e);
            return 1;
        }
    };
    if let Err(e) = escribir_fichero(dst, &data) {
        err_path(dst, e);
        return 1;
    }
    0
}
