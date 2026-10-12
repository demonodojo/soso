//! TCP loopback para vozd.

use libsoso::sys;
use soso_abi;

pub struct TcpFd {
    pub fd: u64,
}

impl TcpFd {
    pub fn connect(addr: &soso_abi::SockAddr, timeout_ms: u64) -> Result<Self, i64> {
        let fd = sys::tcp_connect(addr, timeout_ms);
        if fd < 0 { Err(fd) } else { Ok(Self { fd: fd as u64 }) }
    }

    pub fn listen(port: u16) -> Result<Self, i64> {
        let fd = sys::tcp_listen(port);
        if fd < 0 { Err(fd) } else { Ok(Self { fd: fd as u64 }) }
    }

    pub fn accept(listener: &Self, timeout_ms: u64) -> Result<Self, i64> {
        let fd = sys::tcp_accept(listener.fd, timeout_ms);
        if fd < 0 { Err(fd) } else { Ok(Self { fd: fd as u64 }) }
    }
}

impl Drop for TcpFd {
    fn drop(&mut self) {
        let _ = sys::close(self.fd);
    }
}

pub fn parse_sock_addr(s: &str) -> Option<soso_abi::SockAddr> {
    let (host, port) = s.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    let mut oct = [0u8; 4];
    for (i, part) in host.split('.').enumerate() {
        if i >= 4 {
            return None;
        }
        oct[i] = part.parse().ok()?;
    }
    Some(sys::sock_addr(oct[0], oct[1], oct[2], oct[3], port))
}

pub fn write_all(fd: u64, data: &[u8]) -> Result<(), i64> {
    sys::write_all(fd, data)
}

pub fn read_timeout(fd: u64, buf: &mut [u8], ms: u64) -> i64 {
    sys::read_timeout(fd, buf, ms)
}
