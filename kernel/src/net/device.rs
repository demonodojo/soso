//! Adaptador virtio-net → trait `Device` de smoltcp.
//!
//! Los tokens acceden a la NIC por el Mutex estático del driver: smoltcp
//! entrega los tokens rx y tx a la vez y no se puede repartir un &mut.

use crate::drivers::virtio_net::{BUF_LEN, NET};
use smoltcp::phy::{Checksum, Device, DeviceCapabilities, Medium};
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
        if nic.can_send() { Some(SmolTx) } else { None }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1514;
        caps.max_burst_size = Some(1);
        // La NIC virtual no calcula checksums por nosotros.
        caps.checksum.ipv4 = Checksum::Both;
        caps.checksum.tcp = Checksum::Both;
        caps
    }
}

impl smoltcp::phy::RxToken for SmolRx {
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

impl smoltcp::phy::TxToken for SmolTx {
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
