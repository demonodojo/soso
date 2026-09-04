//! Descarga HTTPS y lectura local de HTML.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use libsoso::sys;
use soso_http::{header_value, https_get_full, HttpError};

use soso_web_core::decode::{bytes_to_text, maybe_decompress};
use crate::net::Net;

pub enum FetchError {
    Http(HttpError),
    Io(i64),
    Status(u16),
}

impl FetchError {
    pub fn mensaje(&self) -> &'static str {
        match self {
            Self::Http(HttpError::Parse) => "error HTTP (parse)",
            Self::Http(HttpError::Tls) => "error TLS",
            Self::Http(HttpError::Io) => "error HTTP (E/S)",
            Self::Http(HttpError::Dns) => "error DNS",
            Self::Io(_) => "error de E/S",
            Self::Status(_) => "respuesta HTTP no válida",
        }
    }
}

/// Descarga una URL HTTPS y devuelve HTML como texto.
pub fn fetch_https(url: &str) -> Result<(String, String), FetchError> {
    let resp = https_get_full(&Net, url, None).map_err(FetchError::Http)?;
    if resp.status != 200 {
        return Err(FetchError::Status(resp.status));
    }
    let enc = header_value(&resp.headers, "content-encoding");
    let body = maybe_decompress(&resp.body, enc).map_err(|_| FetchError::Status(resp.status))?;
    let charset = header_value(&resp.headers, "content-type")
        .and_then(|ct| ct.split("charset=").nth(1).map(|s| s.trim()));
    let html = bytes_to_text(&body, charset);
    Ok((html, url.to_string()))
}

/// Lee HTML desde un fichero local (modo prueba / sin red).
pub fn fetch_local(path: &str) -> Result<String, FetchError> {
    let fd = sys::open(path, soso_abi::O_RDONLY);
    if fd < 0 {
        return Err(FetchError::Io(fd));
    }
    let fd = fd as u64;
    let mut st = soso_abi::Stat::default();
    if sys::stat(path, &mut st) < 0 {
        let _ = sys::close(fd);
        return Err(FetchError::Io(-soso_abi::EIO));
    }
    let mut buf = Vec::with_capacity(st.size.min(512 * 1024) as usize);
    buf.resize(st.size.min(512 * 1024) as usize, 0);
    let n = sys::read(fd, &mut buf);
    let _ = sys::close(fd);
    if n < 0 {
        return Err(FetchError::Io(n));
    }
    buf.truncate(n as usize);
    Ok(bytes_to_text(&buf, Some("utf-8")))
}
