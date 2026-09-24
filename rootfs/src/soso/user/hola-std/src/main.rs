//! Hola mundo con `soso-std` (Hito 2).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use soso_std::println;

libsoso::entry!(main);

fn main(args: &[String]) -> u8 {
    libsoso::heap_init();
    soso_std::init_from_argv(args);
    soso_std::init();
    if let Some(a) = soso_std::env::args().next() {
        println!("arg0: {a}");
    }
    println!("hola desde soso-std");
    0
}
