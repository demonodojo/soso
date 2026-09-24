//! `stat RUTA` — muestra metadatos de un fichero.

#![no_std]
#![no_main]

extern crate alloc;

use libsoso::{abi, errno_str, println, sys};
use alloc::string::String;

libsoso::entry!(main);

fn main(args: &[String]) -> u8 {
    let Some(path) = args.first() else {
        println!("uso: stat RUTA");
        return 2;
    };
    if path.is_empty() {
        println!("uso: stat RUTA");
        return 2;
    }
    let mut st = abi::Stat::default();
    let rc = sys::stat(path, &mut st);
    if rc < 0 {
        println!("stat: {path}: {}", errno_str(rc));
        return 1;
    }
    let tipo = match st.file_type {
        abi::FT_DIR => "dir",
        abi::FT_FILE => "file",
        _ => "?",
    };
    println!("  ruta: {path}");
    println!("  ino:  {}", st.ino);
    println!("  tipo: {tipo}");
    println!("  size: {}", st.size);
    println!("  mtime: {}", st.mtime);
    0
}
