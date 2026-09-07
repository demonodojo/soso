//! Stub de `rustc` en el guest (Hito 3): comprueba sysroot y canal dev.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use libsoso::{println, sys};

libsoso::entry!(main);

const SYSROOT: &str = "/usr/lib/rustlib/x86_64-unknown-soso";
const VERSION: &str = "soso-rustc 0.0.0-dev (bootstrap pendiente)";

fn existe(path: &str) -> bool {
    sys::open(path, libsoso::abi::O_RDONLY) >= 0
}

fn main(args: &str) -> u8 {
    if args.contains("--version") || args.contains("-V") {
        println!("{VERSION}");
        return 0;
    }
    if args.contains("--print") && args.contains("sysroot") {
        println!("{SYSROOT}");
        return 0;
    }
    if args.contains("help") || args.is_empty() {
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
