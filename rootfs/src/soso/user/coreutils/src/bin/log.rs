//! Muestra el ring de registros de aplicaciones (fd 3).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use libsoso::{print, sys};

libsoso::entry!(main);

#[unsafe(no_mangle)]
pub fn main(_args: &[String]) -> u8 {
    let mut offset = 0u64;
    let mut buf = [0u8; 4096];
    loop {
        let n = sys::log_read(offset, &mut buf);
        if n <= 0 {
            break;
        }
        let chunk = &buf[..n as usize];
        let text = core::str::from_utf8(chunk).unwrap_or("");
        print!("{text}");
        offset += n as u64;
    }
    0
}
