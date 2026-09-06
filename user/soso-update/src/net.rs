//! HTTPS para descargas de actualización.

use alloc::vec::Vec;
use libsoso::sys;
use soso_abi::SockAddr;
use soso_http::TcpTransport;

struct Net;

impl TcpTransport for Net {
    fn dns_resolve(&self, host: &str, out: &mut [u8; 4]) -> Result<(), i64> {
        sys::dns_resolve(host, out)
    }

    fn tcp_connect(&self, addr: SockAddr, timeout_ms: u64) -> Result<u64, i64> {
        let fd = sys::tcp_connect(&addr, timeout_ms);
        if fd < 0 {
            Err(fd)
        } else {
            Ok(fd as u64)
        }
    }

    fn read_timeout(&self, fd: u64, buf: &mut [u8], timeout_ms: u64) -> i64 {
        sys::read_timeout(fd, buf, timeout_ms)
    }

    fn write_all(&self, fd: u64, data: &[u8]) -> Result<(), i64> {
        sys::write_all(fd, data)
    }

    fn close(&self, fd: u64) {
        let _ = sys::close(fd);
    }
}

pub fn https_get_bytes(url: &str, token: Option<&str>) -> Result<Vec<u8>, &'static str> {
    let resp = soso_http::https_get(&Net, url, token).map_err(|_| "descarga HTTP")?;
    if resp.status != 200 {
        return Err("HTTP != 200");
    }
    Ok(resp.body)
}

const MAX_RANGE: u64 = 8 * 1024 * 1024;

pub fn https_download_all(
    url: &str,
    token: Option<&str>,
    total: u64,
) -> Result<Vec<u8>, &'static str> {
    https_download_span(url, token, 0, total)
}

/// Descarga `len` bytes desde `start` con peticiones `Range` de como mucho
/// `MAX_RANGE`. Es la primitiva de la actualización parcial: pedimos sólo los
/// tramos del pack que cubren ficheros cambiados.
pub fn https_download_span(
    url: &str,
    token: Option<&str>,
    start: u64,
    len: u64,
) -> Result<Vec<u8>, &'static str> {
    let mut out = Vec::new();
    if len == 0 {
        return Ok(out);
    }
    if out.try_reserve(len as usize).is_err() {
        return Err("sin memoria");
    }
    let mut off = start;
    let fin = start + len;
    while off < fin {
        let end = (off + MAX_RANGE).min(fin) - 1;
        let (status, chunk) =
            soso_http::https_get_range(&Net, url, token, off, end).map_err(|_| "range HTTP")?;
        // 200 sólo vale si el servidor ignoró el Range y nos dio el fichero
        // entero, y eso únicamente sirve cuando pedíamos desde el principio.
        if status != 206 {
            if start == 0 && off == 0 && status == 200 && chunk.len() as u64 >= len {
                out.extend_from_slice(&chunk[..len as usize]);
                return Ok(out);
            }
            return Err("HTTP range");
        }
        if chunk.is_empty() {
            return Err("range vacío");
        }
        out.extend_from_slice(&chunk);
        off += chunk.len() as u64;
    }
    if out.len() as u64 != len {
        return Err("tamaño inesperado");
    }
    Ok(out)
}
