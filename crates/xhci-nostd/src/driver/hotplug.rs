//! Conexión y desconexión en caliente.
//!
//! El despacho de eventos (que puede correr dentro de la IRQ) sólo apunta en
//! `port_csc` qué puertos root vieron un Connect Status Change. Todo lo caro
//! —desmontar slots, resetear puertos, pedir descriptores— lo hace
//! `poll_hotplug`, que el kernel llama desde contexto normal, con el candado
//! del controlador cogido.
//!
//! Lo que cuelga de un hub no genera eventos del HC: el hub avisa por su
//! endpoint de interrupción, que no se usa. Sus puertos se sondean con
//! GET_PORT_STATUS cada `HUB_POLL_EVERY` pasadas, como `hub_events` de Linux
//! pero por consulta.

use super::*;

/// Pasadas de `poll_hotplug` entre dos repasos de los puertos de los hubs.
const HUB_POLL_EVERY: u32 = 4;
/// Linux `hub_port_debounce`: la conexión debe aguantar 100 ms estable.
const DEBOUNCE_STABLE_US: u32 = 100_000;
/// Una sonda GET_STATUS a un dispositivo que sigue ahí contesta en µs.
const PROBE_TIMEOUT_US: u32 = 500_000;

impl XhciController {
    /// Atiende conexiones y desconexiones de dispositivos.
    ///
    /// Llamar fuera de IRQ, periódicamente (unos 250 ms). Sin cambios cuesta
    /// unas lecturas MMIO; una conexión cuesta lo que la enumeración
    /// (cientos de ms), una pasada después de verla: el debounce es esperar
    /// a la pasada siguiente, no dormir. Devuelve lo que ha cambiado.
    pub fn poll_hotplug(&mut self) -> Vec<UsbHotplugEvent> {
        let mut out = Vec::new();
        // Eventos aún en el anillo: puede haber Port Status Change sin leer
        // si nadie lo vació (controlador sin IRQ armada).
        self.drain_hw_events();
        self.report_irq_notes();
        let n = self.max_ports.min(64);
        // CSC que no llegaron como evento: `ack` los anota en `port_csc`.
        for port in 1..=n {
            if self.op.portsc(port) & PORTSC_CSC != 0 {
                self.ack_port_status_change(port);
            }
        }
        let csc = core::mem::take(&mut self.port_csc);
        for port in 1..=n {
            let flagged = csc & (1u64 << (port - 1)) != 0;
            let portsc = self.op.portsc(port);
            let connected = portsc & PORTSC_CCS != 0;
            if self.root_port_has_device(port)
                && (!connected || (flagged && !self.root_device_alive(port, portsc)))
            {
                self.detach_root_port(port, &mut out);
            }
            if !connected {
                self.pending_connect &= !(1u64 << (port - 1));
                // Lo que se ignoró (BT y demás) puede no ser lo próximo que
                // se enchufe en ese puerto.
                if port <= 32 {
                    self.ignored_ports &= !(1u32 << (port - 1));
                }
                if let Some(m) = self.root_port_slot.get_mut(port as usize) {
                    *m = 0;
                }
                continue;
            }
            let bit = 1u64 << (port - 1);
            if self.root_port_has_device(port) {
                self.pending_connect &= !bit;
                continue;
            }
            if flagged {
                // Debounce sin esperar: se enumera en la pasada siguiente si
                // la conexión sigue ahí y no ha vuelto a cambiar (≥ un periodo
                // de sondeo estable, más que los 100 ms de Linux).
                self.pending_connect |= bit;
            } else if self.pending_connect & bit != 0 {
                self.pending_connect &= !bit;
                self.attach_root_port(port, &mut out);
            }
        }

        self.hotplug_ticks = self.hotplug_ticks.wrapping_add(1);
        if self.hotplug_ticks % HUB_POLL_EVERY == 0 {
            self.poll_hubs(&mut out);
        }
        self.rearm_failed_hid();
        // Transfer events de slots que ya no existen: nadie los va a reclamar.
        let devices = &self.devices;
        self.pending_transfers.retain(|e| {
            devices
                .get(e.slot_id() as usize)
                .is_some_and(|d| d.is_some())
        });
        out
    }

    /// Mass storage activadas además del disco del arranque: las enchufadas
    /// en caliente y las que ya estaban al arrancar. El kernel decide si
    /// alguna es el disco del arranque que vuelve o un pendrive de datos.
    pub fn take_extra_mass_storage(&mut self) -> Vec<MassStorage> {
        core::mem::take(&mut self.extra_ms)
    }

    /// Lo que la IRQ no pudo contar.
    fn report_irq_notes(&mut self) {
        let notes = core::mem::take(&mut self.irq_notes);
        if notes.hid_errors != 0 {
            log::info!(
                "xhci: {} transferencia(s) HID con error (¿desenchufado?)",
                notes.hid_errors
            );
        }
        if notes.pending_dropped != 0 {
            log::warn!(
                "xhci: cola de transferencias llena; {} evento(s) descartado(s)",
                notes.pending_dropped
            );
        }
    }

    /// El puerto vio CSC pero sigue conectado: ¿es el mismo dispositivo, o
    /// lo han cambiado entre dos sondeos?
    ///
    /// Uno recién enchufado está en estado Default (dirección 0): en USB2 el
    /// puerto ni siquiera está habilitado hasta el reset, y en USB3 una
    /// petición a la dirección vieja no llega a nadie.
    fn root_device_alive(&mut self, port: u8, portsc: u32) -> bool {
        if portsc & PORTSC_PED == 0 {
            return false;
        }
        let Some(slot) = self
            .devices
            .iter()
            .flatten()
            .find(|d| d.port == port && d.route_string == 0)
            .map(|d| d.slot_id)
        else {
            return false;
        };
        self.probe_device(slot)
    }

    /// GET_STATUS (dispositivo) por EP0.
    fn probe_device(&mut self, slot: u8) -> bool {
        let (_, phys) = match self.hub_status_buf {
            Some(b) => b,
            None => {
                let b = unsafe { crate::device::alloc_dma_buffer_persistent(4) };
                self.hub_status_buf = Some(b);
                b
            }
        };
        let Some(ring) = self
            .transfer_rings
            .get_mut(slot as usize)
            .and_then(|eps| eps.get_mut(EP0_DCI as usize))
            .and_then(|r| r.as_mut())
        else {
            return false;
        };
        let handles = ring.enqueue_control_transfer(
            USB_DIR_IN | USB_TYPE_STANDARD | USB_RECIP_DEVICE,
            USB_REQ_GET_STATUS,
            0,
            0,
            phys,
            2,
        );
        self.db.ring_endpoint(slot, EP0_DCI);
        matches!(
            self.wait_transfer_event_timeout(slot, Some(handles.status_trb_phys), PROBE_TIMEOUT_US),
            Some(evt) if evt.completion_code() == TRB_COMPLETION_SUCCESS
                || evt.completion_code() == TRB_COMPLETION_SHORT_PACKET
        )
    }

    fn live_slots(&self) -> Vec<u8> {
        self.devices.iter().flatten().map(|d| d.slot_id).collect()
    }

    /// Enumera lo que se enchufó en un puerto root y anota los slots nuevos.
    fn attach_root_port(&mut self, port: u8, out: &mut Vec<UsbHotplugEvent>) {
        if self.op.portsc(port) & PORTSC_CCS == 0 {
            return;
        }
        let before = self.live_slots();
        self.attach_with(out, &before, |c| {
            if c.enumerate_root_port(port).is_none() {
                log::warn!("xhci: puerto {port}: Enable Slot falló en caliente");
            }
        });
    }

    /// Corre `f` (una enumeración) y emite un evento por cada slot nuevo.
    fn attach_with(
        &mut self,
        out: &mut Vec<UsbHotplugEvent>,
        before: &[u8],
        f: impl FnOnce(&mut Self),
    ) {
        // Todo lo nuevo va a `extra_ms`, aunque el disco del arranque se haya
        // ido: decidir si es él quien vuelve es cosa del kernel.
        let prev_ms = self.mass_storage.take();
        f(self);
        if let Some(ms) = self.mass_storage.take() {
            self.extra_ms.push(ms);
        }
        self.mass_storage = prev_ms;
        // Con el dispositivo ya configurado, los CSC de su propio reset no son
        // una conexión nueva.
        for slot in self.live_slots() {
            if before.contains(&slot) {
                continue;
            }
            if let Some(evt) = self.slot_event(slot, true) {
                log::info!(
                    "xhci: conectado slot={} puerto={} hub_port={} {:04x}:{:04x} {:?}",
                    evt.slot,
                    evt.root_port,
                    evt.hub_port,
                    evt.vendor_id,
                    evt.product_id,
                    evt.kind
                );
                out.push(evt);
            }
        }
    }

    fn slot_kind(&self, slot: u8) -> UsbKind {
        if self.keyboards.iter().any(|k| k.slot_id == slot) {
            return UsbKind::Keyboard;
        }
        if self.pointers.iter().any(|p| p.slot_id == slot) {
            return UsbKind::Pointer;
        }
        if self.hubs.iter().any(|h| h.slot == slot) {
            return UsbKind::Hub;
        }
        let dev = self.devices.get(slot as usize).and_then(|d| d.as_ref());
        if dev
            .and_then(|d| d.config.as_ref())
            .is_some_and(|c| c.is_mass_storage())
        {
            return UsbKind::MassStorage;
        }
        if dev
            .and_then(|d| d.device_desc.as_ref())
            .is_some_and(|d| d.is_hub())
        {
            return UsbKind::Hub;
        }
        UsbKind::Other
    }

    fn slot_event(&self, slot: u8, connected: bool) -> Option<UsbHotplugEvent> {
        let dev = self.devices.get(slot as usize)?.as_ref()?;
        let (vendor_id, product_id) = dev
            .device_desc
            .as_ref()
            .map(|d| (d.id_vendor, d.id_product))
            .unwrap_or((0, 0));
        Some(UsbHotplugEvent {
            connected,
            slot,
            root_port: dev.port,
            hub_port: if dev.route_string != 0 { dev.tt_port } else { 0 },
            vendor_id,
            product_id,
            kind: self.slot_kind(slot),
        })
    }

    /// Desmonta todo lo que cuelga de un puerto root.
    fn detach_root_port(&mut self, port: u8, out: &mut Vec<UsbHotplugEvent>) {
        let roots: Vec<u8> = self
            .devices
            .iter()
            .flatten()
            .filter(|d| d.port == port && d.route_string == 0)
            .map(|d| d.slot_id)
            .collect();
        for slot in roots {
            self.detach_subtree(slot, out);
        }
        // Huérfanos (un hub anidado cuya ruta no casara): mismo puerto root.
        let rest: Vec<u8> = self
            .devices
            .iter()
            .flatten()
            .filter(|d| d.port == port)
            .map(|d| d.slot_id)
            .collect();
        for slot in rest {
            self.detach_slot(slot, out);
        }
    }

    /// Desmonta un slot y, si es un hub, lo que cuelga de él (hijos primero).
    fn detach_subtree(&mut self, slot: u8, out: &mut Vec<UsbHotplugEvent>) {
        let children: Vec<u8> = self
            .devices
            .iter()
            .flatten()
            .filter(|d| d.route_string != 0 && d.tt_hub_slot == slot && d.slot_id != slot)
            .map(|d| d.slot_id)
            .collect();
        for child in children {
            self.detach_subtree(child, out);
        }
        self.detach_slot(slot, out);
    }

    /// Olvida un slot: HID, inventario, cola, mass storage y memoria DMA.
    fn detach_slot(&mut self, slot: u8, out: &mut Vec<UsbHotplugEvent>) {
        let Some(evt) = self.slot_event(slot, false) else {
            return;
        };
        // Teclas que estaban pulsadas: soltarlas, o el keymap se queda con
        // un Shift (o una tecla repitiendo) para siempre.
        let mut i = 0;
        while i < self.keyboards.len() {
            if self.keyboards[i].slot_id != slot {
                i += 1;
                continue;
            }
            let mut kb = self.keyboards.remove(i);
            if let Some(empty) = crate::hid::BootKeyboardReport::parse(&[0u8; 8]) {
                kb.state.process_report(&empty);
            }
            while let Some(k) = kb.state.next_event() {
                self.held_keys.push_back(k);
            }
        }
        self.pointers.retain(|p| p.slot_id != slot);
        self.pending_transfers.retain(|e| e.slot_id() != slot);
        self.inventory.retain(|d| d.slot != slot);
        self.hubs.retain(|h| h.slot != slot);
        if self.mass_storage.as_ref().is_some_and(|m| m.slot_id == slot) {
            self.mass_storage = None;
        }
        self.extra_ms.retain(|m| m.slot_id != slot);
        // Disable Slot: tras completarse, el HC ya no toca sus contextos ni
        // sus anillos, y la memoria puede volver a la reserva.
        self.disable_slot(slot);
        for m in self.root_port_slot.iter_mut() {
            if *m == slot {
                *m = 0;
            }
        }
        if let Some(blocks) = self.slot_dma.get_mut(slot as usize) {
            crate::dma::release(core::mem::take(blocks));
        }
        log::info!(
            "xhci: desconectado slot={} puerto={} hub_port={} {:04x}:{:04x} {:?}",
            evt.slot,
            evt.root_port,
            evt.hub_port,
            evt.vendor_id,
            evt.product_id,
            evt.kind
        );
        out.push(evt);
    }

    /// Repasa los puertos downstream de cada hub.
    fn poll_hubs(&mut self, out: &mut Vec<UsbHotplugEvent>) {
        let hubs = self.hubs.clone();
        for hub in hubs {
            if !self
                .devices
                .get(hub.slot as usize)
                .is_some_and(|d| d.is_some())
            {
                continue;
            }
            for hp in 1..=hub.n_ports {
                let Some((status, change)) = self.hub_get_port_status(hub.slot, hp) else {
                    // El hub no contesta: se fue y el puerto root lo dirá.
                    break;
                };
                if change & HUB_PORT_CHANGE_ENABLE != 0 {
                    let _ = self.hub_clear_port_feature(hub.slot, hp, HUB_FEATURE_C_PORT_ENABLE);
                }
                if change & HUB_PORT_CHANGE_CONNECTION == 0 {
                    continue;
                }
                let _ = self.hub_clear_port_feature(hub.slot, hp, HUB_FEATURE_C_PORT_CONNECTION);
                let child = self
                    .devices
                    .iter()
                    .flatten()
                    .find(|d| d.route_string != 0 && d.tt_hub_slot == hub.slot && d.tt_port == hp)
                    .map(|d| d.slot_id);
                if let Some(child) = child {
                    self.detach_subtree(child, out);
                }
                if status & HUB_PORT_CONNECTED == 0 {
                    continue;
                }
                delay_us(DEBOUNCE_STABLE_US);
                let before = self.live_slots();
                self.attach_with(out, &before, |c| {
                    c.attach_hub_port(hub.slot, hub.root_port, hp)
                });
            }
        }
    }

    /// HID cuyo IN acabó en error con el dispositivo aún enchufado: Reset
    /// Endpoint + Set TR Dequeue y volver a pedir informe.
    fn rearm_failed_hid(&mut self) {
        let mut todo: Vec<(bool, usize)> = Vec::new();
        for (i, k) in self.keyboards.iter().enumerate() {
            if !k.transfer_pending {
                todo.push((true, i));
            }
        }
        for (i, p) in self.pointers.iter().enumerate() {
            if !p.transfer_pending {
                todo.push((false, i));
            }
        }
        for (is_kbd, i) in todo {
            let (slot, dci, phys, len) = if is_kbd {
                let k = &self.keyboards[i];
                (k.slot_id, k.dci, k.report_buf_phys, k.report_buf_len)
            } else {
                let p = &self.pointers[i];
                (p.slot_id, p.dci, p.report_buf_phys, p.report_buf_len)
            };
            // Se marca pendiente aunque falle: si no, cada sondeo lo
            // reintentaría y llenaría el log.
            if !self.reset_and_requeue_ep(slot, dci) {
                log::warn!("xhci: no pude rearmar el HID slot={slot} dci={dci}");
            }
            if let Some(ring) = self
                .transfer_rings
                .get_mut(slot as usize)
                .and_then(|eps| eps.get_mut(dci as usize))
                .and_then(|r| r.as_mut())
            {
                ring.enqueue_interrupt_in(phys, len as u32);
                self.db.ring_endpoint(slot, dci);
            }
            if is_kbd {
                self.keyboards[i].transfer_pending = true;
            } else {
                self.pointers[i].transfer_pending = true;
            }
        }
    }
}
