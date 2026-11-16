//! `kill` — manda una señal a un proceso o a un grupo.
//!
//! `kill [-SEÑAL | -s SEÑAL | -N] pid…` (por defecto TERM). Un pid negativo
//! es un grupo (`kill -STOP -12`). `kill -l` lista las señales.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use libsoso::{abi, errno_str, println, sys};

libsoso::entry!(main);

fn uso() -> u8 {
    println!("uso: kill [-SEÑAL | -s SEÑAL] pid|-pgid …   ·   kill -l");
    2
}

fn listar() -> u8 {
    for (n, nombre) in abi::NOMBRES_SENALES {
        println!("{n:2} {nombre}");
    }
    0
}

fn main(args: &[String]) -> u8 {
    let mut sig = abi::SIGTERM;
    let mut i = 0;
    match args.first().map(|s| s.as_str()) {
        None => return uso(),
        Some("-l") | Some("-L") => return listar(),
        Some("-s") => {
            let Some(nombre) = args.get(1) else {
                return uso();
            };
            let Some(s) = abi::senal_por_nombre(nombre) else {
                println!("kill: señal desconocida: {nombre}");
                return 2;
            };
            sig = s;
            i = 2;
        }
        // `-9`, `-KILL`, `-SIGKILL`… salvo que sea un grupo y no una señal:
        // `kill -12` es SIGUSR2; para el grupo 12 hay que dar la señal antes.
        Some(a) if a.starts_with('-') && args.len() > 1 => {
            let Some(s) = abi::senal_por_nombre(&a[1..]) else {
                println!("kill: señal desconocida: {}", &a[1..]);
                return 2;
            };
            sig = s;
            i = 1;
        }
        Some(_) => {}
    }
    if i >= args.len() {
        return uso();
    }
    let mut fallos = 0u8;
    for a in &args[i..] {
        let Ok(pid) = a.parse::<i64>() else {
            println!("kill: {a}: no es un pid");
            fallos = 1;
            continue;
        };
        if pid == 0 {
            println!("kill: 0: no es un pid");
            fallos = 1;
            continue;
        }
        let r = sys::kill(pid, sig);
        if r < 0 {
            println!("kill: {pid}: {}", errno_str(r));
            fallos = 1;
        }
    }
    fallos
}
