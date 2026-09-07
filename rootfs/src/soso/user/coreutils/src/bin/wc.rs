//! `wc [-l] [FICHERO...]` — cuenta bytes y líneas.

#![no_std]
#![no_main]

extern crate alloc;

use coreutils::util::{err_path, leer_fichero};
use libsoso::{sys, println};

libsoso::entry!(main);

fn contar(data: &[u8]) -> (usize, usize) {
    let mut lineas = 0usize;
    if !data.is_empty() {
        lineas = 1;
    }
    for &b in data {
        if b == b'\n' {
            lineas += 1;
        }
    }
    (data.len(), lineas)
}

fn wc_fichero(path: &str, _solo_lineas: bool) -> Result<(usize, usize), i64> {
    let data = if path == "-" {
        let mut out = alloc::vec::Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = sys::read(0, &mut buf);
            if n < 0 {
                return Err(n);
            }
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
        out
    } else {
        leer_fichero(path)?
    };
    Ok(contar(&data))
}

fn main(args: &str) -> u8 {
    let mut solo_lineas = false;
    let mut paths: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    for tok in args.split_whitespace() {
        if tok == "-l" {
            solo_lineas = true;
        } else {
            paths.push(tok);
        }
    }
    if paths.is_empty() {
        paths.push("-");
    }
    let mut total_b = 0usize;
    let mut total_l = 0usize;
    let mut fallo = 0u8;
    for path in paths.iter() {
        match wc_fichero(path, solo_lineas) {
            Ok((b, l)) => {
                total_b += b;
                total_l += l;
                if solo_lineas {
                    println!("{l:>8} {path}");
                } else {
                    println!("{l:>8} {b:>8} {path}");
                }
            }
            Err(e) => {
                err_path(path, e);
                fallo = 1;
            }
        }
    }
    if paths.len() > 1 && fallo == 0 {
        if solo_lineas {
            println!("{total_l:>8} total");
        } else {
            println!("{total_l:>8} {total_b:>8} total");
        }
    }
    fallo
}
