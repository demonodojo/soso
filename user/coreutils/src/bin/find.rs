//! `find RUTA [-name PATRON]` — recorre un árbol de directorios.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use coreutils::util::err_path;
use libsoso::{abi, println, sys};

libsoso::entry!(main);

fn recorrer(dir: &str, patron: Option<&str>) -> u8 {
    let fd = sys::open(dir, abi::O_RDONLY);
    if fd < 0 {
        err_path(dir, fd);
        return 1;
    }
    let mut ents = [abi::Dirent::default(); 16];
    let mut fallo = 0u8;
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n < 0 {
            err_path(dir, n);
            fallo = 1;
            break;
        }
        if n == 0 {
            break;
        }
        for d in &ents[..n as usize / abi::DIRENT_SIZE] {
            let name = core::str::from_utf8(d.name_bytes()).unwrap_or("?");
            if name == "." || name == ".." {
                continue;
            }
            let ruta = if dir == "/" {
                format!("/{name}")
            } else {
                format!("{dir}/{name}")
            };
            let coincide = patron.map(|p| name.contains(p)).unwrap_or(true);
            if coincide {
                println!("{ruta}");
            }
            if d.file_type == abi::FT_DIR {
                fallo |= recorrer(&ruta, patron);
            }
        }
    }
    sys::close(fd as u64);
    fallo
}

fn main(args: &str) -> u8 {
    let mut it = args.split_whitespace();
    let Some(root) = it.next() else {
        println!("uso: find RUTA [-name PATRON]");
        return 2;
    };
    let mut patron = None;
    let mut rest = it;
    if let Some(p) = rest.next() {
        if p == "-name" {
            patron = rest.next();
        }
    }
    recorrer(root, patron)
}
