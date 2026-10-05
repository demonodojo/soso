//! GET HTTPS de prueba sobre `std::net` (T42): `cargo run --example get --features std
//! [--no-default-features --features crypto-rust] -- <url>...`
//!
//! Sirve para comprobar un proveedor criptográfico de rustls contra servidores
//! reales (TLS 1.2 y 1.3, cadenas RSA y ECDSA) sin arrancar soso.

use soso_abi::SockAddr;
use soso_http::{https_get_full, set_wall_clock, TcpTransport};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::Duration;

struct Std(Mutex<Vec<TcpStream>>);

impl TcpTransport for Std {
    fn dns_resolve(&self, host: &str, out: &mut [u8; 4]) -> Result<(), i64> {
        let a = (host, 443).to_socket_addrs().map_err(|_| -1)?;
        for s in a {
            if let SocketAddr::V4(v4) = s {
                *out = v4.ip().octets();
                return Ok(());
            }
        }
        Err(-1)
    }
    fn tcp_connect(&self, addr: SockAddr, timeout_ms: u64) -> Result<u64, i64> {
        let sa = SocketAddr::from((Ipv4Addr::from(addr.addr), addr.port));
        let s = TcpStream::connect_timeout(&sa, Duration::from_millis(timeout_ms)).map_err(|_| -1)?;
        let mut v = self.0.lock().unwrap();
        v.push(s);
        Ok(v.len() as u64 - 1)
    }
    fn read_timeout(&self, fd: u64, buf: &mut [u8], timeout_ms: u64) -> i64 {
        let mut v = self.0.lock().unwrap();
        let s = &mut v[fd as usize];
        let _ = s.set_read_timeout(Some(Duration::from_millis(timeout_ms.max(1))));
        match s.read(buf) {
            Ok(n) => n as i64,
            Err(_) => -1,
        }
    }
    fn write_all(&self, fd: u64, data: &[u8]) -> Result<(), i64> {
        self.0.lock().unwrap()[fd as usize].write_all(data).map_err(|_| -1)
    }
    fn close(&self, _fd: u64) {}
}

fn main() {
    set_wall_clock(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs())
    });
    let t = Std(Mutex::new(Vec::new()));
    for url in std::env::args().skip(1) {
        match https_get_full(&t, &url, None) {
            Ok(r) => println!("{url}: HTTP {} ({} bytes)", r.status, r.body.len()),
            Err(e) => println!("{url}: ERROR {e:?}"),
        }
    }
}
