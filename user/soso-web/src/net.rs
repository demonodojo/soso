//! TCP/HTTPS para soso-web.

use libsoso::sys;
use soso_abi::SockAddr;
use soso_http::TcpTransport;

pub struct Net;

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
