//! HTTPS (POST a la API de GitHub).

use libsoso::{abi, println, sys};
use soso_abi::SockAddr;
use soso_http::{FullResponse, HttpError, TcpTransport};

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

    fn log_red(&self, msg: &str) {
        println!("  red: handshake TLS — {msg}");
    }
}

pub fn https_post_json(url: &str, token: &str, body: &[u8]) -> Result<FullResponse, HttpError> {
    ensure_wall_clock();
    soso_http::https_post_full(&Net, url, Some(token), body)
}

pub fn map_http_err(e: HttpError) -> &'static str {
    match e {
        HttpError::Interrupted => "interrumpido",
        HttpError::Clock => "reloj del sistema no utilizable (TLS)",
        HttpError::Dns => "no pude resolver api.github.com",
        HttpError::Tls(motivo) => {
            println!("  red: TLS falló — {motivo}");
            "TLS"
        }
        HttpError::Parse => "HTTP rechazado o redirección (notifybug no sigue redirects)",
        HttpError::ParseDetail(detail) => {
            println!("  red: HTTP parse — {detail}");
            "HTTP parse"
        }
        HttpError::Io(donde) => {
            println!("  red: E/S falló — {donde}");
            "red"
        }
    }
}
