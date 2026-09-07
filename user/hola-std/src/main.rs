//! Hola mundo con `soso-std` (Hito 2).

#![no_std]
#![no_main]

extern crate alloc;

use soso_std::println;

libsoso::entry!(main);

fn main(_args: &str) -> u8 {
    libsoso::heap_init();
    soso_std::init();
    println!("hola desde soso-std");
    0
}
