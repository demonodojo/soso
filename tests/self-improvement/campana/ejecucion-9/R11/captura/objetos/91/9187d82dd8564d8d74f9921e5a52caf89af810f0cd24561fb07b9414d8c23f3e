//! `diff A B` — compara dos ficheros línea a línea.

#![no_std]
#![no_main]

extern crate alloc;

use coreutils::util::{err_path, leer_fichero};
use libsoso::println;

libsoso::entry!(main);

fn main(args: &str) -> u8 {
    let mut it = args.split_whitespace();
    let Some(a) = it.next() else {
        println!("uso: diff A B");
        return 2;
    };
    let Some(b) = it.next() else {
        println!("uso: diff A B");
        return 2;
    };
    let da = match leer_fichero(a) {
        Ok(d) => d,
        Err(e) => {
            err_path(a, e);
            return 1;
        }
    };
    let db = match leer_fichero(b) {
        Ok(d) => d,
        Err(e) => {
            err_path(b, e);
            return 1;
        }
    };
    if da == db {
        return 0;
    }
    let sa = core::str::from_utf8(&da).unwrap_or("");
    let sb = core::str::from_utf8(&db).unwrap_or("");
    let la: alloc::vec::Vec<_> = sa.lines().collect();
    let lb: alloc::vec::Vec<_> = sb.lines().collect();
    let max = la.len().max(lb.len());
    let mut dif = 0usize;
    for i in 0..max {
        let va = la.get(i).copied().unwrap_or("");
        let vb = lb.get(i).copied().unwrap_or("");
        if va != vb {
            dif += 1;
            println!("{}c{}", i + 1, i + 1);
            println!("< {va}");
            println!("---");
            println!("> {vb}");
        }
    }
    if dif == 0 {
        println!("los ficheros difieren en tamaño");
        return 1;
    }
    1
}
