//! Resolución DNS A (IPv4) vía UDP/53.
//!
//! Los servidores salen del **DHCP**; 8.8.8.8 y el de QEMU (10.0.2.3) quedan
//! sólo como respaldo para cuando el lease no anuncia ninguno. Adivinarlos era
//! el primero de los dos motivos por los que esto no funcionaba fuera de QEMU.

use alloc::vec;
use alloc::vec::Vec;
use smoltcp::socket::udp;
use smoltcp::time::{Duration, Instant};
use smoltcp::wire::{IpAddress, IpEndpoint, Ipv4Address};

const DNS_PORT: u16 = 53;
const SLIRP_DNS: Ipv4Address = Ipv4Address::new(10, 0, 2, 3);
const FALLBACK_DNS: Ipv4Address = Ipv4Address::new(8, 8, 8, 8);
const TIMEOUT: Duration = Duration::from_secs(5);
const RCODE_NXDOMAIN: u8 = 3;

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

fn dns_id(pkt: &[u8]) -> Option<u16> {
    if pkt.len() < 2 {
        return None;
    }
    Some(u16::from_be_bytes([pkt[0], pkt[1]]))
}

fn dns_rcode(pkt: &[u8]) -> Option<u8> {
    if pkt.len() < 4 {
        return None;
    }
    Some(pkt[3] & 0x0f)
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
/// Presupuesto por servidor: con varios en la lista, uno que no conteste no se
/// puede comer el tiempo de los demás.
const POR_SERVIDOR: Duration = Duration::from_millis(1500);

#[derive(Copy, Clone, Eq, PartialEq)]
enum ResolveFail {
    Timeout,
    Interrupted,
    NxDomain,
}

pub fn resolve_a(
    host: &str,
    deadline: Instant,
    servidores: &[Ipv4Address],
) -> Result<[u8; 4], ResolveFail> {
    let net = super::NET.get().ok_or(ResolveFail::Timeout)?;
    let id = (crate::arch::pit::uptime_ms() as u16).max(1);
    let query = build_query(host, id).map_err(|_| ResolveFail::Timeout)?;
    let handle = {
        let mut n = net.lock();
        let rx = udp::PacketBuffer::new([udp::PacketMetadata::EMPTY; 2], vec![0u8; 768]);
        let tx = udp::PacketBuffer::new([udp::PacketMetadata::EMPTY; 2], vec![0u8; 512]);
        let handle = n.sockets.add(udp::Socket::new(rx, tx));
        let sock = n.sockets.get_mut::<udp::Socket>(handle);
        if sock.bind(49152).is_err() {
            crate::println!("dns: bind puerto 49152 falló (host={host})");
            n.sockets.remove(handle);
            return Err(ResolveFail::Timeout);
        }
        handle
    };

    for server in servidores {
        let remote = IpEndpoint::new(IpAddress::Ipv4(*server), DNS_PORT);
        {
            let mut n = net.lock();
            let sock = n.sockets.get_mut::<udp::Socket>(handle);
            if sock.send_slice(&query, remote).is_err() {
                crate::println!("dns: send falló hacia {server} host={host}");
            } else {
                crate::println!("dns: consulta {server} host={host} id={id}");
            }
        }
        let fin = min_instant(crate::net::now() + POR_SERVIDOR, deadline);
        loop {
            if crate::task::interrupt_requested() {
                let mut n = net.lock();
                n.sockets.remove(handle);
                return Err(ResolveFail::Interrupted);
            }
            let t = crate::net::now();
            if t >= fin {
                crate::println!("dns: sin respuesta de {server} host={host} (plazo servidor)");
                break;
            }
            super::poll();
            let mut n = net.lock();
            let sock = n.sockets.get_mut::<udp::Socket>(handle);
            if sock.can_recv() {
                let mut buf = [0u8; 512];
                if let Ok((nbytes, meta)) = sock.recv_slice(&mut buf) {
                    if meta.endpoint.port == DNS_PORT && nbytes > 0 {
                        let pkt = &buf[..nbytes];
                        let id_ok = dns_id(pkt) == Some(id);
                        let rcode = dns_rcode(pkt).unwrap_or(0xff);
                        let a_ok = id_ok && parse_a_record(pkt).is_ok();
                        crate::println!(
                            "dns: resp {server} host={host} {nbytes}B id={} rcode={rcode} registro_a={}",
                            if id_ok { "ok" } else { "distinto" },
                            if a_ok { "ok" } else { "no" }
                        );
                        if id_ok && rcode == RCODE_NXDOMAIN {
                            n.sockets.remove(handle);
                            return Err(ResolveFail::NxDomain);
                        }
                        if let Ok(ip) = parse_a_record(pkt) {
                            if id_ok {
                                n.sockets.remove(handle);
                                return Ok(ip);
                            }
                        }
                    }
                }
            }
        }
    }
    {
        let mut n = net.lock();
        n.sockets.remove(handle);
    }
    Err(ResolveFail::Timeout)
}

fn min_instant(a: Instant, b: Instant) -> Instant {
    if a < b { a } else { b }
}

/// Servidores a los que preguntar: primero los que anunció el DHCP.
///
/// Esto **sí** reserva, y puede: corre en el contexto de la syscall que
/// resuelve un nombre, no en una interrupción.
pub fn servidores(dhcp: &[Option<Ipv4Address>]) -> alloc::vec::Vec<Ipv4Address> {
    let mut v: alloc::vec::Vec<Ipv4Address> = dhcp.iter().flatten().copied().collect();
    for respaldo in [FALLBACK_DNS, SLIRP_DNS] {
        if !v.contains(&respaldo) {
            v.push(respaldo);
        }
    }
    v
}

pub fn resolve_hostname(host: &str) -> Result<soso_abi::SockAddr, i64> {
    crate::mm::heap::vigilar();
    crate::mm::heap::punto("dns");
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
    let (configured, lista) = {
        let n = net.lock();
        (n.configured, servidores(&n.dns))
    };
    if !configured {
        return Err(-soso_abi::ENOTCONN);
    }
    let start = super::now();
    let deadline = start + TIMEOUT;
    let resultado = match resolve_a(trimmed, deadline, &lista) {
        Ok(ip) => Ok(ip),
        Err(ResolveFail::Interrupted) => Err(-soso_abi::EINTR),
        Err(ResolveFail::NxDomain) => Err(-soso_abi::ENOENT),
        Err(ResolveFail::Timeout) => Err(-soso_abi::ETIMEDOUT),
    };

    crate::mm::heap::comprobar_listas("tras resolve_a");

    crate::println!(
        "heap: liberando servidores DNS ptr={:p} len={} cap={}",
        lista.as_ptr(),
        lista.len(),
        lista.capacity()
    );
    drop(lista);
    crate::mm::heap::comprobar_listas("tras liberar servidores DNS");

    let ip = resultado?;
    Ok(soso_abi::SockAddr {
        addr: ip,
        port: 0,
        _pad: 0,
    })
}

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
