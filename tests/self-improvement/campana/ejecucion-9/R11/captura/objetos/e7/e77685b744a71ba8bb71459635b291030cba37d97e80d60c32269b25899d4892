//! rm <ruta>...: borra ficheros y directorios vacíos.

#![no_std]
#![no_main]

use libsoso::{errno_str, println, sys};

libsoso::entry!(main);

fn main(args: &str) -> u8 {
    let mut alguno = false;
    let mut fallo = 0u8;
    for path in args.split_whitespace() {
        alguno = true;
        let r = sys::unlink(path);
        if r < 0 {
            println!("rm: {path}: {}", errno_str(r));
            fallo = 1;
        }
    }
    if !alguno {
        println!("uso: rm <ruta>...");
        return 2;
    }
    fallo
}
