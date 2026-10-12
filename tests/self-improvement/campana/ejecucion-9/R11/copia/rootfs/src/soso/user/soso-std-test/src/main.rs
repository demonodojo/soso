//! Pruebas de humo de `soso-std` en el guest (Hito 2).

#![no_std]
#![no_main]

extern crate alloc;

use soso_std::{env, fs, pipe, sync, time};

libsoso::entry!(main);

fn main(args: &str) -> u8 {
    libsoso::heap_init();
    soso_std::init_from_args(args);
    let mut fallo = 0u8;

    if env::args().next().is_none() {
        libsoso::println!("std-test: args vacios (ok si no hay argv)");
    }

    let _ = sys::mkdir("/tmp/std-test");
    let path = "/tmp/std-test/hola";
    let fd = libsoso::sys::open(path, libsoso::abi::O_WRONLY | libsoso::abi::O_CREAT | libsoso::abi::O_TRUNC);
    if fd < 0 {
        libsoso::println!("std-test: open escribir fallo");
        return 1;
    }
    let _ = libsoso::sys::write_all(fd as u64, b"soso-std");
    libsoso::sys::close(fd as u64);

    match fs::File::open(path) {
        Ok(f) => match f.read_to_end() {
            Ok(data) if data == b"soso-std" => {}
            Ok(_) => {
                libsoso::println!("std-test: contenido incorrecto");
                fallo = 1;
            }
            Err(e) => {
                libsoso::println!("std-test: read_to_end {e}");
                fallo = 1;
            }
        },
        Err(e) => {
            libsoso::println!("std-test: File::open {e}");
            fallo = 1;
        }
    }

    let mut ts = libsoso::abi::Timespec::default();
    if time::now(&mut ts) != 0 {
        libsoso::println!("std-test: time::now fallo");
        fallo = 1;
    }

    match pipe::pipe() {
        Ok((r, w)) => {
            if w.write_all(b"xy").is_err() {
                fallo = 1;
            }
            let mut buf = [0u8; 2];
            if r.read(&mut buf) != 2 || &buf != b"xy" {
                fallo = 1;
            }
        }
        Err(e) => {
            libsoso::println!("std-test: pipe {e}");
            fallo = 1;
        }
    }

    let m = sync::Mutex::new(0u32);
    *m.lock() = 42;
    if *m.lock() != 42 {
        fallo = 1;
    }

    if fallo == 0 {
        libsoso::println!("soso-std-test: OK");
    }
    fallo
}

mod sys {
    pub use libsoso::sys::*;
}
