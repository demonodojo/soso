//! `std` mínima para el target `x86_64-unknown-soso` (Hito 2).
//!
//! Implementación sobre `soso-abi` / `libsoso`, modelo Hermit:
//! alloc (dlmalloc), I/O, fs, net, thread, time, futex locks.

#![no_std]

extern crate alloc;

pub mod io;
pub mod fs;
pub mod net;
pub mod thread;
pub mod time;
pub mod env;
pub mod process;
pub mod sync;
pub mod pipe;

pub use alloc::{boxed, rc, string, vec};

/// Inicialización de la mini-std (llamar tras `heap_init`).
pub fn init() {
    env::init();
}

/// Parsea la cadena de argumentos del crt0 (`__soso_main`) en `env::args`.
pub fn init_from_args(args: &str) {
    env::init_from_args(args);
}
