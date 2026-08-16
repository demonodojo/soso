//! HTTPS y descarga en guest (scratch sosomfs o fichero sosofs).

use alloc::vec::Vec;
use crate::guest_io::ScratchSink;
use libsoso::sys;
use soso_abi::SockAddr;
use soso_http::{HttpError, Response, TcpTransport};

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

/// Respuestas pequeñas (JSON del árbol Hub) en RAM.
pub fn https_get_body(url: &str, token: Option<&str>) -> Result<Vec<u8>, HttpError> {
    let Response { status, body } = soso_http::https_get(&Net, url, token)?;
    if status != 200 {
        return Err(HttpError::Parse);
    }
    Ok(body)
}

/// Descarga grande en scratch sosomfs (cola de p3).
pub fn download_to_scratch(
    url: &str,
    token: Option<&str>,
    mut sink: ScratchSink,
) -> Result<(), HttpError> {
    let (status, _) = soso_http::https_download(&Net, url, token, &mut sink)?;
    if status != 200 {
        return Err(HttpError::Parse);
    }
    sink.finish().map_err(|_| HttpError::Io)
}

/// GET parcial HTTPS (`Range: bytes=start-end`).
pub fn https_get_range_bytes(
    url: &str,
    token: Option<&str>,
    start: u64,
    end: u64,
) -> Result<(u16, alloc::vec::Vec<u8>), HttpError> {
    soso_http::https_get_range(&Net, url, token, start, end)
}
