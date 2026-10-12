//! Codificación de argv para el crt0 de userspace (magic `SOSA`).

use super::addrspace::{AddrSpace, STACK_TOP};
use alloc::string::String;
use alloc::vec::Vec;

const MAGIC: &[u8; 4] = b"SOSA";
const MAX_BLOB: usize = 4096;

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
    let args_va = STACK_TOP - 4096;
    space.write(args_va, &blob)?;
    Some((args_va, blob.len() as u64))
}
