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
use vozd::{conectar_vozd, dialogar, preguntar_via_vozd, run_vozd, spawn_vozd};

libsoso::entry!(main);

fn main(args: &[String]) -> u8 {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");
    if cmd == "vozd" {
        return run_vozd();
    }
    if cmd == "dictar" {
        return dictar(&args[1..]);
    }
    if args.is_empty() {
        println!("uso: soso-voz vozd | dictar [--wav fichero] [--max-tokens N]");
        return 1;
    }
    println!("comando desconocido");
    1
}

fn dictar(args: &[String]) -> u8 {
    let mut wav: Option<&str> = None;
    let mut max_tokens: Option<usize> = None;
    let mut i = 0;
    let parts: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
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

/// Espera a que el demonio abra el socket: 200 × (100 ms de conexión + 50 de
/// pausa) ≈ 30 s como mucho. El bucle sólo reintenta **la conexión**; la
/// petición se manda una vez y espera con el margen largo de `copiar_respuesta`,
/// que es donde caben la carga del modelo y la transcripción.
const VOZD_INTENTOS: u32 = 200;
const VOZD_ESPERA_MS: u64 = 50;

fn transcribir(cmd: &str) -> Option<String> {
    if let Some(t) = preguntar_via_vozd(cmd) {
        return Some(t);
    }
    // El demonio se arranca **una vez**. Lanzar uno por intento fallido llenaba
    // la máquina de procesos cargando el modelo.
    if spawn_vozd().is_err() {
        println!("soso-voz: no se pudo arrancar vozd");
        return None;
    }
    for _ in 0..VOZD_INTENTOS {
        let _ = libsoso::sys::sleep_ms(VOZD_ESPERA_MS);
        if let Some(client) = conectar_vozd(100) {
            return dialogar(&client, cmd);
        }
    }
    println!("soso-voz: vozd no abrió el socket; ¿arrancó el demonio?");
    None
}
