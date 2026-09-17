//! HTTPS para descargas de actualización.

use alloc::vec::Vec;
use libsoso::{abi, sys};
use soso_abi::SockAddr;
use soso_http::TcpTransport;

struct Net;

fn guest_wall_clock() -> Option<u64> {
    let mut ts = abi::Timespec::default();
    if sys::clock_gettime(abi::CLOCK_REALTIME, &mut ts) != 0 {
        return None;
    }
    let secs = ts.tv_sec as u64;
    if secs < 1_000_000_000 {
        None
    } else {
        Some(secs)
    }
}

fn ensure_wall_clock() {
    static INSTALLED: core::sync::atomic::AtomicBool =
        core::sync::atomic::AtomicBool::new(false);
    if !INSTALLED.swap(true, core::sync::atomic::Ordering::AcqRel) {
        soso_http::set_wall_clock(guest_wall_clock);
    }
}

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
    ensure_wall_clock();
    let resp = soso_http::https_get(&Net, url, token).map_err(map_http_err)?;
    if resp.status != 200 {
        return Err("HTTP != 200");
    }
    Ok(resp.body)
}

/// Trozo máximo en vuelo. Lo fija `soso-update-core` (U4) y acota dos cosas a
/// la vez: la RAM del cliente y lo que se pierde si se corta la red.
const MAX_RANGE: u64 = soso_update_core::TROZO_MAX;

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
    https_download_span_a(url, token, start, len, &mut |trozo| {
        out.extend_from_slice(trozo);
        Ok(())
    })?;
    Ok(out)
}

/// Igual, pero entregando cada trozo según llega.
///
/// Es la variante que usa la descarga durable: el llamante lo escribe en el
/// área de preparación y lo hashea al vuelo, así que en RAM nunca hay más de
/// `MAX_RANGE`. Acumular el tramo entero —lo que se hacía antes— es memoria que
/// una máquina pequeña no tiene, y obliga a repetirlo todo si se corta la red.
pub fn https_download_span_a(
    url: &str,
    token: Option<&str>,
    start: u64,
    len: u64,
    recibe: &mut dyn FnMut(&[u8]) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    if len == 0 {
        return Ok(());
    }
    let mut off = start;
    let fin = start + len;
    while off < fin {
        let end = (off + MAX_RANGE).min(fin) - 1;
        let (status, chunk) =
            soso_http::https_get_range(&Net, url, token, off, end).map_err(map_http_err)?;
        // 200 sólo vale si el servidor ignoró el Range y nos dio el fichero
        // entero, y eso únicamente sirve cuando pedíamos desde el principio.
        if status != 206 {
            if start == 0 && off == 0 && status == 200 && chunk.len() as u64 >= len {
                recibe(&chunk[..len as usize])?;
                return Ok(());
            }
            return Err("HTTP range");
        }
        if chunk.is_empty() {
            return Err("range vacío");
        }
        // Un servidor que devuelva más de lo pedido no puede desbordar el plan.
        let n = (chunk.len() as u64).min(fin - off) as usize;
        recibe(&chunk[..n])?;
        off += n as u64;
    }
    Ok(())
}

fn map_http_err(e: soso_http::HttpError) -> &'static str {
    match e {
        soso_http::HttpError::Clock => "reloj del sistema no utilizable",
        soso_http::HttpError::Dns => "DNS",
        soso_http::HttpError::Tls => "TLS",
        soso_http::HttpError::Parse => "HTTP parse",
        soso_http::HttpError::Io => "descarga HTTP",
    }
}
