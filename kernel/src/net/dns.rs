//! Resolución DNS A (IPv4) vía UDP/53. Usa el resolver de QEMU slirp
//! (10.0.2.3) o 8.8.8.8 como respaldo.

use alloc::vec;
use alloc::vec::Vec;
use smoltcp::iface::{Interface, SocketHandle, SocketSet};
use smoltcp::socket::udp;
use smoltcp::time::{Duration, Instant};
use smoltcp::wire::{IpAddress, IpEndpoint, Ipv4Address};

const DNS_PORT: u16 = 53;
const SLIRP_DNS: Ipv4Address = Ipv4Address::new(10, 0, 2, 3);
const FALLBACK_DNS: Ipv4Address = Ipv4Address::new(8, 8, 8, 8);
const TIMEOUT: Duration = Duration::from_secs(5);

fn encode_qname(host: &str, out: &mut Vec<u8>) -> Result<(), ()> {
    if host.is_empty() || host.len() > 253 {
        return Err(());
    }
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(());
        }
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    Ok(())
}

fn build_query(host: &str, id: u16) -> Result<Vec<u8>, ()> {
    let mut q = Vec::new();
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00]); // standard query, recursion desired
    q.extend_from_slice(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // 1 question
    encode_qname(host, &mut q)?;
    q.extend_from_slice(&[0x00, 0x01]); // type A
    q.extend_from_slice(&[0x00, 0x01]); // class IN
    Ok(q)
}

fn parse_a_record(pkt: &[u8]) -> Result<[u8; 4], ()> {
    if pkt.len() < 12 {
        return Err(());
    }
    let qd = u16::from_be_bytes([pkt[4], pkt[5]]) as usize;
    let an = u16::from_be_bytes([pkt[6], pkt[7]]) as usize;
    if qd != 1 || an == 0 {
        return Err(());
    }
    let mut i = 12usize;
    while i < pkt.len() {
        let len = pkt[i] as usize;
        if len == 0 {
            i += 1;
            break;
        }
        if len >= 0xC0 {
            i += 2;
            break;
        }
        i += 1 + len;
    }
    i += 4; // QTYPE + QCLASS
    for _ in 0..an {
        if i >= pkt.len() {
            break;
        }
        if pkt[i] >= 0xC0 {
            i += 2;
        } else {
            while i < pkt.len() && pkt[i] != 0 {
                i += 1 + pkt[i] as usize;
            }
            i += 1;
        }
        if i + 10 > pkt.len() {
            return Err(());
        }
        let ty = u16::from_be_bytes([pkt[i], pkt[i + 1]]);
        let class = u16::from_be_bytes([pkt[i + 2], pkt[i + 3]]);
        let rdlen = u16::from_be_bytes([pkt[i + 8], pkt[i + 9]]) as usize;
        i += 10;
        if i + rdlen > pkt.len() {
            return Err(());
        }
        if ty == 1 && class == 1 && rdlen == 4 {
            return Ok([pkt[i], pkt[i + 1], pkt[i + 2], pkt[i + 3]]);
        }
        i += rdlen;
    }
    Err(())
}

/// Resuelve `host` a IPv4. Requiere interfaz configurada y bloquea hasta
/// timeout.
pub fn resolve_a(
    iface: &mut Interface,
    sockets: &mut SocketSet<'_>,
    dev: &mut crate::net::device::NicDev,
    host: &str,
    now: Instant,
    deadline: Instant,
) -> Result<[u8; 4], ()> {
    let id = (crate::arch::pit::uptime_ms() as u16).max(1);
    let query = build_query(host, id)?;
    let rx = udp::PacketBuffer::new([udp::PacketMetadata::EMPTY; 2], vec![0u8; 768]);
    let tx = udp::PacketBuffer::new([udp::PacketMetadata::EMPTY; 2], vec![0u8; 512]);
    let handle = sockets.add(udp::Socket::new(rx, tx));
    {
        let sock = sockets.get_mut::<udp::Socket>(handle);
        sock.bind(49152).map_err(|_| ())?;
    }

    let servers = [SLIRP_DNS, FALLBACK_DNS];
    for server in servers {
        let remote = IpEndpoint::new(IpAddress::Ipv4(server), DNS_PORT);
        {
            let sock = sockets.get_mut::<udp::Socket>(handle);
            sock.send_slice(&query, remote).ok();
        }
        let mut t = now;
        while t < deadline {
            iface.poll(t, dev, sockets);
            let sock = sockets.get_mut::<udp::Socket>(handle);
            if sock.can_recv() {
                let mut buf = [0u8; 512];
                if let Ok((n, meta)) = sock.recv_slice(&mut buf) {
                    if meta.endpoint.port == DNS_PORT && n > 0 {
                        if let Ok(ip) = parse_a_record(&buf[..n]) {
                            sockets.remove(handle);
                            return Ok(ip);
                        }
                    }
                }
            }
            t += Duration::from_millis(10);
        }
    }
    sockets.remove(handle);
    Err(())
}

pub fn resolve_hostname(host: &str) -> Result<soso_abi::SockAddr, i64> {
    let trimmed = host.trim_end_matches('.');
    if trimmed.is_empty() || trimmed.len() > 253 {
        return Err(-soso_abi::EINVAL);
    }
    if let Some(ip) = parse_dotted_ipv4(trimmed) {
        return Ok(soso_abi::SockAddr {
            addr: ip,
            port: 0,
            _pad: 0,
        });
    }
    let net = super::NET.get().ok_or(-soso_abi::EIO)?;
    let mut n = net.lock();
    let configured = n.configured;
    if !configured {
        return Err(-soso_abi::ENOTCONN);
    }
    let NetStack {
        iface,
        sockets,
        dev,
        ..
    } = &mut *n;
    let start = super::now();
    let deadline = start + TIMEOUT;
    let ip = resolve_a(iface, sockets, dev, trimmed, start, deadline).map_err(|_| -soso_abi::ENOENT)?;
    Ok(soso_abi::SockAddr {
        addr: ip,
        port: 0,
        _pad: 0,
    })
}

use super::{NetStack, NET};

fn parse_dotted_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut oct = [0u8; 4];
    for (i, part) in s.split('.').enumerate() {
        if i >= 4 {
            return None;
        }
        oct[i] = part.parse().ok()?;
    }
    Some(oct)
}
