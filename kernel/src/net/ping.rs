//! ICMP Echo Request (un paquete). El binario `/bin/ping` y kshell lo llaman
//! en bucle.

use alloc::vec;
use smoltcp::phy::Device;
use smoltcp::socket::icmp;
use smoltcp::wire::{Icmpv4Packet, Icmpv4Repr, IpAddress, Ipv4Address};

use super::{now, NetStack, NET};

const PAYLOAD: &[u8] = b"soso-ping";
const DEFAULT_MS: u64 = 1000;
const MAX_MS: u64 = 10_000;

/// Envía un eco a `addr` y espera la respuesta. Devuelve el RTT en ms.
pub fn ping(addr: [u8; 4], timeout_ms: u64) -> Result<u64, i64> {
    if addr == [0, 0, 0, 0] || addr == [255, 255, 255, 255] {
        return Err(-soso_abi::EINVAL);
    }
    if addr[0] == 127 {
        return Ok(0);
    }

    let info = super::info();
    if info.flags & soso_abi::NET_FLAG_CONFIGURED != 0 && info.addr == addr {
        return Ok(0);
    }
    if info.flags & soso_abi::NET_FLAG_CONFIGURED == 0 {
        return Err(-soso_abi::ENOTCONN);
    }

    let timeout_ms = if timeout_ms == 0 {
        DEFAULT_MS
    } else {
        timeout_ms.min(MAX_MS)
    };

    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let mut n = net.lock();
    let NetStack {
        iface,
        sockets,
        dev,
        ..
    } = &mut *n;

    let checksum = dev.capabilities().checksum;
    let rx = icmp::PacketBuffer::new(vec![icmp::PacketMetadata::EMPTY], vec![0u8; 256]);
    let tx = icmp::PacketBuffer::new(vec![icmp::PacketMetadata::EMPTY], vec![0u8; 256]);
    let handle = sockets.add(icmp::Socket::new(rx, tx));
    let ident = (crate::arch::pit::uptime_ms() as u16) | 1;
    let dest = IpAddress::Ipv4(Ipv4Address::new(addr[0], addr[1], addr[2], addr[3]));
    let repr = Icmpv4Repr::EchoRequest {
        ident,
        seq_no: 1,
        data: PAYLOAD,
    };

    {
        let sock = sockets.get_mut::<icmp::Socket>(handle);
        if sock.bind(icmp::Endpoint::Ident(ident)).is_err() {
            sockets.remove(handle);
            return Err(-soso_abi::EIO);
        }
        match sock.send(repr.buffer_len(), dest) {
            Ok(buf) => {
                let mut pkt = Icmpv4Packet::new_unchecked(buf);
                repr.emit(&mut pkt, &checksum);
            }
            Err(_) => {
                sockets.remove(handle);
                return Err(-soso_abi::EAGAIN);
            }
        }
    }

    let start_ms = crate::arch::pit::uptime_ms();
    let deadline_ms = start_ms.saturating_add(timeout_ms);
    loop {
        iface.poll(now(), dev, sockets);
        {
            let sock = sockets.get_mut::<icmp::Socket>(handle);
            if sock.can_recv() {
                if let Ok((data, _)) = sock.recv() {
                    if let Ok(pkt) = Icmpv4Packet::new_checked(data) {
                        if let Ok(Icmpv4Repr::EchoReply { ident: id, seq_no, .. }) =
                            Icmpv4Repr::parse(&pkt, &checksum)
                        {
                            if id == ident && seq_no == 1 {
                                let rtt = crate::arch::pit::uptime_ms().saturating_sub(start_ms);
                                sockets.remove(handle);
                                return Ok(rtt);
                            }
                        }
                    }
                }
            }
        }
        if crate::arch::pit::uptime_ms() >= deadline_ms {
            break;
        }
        // Evita un spin a tope: smoltcp no duerme, pero 10 ms sueltan el
        // bus a otras IRQ. El candado de NET se mantiene a propósito: si
        // lo soltamos, `poll()` del ocioso puede vaciar el socket.
        let hasta = crate::arch::pit::uptime_ms().saturating_add(10);
        while crate::arch::pit::uptime_ms() < hasta {
            core::hint::spin_loop();
        }
    }

    sockets.remove(handle);
    Err(-soso_abi::ETIMEDOUT)
}
