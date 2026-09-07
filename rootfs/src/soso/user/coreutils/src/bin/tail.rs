//! `tail [-n N] [FICHERO...]` — últimas líneas.

#![no_std]
#![no_main]

extern crate alloc;

use coreutils::util::{err_path, leer_fichero};
use libsoso::sys;

libsoso::entry!(main);

fn tail_data(data: &[u8], n: usize) {
    let s = core::str::from_utf8(data).unwrap_or("");
    let lineas: alloc::vec::Vec<_> = s.split_inclusive('\n').collect();
    let start = lineas.len().saturating_sub(n);
    for linea in &lineas[start..] {
        let _ = sys::write_all(1, linea.as_bytes());
    }
}

fn main(args: &str) -> u8 {
    let mut n = 10usize;
    let mut paths: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    let mut it = args.split_whitespace();
    while let Some(tok) = it.next() {
        if tok == "-n" {
            if let Some(v) = it.next() {
                n = v.bytes().fold(0usize, |a, b| a * 10 + (b - b'0') as usize);
            }
        } else {
            paths.push(tok);
        }
    }
    if paths.is_empty() {
        paths.push("-");
    }
    let mut fallo = 0u8;
    for path in paths.iter() {
        let data = if *path == "-" {
            let mut out = alloc::vec::Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let r = sys::read(0, &mut buf);
                if r <= 0 {
                    break;
                }
                out.extend_from_slice(&buf[..r as usize]);
            }
            out
        } else {
            match leer_fichero(path) {
                Ok(d) => d,
                Err(e) => {
                    err_path(path, e);
                    fallo = 1;
                    continue;
                }
            }
        };
        tail_data(&data, n);
    }
    fallo
}
