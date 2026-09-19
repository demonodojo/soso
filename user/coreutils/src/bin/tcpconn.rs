//! `tcpconn` — abre una conexión TCP **saliente** y cuenta qué pasa.
//!
//! Existe porque ese camino no lo ejercitaba nadie: el eco de la suite es
//! entrante y los modelos se bajan desde el host. La primera vez que se usó de
//! verdad —el `comprobar` del OTA— se quedó colgado sin decir nada, y no había
//! forma de separar «no conecta» de «no vuelve del kernel».
//!
//! Uso: `tcpconn <a.b.c.d> <puerto> [texto] [--timeout MS]`
//!
//! Salida: 0 si conectó (y, con texto, si le contestaron); 1 si no.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use libsoso::{abi, errno_str, println, sys};

libsoso::entry!(main);

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut oct = [0u8; 4];
    let mut n = 0;
    for parte in s.split('.') {
        if n >= 4 {
            return None;
        }
        oct[n] = parte.parse().ok()?;
        n += 1;
    }
    (n == 4).then_some(oct)
}

fn main(args: &str) -> u8 {
    let mut it = args.split_whitespace();
    let (Some(ip_s), Some(port_s)) = (it.next(), it.next()) else {
        println!("uso: tcpconn <a.b.c.d> <puerto> [texto] [--timeout MS]");
        return 2;
    };
    let Some(addr) = parse_ip(ip_s) else {
        println!("tcpconn: IP inválida: {ip_s}");
        return 2;
    };
    let Ok(port) = port_s.parse::<u16>() else {
        println!("tcpconn: puerto inválido: {port_s}");
        return 2;
    };

    let resto: alloc::vec::Vec<&str> = it.collect();
    let mut timeout = 10_000u64;
    let mut texto = alloc::string::String::new();
    let mut i = 0;
    while i < resto.len() {
        if resto[i] == "--timeout" {
            timeout = resto.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(timeout);
            i += 2;
            continue;
        }
        if !texto.is_empty() {
            texto.push(' ');
        }
        texto.push_str(resto[i]);
        i += 1;
    }

    let sa = abi::SockAddr { addr, port, _pad: 0 };
    let t0 = ahora_ms();
    let fd = sys::tcp_connect(&sa, timeout);
    let tardo = ahora_ms().saturating_sub(t0);
    if fd < 0 {
        // El **tiempo** importa tanto como el error: un fallo que tarda más que
        // su propio plazo es un plazo que no se respeta.
        println!("tcpconn: sin conexión tras {tardo} ms — {}", errno_str(fd));
        return 1;
    }
    println!("tcpconn: conectado a {ip_s}:{port} en {tardo} ms (fd {fd})");

    let mut rc = 0;
    if !texto.is_empty() {
        if let Err(e) = sys::write_all(fd as u64, texto.as_bytes()) {
            println!("tcpconn: no pude enviar — {}", errno_str(e));
            rc = 1;
        } else {
            let mut buf = vec![0u8; 512];
            let n = sys::read_timeout(fd as u64, &mut buf, timeout);
            if n > 0 {
                let vistos = core::str::from_utf8(&buf[..n as usize]).unwrap_or("<binario>");
                println!("tcpconn: recibido {n} B: {vistos}");
            } else {
                println!("tcpconn: sin respuesta ({})", errno_str(n));
                rc = 1;
            }
        }
    }
    let _ = sys::close(fd as u64);
    rc
}

fn ahora_ms() -> u64 {
    let mut t = abi::Timespec::default();
    if sys::clock_gettime(abi::CLOCK_MONOTONIC, &mut t) < 0 {
        return 0;
    }
    t.tv_sec as u64 * 1000 + t.tv_nsec as u64 / 1_000_000
}
