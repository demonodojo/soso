//! Stub de `rustc` en el guest (Hito 3): comprueba sysroot y canal dev.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::format;
use libsoso::{println, sys};

libsoso::entry!(main);

const SYSROOT: &str = "/usr/lib/rustlib/x86_64-unknown-soso";
const VERSION: &str = "soso-rustc 0.0.0-dev (bootstrap pendiente)";

fn existe(path: &str) -> bool {
    sys::open(path, libsoso::abi::O_RDONLY) >= 0
}

fn main(args: &[String]) -> u8 {
    // Antes esto era `contains` sobre la línea entera: `--versionado` casaba
    // con `--version`. Con el argv de verdad se compara el argumento.
    let tiene = |f: &str| args.iter().any(|a| a == f);
    if tiene("--version") || tiene("-V") {
        println!("{VERSION}");
        return 0;
    }
    if tiene("--print") && tiene("sysroot") {
        println!("{SYSROOT}");
        return 0;
    }
    if tiene("help") || args.is_empty() {
        println!("{VERSION}");
        println!("sysroot: {SYSROOT}");
        println!("usa: soso-rustc --version | --print sysroot");
        println!("compilación nativa: ./scripts/soso-rust-bootstrap.sh (host)");
        return 0;
    }
    let libdir = format!("{SYSROOT}/lib");
    if !existe(&libdir) {
        println!("soso-rustc: sin sysroot en {SYSROOT}");
        println!("  instala stage2 del fork (config/rust-soso/README.md)");
        return 1;
    }
    println!("soso-rustc: sysroot OK, compilador aún no enlazado");
    println!("  args: {args:?}");
    2
}
