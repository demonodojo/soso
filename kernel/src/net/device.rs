//! Adaptadores PHY → trait `Device` de smoltcp (virtio-net, e1000e o rtl8169).

#[cfg(feature = "drv-e1000e")]
use crate::drivers::e1000e;
#[cfg(feature = "drv-rtl8169")]
use crate::drivers::rtl8169;
#[cfg(feature = "drv-virtio-net")]
use crate::drivers::virtio_net::{BUF_LEN, NET};
use smoltcp::phy::{Checksum, Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;
#[cfg(feature = "drv-virtio-net")]
use virtio_drivers::device::net::RxBuffer;

#[cfg(feature = "drv-virtio-net")]
pub struct SmolDev;

#[cfg(feature = "drv-virtio-net")]
pub struct SmolRx(RxBuffer);
#[cfg(feature = "drv-virtio-net")]
pub struct SmolTx;

#[cfg(feature = "drv-virtio-net")]
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

#[cfg(feature = "drv-virtio-net")]
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

#[cfg(feature = "drv-virtio-net")]
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

// ---- lx-wifi (Intel AX211 vía lxdde) ----

#[cfg(feature = "lxdde")]
pub struct LxWifiDev;

#[cfg(feature = "lxdde")]
pub struct LxWifiRx {
    buf: [u8; 2048],
    len: usize,
}

#[cfg(feature = "lxdde")]
pub struct LxWifiTx;

#[cfg(feature = "lxdde")]
impl Device for LxWifiDev {
    type RxToken<'a> = LxWifiRx;
    type TxToken<'a> = LxWifiTx;

    fn receive(&mut self, _ts: Instant) -> Option<(LxWifiRx, LxWifiTx)> {
        crate::lxdde::poll();
        let mut buf = [0u8; 2048];
        let len = crate::lxdde::wifi_receive(&mut buf)?;
        Some((LxWifiRx { buf, len }, LxWifiTx))
    }

    fn transmit(&mut self, _ts: Instant) -> Option<LxWifiTx> {
        if crate::lxdde::wifi_can_send() {
            Some(LxWifiTx)
        } else {
            None
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        eth_caps()
    }
}

#[cfg(feature = "lxdde")]
impl RxToken for LxWifiRx {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.buf[..self.len])
    }
}

#[cfg(feature = "lxdde")]
impl TxToken for LxWifiTx {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = [0u8; 2048];
        let n = len.min(buf.len());
        let r = f(&mut buf[..n]);
        let _ = crate::lxdde::wifi_send(&buf[..n]);
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

#[cfg(feature = "drv-e1000e")]
pub struct E1000Dev;

#[cfg(feature = "drv-e1000e")]
pub struct E1000Rx {
    buf: [u8; 2048],
    len: usize,
}

#[cfg(feature = "drv-e1000e")]
pub struct E1000Tx;

#[cfg(feature = "drv-e1000e")]
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

#[cfg(feature = "drv-e1000e")]
impl RxToken for E1000Rx {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.buf[..self.len])
    }
}

#[cfg(feature = "drv-e1000e")]
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
///
/// `Ninguno` no es un hueco: la pila se monta igual sin NIC para que exista la
/// tabla TCP de usuario, y con ella el loopback de `127.0.0.1` (askd). Sin esto,
/// `tcp_listen` devolvía EIO en una máquina sin driver de red y `ask` no
/// levantaba nunca (placa real, 2026-08-31).
pub enum NicDev {
    Ninguno,
    #[cfg(feature = "drv-virtio-net")]
    Virtio(SmolDev),
    #[cfg(feature = "drv-e1000e")]
    E1000e(E1000Dev),
    #[cfg(feature = "drv-rtl8169")]
    Rtl8169(Rtl8169Dev),
    #[cfg(feature = "lxdde")]
    LxE1000e(LxE1000Dev),
    #[cfg(feature = "lxdde")]
    LxWifi(LxWifiDev),
}

impl NicDev {
    /// Fallback 10.0.2.x solo en NICs de QEMU (virtio/e1000e). El Realtek de placa
    /// espera DHCP de verdad, como el WiFi.
    pub fn fallback_slirp(&self) -> bool {
        match self {
            #[cfg(feature = "drv-virtio-net")]
            NicDev::Virtio(_) => true,
            #[cfg(feature = "drv-e1000e")]
            NicDev::E1000e(_) => true,
            #[cfg(feature = "lxdde")]
            NicDev::LxE1000e(_) => true,
            _ => false,
        }
    }
}

impl Device for NicDev {
    type RxToken<'a> = NicRx;
    type TxToken<'a> = NicTx;

    fn receive(&mut self, ts: Instant) -> Option<(NicRx, NicTx)> {
        match self {
            NicDev::Ninguno => None,
            #[cfg(feature = "drv-virtio-net")]
            NicDev::Virtio(d) => d.receive(ts).map(|(r, t)| (NicRx::Virtio(r), NicTx::Virtio(t))),
            #[cfg(feature = "drv-e1000e")]
            NicDev::E1000e(d) => d.receive(ts).map(|(r, t)| (NicRx::E1000e(r), NicTx::E1000e(t))),
            #[cfg(feature = "drv-rtl8169")]
            NicDev::Rtl8169(d) => d.receive(ts).map(|(r, t)| (NicRx::Rtl8169(r), NicTx::Rtl8169(t))),
            #[cfg(feature = "lxdde")]
            NicDev::LxE1000e(d) => d.receive(ts).map(|(r, t)| (NicRx::LxE1000e(r), NicTx::LxE1000e(t))),
            #[cfg(feature = "lxdde")]
            NicDev::LxWifi(d) => d.receive(ts).map(|(r, t)| (NicRx::LxWifi(r), NicTx::LxWifi(t))),
        }
    }

    fn transmit(&mut self, ts: Instant) -> Option<NicTx> {
        match self {
            NicDev::Ninguno => None,
            #[cfg(feature = "drv-virtio-net")]
            NicDev::Virtio(d) => d.transmit(ts).map(NicTx::Virtio),
            #[cfg(feature = "drv-e1000e")]
            NicDev::E1000e(d) => d.transmit(ts).map(NicTx::E1000e),
            #[cfg(feature = "drv-rtl8169")]
            NicDev::Rtl8169(d) => d.transmit(ts).map(NicTx::Rtl8169),
            #[cfg(feature = "lxdde")]
            NicDev::LxE1000e(d) => d.transmit(ts).map(NicTx::LxE1000e),
            #[cfg(feature = "lxdde")]
            NicDev::LxWifi(d) => d.transmit(ts).map(NicTx::LxWifi),
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        match self {
            NicDev::Ninguno => eth_caps(),
            #[cfg(feature = "drv-virtio-net")]
            NicDev::Virtio(_) => eth_caps(),
            #[cfg(feature = "drv-e1000e")]
            NicDev::E1000e(_) => e1000_caps(),
            #[cfg(feature = "drv-rtl8169")]
            NicDev::Rtl8169(_) => e1000_caps(),
            #[cfg(feature = "lxdde")]
            NicDev::LxE1000e(_) => e1000_caps(),
            #[cfg(feature = "lxdde")]
            NicDev::LxWifi(_) => eth_caps(),
        }
    }
}

pub enum NicRx {
    #[cfg(feature = "drv-virtio-net")]
    Virtio(SmolRx),
    #[cfg(feature = "drv-e1000e")]
    E1000e(E1000Rx),
    #[cfg(feature = "drv-rtl8169")]
    Rtl8169(Rtl8169Rx),
    #[cfg(feature = "lxdde")]
    LxE1000e(LxE1000Rx),
    #[cfg(feature = "lxdde")]
    LxWifi(LxWifiRx),
}

pub enum NicTx {
    #[cfg(feature = "drv-virtio-net")]
    Virtio(SmolTx),
    #[cfg(feature = "drv-e1000e")]
    E1000e(E1000Tx),
    #[cfg(feature = "drv-rtl8169")]
    Rtl8169(Rtl8169Tx),
    #[cfg(feature = "lxdde")]
    LxE1000e(LxE1000Tx),
    #[cfg(feature = "lxdde")]
    LxWifi(LxWifiTx),
}

impl RxToken for NicRx {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        match self {
            #[cfg(feature = "drv-virtio-net")]
            NicRx::Virtio(t) => t.consume(f),
            #[cfg(feature = "drv-e1000e")]
            NicRx::E1000e(t) => t.consume(f),
            #[cfg(feature = "drv-rtl8169")]
            NicRx::Rtl8169(t) => t.consume(f),
            #[cfg(feature = "lxdde")]
            NicRx::LxE1000e(t) => t.consume(f),
            #[cfg(feature = "lxdde")]
            NicRx::LxWifi(t) => t.consume(f),
        }
    }
}

impl TxToken for NicTx {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        match self {
            #[cfg(feature = "drv-virtio-net")]
            NicTx::Virtio(t) => t.consume(len, f),
            #[cfg(feature = "drv-e1000e")]
            NicTx::E1000e(t) => t.consume(len, f),
            #[cfg(feature = "drv-rtl8169")]
            NicTx::Rtl8169(t) => t.consume(len, f),
            #[cfg(feature = "lxdde")]
            NicTx::LxE1000e(t) => t.consume(len, f),
            #[cfg(feature = "lxdde")]
            NicTx::LxWifi(t) => t.consume(len, f),
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
    #[cfg(feature = "drv-virtio-net")]
    let _ = BUF_LEN;
    caps
}

fn e1000_caps() -> DeviceCapabilities {
    let mut caps = eth_caps();
    caps.max_burst_size = Some(16);
    caps
}

// ---- rtl8169 nativo (Linux r8169) ----

#[cfg(feature = "drv-rtl8169")]
pub struct Rtl8169Dev;

#[cfg(feature = "drv-rtl8169")]
pub struct Rtl8169Rx {
    buf: [u8; 2048],
    len: usize,
}

#[cfg(feature = "drv-rtl8169")]
pub struct Rtl8169Tx;

#[cfg(feature = "drv-rtl8169")]
impl Device for Rtl8169Dev {
    type RxToken<'a> = Rtl8169Rx;
    type TxToken<'a> = Rtl8169Tx;

    fn receive(&mut self, _ts: Instant) -> Option<(Rtl8169Rx, Rtl8169Tx)> {
        let mut buf = [0u8; 2048];
        let len = rtl8169::receive(&mut buf)?;
        Some((Rtl8169Rx { buf, len }, Rtl8169Tx))
    }

    fn transmit(&mut self, _ts: Instant) -> Option<Rtl8169Tx> {
        if rtl8169::can_send() {
            Some(Rtl8169Tx)
        } else {
            None
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        e1000_caps()
    }
}

#[cfg(feature = "drv-rtl8169")]
impl RxToken for Rtl8169Rx {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.buf[..self.len])
    }
}

#[cfg(feature = "drv-rtl8169")]
impl TxToken for Rtl8169Tx {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = [0u8; 2048];
        let n = len.min(buf.len());
        let r = f(&mut buf[..n]);
        let _ = rtl8169::send(&buf[..n]);
        r
    }
}
