//! Ensamblador GAS x86_64 de subconjunto para el toolchain nativo de soso.
//!
//! Uso: `sosoas -o out.o in.s`
//! Stub inicial: valida el fichero y escribe un marcador; el port completo
//! con iced-x86 vendrá en la siguiente iteración del Hito 3c.

use std::env;
use std::fs;
use std::process::exit;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 || args[1] != "-o" {
        eprintln!("uso: sosoas -o SALIDA.o ENTRADA.s");
        exit(2);
    }
    let out = &args[2];
    let inp = &args[3];
    let src = fs::read_to_string(inp).unwrap_or_else(|e| {
        eprintln!("sosoas: {inp}: {e}");
        exit(1);
    });
    // Stub: valida que el fichero es ensamblador y escribe un objeto ELF vacío
    // marcador. El port completo de wild + codegen vendrá en iteraciones siguientes.
    if !src.contains(".text") && !src.contains(".globl") {
        eprintln!("sosoas: {inp}: no parece GAS x86_64");
        exit(1);
    }
    let mut fmt_label = String::new();
    fmt_label.push_str(inp);
    eprintln!("sosoas: ensamblado simbólico de {fmt_label} ({} B)", src.len());
    let marker = format!("sosoas-stub\0{inp}\0");
    fs::write(out, marker.as_bytes()).unwrap_or_else(|e| {
        eprintln!("sosoas: no se pudo escribir {out}: {e}");
        exit(1);
    });
}
