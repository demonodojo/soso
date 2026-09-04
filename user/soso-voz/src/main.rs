//! Reconocimiento de voz nativo en soso.

#![no_std]
#![no_main]

extern crate alloc;

mod net;
mod vozd;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libsoso::println;
use vozd::{preguntar_via_vozd, run_vozd, spawn_vozd};

libsoso::entry!(main);

fn main(args: &str) -> u8 {
    let args = args.trim();
    if args == "vozd" {
        return run_vozd();
    }
    if args.starts_with("dictar ") {
        return dictar(&args["dictar ".len()..].trim());
    }
    if args.is_empty() {
        println!("uso: soso-voz vozd | dictar [--wav fichero] [--max-tokens N]");
        return 1;
    }
    println!("comando desconocido");
    1
}

fn dictar(args: &str) -> u8 {
    let mut wav: Option<&str> = None;
    let mut max_tokens: Option<usize> = None;
    let mut i = 0;
    let parts: Vec<&str> = args.split_whitespace().collect();
    while i < parts.len() {
        match parts[i] {
            "--wav" => {
                i += 1;
                if i >= parts.len() {
                    println!("soso-voz: falta ruta tras --wav");
                    return 1;
                }
                wav = Some(parts[i]);
            }
            "--max-tokens" => {
                i += 1;
                if i >= parts.len() {
                    println!("soso-voz: falta N tras --max-tokens");
                    return 1;
                }
                max_tokens = parts[i].parse().ok();
                if max_tokens.is_none() {
                    println!("soso-voz: --max-tokens inválido");
                    return 1;
                }
            }
            other => {
                println!("soso-voz: opción desconocida {other}");
                return 1;
            }
        }
        i += 1;
    }
    match wav {
        Some(path) => dictar_wav(path, max_tokens),
        None => dictar_escucha(),
    }
}

fn dictar_wav(path: &str, max_tokens: Option<usize>) -> u8 {
    let cmd = match max_tokens {
        Some(n) => format!(":wav {path} {n}"),
        None => format!(":wav {path}"),
    };
    match transcribir(&cmd) {
        Some(t) => {
            println!("soso-voz: transcrito — {t}");
            0
        }
        None => {
            println!("soso-voz: error");
            1
        }
    }
}

fn dictar_escucha() -> u8 {
    match transcribir(":escucha") {
        Some(t) => {
            println!("soso-voz: transcrito — {t}");
            0
        }
        None => {
            println!("soso-voz: error");
            1
        }
    }
}

fn transcribir(cmd: &str) -> Option<String> {
    if let Some(t) = preguntar_via_vozd(cmd) {
        return Some(t);
    }
    let _ = spawn_vozd();
    for _ in 0..100 {
        if let Some(t) = preguntar_via_vozd(cmd) {
            return Some(t);
        }
        let _ = libsoso::sys::sleep_ms(50);
    }
    None
}
