//! mkdir <ruta>...: crea directorios.

#![no_std]
#![no_main]

extern crate alloc;

use libsoso::{errno_str, println, sys};
use alloc::string::String;

libsoso::entry!(main);

fn main(args: &[String]) -> u8 {
    let mut alguno = false;
    let mut fallo = 0u8;
    for path in args {
        alguno = true;
        let r = sys::mkdir(path);
        if r < 0 {
            println!("mkdir: {path}: {}", errno_str(r));
            fallo = 1;
        }
    }
    if !alguno {
        println!("uso: mkdir <ruta>...");
        return 2;
    }
    fallo
}
