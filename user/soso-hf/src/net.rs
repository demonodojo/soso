//! HTTPS y descarga a fichero en guest (streaming a disco).

use alloc::vec::Vec;
use libsoso::sys;
use soso_abi::SockAddr;
use soso_http::{BodySink, HttpError, Response, TcpTransport};

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

struct FileSink {
    fd: u64,
}

impl BodySink for FileSink {
    fn write_body(&mut self, chunk: &[u8]) -> Result<(), HttpError> {
        sys::write_all(self.fd, chunk).map_err(|_| HttpError::Io)
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

/// Descarga grande: escribe cada chunk TLS en `dest` (StreamWrite del kernel).
pub fn download_url(url: &str, dest: &str, token: Option<&str>) -> Result<(), HttpError> {
    let fd = sys::open(dest, soso_abi::O_WRONLY | soso_abi::O_CREAT);
    if fd < 0 {
        return Err(HttpError::Io);
    }
    let fd = fd as u64;
    let mut sink = FileSink { fd };
    let (status, _) = soso_http::https_download(&Net, url, token, &mut sink)?;
    let _ = sys::close(fd);
    if status != 200 {
        let _ = sys::unlink(dest);
        return Err(HttpError::Parse);
    }
    Ok(())
}
