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
    let mut out = Vec::new();
    if out.try_reserve(total as usize).is_err() {
        return Err("sin memoria");
    }
    let mut off = 0u64;
    while off < total {
        let end = (off + MAX_RANGE - 1).min(total - 1);
        let (status, chunk) =
            soso_http::https_get_range(&Net, url, token, off, end).map_err(|_| "range HTTP")?;
        if status != 206 && !(off == 0 && status == 200) {
            return Err("HTTP range");
        }
        out.extend_from_slice(&chunk);
        off = end + 1;
    }
    if out.len() as u64 != total {
        return Err("tamaño inesperado");
    }
    Ok(out)
}
