//! Resolución DNS A (IPv4) vía UDP/53.
//!
//! Los servidores salen del **DHCP**; 8.8.8.8 y el de QEMU (10.0.2.3) quedan
//! sólo como respaldo para cuando el lease no anuncia ninguno. Adivinarlos era
//! el primero de los dos motivos por los que esto no funcionaba fuera de QEMU.

use alloc::vec;
use alloc::vec::Vec;
use smoltcp::iface::{Interface, SocketSet};
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
/// Presupuesto por servidor: con varios en la lista, uno que no conteste no se
/// puede comer el tiempo de los demás.
const POR_SERVIDOR: Duration = Duration::from_millis(1500);

pub fn resolve_a(
    iface: &mut Interface,
    sockets: &mut SocketSet<'_>,
    dev: &mut crate::net::device::NicDev,
    host: &str,
    _now: Instant,
    deadline: Instant,
    servidores: &[Ipv4Address],
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

    for server in servidores {
        let remote = IpEndpoint::new(IpAddress::Ipv4(*server), DNS_PORT);
        {
            let sock = sockets.get_mut::<udp::Socket>(handle);
            sock.send_slice(&query, remote).ok();
        }
        // El reloj de verdad, no uno inventado. Antes esto avanzaba `t` de 10
        // en 10 ms sin mirar la hora: los «5 segundos» de espera se gastaban en
        // unos pocos milisegundos reales, así que en QEMU —donde la respuesta
        // viene del propio host— llegaba a tiempo y por WiFi no llegaba nunca.
        let fin = min_instant(crate::net::now() + POR_SERVIDOR, deadline);
        loop {
            let t = crate::net::now();
            if t >= fin {
                break;
            }
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
        }
    }
    sockets.remove(handle);
    Err(())
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
        dns,
        ..
    } = &mut *n;
    let lista = servidores(dns);
    let start = super::now();
    let deadline = start + TIMEOUT;
    let resultado = resolve_a(iface, sockets, dev, trimmed, start, deadline, &lista)
        .map_err(|_| -soso_abi::ENOENT);

    // El socket UDP y `query` ya se han destruido al volver de `resolve_a`.
    // Este recorrido separa su posible corrupción de la liberación siguiente.
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

use super::NetStack;

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
