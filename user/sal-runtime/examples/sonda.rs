//! Ejecuta un caso y comprueba su código real, sin interpretar mensajes del shell.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::{string::String, vec::Vec};
use libsoso::{sys, println};
libsoso::entry!(main);
fn main(args: &[String]) -> u8 {
    if args.len() < 2 { println!("SAL_FALLO argumentos {:?}", args); return 2; }
    let Ok(code) = args[0].parse::<u8>() else { println!("SAL_FALLO argumentos {:?}", args); return 2; };
    let argv: Vec<&str> = args[1..].iter().map(String::as_str).collect();
    for fd in 0..3 { if sys::dup2(fd, fd + 10) < 0 { return 1; } }
    let pid = sys::spawn_io_ex(argv[0], &argv, &[], 10, 11, 12);
    for fd in 10..13 { sys::close(fd); }
    if pid < 0 { println!("SAL_FALLO spawn {}", pid); return 1; }
    match sys::wait() {
        Ok((p, c)) if p == pid as u64 && c == code => { println!("SAL_OK código {}", c); 0 },
        other => { println!("SAL_FALLO código {:?}, esperado {}", other, code); 1 }
    }
}
