//! Sockets TCP de userspace sobre smoltcp.

use crate::arch::pit;
use alloc::vec;
use alloc::vec::Vec;
use smoltcp::iface::{Interface, SocketHandle};
use smoltcp::socket::tcp;
use smoltcp::wire::{IpAddress, IpEndpoint, IpListenEndpoint, Ipv4Address};

pub const MAX_USER_TCP: usize = 8;
const TCP_BUF: usize = 65536;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TcpRole {
    Listening,
    Connecting,
    Connected,
}

pub struct UserTcp {
    pub handle: SocketHandle,
    pub role: TcpRole,
    pub port: u16,
    pub remote: Option<IpEndpoint>,
    pub closed: bool,
    pub connect_started: bool,
    /// Conexión 127.0.0.1 (sin smoltcp).
    pub loopback: bool,
    pub loop_pair: Option<usize>,
    pub loop_side: super::loopback::LoopSide,
    /// Listener loopback: no pasa a Connected al aceptar.
    pub loop_listener: bool,
}

pub struct TcpTable {
    pub entries: Vec<Option<UserTcp>>,
}

impl TcpTable {
    pub fn new() -> Self {
        Self {
            entries: (0..MAX_USER_TCP).map(|_| None).collect(),
        }
    }

    pub fn alloc(
        &mut self,
        sockets: &mut smoltcp::iface::SocketSet<'static>,
        role: TcpRole,
        port: u16,
        remote: Option<IpEndpoint>,
    ) -> Result<usize, ()> {
        let slot = self.entries.iter().position(|e| e.is_none()).ok_or(())?;
        let handle = sockets.add(tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0; TCP_BUF]),
            tcp::SocketBuffer::new(vec![0; TCP_BUF]),
        ));
        self.entries[slot] = Some(UserTcp {
            handle,
            role,
            port,
            remote,
            closed: false,
            connect_started: false,
            loopback: false,
            loop_pair: None,
            loop_side: super::loopback::LoopSide::Client,
            loop_listener: false,
        });
        Ok(slot)
    }

    /// Slot loopback sin socket smoltcp (127.0.0.1).
    pub fn alloc_loopback(
        &mut self,
        sockets: &mut smoltcp::iface::SocketSet<'static>,
        role: TcpRole,
        pair_id: usize,
        side: super::loopback::LoopSide,
    ) -> Result<usize, ()> {
        let slot = self.entries.iter().position(|e| e.is_none()).ok_or(())?;
        let handle = sockets.add(tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0; TCP_BUF]),
            tcp::SocketBuffer::new(vec![0; TCP_BUF]),
        ));
        self.entries[slot] = Some(UserTcp {
            handle,
            role,
            port: 0,
            remote: None,
            closed: false,
            connect_started: false,
            loopback: true,
            loop_pair: Some(pair_id),
            loop_side: side,
            loop_listener: false,
        });
        Ok(slot)
    }

    pub fn free(&mut self, sockets: &mut smoltcp::iface::SocketSet<'static>, slot: usize) {
        if let Some(entry) = self.entries[slot].take() {
            if entry.loopback {
                if let (Some(pair), side) = (entry.loop_pair, entry.loop_side) {
                    super::loopback::close_side(pair, side);
                }
            } else {
                let s = sockets.get_mut::<tcp::Socket>(entry.handle);
                if s.is_open() {
                    s.close();
                }
            }
            // `alloc_loopback` también mete un socket smoltcp (no se usa para
            // copiar, pero ocupa 128 KiB). Sin remove, cada `ask` fugaba uno y
            // a la octava conexión `tcp_listen`/`connect` morían con EMFILE.
            sockets.remove(entry.handle);
        }
    }

    /// Slot en escucha sobre `port`, si existe.
    pub fn find_listener(&self, port: u16) -> Option<usize> {
        self.entries.iter().enumerate().find_map(|(i, e)| {
            let e = e.as_ref()?;
            if e.role == TcpRole::Listening && e.port == port && !e.closed {
                Some(i)
            } else {
                None
            }
        })
    }
}

pub fn listen_start(
    sockets: &mut smoltcp::iface::SocketSet<'static>,
    entry: &mut UserTcp,
) -> Result<(), ()> {
    let s = sockets.get_mut::<tcp::Socket>(entry.handle);
    if !s.is_open() {
        s.listen(entry.port).map_err(|_| ())?;
    }
    Ok(())
}

pub fn is_established(sockets: &smoltcp::iface::SocketSet<'static>, handle: SocketHandle) -> bool {
    let s = sockets.get::<tcp::Socket>(handle);
    s.state() == tcp::State::Established
}

pub fn try_read_loopback(
    entry: &UserTcp,
    buf: u64,
    len: u64,
) -> Result<u64, i64> {
    if entry.role != TcpRole::Connected {
        return Err(-soso_abi::ENOTCONN);
    }
    let pair = entry.loop_pair.ok_or(-soso_abi::ENOTCONN)?;
    super::loopback::try_read(pair, entry.loop_side, buf, len)
}

pub fn try_write_loopback(
    entry: &UserTcp,
    buf: u64,
    len: u64,
) -> Result<u64, i64> {
    if entry.role != TcpRole::Connected {
        return Err(-soso_abi::ENOTCONN);
    }
    let pair = entry.loop_pair.ok_or(-soso_abi::ENOTCONN)?;
    super::loopback::try_write(pair, entry.loop_side, buf, len)
}

pub fn try_read(
    sockets: &mut smoltcp::iface::SocketSet<'static>,
    handle: SocketHandle,
    role: TcpRole,
    buf: u64,
    len: u64,
) -> Result<u64, i64> {
    if role != TcpRole::Connected {
        return Err(-soso_abi::ENOTCONN);
    }
    let s = sockets.get_mut::<tcp::Socket>(handle);
    if !s.can_recv() {
        if matches!(s.state(), tcp::State::CloseWait | tcp::State::Closed) {
            return Ok(0);
        }
        return Ok(0);
    }
    let dst_len = len as usize;
    if dst_len == 0 {
        return Ok(0);
    }
    let mut tmp = [0u8; 4096];
    let chunk = dst_len.min(tmp.len());
    let n = s.recv_slice(&mut tmp[..chunk]).unwrap_or(0);
    if n == 0 {
        return Ok(0);
    }
    for i in 0..n {
        unsafe {
            *((buf + i as u64) as *mut u8) = tmp[i];
        }
    }
    Ok(n as u64)
}

pub fn try_write(
    sockets: &mut smoltcp::iface::SocketSet<'static>,
    handle: SocketHandle,
    role: TcpRole,
    buf: u64,
    len: u64,
) -> Result<u64, i64> {
    if role != TcpRole::Connected {
        return Err(-soso_abi::ENOTCONN);
    }
    let s = sockets.get_mut::<tcp::Socket>(handle);
    if !s.can_send() {
        return Ok(0);
    }
    let src_len = len as usize;
    if src_len == 0 {
        return Ok(0);
    }
    let mut tmp = [0u8; 4096];
    let chunk = src_len.min(tmp.len());
    for i in 0..chunk {
        tmp[i] = unsafe { *((buf + i as u64) as *const u8) };
    }
    let n = s.send_slice(&tmp[..chunk]).unwrap_or(0);
    Ok(n as u64)
}

pub fn poll_entry(
    iface: &mut Interface,
    sockets: &mut smoltcp::iface::SocketSet<'static>,
    slot: usize,
    entry: &mut UserTcp,
    configured: bool,
) {
    if entry.closed || entry.loopback {
        return;
    }
    if !configured {
        return;
    }
    match entry.role {
        TcpRole::Listening => {
            let s = sockets.get_mut::<tcp::Socket>(entry.handle);
            if !s.is_open() {
                let _ = s.listen(entry.port);
            } else if s.state() == tcp::State::Established {
                entry.role = TcpRole::Connected;
            }
        }
        TcpRole::Connecting => {
            if !entry.connect_started {
                if let Some(remote) = entry.remote {
                    let local_port = 49152u16.wrapping_add(slot as u16);
                    let local_ip = iface
                        .ip_addrs()
                        .iter()
                        .find_map(|cidr| match cidr.address() {
                            IpAddress::Ipv4(v4) => Some(v4),
                        })
                        .unwrap_or(Ipv4Address::UNSPECIFIED);
                    let local = IpListenEndpoint::from((
                        IpAddress::Ipv4(local_ip),
                        local_port,
                    ));
                    let s = sockets.get_mut::<tcp::Socket>(entry.handle);
                    let cx = iface.context();
                    if s.connect(cx, remote, local).is_ok() {
                        entry.connect_started = true;
                    }
                }
            } else {
                let s = sockets.get::<tcp::Socket>(entry.handle);
                if s.state() == tcp::State::Established {
                    entry.role = TcpRole::Connected;
                } else if matches!(s.state(), tcp::State::Closed | tcp::State::TimeWait) {
                    entry.closed = true;
                }
            }
        }
        TcpRole::Connected => {
            let s = sockets.get_mut::<tcp::Socket>(entry.handle);
            if s.state() == tcp::State::CloseWait && !s.can_recv() {
                s.close();
            }
            if matches!(s.state(), tcp::State::Closed | tcp::State::TimeWait) {
                entry.closed = true;
            }
        }
    }
}

pub fn endpoint_from_abi(addr: &soso_abi::SockAddr) -> IpEndpoint {
    IpEndpoint::new(
        IpAddress::Ipv4(Ipv4Address::new(
            addr.addr[0],
            addr.addr[1],
            addr.addr[2],
            addr.addr[3],
        )),
        addr.port,
    )
}

pub fn fallback_addr_from_mac(mac: [u8; 6]) -> Ipv4Address {
    let last = mac[5];
    if last == 0x15 {
        Ipv4Address::new(10, 0, 2, 15)
    } else {
        Ipv4Address::new(10, 0, 2, last)
    }
}

#[allow(dead_code)]
pub fn uptime_ms() -> u64 {
    pit::uptime_ms()
}
