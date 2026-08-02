//! USB mass storage (BOT / SCSI READ(10)) sobre xHCI.

use crate::device::{
    alloc_dma_buffer, read_dma_buffer, EndpointDescriptor, ParsedConfiguration, UsbSpeed,
};
use crate::driver::XhciController;
use crate::registers::{PORTSC_CCS, PORTSC_PED};
use crate::ring::{Trb, TRB_COMPLETION_SHORT_PACKET, TRB_COMPLETION_SUCCESS};

const CBW_SIG: u32 = 0x4342_5355;
const CSW_SIG: u32 = 0x5342_5355;
const MS_CLASS: u8 = 0x08;
const MS_SUBCLASS_SCSI: u8 = 0x06;
const MS_PROTO_BOT: u8 = 0x50;

static TAG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(1);

/// Dispositivo mass-storage enumerado (bulk IN/OUT + capacidad).
#[derive(Clone, Copy, Debug)]
pub struct MassStorage {
    pub slot_id: u8,
    pub iface: u8,
    pub bulk_out_dci: u8,
    pub bulk_in_dci: u8,
    pub sectors: u64,
}

impl XhciController {
    /// Escanea puertos y configura el primer stick BOT encontrado.
    pub fn probe_mass_storage(&mut self) -> Option<MassStorage> {
        for port in 1..=self.max_ports() {
            let portsc = self.portsc(port);
            if portsc & PORTSC_CCS == 0 {
                continue;
            }
            if portsc & PORTSC_PED == 0 {
                self.reset_port(port);
            }
            let speed = self.port_speed(port);
            let slot = self.enable_slot()?;
            if let Some(ms) = self.setup_mass_storage(slot, port, speed) {
                return Some(ms);
            }
        }
        None
    }

    /// BOT READ(10) — un sector 512 B.
    pub fn read_sector10(&mut self, ms: &MassStorage, lba: u32, buf: &mut [u8; 512]) -> bool {
        self.read_sectors10(ms, lba, buf)
    }

    /// BOT READ(10) — `buf.len()/512` sectores consecutivos en **una** sola
    /// transacción CBW/datos/CSW.
    ///
    /// Antes sólo existía la variante de un sector, así que leer un bloque de
    /// 4 KiB del FS costaba **ocho** viajes de ida y vuelta por USB. La cuenta
    /// va en `cdb[7..9]` (big-endian) y los bytes esperados en `cbw[8..12]`;
    /// las dos estaban clavadas a 1 y a 512.
    pub fn read_sectors10(&mut self, ms: &MassStorage, lba: u32, buf: &mut [u8]) -> bool {
        if buf.is_empty() || buf.len() % 512 != 0 || buf.len() / 512 > u16::MAX as usize {
            return false;
        }
        let count = (buf.len() / 512) as u16;
        let tag = TAG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let mut cdb = [0u8; 16];
        cdb[0] = 0x28;
        cdb[2] = (lba >> 24) as u8;
        cdb[3] = (lba >> 16) as u8;
        cdb[4] = (lba >> 8) as u8;
        cdb[5] = lba as u8;
        cdb[7] = (count >> 8) as u8;
        cdb[8] = count as u8;

        let mut cbw = [0u8; 31];
        cbw[0..4].copy_from_slice(&CBW_SIG.to_le_bytes());
        cbw[4..8].copy_from_slice(&tag.to_le_bytes());
        cbw[8..12].copy_from_slice(&(buf.len() as u32).to_le_bytes());
        cbw[12] = 0x80;
        cbw[14] = 10;
        cbw[15..25].copy_from_slice(&cdb[..10]);

        if !self.bulk_out(ms.slot_id, ms.bulk_out_dci, &cbw) {
            return false;
        }
        if !self.bulk_in(ms.slot_id, ms.bulk_in_dci, buf) {
            return false;
        }
        let mut csw = [0u8; 13];
        if !self.bulk_in(ms.slot_id, ms.bulk_in_dci, &mut csw) {
            return false;
        }
        let sig = u32::from_le_bytes([csw[0], csw[1], csw[2], csw[3]]);
        let csw_tag = u32::from_le_bytes([csw[4], csw[5], csw[6], csw[7]]);
        sig == CSW_SIG && csw_tag == tag && csw[12] == 0
    }

    fn setup_mass_storage(&mut self, slot_id: u8, port: u8, speed: UsbSpeed) -> Option<MassStorage> {
        self.set_device(slot_id, port, speed);
        if !self.address_device(slot_id, port, speed) {
            return None;
        }
        let _dev_desc = self.get_device_descriptor(slot_id)?;
        let config = self.get_configuration_descriptor(slot_id, 0)?;
        let (iface, bulk_out, bulk_in) = config.find_mass_storage()?;
        if !self.set_configuration(slot_id, config.config.b_configuration_value, &config) {
            return None;
        }
        let sectors = self.read_capacity10(slot_id, bulk_out.dci(), bulk_in.dci())?;
        log::info!(
            "xhci: mass storage slot={} iface={} sectors={}",
            slot_id,
            iface,
            sectors
        );
        Some(MassStorage {
            slot_id,
            iface,
            bulk_out_dci: bulk_out.dci(),
            bulk_in_dci: bulk_in.dci(),
            sectors,
        })
    }

    fn read_capacity10(&mut self, slot_id: u8, bulk_out_dci: u8, bulk_in_dci: u8) -> Option<u64> {
        let tag = TAG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let mut cbw = [0u8; 31];
        cbw[0..4].copy_from_slice(&CBW_SIG.to_le_bytes());
        cbw[4..8].copy_from_slice(&tag.to_le_bytes());
        cbw[8..12].copy_from_slice(&8u32.to_le_bytes());
        cbw[12] = 0x80;
        cbw[14] = 10;
        cbw[15] = 0x25;

        if !self.bulk_out(slot_id, bulk_out_dci, &cbw) {
            return None;
        }
        let mut data = [0u8; 8];
        if !self.bulk_in(slot_id, bulk_in_dci, &mut data) {
            return None;
        }
        let mut csw = [0u8; 13];
        if !self.bulk_in(slot_id, bulk_in_dci, &mut csw) {
            return None;
        }
        let last_lba = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as u64;
        Some(last_lba + 1)
    }

    pub(crate) fn bulk_out(&mut self, slot_id: u8, dci: u8, data: &[u8]) -> bool {
        let (va, phys) = unsafe { alloc_dma_buffer(data.len().max(64)) };
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), va, data.len());
        }
        self.bulk_xfer(slot_id, dci, phys, data.len() as u32, false)
    }

    pub(crate) fn bulk_in(&mut self, slot_id: u8, dci: u8, buf: &mut [u8]) -> bool {
        let (va, phys) = unsafe { alloc_dma_buffer(buf.len().max(64)) };
        if !self.bulk_xfer(slot_id, dci, phys, buf.len() as u32, true) {
            return false;
        }
        let got = unsafe { read_dma_buffer(va, buf.len()) };
        buf.copy_from_slice(&got[..buf.len()]);
        true
    }

    fn bulk_xfer(&mut self, slot_id: u8, dci: u8, phys: u64, len: u32, _dir_in: bool) -> bool {
        let ring = match self.transfer_ring(slot_id, dci) {
            Some(r) => r,
            None => return false,
        };
        let trb = Trb::normal(phys, len, true, false);
        ring.enqueue(trb);
        self.ring_ep(slot_id, dci);
        match self.wait_transfer_event(slot_id) {
            Some(evt) => {
                let code = evt.completion_code();
                code == TRB_COMPLETION_SUCCESS || code == TRB_COMPLETION_SHORT_PACKET
            }
            None => false,
        }
    }
}

impl ParsedConfiguration {
    /// Interfaz BOT (8/6/50) con bulk OUT + bulk IN.
    pub fn find_mass_storage(&self) -> Option<(u8, EndpointDescriptor, EndpointDescriptor)> {
        for iface in &self.interfaces {
            if iface.b_interface_class != MS_CLASS
                || iface.b_interface_sub_class != MS_SUBCLASS_SCSI
                || iface.b_interface_protocol != MS_PROTO_BOT
            {
                continue;
            }
            let inum = iface.b_interface_number;
            let mut out_ep = None;
            let mut in_ep = None;
            for (n, ep) in &self.endpoints {
                if *n != inum {
                    continue;
                }
                if ep.is_bulk() {
                    if ep.is_in() {
                        in_ep = Some(ep.clone());
                    } else {
                        out_ep = Some(ep.clone());
                    }
                }
            }
            if let (Some(o), Some(i)) = (out_ep, in_ep) {
                return Some((inum, o, i));
            }
        }
        None
    }
}
