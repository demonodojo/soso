//! echo [texto]: imprime sus argumentos.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use libsoso::println;

libsoso::entry!(main);

fn main(args: &[String]) -> u8 {
    // Juntar es lo que hace `echo`; la diferencia es que ahora junta el
    // argv real, así que `echo "a  b"` sale con sus dos espacios.
    println!("{}", args.join(" "));
    0
}
