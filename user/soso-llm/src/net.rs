//! Transporte TCP userspace vía syscalls del kernel.

use libsoso::sys;
use soso_abi;
use soso_llm_core::pipeline::Transport;

/// Lectura por trozos para no bloquear más de este intervalo (ms).
pub const READ_CHUNK_MS: u64 = 1_000;

pub struct TcpFd {
    pub fd: u64,
}

impl TcpFd {
    pub fn connect(addr: &soso_abi::SockAddr, timeout_ms: u64) -> Result<Self, i64> {
        let fd = sys::tcp_connect(addr, timeout_ms);
        if fd < 0 {
            Err(fd)
        } else {
            Ok(Self { fd: fd as u64 })
        }
    }

    pub fn listen(port: u16) -> Result<Self, i64> {
        let fd = sys::tcp_listen(port);
        if fd < 0 {
            Err(fd)
        } else {
            Ok(Self { fd: fd as u64 })
        }
    }

    pub fn accept(listener: &Self, timeout_ms: u64) -> Result<Self, i64> {
        let fd = sys::tcp_accept(listener.fd, timeout_ms);
        if fd < 0 {
            Err(fd)
        } else {
            Ok(Self { fd: fd as u64 })
        }
    }
}

impl Drop for TcpFd {
    fn drop(&mut self) {
        let _ = sys::close(self.fd);
    }
}

impl Transport for TcpFd {
    fn send_all(&mut self, data: &[u8]) -> Result<(), ()> {
        sys::write_all(self.fd, data).map_err(|_| ())
    }

    fn recv_some(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        let n = sys::read_timeout(self.fd, buf, READ_CHUNK_MS);
        if n == -(soso_abi::EAGAIN as i64) {
            return Ok(0);
        }
        if n < 0 {
            return Err(());
        }
        if n == 0 {
            return Err(());
        }
        Ok(n as usize)
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
