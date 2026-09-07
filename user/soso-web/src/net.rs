//! TCP/HTTPS para soso-web.

use libsoso::{abi, sys};
use soso_abi::SockAddr;
use soso_http::TcpTransport;

pub struct Net;

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

pub fn ensure_wall_clock() {
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
        if fd < 0 { Err(fd) } else { Ok(fd as u64) }
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
