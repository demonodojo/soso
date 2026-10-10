//! Lectura de ficheros de log y configuración.

use alloc::string::String;
use alloc::vec::Vec;
use libsoso::sys;
use soso_abi::{O_RDONLY, SEEK_SET, Stat};

const READ_BUF: usize = 8192;
/// Tope de lectura por fichero de log (activo + rotado caben en el heap del guest).
const LEE_LOG_MAX: usize = 1024 * 1024 + 256 * 1024;

pub fn read_text(path: &str, max: usize) -> Option<String> {
    let bytes = read_bytes(path, max)?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn read_bytes(path: &str, max: usize) -> Option<Vec<u8>> {
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return None;
    }
    let fd = fd as u64;
    let mut out = Vec::new();
    let mut buf = [0u8; READ_BUF];
    loop {
        if out.len() >= max {
            break;
        }
        let n = sys::read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        let take = (n as usize).min(max - out.len());
        out.extend_from_slice(&buf[..take]);
    }
    let _ = sys::close(fd);
    Some(out)
}

/// Lee un log completo si cabe en `cap`, o la cola final de hasta `cap` bytes.
pub fn read_log_tail(path: &str, cap: usize) -> Vec<u8> {
    let mut st = Stat::default();
    if sys::stat(path, &mut st) < 0 || st.size <= 0 {
        return Vec::new();
    }
    let total = st.size as u64;
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return Vec::new();
    }
    let fd = fd as u64;
    let start = if total as usize > cap {
        total - cap as u64
    } else {
        0
    };
    if sys::seek(fd, start as i64, SEEK_SET) < 0 {
        let _ = sys::close(fd);
        return Vec::new();
    }
    let mut out = Vec::new();
    if out.try_reserve(cap.min(total as usize)).is_err() {
        let _ = sys::close(fd);
        return out;
    }
    let mut buf = [0u8; READ_BUF];
    while (out.len() as u64) < total - start {
        let n = sys::read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    let _ = sys::close(fd);
    out
}

pub fn log_rotado(base: &str) -> String {
    alloc::format!("{base}.1")
}

pub const LEE_LOG_CAP: usize = LEE_LOG_MAX;
