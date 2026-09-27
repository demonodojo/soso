//! reboot: reinicia la máquina.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use libsoso::{println, sys};

libsoso::entry!(main);

fn main(_args: &[String]) -> u8 {
    println!("reiniciando…");
    sys::reboot();
    println!("reboot: no se pudo reiniciar");
    1
}
