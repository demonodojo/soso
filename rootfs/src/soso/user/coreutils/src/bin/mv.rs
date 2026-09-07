//! `mv ORIGEN DESTINO` — renombra o mueve un fichero.

#![no_std]
#![no_main]

extern crate alloc;

use coreutils::util::{err_path, escribir_fichero, leer_fichero};
use libsoso::{abi, sys};

libsoso::entry!(main);

fn main(args: &str) -> u8 {
    let mut it = args.split_whitespace();
    let Some(src) = it.next() else {
        libsoso::println!("uso: mv ORIGEN DESTINO");
        return 2;
    };
    let Some(dst) = it.next() else {
        libsoso::println!("uso: mv ORIGEN DESTINO");
        return 2;
    };
    let rc = sys::rename(src, dst);
    if rc == 0 {
        return 0;
    }
    if rc != -abi::ENOSYS {
        err_path(src, rc);
        return 1;
    }
    // Fallback antes de SYS_RENAME: copiar y borrar.
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
    if sys::unlink(src) < 0 {
        err_path(src, -abi::EIO);
        return 1;
    }
    0
}
