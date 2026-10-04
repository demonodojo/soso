//! Codificación de argv para el crt0 de userspace (magic `SOSA`).

use super::addrspace::{AddrSpace, STACK_TOP};
use alloc::string::String;
use alloc::vec::Vec;

const MAGIC: &[u8; 4] = b"SOSA";
/// 128 KiB: ver `read_spawn_args`. El blob va justo bajo `STACK_TOP`, así que
/// puede pasar de la pila inicial de 64 KiB; sus páginas se mapean aquí.
const MAX_BLOB: usize = 128 * 1024;

/// Serializa argv en el formato que `soso_std::init_from_args` decodifica.
pub fn encode(argv: &[String]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&(argv.len() as u32).to_le_bytes());
    for s in argv {
        let b = s.as_bytes();
        buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
        buf.extend_from_slice(b);
    }
    buf
}

/// Escribe el blob en la pila del hijo; devuelve (va, len) para rdi/rsi.
pub fn write_stack(space: &AddrSpace, argv: &[String]) -> Option<(u64, u64)> {
    let blob = encode(argv);
    if blob.is_empty() || blob.len() > MAX_BLOB {
        return None;
    }
    let largo = (blob.len() as u64).next_multiple_of(4096).max(4096);
    let args_va = STACK_TOP - largo;
    let mut va = args_va;
    while va < STACK_TOP {
        space.ensure_mapped(va)?;
        va += 4096;
    }
    // El blob acaba pegado a `STACK_TOP - 4096 + len` como antes cuando cabe en
    // una página; si no, empieza en `args_va`.
    space.write(args_va, &blob)?;
    Some((args_va, blob.len() as u64))
}
