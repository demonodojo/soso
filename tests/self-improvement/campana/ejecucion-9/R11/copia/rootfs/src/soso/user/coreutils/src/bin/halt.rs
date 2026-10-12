//! halt: apaga la máquina.

#![no_std]
#![no_main]

use libsoso::{println, sys};

libsoso::entry!(main);

fn main(_args: &str) -> u8 {
    println!("apagando…");
    sys::halt();
    // Si volvemos, el apagado falló.
    println!("halt: no se pudo apagar");
    1
}
