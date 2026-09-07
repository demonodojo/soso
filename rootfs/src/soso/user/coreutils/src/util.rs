//! Utilidades compartidas entre los coreutils de soso.

extern crate alloc;

use core::result::Result;
use libsoso::{abi, errno_str, sys};

/// Lee un fichero entero a memoria.
pub fn leer_fichero(path: &str) -> Result<alloc::vec::Vec<u8>, i64> {
    let fd = sys::open(path, abi::O_RDONLY);
    if fd < 0 {
        return Err(fd);
    }
    let mut out = alloc::vec::Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n < 0 {
            sys::close(fd as u64);
            return Err(n);
        }
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    sys::close(fd as u64);
    Ok(out)
}

/// Escribe un búfer creando o truncando el destino.
pub fn escribir_fichero(path: &str, data: &[u8]) -> Result<(), i64> {
    let fd = sys::open(path, abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC);
    if fd < 0 {
        return Err(fd);
    }
    if !data.is_empty() && sys::write_all(fd as u64, data).is_err() {
        sys::close(fd as u64);
        return Err(-abi::EIO);
    }
    if sys::close(fd as u64) < 0 {
        return Err(-abi::EIO);
    }
    Ok(())
}

pub fn err_path(path: &str, rc: i64) {
    libsoso::println!("{path}: {}", errno_str(rc));
}
