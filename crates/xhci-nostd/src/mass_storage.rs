//! USB mass storage (BOT / SCSI READ(10)) sobre xHCI.

use crate::device::{
    alloc_dma_buffer, read_dma_buffer, EndpointDescriptor, ParsedConfiguration, UsbSpeed,
};
use crate::dma::delay_us;
use crate::driver::DevPath;
use crate::driver::XhciController;
use crate::registers::{PORTSC_CCS, PORTSC_SPEED_MASK, PORTSC_SPEED_SHIFT};
use crate::ring::{
    completion_name, Trb, TRB_COMPLETION_SHORT_PACKET, TRB_COMPLETION_SUCCESS,
};

/// CDB de 10 bytes para READ(10)/WRITE(10).
fn rw10_cdb(opcode: u8, lba: u32, count: u16) -> [u8; 10] {
    let mut cdb = [0u8; 10];
    cdb[0] = opcode;
    cdb[2] = (lba >> 24) as u8;
    cdb[3] = (lba >> 16) as u8;
    cdb[4] = (lba >> 8) as u8;
    cdb[5] = lba as u8;
    cdb[7] = (count >> 8) as u8;
    cdb[8] = count as u8;
    cdb
}

/// Bytes por transacción BOT. El límite duro es el campo de longitud del
/// Normal TRB (17 bits); 64 KiB deja margen y es lo que QEMU acepta escribir.
const MAX_XFER: usize = 64 * 1024;

const CBW_SIG: u32 = 0x4342_5355;
const CSW_SIG: u32 = 0x5342_5355;
const MS_CLASS: u8 = 0x08;
const MS_SUBCLASS_SCSI: u8 = 0x06;
const MS_PROTO_BOT: u8 = 0x50;

// Opcodes SCSI usados.
const SCSI_TEST_UNIT_READY: u8 = 0x00;
const SCSI_REQUEST_SENSE: u8 = 0x03;
const SCSI_READ_CAPACITY10: u8 = 0x25;
const SCSI_READ10: u8 = 0x28;
const SCSI_WRITE10: u8 = 0x2A;

/// Único tamaño de bloque que entiende el resto del kernel (`SECTOR` = 512).
const BLOCK_SIZE: u32 = 512;

/// Reintentos de TEST UNIT READY al enumerar. Un pendrive recién reseteado
/// contesta CHECK CONDITION por Unit Attention en el primer comando; el sense
/// la limpia, pero un lector de tarjetas puede tardar más en tener medio.
const UNIT_READY_TRIES: u32 = 10;
const UNIT_READY_DELAY_US: u32 = 100_000;

static TAG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(1);

/// Resultado de un comando BOT completo (CBW → [datos] → CSW).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Bot {
    /// CSW válido con estado 0. Lleva los bytes de datos recibidos de verdad.
    Ok(usize),
    /// CSW válido con estado 1 (Command Failed): hay sense pendiente.
    Failed,
    /// Fallo de transporte, o CSW ausente/inválido.
    Error,
}

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
    /// Escanea puertos (root y hubs) y configura HID + el primer stick BOT válido.
    pub fn probe_mass_storage(&mut self) -> Option<MassStorage> {
        self.enumerate_usb_devices()
    }

    /// Tras SET_CONFIGURATION de un dispositivo BOT, deja la unidad lista y
    /// guarda el primer mass storage válido (root o detrás de hub).
    pub(crate) fn try_activate_mass_storage(
        &mut self,
        slot_id: u8,
        config: &ParsedConfiguration,
    ) {
        if self.mass_storage.is_some() {
            return;
        }
        let (iface, bulk_out, bulk_in) = match config.find_mass_storage() {
            Some(t) => t,
            None => return,
        };
        if !self.wait_unit_ready(slot_id, bulk_out.dci(), bulk_in.dci()) {
            log::warn!("xhci: mass storage slot={slot_id} no quedó lista (TEST UNIT READY)");
            return;
        }
        let Some(sectors) = self.read_capacity10(slot_id, bulk_out.dci(), bulk_in.dci()) else {
            log::warn!("xhci: mass storage slot={slot_id} READ CAPACITY falló");
            return;
        };
        log::info!(
            "xhci: mass storage slot={slot_id} iface={iface} sectors={sectors}"
        );
        self.mass_storage = Some(MassStorage {
            slot_id,
            iface,
            bulk_out_dci: bulk_out.dci(),
            bulk_in_dci: bulk_in.dci(),
            sectors,
        });
    }

    /// Escanea puertos root con BOT dedicado (legacy; preferir `enumerate_usb_devices`).
    #[allow(dead_code)]
    fn probe_mass_storage_root_only(&mut self) -> Option<MassStorage> {
        self.drain_port_events();
        for port in 1..=self.max_ports() {
            let portsc = self.portsc(port);
            if portsc & PORTSC_CCS == 0 {
                continue;
            }
            log::info!("xhci: puerto {port} conectado PORTSC={portsc:#010x}");
            let speed_code = (portsc & PORTSC_SPEED_MASK) >> PORTSC_SPEED_SHIFT;
            if speed_code < 4 {
                self.reset_port(port);
            }
            let speed = self.port_speed(port);
            let Some(slot) = self.enable_slot() else {
                log::warn!("xhci: puerto {port}: Enable Slot falló");
                continue;
            };
            match self.setup_mass_storage(slot, DevPath::root(port), speed) {
                Some(ms) => return Some(ms),
                None => {
                    log::warn!("xhci: puerto {port} no es mass storage BOT");
                    self.disable_slot(slot);
                }
            }
        }
        None
    }

    /// Construye el CBW de un comando.
    fn cbw(tag: u32, dlen: u32, dir_in: bool, cdb: &[u8]) -> [u8; 31] {
        let mut cbw = [0u8; 31];
        cbw[0..4].copy_from_slice(&CBW_SIG.to_le_bytes());
        cbw[4..8].copy_from_slice(&tag.to_le_bytes());
        cbw[8..12].copy_from_slice(&dlen.to_le_bytes());
        cbw[12] = if dir_in { 0x80 } else { 0x00 };
        cbw[14] = cdb.len() as u8;
        cbw[15..15 + cdb.len()].copy_from_slice(cdb);
        cbw
    }

    /// Lee y valida el CSW (firma, tag y estado), como `usb_stor_Bulk_transport`
    /// de Linux. Saltarse esta comprobación hace que un comando fallido pase por
    /// bueno y se interpreten como datos los bytes que el búfer DMA tuviera.
    fn read_csw(&mut self, slot_id: u8, in_dci: u8, tag: u32, got: usize) -> Bot {
        let mut csw = [0u8; 13];
        let n = match self.bulk_in_len(slot_id, in_dci, &mut csw) {
            Some(n) => n,
            None => return Bot::Error,
        };
        if n < 13 {
            log::warn!("xhci: CSW corto ({n} bytes) slot={slot_id}");
            return Bot::Error;
        }
        let sig = u32::from_le_bytes([csw[0], csw[1], csw[2], csw[3]]);
        let csw_tag = u32::from_le_bytes([csw[4], csw[5], csw[6], csw[7]]);
        if sig != CSW_SIG || csw_tag != tag {
            log::warn!(
                "xhci: CSW inválido slot={slot_id} sig={sig:#010x} tag={csw_tag} (esperado {tag})"
            );
            return Bot::Error;
        }
        match csw[12] {
            0 => Bot::Ok(got),
            1 => Bot::Failed,
            st => {
                log::warn!("xhci: CSW phase error slot={slot_id} status={st}");
                Bot::Error
            }
        }
    }

    /// Comando BOT con fase de datos IN opcional.
    fn bot_in(
        &mut self,
        slot_id: u8,
        out_dci: u8,
        in_dci: u8,
        cdb: &[u8],
        data: Option<&mut [u8]>,
    ) -> Bot {
        let tag = TAG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let dlen = data.as_ref().map(|d| d.len()).unwrap_or(0);
        let cbw = Self::cbw(tag, dlen as u32, dlen > 0, cdb);
        if !self.bulk_out(slot_id, out_dci, &cbw) {
            return Bot::Error;
        }
        let mut got = 0usize;
        if let Some(buf) = data {
            match self.bulk_in_len(slot_id, in_dci, buf) {
                Some(n) => got = n,
                None => return Bot::Error,
            }
        }
        self.read_csw(slot_id, in_dci, tag, got)
    }

    /// Comando BOT con fase de datos OUT.
    fn bot_out(&mut self, slot_id: u8, out_dci: u8, in_dci: u8, cdb: &[u8], data: &[u8]) -> Bot {
        let tag = TAG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let cbw = Self::cbw(tag, data.len() as u32, false, cdb);
        if !self.bulk_out(slot_id, out_dci, &cbw) {
            return Bot::Error;
        }
        if !self.bulk_out(slot_id, out_dci, data) {
            return Bot::Error;
        }
        self.read_csw(slot_id, in_dci, tag, data.len())
    }

    /// REQUEST SENSE: además de informar, es lo que **limpia** la condición
    /// pendiente para que el siguiente comando no vuelva a fallar.
    fn request_sense(&mut self, slot_id: u8, out_dci: u8, in_dci: u8) {
        let cdb = [SCSI_REQUEST_SENSE, 0, 0, 0, 18, 0];
        let mut sense = [0u8; 18];
        match self.bot_in(slot_id, out_dci, in_dci, &cdb, Some(&mut sense)) {
            Bot::Ok(n) if n >= 14 => log::info!(
                "xhci: sense slot={slot_id} key={:#x} asc={:#04x} ascq={:#04x}",
                sense[2] & 0x0F,
                sense[12],
                sense[13],
            ),
            Bot::Ok(n) => log::warn!("xhci: sense corto ({n} bytes) slot={slot_id}"),
            r => log::warn!("xhci: REQUEST SENSE slot={slot_id}: {r:?}"),
        }
    }

    /// TEST UNIT READY hasta que el dispositivo conteste bien, limpiando con
    /// REQUEST SENSE entre intentos. Es el arranque que hace el midlayer SCSI de
    /// Linux antes de fiarse de un READ CAPACITY.
    fn wait_unit_ready(&mut self, slot_id: u8, out_dci: u8, in_dci: u8) -> bool {
        let cdb = [SCSI_TEST_UNIT_READY, 0, 0, 0, 0, 0];
        for intento in 0..UNIT_READY_TRIES {
            match self.bot_in(slot_id, out_dci, in_dci, &cdb, None) {
                Bot::Ok(_) => {
                    if intento > 0 {
                        log::info!("xhci: unidad lista slot={slot_id} tras {intento} reintento(s)");
                    }
                    return true;
                }
                Bot::Failed => {
                    log::info!(
                        "xhci: TEST UNIT READY slot={slot_id} falló (intento {}); pido sense",
                        intento + 1
                    );
                    self.request_sense(slot_id, out_dci, in_dci);
                    delay_us(UNIT_READY_DELAY_US);
                }
                Bot::Error => {
                    log::warn!("xhci: TEST UNIT READY slot={slot_id}: error de transporte");
                    return false;
                }
            }
        }
        log::warn!("xhci: slot={slot_id} nunca quedó lista tras {UNIT_READY_TRIES} intentos");
        false
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
        // Un Normal TRB lleva la longitud en 17 bits: 0x20000 se desborda a
        // cero, el dispositivo manda datos que nadie recoge y el endpoint se
        // queda en Stall («CSW inválido sig=0»). Se trocea igual que la
        // escritura, y así 128 KiB —el tamaño de petición de sosomfs— dejan de
        // ser una bomba de relojería en el camino live.
        if buf.len() > MAX_XFER {
            let mut off = 0usize;
            let mut cur_lba = lba;
            while off < buf.len() {
                let chunk = (buf.len() - off).min(MAX_XFER);
                if !self.read_sectors10(ms, cur_lba, &mut buf[off..off + chunk]) {
                    return false;
                }
                cur_lba += (chunk / 512) as u32;
                off += chunk;
            }
            return true;
        }
        let count = (buf.len() / 512) as u16;
        let cdb = rw10_cdb(SCSI_READ10, lba, count);
        let esperado = buf.len();

        // Un reintento tras REQUEST SENSE: el midlayer SCSI de Linux hace lo
        // mismo, y así una Unit Attention transitoria no tumba la lectura.
        for intento in 0..2 {
            match self.bot_in(ms.slot_id, ms.bulk_out_dci, ms.bulk_in_dci, &cdb, Some(buf)) {
                Bot::Ok(n) if n == esperado => return true,
                Bot::Ok(n) => {
                    log::warn!("xhci: READ(10) lba={lba} corto: {n}/{esperado} bytes");
                    return false;
                }
                Bot::Failed if intento == 0 => {
                    log::info!("xhci: READ(10) lba={lba} falló; pido sense y reintento");
                    self.request_sense(ms.slot_id, ms.bulk_out_dci, ms.bulk_in_dci);
                }
                Bot::Failed => {
                    log::warn!("xhci: READ(10) lba={lba} falló también en el reintento");
                    return false;
                }
                Bot::Error => return false,
            }
        }
        false
    }

    /// BOT WRITE(10) — un sector 512 B.
    pub fn write_sector10(&mut self, ms: &MassStorage, lba: u32, buf: &[u8; 512]) -> bool {
        self.write_sectors10(ms, lba, buf)
    }

    /// BOT WRITE(10) — `buf.len()/512` sectores consecutivos en una sola transacción.
    pub fn write_sectors10(&mut self, ms: &MassStorage, lba: u32, buf: &[u8]) -> bool {
        if buf.is_empty() || buf.len() % 512 != 0 {
            return false;
        }
        if buf.len() > MAX_XFER {
            let mut off = 0usize;
            let mut cur_lba = lba;
            while off < buf.len() {
                let chunk = (buf.len() - off).min(MAX_XFER);
                if !self.write_sectors10(ms, cur_lba, &buf[off..off + chunk]) {
                    return false;
                }
                cur_lba += (chunk / 512) as u32;
                off += chunk;
            }
            return true;
        }
        if buf.len() / 512 > u16::MAX as usize {
            return false;
        }
        let count = (buf.len() / 512) as u16;
        let cdb = rw10_cdb(SCSI_WRITE10, lba, count);

        for intento in 0..2 {
            match self.bot_out(ms.slot_id, ms.bulk_out_dci, ms.bulk_in_dci, &cdb, buf) {
                Bot::Ok(_) => return true,
                Bot::Failed if intento == 0 => {
                    log::info!("xhci: WRITE(10) lba={lba} falló; pido sense y reintento");
                    self.request_sense(ms.slot_id, ms.bulk_out_dci, ms.bulk_in_dci);
                }
                Bot::Failed => {
                    log::warn!("xhci: WRITE(10) lba={lba} falló también en el reintento");
                    return false;
                }
                Bot::Error => return false,
            }
        }
        false
    }

    fn setup_mass_storage(&mut self, slot_id: u8, path: DevPath, speed: UsbSpeed) -> Option<MassStorage> {
        self.set_device(slot_id, path, speed);
        if !self.address_device(slot_id, path, speed) {
            return None;
        }
        let dev_desc = self.get_device_descriptor(slot_id)?;
        let config = self.get_configuration_descriptor(slot_id, 0)?;
        let (iface, bulk_out, bulk_in) = match config.find_mass_storage() {
            Some(t) => t,
            None => {
                // Distinguir «no se vio el pendrive» de «se vio y se rechazó»:
                // sólo aceptamos BOT (8/6/0x50), no UAS (8/6/0x62).
                log::info!(
                    "xhci: slot={} VID={:#06x} PID={:#06x} sin interfaz BOT (8/6/0x50)",
                    slot_id,
                    dev_desc.id_vendor,
                    dev_desc.id_product,
                );
                for i in config.interfaces.iter().filter(|i| i.b_alternate_setting == 0) {
                    log::info!(
                        "xhci:   iface {} alt0: class={:#x} sub={:#x} proto={:#x}",
                        i.b_interface_number,
                        i.b_interface_class,
                        i.b_interface_sub_class,
                        i.b_interface_protocol,
                    );
                }
                return None;
            }
        };
        if !self.set_configuration(
            slot_id,
            config.config.b_configuration_value,
            &config,
            &dev_desc,
        ) {
            return None;
        }
        // Antes de fiarse de la capacidad hay que dejar la unidad lista: el
        // primer comando tras enumerar suele fallar por Unit Attention.
        if !self.wait_unit_ready(slot_id, bulk_out.dci(), bulk_in.dci()) {
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

    /// READ CAPACITY(10). Devuelve el número de sectores de 512 B.
    ///
    /// Aquí estaba el fallo que dejaba el pendrive «detectado» pero ilegible: se
    /// leía el CSW y no se miraba, así que un CHECK CONDITION (típico Unit
    /// Attention en el primer comando tras enumerar) pasaba por bueno y se
    /// tomaba como capacidad lo que hubiera en el búfer DMA. Con una capacidad
    /// falsa, `usb_storage::read_sector` rechaza la LBA 1 por `lba >= sectors` y
    /// la cabecera GPT no llega a leerse nunca.
    fn read_capacity10(&mut self, slot_id: u8, bulk_out_dci: u8, bulk_in_dci: u8) -> Option<u64> {
        let cdb = [SCSI_READ_CAPACITY10, 0, 0, 0, 0, 0, 0, 0, 0, 0];

        for intento in 0..2 {
            let mut data = [0u8; 8];
            match self.bot_in(slot_id, bulk_out_dci, bulk_in_dci, &cdb, Some(&mut data)) {
                Bot::Ok(n) if n >= 8 => {
                    let last_lba =
                        u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as u64;
                    let block_len = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                    if block_len != BLOCK_SIZE {
                        log::warn!(
                            "xhci: slot={slot_id} bloque de {block_len} B; el kernel sólo \
                             maneja {BLOCK_SIZE} B — descartado"
                        );
                        return None;
                    }
                    if last_lba == 0 {
                        log::warn!("xhci: slot={slot_id} READ CAPACITY dice 1 sector; descartado");
                        return None;
                    }
                    log::info!(
                        "xhci: slot={slot_id} capacidad {} sectores de {block_len} B",
                        last_lba + 1
                    );
                    return Some(last_lba + 1);
                }
                Bot::Ok(n) => {
                    log::warn!("xhci: READ CAPACITY corto ({n}/8 bytes) slot={slot_id}");
                    return None;
                }
                Bot::Failed if intento == 0 => {
                    log::info!("xhci: READ CAPACITY slot={slot_id} falló; pido sense y reintento");
                    self.request_sense(slot_id, bulk_out_dci, bulk_in_dci);
                }
                Bot::Failed => {
                    log::warn!("xhci: READ CAPACITY slot={slot_id} falló en el reintento");
                    return None;
                }
                Bot::Error => return None,
            }
        }
        None
    }

    /// Búfer de rebote persistente para las transferencias bulk: se agranda si
    /// hace falta y se reutiliza siempre. El asignador DMA del kernel no
    /// libera, así que pedir uno por comando era una fuga proporcional a los
    /// datos movidos.
    fn bounce_buffer(&mut self, size: usize) -> (*mut u8, u64) {
        let size = size.max(64);
        if let Some((va, pa, cap)) = self.bounce {
            if cap >= size {
                return (va, pa);
            }
        }
        let (va, pa) = unsafe { alloc_dma_buffer(size) };
        self.bounce = Some((va, pa, size));
        (va, pa)
    }

    pub(crate) fn bulk_out(&mut self, slot_id: u8, dci: u8, data: &[u8]) -> bool {
        let (va, phys) = self.bounce_buffer(data.len());
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), va, data.len());
        }
        self.bulk_xfer(slot_id, dci, phys, data.len() as u32, false)
            .is_some()
    }

    /// Bulk IN devolviendo **cuántos bytes llegaron**. Un BOT que falla salta la
    /// fase de datos y contesta el CSW directamente: sin el recuento no hay forma
    /// de distinguirlo de una lectura buena.
    pub(crate) fn bulk_in_len(&mut self, slot_id: u8, dci: u8, buf: &mut [u8]) -> Option<usize> {
        let (va, phys) = self.bounce_buffer(buf.len());
        let n = self.bulk_xfer(slot_id, dci, phys, buf.len() as u32, true)? as usize;
        let n = n.min(buf.len());
        let got = unsafe { read_dma_buffer(va, buf.len()) };
        buf[..n].copy_from_slice(&got[..n]);
        Some(n)
    }

    /// Devuelve los bytes transferidos (`len` menos el residual del evento).
    fn bulk_xfer(&mut self, slot_id: u8, dci: u8, phys: u64, len: u32, _dir_in: bool) -> Option<u32> {
        let ring = match self.transfer_ring(slot_id, dci) {
            Some(r) => r,
            None => return None,
        };
        let trb = Trb::normal(phys, len, true, false);
        ring.enqueue(trb);
        self.ring_ep(slot_id, dci);
        let evt = self.wait_transfer_event(slot_id, None)?;
        let code = evt.completion_code();
        if code != TRB_COMPLETION_SUCCESS && code != TRB_COMPLETION_SHORT_PACKET {
            log::warn!(
                "xhci: bulk slot={slot_id} dci={dci} len={len}: code={code} ({})",
                completion_name(code),
            );
            return None;
        }
        // En un Transfer Event el campo de longitud es el **residual**.
        Some(len.saturating_sub(evt.transfer_length()))
    }
}

impl ParsedConfiguration {
    /// Interfaz BOT (8/6/50) con bulk OUT + bulk IN.
    pub fn find_mass_storage(&self) -> Option<(u8, EndpointDescriptor, EndpointDescriptor)> {
        for iface in &self.interfaces {
            if iface.b_alternate_setting != 0
                || iface.b_interface_class != MS_CLASS
                || iface.b_interface_sub_class != MS_SUBCLASS_SCSI
                || iface.b_interface_protocol != MS_PROTO_BOT
            {
                continue;
            }
            let inum = iface.b_interface_number;
            let mut out_ep = None;
            let mut in_ep = None;
            for (n, alt, ep) in &self.endpoints {
                if *n != inum || *alt != 0 {
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
