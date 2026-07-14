//! echo [texto]: imprime sus argumentos.

#![no_std]
#![no_main]

use libsoso::println;

libsoso::entry!(main);

fn main(args: &str) -> u8 {
    println!("{args}");
    0
}
