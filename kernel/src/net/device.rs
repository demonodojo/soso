//! Adaptadores PHY → trait `Device` de smoltcp (virtio-net o e1000e).

use crate::drivers::{e1000e, virtio_net};
use crate::drivers::virtio_net::{BUF_LEN, NET};
use smoltcp::phy::{Checksum, Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;
use virtio_drivers::device::net::RxBuffer;

pub struct SmolDev;

pub struct SmolRx(RxBuffer);
pub struct SmolTx;

impl Device for SmolDev {
    type RxToken<'a> = SmolRx;
    type TxToken<'a> = SmolTx;

    fn receive(&mut self, _ts: Instant) -> Option<(SmolRx, SmolTx)> {
        let mut nic = NET.get()?.lock();
        match nic.receive() {
            Ok(buf) => Some((SmolRx(buf), SmolTx)),
            Err(_) => None,
        }
    }

    fn transmit(&mut self, _ts: Instant) -> Option<SmolTx> {
        let nic = NET.get()?.lock();
        if nic.can_send() {
            Some(SmolTx)
        } else {
            None
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        eth_caps()
    }
}

impl RxToken for SmolRx {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        let r = f(self.0.packet_mut());
        if let Some(nic) = NET.get() {
            nic.lock().recycle_rx_buffer(self.0).ok();
        }
        r
    }
}

impl TxToken for SmolTx {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut nic = NET.get().expect("tx sin nic").lock();
        let mut tx = nic.new_tx_buffer(len.min(BUF_LEN));
        let r = f(tx.packet_mut());
        nic.send(tx).ok();
        r
    }
}

// ---- lx-e1000e (driver Linux vía lxdde) ----

#[cfg(feature = "lxdde")]
pub struct LxE1000Dev;

#[cfg(feature = "lxdde")]
pub struct LxE1000Rx {
    buf: [u8; 2048],
    len: usize,
}

#[cfg(feature = "lxdde")]
pub struct LxE1000Tx;

#[cfg(feature = "lxdde")]
impl Device for LxE1000Dev {
    type RxToken<'a> = LxE1000Rx;
    type TxToken<'a> = LxE1000Tx;

    fn receive(&mut self, _ts: Instant) -> Option<(LxE1000Rx, LxE1000Tx)> {
        crate::lxdde::poll_rx();
        let mut buf = [0u8; 2048];
        let len = crate::lxdde::e1000e_receive(&mut buf)?;
        Some((LxE1000Rx { buf, len }, LxE1000Tx))
    }

    fn transmit(&mut self, _ts: Instant) -> Option<LxE1000Tx> {
        if crate::lxdde::e1000e_can_send() {
            Some(LxE1000Tx)
        } else {
            None
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        e1000_caps()
    }
}

#[cfg(feature = "lxdde")]
impl RxToken for LxE1000Rx {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.buf[..self.len])
    }
}

#[cfg(feature = "lxdde")]
impl TxToken for LxE1000Tx {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = [0u8; 2048];
        let n = len.min(buf.len());
        let r = f(&mut buf[..n]);
        let _ = crate::lxdde::e1000e_send(&buf[..n]);
        r
    }
}

// ---- e1000e nativo ----

pub struct E1000Dev;

pub struct E1000Rx {
    buf: [u8; 2048],
    len: usize,
}

pub struct E1000Tx;

impl Device for E1000Dev {
    type RxToken<'a> = E1000Rx;
    type TxToken<'a> = E1000Tx;

    fn receive(&mut self, _ts: Instant) -> Option<(E1000Rx, E1000Tx)> {
        let mut buf = [0u8; 2048];
        let len = e1000e::receive(&mut buf)?;
        Some((E1000Rx { buf, len }, E1000Tx))
    }

    fn transmit(&mut self, _ts: Instant) -> Option<E1000Tx> {
        if e1000e::can_send() {
            Some(E1000Tx)
        } else {
            None
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        e1000_caps()
    }
}

impl RxToken for E1000Rx {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.buf[..self.len])
    }
}

impl TxToken for E1000Tx {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = [0u8; 2048];
        let n = len.min(buf.len());
        let r = f(&mut buf[..n]);
        let _ = e1000e::send(&buf[..n]);
        r
    }
}

/// Backend de red elegido en `net::init`.
pub enum NicDev {
    Virtio(SmolDev),
    E1000e(E1000Dev),
    #[cfg(feature = "lxdde")]
    LxE1000e(LxE1000Dev),
}

impl Device for NicDev {
    type RxToken<'a> = NicRx;
    type TxToken<'a> = NicTx;

    fn receive(&mut self, ts: Instant) -> Option<(NicRx, NicTx)> {
        match self {
            NicDev::Virtio(d) => d.receive(ts).map(|(r, t)| (NicRx::Virtio(r), NicTx::Virtio(t))),
            NicDev::E1000e(d) => d.receive(ts).map(|(r, t)| (NicRx::E1000e(r), NicTx::E1000e(t))),
            #[cfg(feature = "lxdde")]
            NicDev::LxE1000e(d) => d.receive(ts).map(|(r, t)| (NicRx::LxE1000e(r), NicTx::LxE1000e(t))),
        }
    }

    fn transmit(&mut self, ts: Instant) -> Option<NicTx> {
        match self {
            NicDev::Virtio(d) => d.transmit(ts).map(NicTx::Virtio),
            NicDev::E1000e(d) => d.transmit(ts).map(NicTx::E1000e),
            #[cfg(feature = "lxdde")]
            NicDev::LxE1000e(d) => d.transmit(ts).map(NicTx::LxE1000e),
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        match self {
            NicDev::Virtio(_) => eth_caps(),
            NicDev::E1000e(_) => e1000_caps(),
            #[cfg(feature = "lxdde")]
            NicDev::LxE1000e(_) => e1000_caps(),
        }
    }
}

pub enum NicRx {
    Virtio(SmolRx),
    E1000e(E1000Rx),
    #[cfg(feature = "lxdde")]
    LxE1000e(LxE1000Rx),
}

pub enum NicTx {
    Virtio(SmolTx),
    E1000e(E1000Tx),
    #[cfg(feature = "lxdde")]
    LxE1000e(LxE1000Tx),
}

impl RxToken for NicRx {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        match self {
            NicRx::Virtio(t) => t.consume(f),
            NicRx::E1000e(t) => t.consume(f),
            #[cfg(feature = "lxdde")]
            NicRx::LxE1000e(t) => t.consume(f),
        }
    }
}

impl TxToken for NicTx {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        match self {
            NicTx::Virtio(t) => t.consume(len, f),
            NicTx::E1000e(t) => t.consume(len, f),
            #[cfg(feature = "lxdde")]
            NicTx::LxE1000e(t) => t.consume(len, f),
        }
    }
}

fn eth_caps() -> DeviceCapabilities {
    let mut caps = DeviceCapabilities::default();
    caps.medium = Medium::Ethernet;
    caps.max_transmission_unit = 1514;
    caps.max_burst_size = Some(1);
    caps.checksum.ipv4 = Checksum::Both;
    caps.checksum.tcp = Checksum::Both;
    let _ = virtio_net::BUF_LEN;
    caps
}

fn e1000_caps() -> DeviceCapabilities {
    let mut caps = eth_caps();
    caps.max_burst_size = Some(16);
    caps
}
