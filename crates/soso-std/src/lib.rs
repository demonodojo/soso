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

/// Fija `env::args` a partir del argv que entrega `entry!` (T62).
pub fn init_from_argv(args: &[alloc::string::String]) {
    env::init_from_argv(args);
}

/// Parsea una **cadena** de argumentos en `env::args`.
///
/// Queda para quien todavía reciba una cadena suelta; parte por espacios, así
/// que un argumento con un espacio dentro se pierde. Lo correcto es
/// [`init_from_argv`].
pub fn init_from_args(args: &str) {
    env::init_from_args(args);
}
