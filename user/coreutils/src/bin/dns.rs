//! `dns nombre` — resuelve un nombre a IPv4.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use libsoso::{errno_str, println, sys};

libsoso::entry!(main);

fn main(args: &[String]) -> u8 {
    let host = match args {
        [h] if !h.is_empty() && !h.starts_with('-') => h.as_str(),
        _ => {
            println!("uso: dns nombre");
            return 1;
        }
    };
    let mut ip = [0u8; 4];
    match sys::dns_resolve(host, &mut ip) {
        Ok(()) => {
            println!(
                "{host} → {}.{}.{}.{}",
                ip[0], ip[1], ip[2], ip[3]
            );
            0
        }
        Err(e) => {
            println!("dns: {host}: {} (errno {})", errno_str(e), -e);
            1
        }
    }
}
