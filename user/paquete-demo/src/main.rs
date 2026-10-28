//! Demo para pruebas de `soso-pkg` (no va en `/bin` del rootfs base).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;

libsoso::entry!(main);

fn main(_args: &[String]) -> u8 {
    libsoso::println!("pkg-demo: ok");
    0
}
