//! High-level xHCI Host Controller Driver
//!
//! `XhciController` is the top-level API. It wraps register access, ring
//! management, device enumeration, and HID keyboard integration into a single
//! struct that the kernel can init once and then poll for keyboard events.

use alloc::vec::Vec;
use alloc::vec;

use crate::context::{
    Dcbaa, EndpointContext, InputContext, SlotContext,
    EP_TYPE_CONTROL,
};
use crate::device::{
    alloc_dma_buffer, read_dma_buffer, DeviceDescriptor, EndpointDescriptor,
    ParsedConfiguration, UsbDevice, UsbSpeed,
    USB_DESC_CONFIGURATION, USB_DESC_DEVICE, USB_DESC_HUB,
    USB_DIR_IN, USB_DIR_OUT, USB_RECIP_DEVICE, USB_RECIP_INTERFACE,
    USB_REQ_GET_DESCRIPTOR, USB_REQ_GET_STATUS, USB_REQ_SET_CONFIGURATION,
    USB_TYPE_CLASS, USB_TYPE_STANDARD,
};
use crate::hid::{
    BootKeyboardReport, KeyEvent, KeyboardState,
    HID_PROTOCOL_BOOT, HID_REQ_SET_IDLE, HID_REQ_SET_PROTOCOL,
};
use crate::mass_storage::MassStorage;
use crate::dma::delay_us;
use crate::registers::*;
use crate::ring::*;

/// Intervalo entre polls del event ring (µs).
const EVENT_POLL_INTERVAL_US: u32 = 10;
/// Timeout de comandos xHCI (Linux XHCI_CMD_DEFAULT_TIMEOUT ≈ 5 s).
const CMD_TIMEOUT_US: u32 = 5_000_000;
/// Timeout de transfer events.
const TRANSFER_TIMEOUT_US: u32 = 5_000_000;
/// Timeout de sondeo EP0 (GET_DESCRIPTOR); el de BOT sigue siendo 5 s.
const EP0_TRANSFER_TIMEOUT_US: u32 = 1_000_000;
/// Debounce de conexión antes de reset (Linux hub_port_debounce).
const PORT_DEBOUNCE_US: u32 = 100_000;
/// TRSTRCY USB2 full/low-speed (≥10 ms).
const TRSTRCY_FS_US: u32 = 10_000;
/// TRSTRCY USB2 high-speed (≥50 ms).
const TRSTRCY_HS_US: u32 = 50_000;
/// Pausa entre reintentos de Address Device (Linux hub_port_init).
const ADDR_RETRY_DELAY_US: u32 = 200_000;
const MAX_ADDR_RETRIES: u32 = 3;
const HUB_POWER_ON_US: u32 = 100_000;
/// Tras Address Device, antes del primer GET_DESCRIPTOR (Linux hub_port_init).
const SET_ADDRESS_SETTLE_US: u32 = 10_000;
/// Reintentos por lectura de descriptor en EP0 (Linux usb_get_device_descriptor).
const GET_DESCRIPTOR_RETRIES: u32 = 3;
const GET_DESCRIPTOR_RETRY_DELAY_US: u32 = 200_000;
/// Reintentos completos con re-reset de puerto si el descriptor sigue fallando.
const GET_DESCRIPTOR_TRIES: u32 = 2;
const EP0_DCI: u8 = 1;
/// Transfer events de otros slots mientras se espera uno concreto (p. ej. HID
/// durante BOT del pendrive). Sin cola, `wait_transfer_event` los tiraba.
const MAX_PENDING_TRANSFERS: usize = 32;

/// Como Linux `XHCI_IRQS`: EIE | HSEIE | EWE.
const USBCMD_IRQS: u32 = USBCMD_INTE | USBCMD_HSEE | USBCMD_EWE;

/// The xHCI Host Controller driver.
pub struct XhciController {
    /// PCI BAR0 base address (MMIO)
    bar0: usize,
    /// Capability registers
    cap: CapabilityRegs,
    /// Operational registers
    op: OperationalRegs,
    /// Runtime registers
    rt: RuntimeRegs,
    /// Doorbell registers
    db: DoorbellRegs,
    /// Context size (32 or 64 bytes)
    ctx_size: usize,
    /// Maximum device slots
    max_slots: u8,
    /// Maximum ports
    max_ports: u8,
    /// Command ring
    cmd_ring: CommandRing,
    /// Event ring (interrupter 0)
    evt_ring: EventRing,
    /// Device Context Base Address Array
    dcbaa: Dcbaa,
    /// Tracked USB devices (indexed by slot ID, slot 0 unused)
    devices: Vec<Option<UsbDevice>>,
    /// Transfer rings per slot/endpoint (slot_id -> dci -> ring)
    transfer_rings: Vec<Vec<Option<TransferRing>>>,
    /// Teclados HID activos (puede haber más de uno en el mismo xHCI).
    keyboards: Vec<KeyboardInfo>,
    /// Transfer completions desviados mientras otro slot esperaba su evento.
    pending_transfers: Vec<Trb>,
    /// CCS en puertos root vistos antes de HCRST (bit N = puerto N+1).
    boot_ccs_mask: u32,
    /// Primer mass storage BOT activado durante `enumerate_usb_devices`.
    pub(crate) mass_storage: Option<MassStorage>,
    /// Búfer de rebote reutilizado por las transferencias bulk (VA, PA, bytes).
    ///
    /// El asignador DMA del kernel no libera: pedir uno nuevo en cada comando
    /// BOT tiraba una copia entera de la imagen a la basura (clonar 8 GiB son
    /// ~128 000 comandos), y el instalador se quedaba sin memoria contigua.
    pub(crate) bounce: Option<(*mut u8, u64, usize)>,
    /// Evita volcar los 6+ PORTSC en cada timeout (el live se veía como bucle).
    timeout_ports_logged: bool,
    /// Bit N = puerto root N+1 ignorado (BT class 224, etc.).
    ignored_ports: u32,
    /// Slot activo por puerto root (0 = libre).
    root_port_slot: Vec<u8>,
}

/// Keyboard-specific state bundled together.
struct KeyboardInfo {
    /// Slot ID of the keyboard device
    slot_id: u8,
    /// Device Context Index of the interrupt IN endpoint
    dci: u8,
    /// DMA buffer for interrupt reports
    report_buf_va: *mut u8,
    report_buf_phys: u64,
    report_buf_len: usize,
    /// Keyboard state tracker for event generation
    state: KeyboardState,
    /// Whether we have an outstanding interrupt transfer
    transfer_pending: bool,
}

/// Ruta xHCI hasta un dispositivo (root o detrás de hub).
#[derive(Clone, Copy, Debug)]
pub(crate) struct DevPath {
    root_port: u8,
    route: u32,
    tt_hub_slot: u8,
    tt_port: u8,
}

impl DevPath {
    pub(crate) fn root(port: u8) -> Self {
        Self {
            root_port: port,
            route: 0,
            tt_hub_slot: 0,
            tt_port: 0,
        }
    }

    fn behind_hub(root_port: u8, hub_port: u8, hub_slot: u8) -> Self {
        Self {
            root_port,
            route: hub_port as u32,
            tt_hub_slot: hub_slot,
            tt_port: hub_port,
        }
    }
}

const USB_RECIP_OTHER: u8 = 0x03;
const HUB_REQ_SET_FEATURE: u8 = 3;
const HUB_REQ_CLEAR_FEATURE: u8 = 1;
const HUB_FEATURE_PORT_POWER: u16 = 8;
const HUB_FEATURE_PORT_RESET: u16 = 4;
const HUB_FEATURE_C_PORT_RESET: u16 = 20;
const HUB_PORT_CONNECTED: u16 = 1 << 0;
const HUB_PORT_LOW_SPEED: u16 = 1 << 9;
const HUB_PORT_HIGH_SPEED: u16 = 1 << 10;
const HUB_PORT_CHANGE_RESET: u16 = 1 << 4;

fn usb_speed_from_hub_status(status: u16) -> UsbSpeed {
    if status & HUB_PORT_LOW_SPEED != 0 {
        UsbSpeed::Low
    } else if status & HUB_PORT_HIGH_SPEED != 0 {
        UsbSpeed::High
    } else {
        UsbSpeed::Full
    }
}

fn trst_recovery_us(speed: UsbSpeed) -> u32 {
    match speed {
        UsbSpeed::High => TRSTRCY_HS_US,
        _ => TRSTRCY_FS_US,
    }
}

impl XhciController {
    /// Initialize the xHCI controller from a PCI BAR0 address.
    ///
    /// Secuencia: USBLEGSUP handoff → halt → HCRST (necesario: sin él,
    /// Address Device falla con TRB Error) → programar anillos → Run →
    /// recuperación de puertos (PP, espera larga, ciclo PP, warm-reset).
    ///
    /// # Safety
    /// `pci_bar0` must be a valid, identity-mapped MMIO address for an xHCI controller.
    pub unsafe fn init(pci_bar0: usize) -> Self {
        log::info!("xhci: initializing controller at BAR0={:#x}", pci_bar0);

        let cap = CapabilityRegs::new(pci_bar0);
        let caplength = cap.caplength();
        let hciversion = cap.hciversion();
        let max_slots = cap.max_slots();
        let max_ports = cap.max_ports();
        let max_intrs = cap.max_intrs();
        let ctx_size = cap.context_size();
        let scratchpad = cap.max_scratchpad_buffers();

        log::info!(
            "xhci: CAPLENGTH={:#x} HCIVERSION={:#x} MaxSlots={} MaxPorts={} MaxIntrs={} CtxSize={} Scratchpad={}",
            caplength, hciversion, max_slots, max_ports, max_intrs, ctx_size, scratchpad
        );

        let op = OperationalRegs::new(cap.operational_base());
        let rt = RuntimeRegs::new(cap.runtime_base());
        let db = DoorbellRegs::new(cap.doorbell_base());

        let mut boot_ccs_mask = 0u32;
        for port in 1..=max_ports.min(32) {
            let portsc = op.portsc(port);
            let ccs = portsc & PORTSC_CCS != 0;
            if ccs {
                boot_ccs_mask |= 1 << (port - 1);
            }
            log::info!(
                "xhci: boot port {port} PORTSC={portsc:#010x} ccs={ccs} pp={}",
                portsc & PORTSC_PP != 0,
            );
        }
        if boot_ccs_mask == 0 {
            log::warn!("xhci: UEFI ya soltó todos los puertos (ccs=0); recovery post-HCRST");
        } else {
            log::info!(
                "xhci: boot CCS mask={boot_ccs_mask:#x} ({} puerto(s))",
                boot_ccs_mask.count_ones()
            );
        }

        linux_bios_handoff(pci_bar0, &cap, &op);
        do_hcrst(&op, max_ports);

        op.set_config(max_slots as u32);

        let dcbaa = Dcbaa::new(max_slots, ctx_size, scratchpad);
        op.set_dcbaap(dcbaa.phys_addr());

        let cmd_ring = CommandRing::new();
        op.set_crcr(cmd_ring.phys_addr_with_cycle());

        let evt_ring = EventRing::new();
        rt.set_erstsz(0, evt_ring.erst_size());
        rt.set_erdp(0, evt_ring.dequeue_phys());
        rt.set_erstba(0, evt_ring.erst_phys());
        rt.set_imod(0, 4000);
        rt.set_iman(0, IMAN_IP | IMAN_IE);

        let usbcmd = op.usbcmd() | USBCMD_RS | USBCMD_INTE;
        op.set_usbcmd(usbcmd);
        log::info!("xhci: controller started (USBCMD={:#x})", op.usbcmd());

        if op.is_halted() {
            panic!(
                "xhci: controller still halted after setting RS (USBSTS={:#x})",
                op.usbsts()
            );
        }

        let mut devices = Vec::with_capacity(max_slots as usize + 1);
        let mut transfer_rings = Vec::with_capacity(max_slots as usize + 1);
        for _ in 0..=max_slots {
            devices.push(None);
            transfer_rings.push(Vec::new());
        }

        let mut ctrl = Self {
            bar0: pci_bar0,
            cap,
            op,
            rt,
            db,
            ctx_size,
            max_slots,
            max_ports,
            cmd_ring,
            evt_ring,
            dcbaa,
            devices,
            transfer_rings,
            keyboards: Vec::new(),
            pending_transfers: Vec::new(),
            boot_ccs_mask,
            mass_storage: None,
            bounce: None,
            timeout_ports_logged: false,
            ignored_ports: 0,
            root_port_slot: vec![0u8; max_ports as usize + 1],
        };

        ctrl.power_ports();
        ctrl.clear_pcd();
        ctrl.clear_port_change_bits();
        delay_us(100_000);
        ctrl.recover_root_ports();
        ctrl.log_ports();
        log::info!("xhci: initialization complete, {} ports available", max_ports);
        ctrl
    }

    pub fn any_root_port_connected(&self) -> bool {
        (1..=self.max_ports()).any(|p| self.portsc(p) & PORTSC_CCS != 0)
    }

    /// Post-HCRST / hub_activate: PP + clear change bits + esperar CCS.
    /// Si no aparece CCS, conmuta PP y prueba warm-reset en puertos con PP.
    pub fn recover_root_ports(&mut self) {
        self.power_ports();
        self.clear_pcd();
        self.clear_port_change_bits();
        if self.wait_for_boot_ports(40) {
            self.reset_connected_without_ped();
            return;
        }

        self.log_root_ports_pls("sin CCS tras espera");

        // Ciclo PP off→on (algunos AMD no reenumeran tras HCRST sin glitch VBus).
        log::info!("xhci: sin CCS tras espera; ciclo PP off/on");
        self.power_ports_off();
        delay_us(100_000);
        self.power_ports();
        delay_us(200_000);
        self.drain_port_events();
        self.clear_port_change_bits();
        if self.wait_for_boot_ports(40) {
            self.reset_connected_without_ped();
            return;
        }

        // Warm reset en Compliance o puertos con PP sin CCS (hub_port_warm_reset_required).
        log::info!("xhci: warm-reset puertos perezosos/compliance");
        self.warm_reset_lazy_ports();
        delay_us(200_000);
        self.drain_port_events();
        self.clear_port_change_bits();
        if self.wait_for_boot_ports(20) {
            self.reset_connected_without_ped();
        } else {
            self.log_root_ports_pls("recovery sin CCS");
            let missing = self.boot_ccs_mask & !self.current_ccs_mask();
            if missing != 0 {
                log::warn!(
                    "xhci: recovery incompleta — faltan puertos CCS mask={missing:#x} \
                     (teníamos boot_ccs={:#x}, ahora={:#x})",
                    self.boot_ccs_mask,
                    self.current_ccs_mask(),
                );
            } else {
                log::warn!("xhci: recovery sin CCS en ningún puerto root");
            }
        }
    }

    fn reset_connected_without_ped(&mut self) {
        for port in 1..=self.max_ports() {
            let portsc = self.op.portsc(port);
            if portsc & PORTSC_CCS != 0 && portsc & PORTSC_PED == 0 {
                log::info!("xhci: reset puerto {port} (CCS sin PED)");
                self.reset_port(port);
            }
        }
    }

    fn clear_pcd(&mut self) {
        if self.op.usbsts() & USBSTS_PCD != 0 {
            self.op.set_usbsts(USBSTS_PCD);
        }
    }

    /// Escribe PORTSC sin tocar PED (escribir 1 en PED deshabilita el puerto).
    fn write_portsc_masked(&self, port: u8, portsc: u32, or_bits: u32) {
        let val = (portsc & !PORTSC_CHANGE_BITS & !PORTSC_PED) | or_bits;
        self.op.set_portsc(port, val);
    }

    fn log_root_ports_pls(&self, tag: &str) {
        for port in 1..=self.max_ports() {
            let portsc = self.op.portsc(port);
            let pls = self.op.port_link_state(port);
            log::info!(
                "xhci: {tag} port {port} PORTSC={portsc:#010x} pls={pls} pp={} ccs={}",
                portsc & PORTSC_PP != 0,
                portsc & PORTSC_CCS != 0,
            );
        }
    }

    fn warm_reset_lazy_ports(&mut self) {
        for port in 1..=self.max_ports() {
            let portsc = self.op.portsc(port);
            let pls = self.op.port_link_state(port);
            let need =
                pls == PLS_COMPLIANCE || (portsc & PORTSC_PP != 0 && portsc & PORTSC_CCS == 0);
            if need {
                log::info!("xhci: warm-reset puerto {port} (pls={pls})");
                self.warm_reset_port(port);
            }
        }
    }

    fn clear_port_change_bits(&mut self) {
        for port in 1..=self.max_ports() {
            let portsc = self.op.portsc(port);
            let ch = portsc & PORTSC_CHANGE_BITS;
            if ch != 0 {
                self.write_portsc_masked(port, portsc, ch);
            }
        }
    }

    fn wait_for_ports_connected(&mut self, max_passes: u32) -> bool {
        self.wait_for_boot_ports(max_passes)
    }

    fn current_ccs_mask(&self) -> u32 {
        let mut mask = 0u32;
        for port in 1..=self.max_ports() {
            if self.portsc(port) & PORTSC_CCS != 0 {
                mask |= 1 << (port - 1);
            }
        }
        mask
    }

    fn boot_ports_satisfied(&self) -> bool {
        if self.boot_ccs_mask == 0 {
            self.any_root_port_connected()
        } else {
            (self.current_ccs_mask() & self.boot_ccs_mask) == self.boot_ccs_mask
        }
    }

    fn wait_for_boot_ports(&mut self, max_passes: u32) -> bool {
        let expected = self.boot_ccs_mask.count_ones().max(1);
        for pass in 0..max_passes {
            self.drain_port_events();
            self.clear_pcd();
            self.power_ports();
            if self.boot_ports_satisfied() {
                let now = self.current_ccs_mask().count_ones();
                log::info!(
                    "xhci: puertos root recuperados {now}/{expected} CCS esperados (pass {pass})"
                );
                return true;
            }
            delay_us(50_000);
        }
        false
    }

    fn power_ports_off(&mut self) {
        for port in 1..=self.max_ports {
            let portsc = self.op.portsc(port);
            if portsc & PORTSC_PP != 0 {
                let val = portsc & !PORTSC_CHANGE_BITS & !PORTSC_PP & !PORTSC_PED;
                self.op.set_portsc(port, val);
            }
        }
    }

    /// Warm port reset (WPR) — USB3; ignora si el puerto no lo soporta.
    fn warm_reset_port(&mut self, port: u8) {
        let portsc = self.op.portsc(port);
        self.write_portsc_masked(port, portsc, PORTSC_WPR | PORTSC_PP);
        for _ in 0..500 {
            let ps = self.op.portsc(port);
            if ps & PORTSC_WPR == 0 {
                if ps & PORTSC_WRC != 0 {
                    self.write_portsc_masked(port, ps, PORTSC_WRC);
                }
                break;
            }
            delay_us(1000);
        }
    }

    /// Enciende PP en todos los puertos root (requerido post-HCRST en placa).
    pub fn power_ports(&mut self) {
        for port in 1..=self.max_ports {
            let portsc = self.op.portsc(port);
            if portsc & PORTSC_PP == 0 {
                self.write_portsc_masked(port, portsc, PORTSC_PP);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Port enumeration
    // -----------------------------------------------------------------------

    /// Scan all ports for connected devices and enumerate them (una pasada).
    pub fn enumerate_ports(&mut self) {
        if !self.any_root_port_connected() {
            log::info!("xhci: enumerate_ports omitido (ningún CCS en root)");
            return;
        }
        log::info!("xhci: scanning {} ports for connected devices...", self.max_ports);

        let mut fallos_seguidos = 0u32;
        for port in 1..=self.max_ports() {
            let portsc = self.op.portsc(port);
            let connected = portsc & PORTSC_CCS != 0;
            if !connected {
                continue;
            }
            if self.ignored_ports & (1u32 << (port - 1)) != 0 {
                continue;
            }
            if let Some(slot) = self.root_port_slot.get(port as usize).copied() {
                if slot != 0
                    && self
                        .devices
                        .get(slot as usize)
                        .and_then(|d| d.as_ref())
                        .is_some()
                {
                    continue;
                }
            }
            let speed_code = (portsc & PORTSC_SPEED_MASK) >> PORTSC_SPEED_SHIFT;
            let mut usb_speed = UsbSpeed::from_port_speed(speed_code);
            log::info!(
                "xhci: port {}: device connected PORTSC={portsc:#010x} speed={:?}",
                port, usb_speed
            );

            // USB2: reset siempre aunque PED=1 (estado heredado del UEFI).
            if speed_code < 4 {
                self.reset_port(port);
                let speed_code = self.op.port_speed(port);
                usb_speed = UsbSpeed::from_port_speed(speed_code);
            }

            match self.enable_slot() {
                Some(slot_id) => {
                    fallos_seguidos = 0;
                    if let Some(m) = self.root_port_slot.get_mut(port as usize) {
                        *m = slot_id;
                    }
                    self.initialize_device(slot_id, DevPath::root(port), usb_speed);
                }
                None => {
                    // `None` incluye el timeout de comando de 5 s, no sólo «sin
                    // slots»: abortar aquí dejaba sin enumerar todos los puertos
                    // siguientes. Linux sigue con el resto del bus.
                    fallos_seguidos += 1;
                    log::warn!(
                        "xhci: Enable Slot falló en port {port} ({fallos_seguidos} seguidos)"
                    );
                    if fallos_seguidos >= 2 {
                        log::warn!("xhci: dos Enable Slot seguidos fallando; corto el escaneo");
                        break;
                    }
                }
            }
        }
    }

    /// Una pasada unificada: HID, hubs y mass storage BOT (root o detrás de hub).
    /// Devuelve el primer BOT con capacidad válida; no para al encontrarlo.
    pub fn enumerate_usb_devices(&mut self) -> Option<MassStorage> {
        self.mass_storage = None;
        self.drain_port_events();
        self.wait_polling_ports();
        self.enumerate_ports();
        self.mass_storage
    }

    /// USB2 en Polling/Recovery (p. ej. el stick tras HCRST) aún no tiene PED.
    /// Enumerar ya habla con los FS internos y deja el live a medias.
    fn wait_polling_ports(&mut self) {
        for pass in 0..40 {
            let mut busy = false;
            for port in 1..=self.max_ports() {
                let pls = self.op.port_link_state(port);
                if pls == PLS_POLLING || pls == PLS_RECOVERY || pls == PLS_HOT_RESET {
                    busy = true;
                }
            }
            if !busy {
                if pass > 0 {
                    log::info!("xhci: puertos salieron de polling (pass {pass})");
                    self.log_ports();
                }
                return;
            }
            self.drain_port_events();
            delay_us(50_000);
        }
        self.log_root_ports_pls("aún en polling");
    }

    /// Reset a port to enable it.
    pub(crate) fn reset_port(&mut self, port: u8) {
        log::debug!("xhci: resetting port {}...", port);

        // Debounce (Linux hub_port_debounce ≈ 100 ms).
        delay_us(PORT_DEBOUNCE_US);

        let portsc = self.op.portsc(port);
        // PP + PR; no tocar bits RW1C (escribir 0 = no clear).
        let val = (portsc & !PORTSC_CHANGE_BITS & !PORTSC_PED) | PORTSC_PP | PORTSC_PR;
        self.op.set_portsc(port, val);

        // Wait for reset to complete (PRC set)
        let mut waited = 0u32;
        loop {
            let portsc = self.op.portsc(port);
            if portsc & PORTSC_PRC != 0 {
                self.write_portsc_masked(port, portsc, PORTSC_PRC);
                break;
            }
            delay_us(100);
            waited += 100;
            if waited >= 500_000 {
                log::warn!("xhci: port {} reset PRC timeout", port);
                return;
            }
        }

        // Wait for port enabled (PED=1)
        waited = 0;
        loop {
            let portsc = self.op.portsc(port);
            if portsc & PORTSC_PED != 0 {
                break;
            }
            delay_us(100);
            waited += 100;
            if waited >= 500_000 {
                log::warn!("xhci: port {} PED timeout after reset", port);
                break;
            }
        }

        let portsc = self.op.portsc(port);
        let enabled = portsc & PORTSC_PED != 0;
        let speed = (portsc & PORTSC_SPEED_MASK) >> PORTSC_SPEED_SHIFT;
        let usb_speed = UsbSpeed::from_port_speed(speed);
        delay_us(trst_recovery_us(usb_speed));
        log::info!(
            "xhci: port {} reset complete: enabled={} speed={}",
            port, enabled, speed
        );
    }

    // -----------------------------------------------------------------------
    // Command ring helpers
    // -----------------------------------------------------------------------

    /// Send a command TRB and wait for the completion event.
    /// Returns the completion event TRB, or None on timeout.
    pub(crate) fn send_command(&mut self, trb: Trb) -> Option<Trb> {
        let phys = self.cmd_ring.enqueue(trb);
        self.db.ring_command();

        let mut elapsed = 0u32;
        while elapsed < CMD_TIMEOUT_US {
            if let Some(evt) = self.evt_ring.dequeue() {
                // Update ERDP
                self.rt.set_erdp(0, self.evt_ring.dequeue_phys() | (1 << 3));

                let evt_type = evt.trb_type();

                if evt_type == TRB_TYPE_COMMAND_COMPLETION {
                    let code = evt.completion_code();
                    let slot = evt.slot_id();
                    // El Command TRB Pointer del evento debe apuntar al TRB que
                    // acabamos de encolar. Si no, es la respuesta tardía de un
                    // comando que expiró antes (Linux casa cada evento contra su
                    // cmd_list): tomarla desfasaría todas las respuestas siguientes.
                    if evt.parameter() & !0xF != phys & !0xF {
                        log::warn!(
                            "xhci: command completion huérfano (ptr={:#x}, esperado {:#x}) \
                             code={} ({}) slot={}; hubo un timeout previo",
                            evt.parameter(),
                            phys,
                            code,
                            completion_name(code),
                            slot,
                        );
                        continue;
                    }
                    log::debug!(
                        "xhci: command completion: code={} ({}) slot={}",
                        code,
                        completion_name(code),
                        slot,
                    );
                    return Some(evt);
                } else if evt_type == TRB_TYPE_PORT_STATUS_CHANGE {
                    let port_id = (evt.parameter() >> 24) as u8;
                    log::info!("xhci: port status change event: port={}", port_id);
                    // Could handle hot-plug here; for now continue polling
                    continue;
                } else if evt_type == TRB_TYPE_TRANSFER_EVENT {
                    self.dispatch_event(evt);
                    continue;
                } else {
                    log::trace!("xhci: unexpected event type {} while waiting for command completion", evt_type);
                    continue;
                }
            }

            delay_us(EVENT_POLL_INTERVAL_US);
            elapsed += EVENT_POLL_INTERVAL_US;
        }

        let sts = self.op.usbsts();
        log::warn!(
            "xhci: command completion timeout after {} ms USBSTS={:#x} HCE={}",
            CMD_TIMEOUT_US / 1000,
            sts,
            sts & USBSTS_HCE != 0,
        );
        self.log_ports();
        None
    }

    // -----------------------------------------------------------------------
    // Device slot management
    // -----------------------------------------------------------------------

    /// Issue an Enable Slot command. Returns the allocated slot ID on success.
    pub(crate) fn enable_slot(&mut self) -> Option<u8> {
        log::debug!("xhci: sending Enable Slot command");

        let trb = Trb::enable_slot(false);
        let evt = self.send_command(trb)?;

        if evt.completion_code() != TRB_COMPLETION_SUCCESS {
            let code = evt.completion_code();
            log::warn!(
                "xhci: Enable Slot failed: completion code={} ({})",
                code,
                completion_name(code),
            );
            return None;
        }

        let slot_id = evt.slot_id();
        log::info!("xhci: slot {} enabled", slot_id);

        // Allocate device context in DCBAA
        unsafe {
            self.dcbaa.alloc_device_context(slot_id, self.ctx_size);
        }

        // Initialize transfer ring storage for this slot (32 possible DCIs: 0..31)
        if self.transfer_rings.len() <= slot_id as usize {
            self.transfer_rings.resize_with(slot_id as usize + 1, Vec::new);
        }
        self.transfer_rings[slot_id as usize] = Vec::new();
        for _ in 0..32 {
            self.transfer_rings[slot_id as usize].push(None);
        }

        Some(slot_id)
    }

    /// Liberar un slot tras fallo de address/config (evita agotar MaxSlots).
    pub(crate) fn disable_slot(&mut self, slot_id: u8) {
        if slot_id == 0 || slot_id as usize >= self.devices.len() {
            return;
        }
        let trb = Trb::disable_slot(slot_id, false);
        let _ = self.send_command(trb);
        self.devices[slot_id as usize] = None;
        self.transfer_rings[slot_id as usize] = Vec::new();
        for _ in 0..32 {
            self.transfer_rings[slot_id as usize].push(None);
        }
    }

    /// Initialize a device: Address Device, Get Descriptors, Configure.
    fn initialize_device(&mut self, mut slot_id: u8, path: DevPath, speed: UsbSpeed) {
        log::info!(
            "xhci: initializing device slot={} root_port={} route={:#x} speed={:?}",
            slot_id, path.root_port, path.route, speed
        );

        let mut addressed = false;
        for attempt in 0..MAX_ADDR_RETRIES {
            if attempt > 0 {
                log::info!(
                    "xhci: Address Device retry {}/{} (slot={})",
                    attempt + 1,
                    MAX_ADDR_RETRIES,
                    slot_id
                );
                self.disable_slot(slot_id);
                if path.route == 0 {
                    self.reset_port(path.root_port);
                } else if !self.hub_reset_child_port(path.tt_hub_slot, path.tt_port) {
                    log::warn!(
                        "xhci: hub child reset failed hub_slot={} port={}",
                        path.tt_hub_slot,
                        path.tt_port
                    );
                }
                delay_us(ADDR_RETRY_DELAY_US);
                slot_id = match self.enable_slot() {
                    Some(s) => s,
                    None => {
                        log::warn!("xhci: Enable Slot failed on Address retry");
                        return;
                    }
                };
            }

            self.devices[slot_id as usize] = Some(UsbDevice::new(
                slot_id,
                path.root_port,
                speed,
                path.route,
                path.tt_hub_slot,
                path.tt_port,
            ));

            if self.address_device(slot_id, path, speed) {
                addressed = true;
                break;
            }
            log::warn!(
                "xhci: Address Device failed attempt {} slot {}",
                attempt + 1,
                slot_id
            );
        }

        if !addressed {
            log::warn!(
                "xhci: Address Device failed for slot {} after {} attempts",
                slot_id, MAX_ADDR_RETRIES
            );
            self.disable_slot(slot_id);
            return;
        }

        let mut dev_desc = None;
        for desc_try in 0..GET_DESCRIPTOR_TRIES {
            if desc_try > 0 {
                log::info!(
                    "xhci: GET_DESCRIPTOR full retry {}/{} slot={}",
                    desc_try + 1,
                    GET_DESCRIPTOR_TRIES,
                    slot_id
                );
                self.disable_slot(slot_id);
                if path.route == 0 {
                    self.reset_port(path.root_port);
                } else if !self.hub_reset_child_port(path.tt_hub_slot, path.tt_port) {
                    log::warn!(
                        "xhci: hub child reset failed hub_slot={} port={}",
                        path.tt_hub_slot,
                        path.tt_port
                    );
                }
                delay_us(ADDR_RETRY_DELAY_US);
                slot_id = match self.enable_slot() {
                    Some(s) => s,
                    None => {
                        log::warn!("xhci: Enable Slot failed on GET_DESCRIPTOR retry");
                        return;
                    }
                };
                self.devices[slot_id as usize] = Some(UsbDevice::new(
                    slot_id,
                    path.root_port,
                    speed,
                    path.route,
                    path.tt_hub_slot,
                    path.tt_port,
                ));
                if !self.address_device(slot_id, path, speed) {
                    continue;
                }
            }

            dev_desc = self.get_device_descriptor(slot_id);
            if dev_desc.is_some() {
                break;
            }
        }

        let dev_desc = match dev_desc {
            Some(d) => d,
            None => {
                log::warn!("xhci: failed to get device descriptor for slot {}", slot_id);
                self.disable_slot(slot_id);
                return;
            }
        };

        if let Some(ref mut dev) = self.devices[slot_id as usize] {
            dev.device_desc = Some(dev_desc.clone());
        }

        let parsed_config = match self.get_configuration_descriptor(slot_id, 0) {
            Some(c) => c,
            None => {
                log::warn!("xhci: failed to get config descriptor for slot {}", slot_id);
                self.disable_slot(slot_id);
                return;
            }
        };

        let keyboard_info = parsed_config
            .find_hid_keyboard()
            .map(|(iface_num, ep)| (iface_num, ep.clone()));

        let config_val = parsed_config.config.b_configuration_value;
        let needs_config = parsed_config.needs_full_config(&dev_desc, keyboard_info.is_some());
        if needs_config {
            if !self.set_configuration(slot_id, config_val, &parsed_config, &dev_desc) {
                log::warn!("xhci: SET_CONFIGURATION failed for slot {}", slot_id);
                if dev_desc.is_hub() || keyboard_info.is_some() || parsed_config.is_mass_storage()
                {
                    // Sin liberar el slot, cada dispositivo que falla se come uno
                    // de los MaxSlots para siempre y acaba ahogando la enumeración.
                    self.disable_slot(slot_id);
                    return;
                }
            } else if let Some(ref mut dev) = self.devices[slot_id as usize] {
                dev.config = Some(parsed_config.clone());
                dev.configured = true;
            }
        } else {
            log::info!(
                "xhci: omitiendo SET_CONFIGURATION slot={} VID={:#06x} (no hub/hid/ms)",
                slot_id,
                dev_desc.id_vendor
            );
            self.disable_slot(slot_id);
            if path.root_port != 0 {
                self.ignored_ports |= 1u32 << (path.root_port - 1);
                if let Some(m) = self.root_port_slot.get_mut(path.root_port as usize) {
                    *m = 0;
                }
            }
            return;
        }

        if let Some((iface_num, ref ep_desc)) = keyboard_info {
            log::info!(
                "xhci: setting up HID keyboard on slot={} interface={}",
                slot_id, iface_num
            );
            self.setup_keyboard(slot_id, iface_num, ep_desc);
        }

        if parsed_config.is_mass_storage()
            && self.devices[slot_id as usize]
                .as_ref()
                .is_some_and(|d| d.configured)
        {
            self.try_activate_mass_storage(slot_id, &parsed_config);
        }

        if dev_desc.is_hub() {
            log::info!("xhci: hub en slot={} root_port={}", slot_id, path.root_port);
            self.enumerate_hub_children(slot_id, path.root_port, parsed_config.hub_num_ports);
        }
    }

    /// Issue an Address Device command.
    pub(crate) fn address_device(&mut self, slot_id: u8, path: DevPath, speed: UsbSpeed) -> bool {
        log::debug!(
            "xhci: Address Device slot={} root_port={} route={:#x}",
            slot_id, path.root_port, path.route
        );

        let ctx_size = self.ctx_size;

        let ep0_ring = unsafe { TransferRing::new() };
        let ep0_ring_phys = ep0_ring.phys_addr_with_dcs();
        self.transfer_rings[slot_id as usize][1] = Some(ep0_ring);

        let input_ctx = unsafe { InputContext::new(ctx_size) };
        input_ctx.set_add_flags(0x3);

        let mut slot = SlotContext::new(ctx_size);
        slot.set_route_string(path.route)
            .set_speed(speed.to_slot_speed())
            .set_context_entries(1)
            .set_root_hub_port(path.root_port);
        if path.tt_hub_slot != 0 {
            slot.set_tt(path.tt_hub_slot, path.tt_port);
        }
        input_ctx.write_slot_context(&slot);

        // EP0 Context
        let mut ep0 = EndpointContext::new(ctx_size);
        let max_pkt = speed.default_max_packet_size0();
        ep0.set_ep_type(EP_TYPE_CONTROL)
            .set_max_packet_size(max_pkt)
            .set_cerr(3)
            .set_tr_dequeue_pointer(ep0_ring_phys)
            .set_average_trb_length(8); // Control transfers average ~8 bytes
        input_ctx.write_endpoint_context(1, &ep0);

        log::debug!(
            "xhci: Address Device input ctx at {:#x}: speed={} max_pkt={} ep0_ring={:#x}",
            input_ctx.phys_addr(), speed.to_slot_speed(), max_pkt, ep0_ring_phys,
        );

        // Send command
        let trb = Trb::address_device(input_ctx.phys_addr(), slot_id, false, false);
        match self.send_command(trb) {
            Some(evt) if evt.completion_code() == TRB_COMPLETION_SUCCESS => {
                log::info!("xhci: slot {} addressed successfully", slot_id);
                delay_us(SET_ADDRESS_SETTLE_US);
                true
            }
            Some(evt) => {
                let code = evt.completion_code();
                log::warn!(
                    "xhci: Address Device failed for slot {}: code={} ({})",
                    slot_id,
                    code,
                    completion_name(code),
                );
                false
            }
            None => {
                log::warn!("xhci: Address Device timeout for slot {}", slot_id);
                false
            }
        }
    }

    /// GET_DESCRIPTOR IN estándar en EP0.
    fn ep0_get_descriptor_in(
        &mut self,
        slot_id: u8,
        desc_type: u8,
        desc_index: u8,
        buf_phys: u64,
        length: u16,
    ) -> bool {
        let handles = {
            let ring = match self.transfer_rings[slot_id as usize][1].as_mut() {
                Some(r) => r,
                None => return false,
            };
            ring.enqueue_control_transfer(
                USB_DIR_IN | USB_TYPE_STANDARD | USB_RECIP_DEVICE,
                USB_REQ_GET_DESCRIPTOR,
                ((desc_type as u16) << 8) | (desc_index as u16),
                0,
                buf_phys,
                length,
            )
        };
        self.db.ring_endpoint(slot_id, 1);
        match self.wait_transfer_event_timeout(
            slot_id,
            Some(handles.status_trb_phys),
            EP0_TRANSFER_TIMEOUT_US,
        ) {
            Some(evt) if evt.completion_code() == TRB_COMPLETION_SUCCESS
                || evt.completion_code() == TRB_COMPLETION_SHORT_PACKET =>
            {
                true
            }
            Some(evt) => {
                let code = evt.completion_code();
                log::warn!(
                    "xhci: GET_DESCRIPTOR(type={desc_type}) failed: code={code}"
                );
                if Self::ep0_needs_recover(code) {
                    let _ = self.recover_ep0(slot_id);
                }
                false
            }
            None => false,
        }
    }

    fn ep0_needs_recover(code: u8) -> bool {
        code == TRB_COMPLETION_BABBLE
            || code == TRB_COMPLETION_USB_TRANSACTION_ERROR
            || code == TRB_COMPLETION_STALL
    }

    /// Recupera EP0 halted: Reset Endpoint + Set TR Dequeue (Linux xhci_cleanup_halted_endpoint).
    fn recover_ep0(&mut self, slot_id: u8) -> bool {
        self.reset_and_requeue_ep(slot_id, EP0_DCI)
    }

    fn set_ep_dequeue(&mut self, slot_id: u8, dci: u8) -> bool {
        let dequeue = match self
            .transfer_rings
            .get(slot_id as usize)
            .and_then(|eps| eps.get(dci as usize))
            .and_then(|r| r.as_ref())
        {
            Some(r) => r.enqueue_phys_with_dcs(),
            None => return false,
        };
        let trb = Trb::set_tr_dequeue(dequeue, slot_id, dci, false);
        match self.send_command(trb) {
            Some(evt) if evt.completion_code() == TRB_COMPLETION_SUCCESS => {
                log::info!("xhci: EP recovered slot={slot_id} dci={dci}");
                true
            }
            Some(evt) => {
                log::warn!(
                    "xhci: Set TR Dequeue slot={slot_id} dci={dci}: code={}",
                    evt.completion_code()
                );
                false
            }
            None => {
                log::warn!("xhci: Set TR Dequeue timeout slot={slot_id} dci={dci}");
                false
            }
        }
    }

    fn reset_and_requeue_ep(&mut self, slot_id: u8, dci: u8) -> bool {
        let trb = Trb::reset_endpoint(slot_id, dci, false);
        match self.send_command(trb) {
            Some(evt) if evt.completion_code() == TRB_COMPLETION_SUCCESS => {}
            Some(evt) => {
                log::warn!(
                    "xhci: Reset EP slot={slot_id} dci={dci}: code={}",
                    evt.completion_code()
                );
                return false;
            }
            None => {
                log::warn!("xhci: Reset EP timeout slot={slot_id} dci={dci}");
                return false;
            }
        }
        self.set_ep_dequeue(slot_id, dci)
    }

    fn drop_pending_ep(&mut self, slot_id: u8, dci: u8) {
        self.pending_transfers.retain(|evt| {
            !(evt.trb_type() == TRB_TYPE_TRANSFER_EVENT
                && evt.slot_id() == slot_id
                && evt.endpoint_id() == dci)
        });
    }

    /// Timeout: el EP sigue Running con TDs viejos. Sin Stop+requeue, cada
    /// reintento (hub GET_STATUS × 500, BOT, GET_DESCRIPTOR) vuelve a expirar
    /// y el live se queda imprimiendo PORTSC.
    fn recover_timeout_ep(&mut self, slot_id: u8, dci: u8) {
        self.drop_pending_ep(slot_id, dci);
        let trb = Trb::stop_endpoint(slot_id, dci, false);
        match self.send_command(trb) {
            Some(evt)
                if evt.completion_code() == TRB_COMPLETION_SUCCESS
                    || evt.completion_code() == TRB_COMPLETION_STOPPED
                    || evt.completion_code() == TRB_COMPLETION_STOPPED_LENGTH_INVALID =>
            {
                let _ = self.set_ep_dequeue(slot_id, dci);
            }
            Some(evt) if evt.completion_code() == TRB_COMPLETION_CONTEXT_STATE_ERROR => {
                let _ = self.reset_and_requeue_ep(slot_id, dci);
            }
            Some(evt) => {
                log::warn!(
                    "xhci: Stop EP slot={slot_id} dci={dci}: code={} ({})",
                    evt.completion_code(),
                    completion_name(evt.completion_code()),
                );
                let _ = self.reset_and_requeue_ep(slot_id, dci);
            }
            None => {
                log::warn!("xhci: Stop EP timeout slot={slot_id} dci={dci}");
            }
        }
        self.drop_pending_ep(slot_id, dci);
    }

    /// Actualiza max packet size de EP0 tras leer bMaxPacketSize0 (Linux usb_get_device_descriptor).
    fn evaluate_ep0_max_packet(&mut self, slot_id: u8, max_pkt: u16) -> bool {
        let ep0_ring_phys = match self.transfer_rings[slot_id as usize][1].as_ref() {
            Some(r) => r.phys_addr_with_dcs(),
            None => return false,
        };

        let input_ctx = unsafe { InputContext::new(self.ctx_size) };
        input_ctx.set_add_flags(1 << 1); // EP0 (DCI 1)

        let mut ep0 = EndpointContext::new(self.ctx_size);
        ep0.set_ep_type(EP_TYPE_CONTROL)
            .set_max_packet_size(max_pkt)
            .set_cerr(3)
            .set_tr_dequeue_pointer(ep0_ring_phys)
            .set_average_trb_length(8);
        input_ctx.write_endpoint_context(1, &ep0);

        let trb = Trb::evaluate_context(input_ctx.phys_addr(), slot_id, false);
        match self.send_command(trb) {
            Some(evt) if evt.completion_code() == TRB_COMPLETION_SUCCESS => {
                log::info!("xhci: EP0 max_packet_size={max_pkt} slot={slot_id}");
                true
            }
            Some(evt) => {
                log::warn!(
                    "xhci: Evaluate Context EP0 failed: code={}",
                    evt.completion_code()
                );
                false
            }
            None => false,
        }
    }

    /// Issue GET_DESCRIPTOR(Device) on EP0.
    pub(crate) fn get_device_descriptor(&mut self, slot_id: u8) -> Option<DeviceDescriptor> {
        log::debug!("xhci: GET_DESCRIPTOR(Device) slot={}", slot_id);

        // Paso 1: leer 8 bytes (incluye bMaxPacketSize0 en offset 7), con reintentos.
        let (header_va, header_phys) = unsafe { alloc_dma_buffer(8) };
        let mut header_ok = false;
        for attempt in 0..GET_DESCRIPTOR_RETRIES {
            if attempt > 0 {
                delay_us(GET_DESCRIPTOR_RETRY_DELAY_US);
            }
            if self.ep0_get_descriptor_in(slot_id, USB_DESC_DEVICE, 0, header_phys, 8) {
                header_ok = true;
                break;
            }
        }
        if !header_ok {
            log::warn!("xhci: GET_DESCRIPTOR(Device) header failed for slot {slot_id}");
            return None;
        }
        let header = unsafe { read_dma_buffer(header_va, 8) };
        if header.len() < 8 {
            return None;
        }

        let max_pkt0 = header[7] as u16;
        if max_pkt0 != 0 && max_pkt0 != 8 {
            let _ = self.evaluate_ep0_max_packet(slot_id, max_pkt0);
        }

        // Paso 2: leer descriptor completo.
        let total_len = header[0].max(DeviceDescriptor::SIZE as u8) as usize;
        let (buf_va, buf_phys) = unsafe { alloc_dma_buffer(total_len) };
        let mut full_ok = false;
        for attempt in 0..GET_DESCRIPTOR_RETRIES {
            if attempt > 0 {
                delay_us(GET_DESCRIPTOR_RETRY_DELAY_US);
            }
            if self.ep0_get_descriptor_in(slot_id, USB_DESC_DEVICE, 0, buf_phys, total_len as u16)
            {
                full_ok = true;
                break;
            }
        }
        if !full_ok {
            log::warn!("xhci: GET_DESCRIPTOR(Device) full failed for slot {slot_id}");
            return None;
        }

        let data = unsafe { read_dma_buffer(buf_va, total_len) };
        DeviceDescriptor::parse(&data)
    }

    /// Issue GET_DESCRIPTOR(Configuration) on EP0.
    pub(crate) fn get_configuration_descriptor(
        &mut self,
        slot_id: u8,
        config_index: u8,
    ) -> Option<ParsedConfiguration> {
        log::debug!(
            "xhci: GET_DESCRIPTOR(Configuration, index={}) slot={}",
            config_index, slot_id
        );

        // First, get just the header to learn wTotalLength
        let header_size = 9;
        let (hdr_va, hdr_phys) = unsafe { alloc_dma_buffer(header_size) };

        let ring = self.transfer_rings[slot_id as usize][1].as_mut()?;
        let handles = ring.enqueue_control_transfer(
            USB_DIR_IN | USB_TYPE_STANDARD | USB_RECIP_DEVICE,
            USB_REQ_GET_DESCRIPTOR,
            (USB_DESC_CONFIGURATION as u16) << 8 | config_index as u16,
            0,
            hdr_phys,
            header_size as u16,
        );
        self.db.ring_endpoint(slot_id, 1);

        let evt = self.wait_transfer_event(slot_id, Some(handles.status_trb_phys))?;
        if evt.completion_code() != TRB_COMPLETION_SUCCESS
            && evt.completion_code() != TRB_COMPLETION_SHORT_PACKET
        {
            log::warn!(
                "xhci: GET_DESCRIPTOR(Config header) failed: code={}",
                evt.completion_code()
            );
            return None;
        }

        let hdr_data = unsafe { read_dma_buffer(hdr_va, header_size) };
        let total_len = u16::from_le_bytes([hdr_data[2], hdr_data[3]]) as usize;
        log::debug!("xhci: config descriptor total length = {}", total_len);

        // Now get the full descriptor set
        let (full_va, full_phys) = unsafe { alloc_dma_buffer(total_len) };

        let ring = self.transfer_rings[slot_id as usize][1].as_mut()?;
        let handles = ring.enqueue_control_transfer(
            USB_DIR_IN | USB_TYPE_STANDARD | USB_RECIP_DEVICE,
            USB_REQ_GET_DESCRIPTOR,
            (USB_DESC_CONFIGURATION as u16) << 8 | config_index as u16,
            0,
            full_phys,
            total_len as u16,
        );
        self.db.ring_endpoint(slot_id, 1);

        let evt = self.wait_transfer_event(slot_id, Some(handles.status_trb_phys))?;
        if evt.completion_code() != TRB_COMPLETION_SUCCESS
            && evt.completion_code() != TRB_COMPLETION_SHORT_PACKET
        {
            log::warn!(
                "xhci: GET_DESCRIPTOR(Config full) failed: code={}",
                evt.completion_code()
            );
            return None;
        }

        let full_data = unsafe { read_dma_buffer(full_va, total_len) };
        ParsedConfiguration::parse(&full_data)
    }

    /// Issue SET_CONFIGURATION and Configure Endpoint command.
    pub(crate) fn set_configuration(
        &mut self,
        slot_id: u8,
        config_value: u8,
        config: &ParsedConfiguration,
        dev_desc: &DeviceDescriptor,
    ) -> bool {
        log::debug!(
            "xhci: SET_CONFIGURATION slot={} value={}",
            slot_id, config_value
        );

        // First, send the USB SET_CONFIGURATION request on EP0
        let ring = match self.transfer_rings[slot_id as usize][1].as_mut() {
            Some(r) => r,
            None => return false,
        };
        let handles = ring.enqueue_control_transfer(
            USB_DIR_OUT | USB_TYPE_STANDARD | USB_RECIP_DEVICE,
            USB_REQ_SET_CONFIGURATION,
            config_value as u16,
            0,
            0,
            0,
        );
        self.db.ring_endpoint(slot_id, 1);

        match self.wait_transfer_event(slot_id, Some(handles.status_trb_phys)) {
            Some(evt) if evt.completion_code() == TRB_COMPLETION_SUCCESS => {
                log::debug!("xhci: SET_CONFIGURATION USB request succeeded");
            }
            Some(evt) => {
                log::warn!(
                    "xhci: SET_CONFIGURATION USB request failed: code={}",
                    evt.completion_code()
                );
                return false;
            }
            None => {
                log::warn!("xhci: SET_CONFIGURATION timeout");
                return false;
            }
        }

        // Now issue xHCI Configure Endpoint command with all non-EP0 endpoints
        let ctx_size = self.ctx_size;
        let input_ctx = unsafe { InputContext::new(ctx_size) };

        // Calculate the highest DCI we need
        let mut max_dci: u8 = 1; // At least EP0

        // Add flags start with Slot Context (bit 0)
        let mut add_flags: u32 = 1; // Bit 0 = Slot Context

        let active: alloc::vec::Vec<(u8, &EndpointDescriptor)> =
            config.active_endpoints().collect();
        log::info!(
            "xhci: Configure Endpoint slot={} eps={} (alt0 only)",
            slot_id,
            active.len()
        );

        let speed = self.devices[slot_id as usize]
            .as_ref()
            .map(|d| d.speed)
            .unwrap_or(UsbSpeed::Unknown);

        // DCIs cuyo anillo instalamos: hay que revertirlos si el HC rechaza el
        // Configure Endpoint, o quedarían anillos vivos para endpoints que no existen.
        let mut dcis_instalados: alloc::vec::Vec<u8> = alloc::vec::Vec::new();

        for (_iface_num, ep_desc) in &active {
            let dci = ep_desc.dci();
            if dci > max_dci {
                max_dci = dci;
            }
            add_flags |= 1 << dci;

            // Allocate transfer ring for this endpoint
            let tr = unsafe { TransferRing::new() };
            let tr_phys = tr.phys_addr_with_dcs();

            let max_pkt = ep_desc.xhci_max_packet_size();
            let interval = ep_desc.xhci_interval(speed);
            let mult = ep_desc.xhci_mult(speed);
            // xHCI 6.2.3.2: CErr «shall be set to 0 for Isoch endpoints».
            // Linux: `if (!usb_endpoint_xfer_isoc(&ep->desc)) err_count = 3;`
            let cerr = if ep_desc.is_isoch() { 0 } else { 3 };

            // Build endpoint context
            let mut ep_ctx = EndpointContext::new(ctx_size);
            ep_ctx
                .set_ep_type(ep_desc.xhci_ep_type())
                .set_max_packet_size(max_pkt)
                .set_mult(mult)
                .set_cerr(cerr)
                .set_interval(interval)
                .set_tr_dequeue_pointer(tr_phys)
                .set_average_trb_length(if ep_desc.is_interrupt() { 8 } else { 1024 });

            input_ctx.write_endpoint_context(dci, &ep_ctx);

            log::debug!(
                "xhci: configure EP DCI={} type={} max_pkt={} mult={} interval={} cerr={} ring={:#x}",
                dci,
                ep_desc.xhci_ep_type(),
                max_pkt,
                mult,
                interval,
                cerr,
                tr_phys,
            );

            // Ensure storage
            while self.transfer_rings[slot_id as usize].len() <= dci as usize {
                self.transfer_rings[slot_id as usize].push(None);
            }
            self.transfer_rings[slot_id as usize][dci as usize] = Some(tr);
            dcis_instalados.push(dci);
        }

        input_ctx.set_add_flags(add_flags);

        // Slot Context: partir del output (conserva USB Device Address) y actualizar entries.
        let (port, route, tt_hub, tt_port) = self.devices[slot_id as usize]
            .as_ref()
            .map(|d| (d.port, d.route_string, d.tt_hub_slot, d.tt_port))
            .unwrap_or((0, 0, 0, 0));

        let mut slot = unsafe {
            self.dcbaa
                .read_slot_context(slot_id, ctx_size)
                .unwrap_or_else(|| {
                    let mut s = SlotContext::new(ctx_size);
                    s.set_route_string(route)
                        .set_speed(speed.to_slot_speed())
                        .set_root_hub_port(port);
                    if tt_hub != 0 {
                        s.set_tt(tt_hub, tt_port);
                    }
                    s
                })
        };
        slot.set_context_entries(max_dci);
        if dev_desc.is_hub() {
            slot.set_hub(true);
            if let Some(n) = config.hub_num_ports {
                slot.set_num_ports(n);
            }
        }
        input_ctx.write_slot_context(&slot);

        log::debug!(
            "xhci: Configure Endpoint: add_flags={:#x} max_dci={}",
            add_flags, max_dci
        );

        let trb = Trb::configure_endpoint(input_ctx.phys_addr(), slot_id, false);
        let ok = match self.send_command(trb) {
            Some(evt) if evt.completion_code() == TRB_COMPLETION_SUCCESS => {
                log::info!("xhci: slot {} configured successfully", slot_id);
                true
            }
            Some(evt) => {
                let code = evt.completion_code();
                log::warn!(
                    "xhci: Configure Endpoint failed for slot {}: code={} ({})",
                    slot_id,
                    code,
                    completion_name(code),
                );
                false
            }
            None => {
                log::warn!("xhci: Configure Endpoint timeout for slot {}", slot_id);
                false
            }
        };

        if !ok {
            // El HC no configuró esos endpoints: dejar los anillos puestos haría
            // que un ring_endpoint() posterior tocase un DCI inexistente.
            for dci in dcis_instalados {
                self.transfer_rings[slot_id as usize][dci as usize] = None;
            }
        }
        ok
    }

    // -----------------------------------------------------------------------
    // HID Keyboard setup
    // -----------------------------------------------------------------------

    /// Set up a HID keyboard: SET_PROTOCOL(Boot), SET_IDLE, start interrupt transfers.
    fn setup_keyboard(&mut self, slot_id: u8, iface_num: u8, ep_desc: &EndpointDescriptor) {
        let dci = ep_desc.dci();

        log::info!(
            "xhci: keyboard setup: slot={} iface={} ep_addr={:#x} dci={} max_pkt={}",
            slot_id, iface_num, ep_desc.b_endpoint_address, dci, ep_desc.w_max_packet_size
        );

        // SET_PROTOCOL(Boot Protocol = 0)
        log::debug!("xhci: SET_PROTOCOL(Boot) on interface {}", iface_num);
        if let Some(ring) = self.transfer_rings[slot_id as usize][1].as_mut() {
            let handles = ring.enqueue_control_transfer(
                USB_DIR_OUT | USB_TYPE_CLASS | USB_RECIP_INTERFACE,
                HID_REQ_SET_PROTOCOL,
                HID_PROTOCOL_BOOT,
                iface_num as u16,
                0,
                0,
            );
            self.db.ring_endpoint(slot_id, 1);

            match self.wait_transfer_event(slot_id, Some(handles.status_trb_phys)) {
                Some(evt) if evt.completion_code() == TRB_COMPLETION_SUCCESS => {
                    log::info!("xhci: SET_PROTOCOL(Boot) succeeded");
                }
                Some(evt) => {
                    log::warn!("xhci: SET_PROTOCOL(Boot) failed: code={}", evt.completion_code());
                }
                None => {
                    log::warn!("xhci: SET_PROTOCOL(Boot) timeout");
                }
            }
        }

        // SET_IDLE(0) — don't wait for changes, report constantly
        log::debug!("xhci: SET_IDLE(0) on interface {}", iface_num);
        if let Some(ring) = self.transfer_rings[slot_id as usize][1].as_mut() {
            let handles = ring.enqueue_control_transfer(
                USB_DIR_OUT | USB_TYPE_CLASS | USB_RECIP_INTERFACE,
                HID_REQ_SET_IDLE,
                0, // duration=0, report_id=0
                iface_num as u16,
                0,
                0,
            );
            self.db.ring_endpoint(slot_id, 1);

            match self.wait_transfer_event(slot_id, Some(handles.status_trb_phys)) {
                Some(evt) if evt.completion_code() == TRB_COMPLETION_SUCCESS => {
                    log::debug!("xhci: SET_IDLE succeeded");
                }
                Some(evt) => {
                    // Some keyboards STALL on SET_IDLE, which is fine
                    log::debug!(
                        "xhci: SET_IDLE returned code={} (may be STALL, continuing)",
                        evt.completion_code()
                    );
                }
                None => {
                    log::debug!("xhci: SET_IDLE timeout (continuing anyway)");
                }
            }
        }

        // Allocate report buffer for interrupt transfers (≥ max packet del EP)
        let report_len = ep_desc.xhci_max_packet_size().max(8) as usize;
        let (report_va, report_phys) = unsafe { alloc_dma_buffer(report_len) };

        // Queue the first interrupt IN transfer
        if let Some(ring) = self.transfer_rings[slot_id as usize].get_mut(dci as usize) {
            if let Some(ring) = ring.as_mut() {
                ring.enqueue_interrupt_in(report_phys, report_len as u32);
                self.db.ring_endpoint(slot_id, dci);
                log::debug!("xhci: first keyboard interrupt transfer queued");
            }
        }

        // Record keyboard info
        if let Some(ref mut dev) = self.devices[slot_id as usize] {
            dev.keyboard_interface = Some(iface_num);
            dev.keyboard_endpoint_dci = Some(dci);
        }

        self.keyboards.push(KeyboardInfo {
            slot_id,
            dci,
            report_buf_va: report_va,
            report_buf_phys: report_phys,
            report_buf_len: report_len,
            state: KeyboardState::new(),
            transfer_pending: true,
        });

        log::info!("xhci: keyboard ready on slot={} dci={}", slot_id, dci);
    }

    // -----------------------------------------------------------------------
    // Event ring dispatch (un solo consumidor del anillo HW)
    // -----------------------------------------------------------------------

    fn dequeue_hw_event(&mut self) -> Option<Trb> {
        let evt = self.evt_ring.dequeue()?;
        self.rt
            .set_erdp(0, self.evt_ring.dequeue_phys() | (1 << 3));
        Some(evt)
    }

    fn clear_event_interrupt(&mut self) {
        let iman = self.rt.iman(0);
        if iman & IMAN_IP != 0 {
            self.rt.set_iman(0, iman | IMAN_IP);
        }
    }

    fn keyboard_index(&self, slot_id: u8, ep_id: u8) -> Option<usize> {
        self.keyboards
            .iter()
            .position(|kb| kb.slot_id == slot_id && kb.dci == ep_id)
    }

    fn queue_pending(&mut self, evt: Trb) {
        if self.pending_transfers.len() >= MAX_PENDING_TRANSFERS {
            log::warn!(
                "xhci: cola de transferencias llena; descarto slot={} ep={}",
                evt.slot_id(),
                evt.endpoint_id()
            );
            return;
        }
        self.pending_transfers.push(evt);
    }

    fn handle_keyboard_transfer(&mut self, idx: usize, evt: &Trb) {
        let code = evt.completion_code();
        let (slot_id, dci, report_va, report_len, buf_phys) = {
            let kb = &self.keyboards[idx];
            (
                kb.slot_id,
                kb.dci,
                kb.report_buf_va,
                kb.report_buf_len,
                kb.report_buf_phys,
            )
        };

        log::trace!(
            "xhci: keyboard transfer event: slot={} code={} residual={}",
            slot_id,
            code,
            evt.transfer_length()
        );

        if code == TRB_COMPLETION_SUCCESS || code == TRB_COMPLETION_SHORT_PACKET {
            let report_data = unsafe { read_dma_buffer(report_va, report_len) };
            if let Some(report) = BootKeyboardReport::parse(&report_data) {
                log::trace!(
                    "xhci: keyboard report: mods={:#x} keys=[{:#x},{:#x},{:#x},{:#x},{:#x},{:#x}]",
                    report.modifiers,
                    report.keycodes[0],
                    report.keycodes[1],
                    report.keycodes[2],
                    report.keycodes[3],
                    report.keycodes[4],
                    report.keycodes[5],
                );
                self.keyboards[idx].state.process_report(&report);
            }
        } else {
            log::warn!("xhci: keyboard transfer error: code={}", code);
        }

        if let Some(ring) = self.transfer_rings[slot_id as usize]
            .get_mut(dci as usize)
            .and_then(|r| r.as_mut())
        {
            ring.enqueue_interrupt_in(buf_phys, report_len as u32);
            self.db.ring_endpoint(slot_id, dci);
        }
        self.keyboards[idx].transfer_pending = true;
    }

    /// Despacha un evento del anillo: teclado → rearma IN; otro slot → cola.
    fn dispatch_event(&mut self, evt: Trb) {
        let evt_type = evt.trb_type();
        if evt_type == TRB_TYPE_TRANSFER_EVENT {
            let slot = evt.slot_id();
            let ep = evt.endpoint_id();
            if let Some(idx) = self.keyboard_index(slot, ep) {
                self.handle_keyboard_transfer(idx, &evt);
            } else {
                self.queue_pending(evt);
            }
        } else if evt_type == TRB_TYPE_PORT_STATUS_CHANGE {
            let port_id = (evt.parameter() >> 24) as u8;
            log::info!("xhci: port status change event: port={}", port_id);
        } else {
            log::trace!("xhci: unhandled event type {}", evt_type);
        }
    }

    fn drain_hw_events(&mut self) {
        while let Some(evt) = self.dequeue_hw_event() {
            self.dispatch_event(evt);
        }
        self.clear_event_interrupt();
    }

    /// ¿Este transfer event satisface la espera activa?
    fn transfer_matches_wait(
        evt: &Trb,
        expected_slot: u8,
        expected_dci: u8,
        status_trb_phys: Option<u64>,
    ) -> bool {
        if evt.trb_type() != TRB_TYPE_TRANSFER_EVENT || evt.slot_id() != expected_slot {
            return false;
        }
        if evt.endpoint_id() != expected_dci {
            return false;
        }
        let code = evt.completion_code();
        let evt_ptr = evt.parameter() & !0xF;
        match status_trb_phys {
            None => true,
            Some(status_phys) => {
                let status_ptr = status_phys & !0xF;
                if evt_ptr == status_ptr {
                    return true;
                }
                if expected_dci == EP0_DCI {
                    if code == TRB_COMPLETION_SHORT_PACKET {
                        return false;
                    }
                    if code != TRB_COMPLETION_SUCCESS {
                        return true;
                    }
                    return false;
                }
                // Bulk/interrupt: un TD activo por EP; el puntero del evento
                // puede no coincidir byte a byte con el TRB encolado.
                true
            }
        }
    }

    /// EP0 en curso: el evento se consumió del anillo pero aún no es la respuesta.
    fn ep0_wait_continues(evt: &Trb, status_trb_phys: Option<u64>) -> bool {
        status_trb_phys.is_some()
            && evt.trb_type() == TRB_TYPE_TRANSFER_EVENT
            && evt.endpoint_id() == EP0_DCI
            && (evt.completion_code() == TRB_COMPLETION_SHORT_PACKET
                || evt.completion_code() == TRB_COMPLETION_SUCCESS)
    }

    fn take_matching_pending(
        &mut self,
        expected_slot: u8,
        expected_dci: u8,
        status_trb_phys: Option<u64>,
    ) -> Option<Trb> {
        let mut i = 0;
        while i < self.pending_transfers.len() {
            let evt = self.pending_transfers[i];
            if Self::transfer_matches_wait(&evt, expected_slot, expected_dci, status_trb_phys) {
                return Some(self.pending_transfers.remove(i));
            }
            if Self::ep0_wait_continues(&evt, status_trb_phys) && evt.slot_id() == expected_slot {
                i += 1;
                continue;
            }
            i += 1;
        }
        None
    }

    fn try_consume_for_wait(
        &mut self,
        evt: Trb,
        expected_slot: u8,
        expected_dci: u8,
        status_trb_phys: Option<u64>,
    ) -> Option<Trb> {
        if evt.trb_type() != TRB_TYPE_TRANSFER_EVENT {
            self.dispatch_event(evt);
            return None;
        }
        if Self::transfer_matches_wait(&evt, expected_slot, expected_dci, status_trb_phys) {
            return Some(evt);
        }
        if Self::ep0_wait_continues(&evt, status_trb_phys) && evt.slot_id() == expected_slot {
            log::trace!("xhci: EP0 event before status, waiting");
            return None;
        }
        self.dispatch_event(evt);
        None
    }

    fn next_buffered_key(&mut self) -> Option<KeyEvent> {
        for kb in &mut self.keyboards {
            if let Some(evt) = kb.state.next_event() {
                return Some(evt);
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // Keyboard polling
    // -----------------------------------------------------------------------

    /// Poll for keyboard events. Call this from the kernel's main loop or
    /// interrupt handler. Returns the next key event, if any.
    pub fn poll_keyboard(&mut self) -> Option<KeyEvent> {
        if self.keyboards.is_empty() {
            return None;
        }
        if let Some(evt) = self.next_buffered_key() {
            return Some(evt);
        }
        self.drain_hw_events();
        self.next_buffered_key()
    }

    /// Check if a keyboard has been found and initialized.
    pub fn has_keyboard(&self) -> bool {
        !self.keyboards.is_empty()
    }

    // -----------------------------------------------------------------------
    // Transfer event waiting
    // -----------------------------------------------------------------------

    /// Wait for a transfer event. Con `status_trb_phys`, espera el evento del Status
    /// TRB de un control transfer (tolerando short-packet del Data Stage).
    pub(crate) fn wait_transfer_event(
        &mut self,
        expected_slot: u8,
        status_trb_phys: Option<u64>,
    ) -> Option<Trb> {
        self.wait_transfer_on(expected_slot, EP0_DCI, status_trb_phys)
    }

    pub(crate) fn wait_transfer_event_timeout(
        &mut self,
        expected_slot: u8,
        status_trb_phys: Option<u64>,
        timeout_us: u32,
    ) -> Option<Trb> {
        self.wait_transfer_on_timeout(expected_slot, EP0_DCI, status_trb_phys, timeout_us)
    }

    pub(crate) fn wait_transfer_on(
        &mut self,
        expected_slot: u8,
        dci: u8,
        status_trb_phys: Option<u64>,
    ) -> Option<Trb> {
        self.wait_transfer_on_timeout(
            expected_slot,
            dci,
            status_trb_phys,
            TRANSFER_TIMEOUT_US,
        )
    }

    pub(crate) fn wait_transfer_on_timeout(
        &mut self,
        expected_slot: u8,
        dci: u8,
        status_trb_phys: Option<u64>,
        timeout_us: u32,
    ) -> Option<Trb> {
        let mut elapsed = 0u32;
        while elapsed < timeout_us {
            if let Some(evt) = self.take_matching_pending(expected_slot, dci, status_trb_phys) {
                return Some(evt);
            }

            if let Some(evt) = self.dequeue_hw_event() {
                if let Some(matched) =
                    self.try_consume_for_wait(evt, expected_slot, dci, status_trb_phys)
                {
                    return Some(matched);
                }
                continue;
            }

            delay_us(EVENT_POLL_INTERVAL_US);
            elapsed += EVENT_POLL_INTERVAL_US;
        }

        let sts = self.op.usbsts();
        log::warn!(
            "xhci: transfer event timeout after {} ms slot={expected_slot} dci={dci} USBSTS={:#x} HCE={}",
            timeout_us / 1000,
            sts,
            sts & USBSTS_HCE != 0,
        );
        if !self.timeout_ports_logged {
            self.log_ports();
            self.timeout_ports_logged = true;
        }
        self.recover_timeout_ep(expected_slot, dci);
        None
    }

    pub(crate) fn set_device(&mut self, slot_id: u8, path: DevPath, speed: UsbSpeed) {
        self.devices[slot_id as usize] = Some(UsbDevice::new(
            slot_id,
            path.root_port,
            speed,
            path.route,
            path.tt_hub_slot,
            path.tt_port,
        ));
    }

    // -----------------------------------------------------------------------
    // Helpers for mass storage
    // -----------------------------------------------------------------------

    pub fn max_ports(&self) -> u8 {
        self.max_ports
    }

    pub fn portsc(&self, port: u8) -> u32 {
        self.op.portsc(port)
    }

    pub fn port_speed(&self, port: u8) -> UsbSpeed {
        UsbSpeed::from_port_speed(self.op.port_speed(port))
    }

    /// Procesa eventos pendientes en el anillo (re-conexión, teclado, cola).
    pub fn drain_port_events(&mut self) {
        self.drain_hw_events();
    }

    /// Estado de cada puerto root (diagnóstico en placa).
    pub fn log_ports(&self) {
        for port in 1..=self.max_ports() {
            let portsc = self.portsc(port);
            log::info!(
                "xhci: port {port}: PORTSC={portsc:#010x} ccs={} ped={}",
                portsc & PORTSC_CCS != 0,
                portsc & PORTSC_PED != 0,
            );
        }
    }

    pub(crate) fn transfer_ring(&mut self, slot_id: u8, dci: u8) -> Option<&mut TransferRing> {
        self.transfer_rings
            .get_mut(slot_id as usize)?
            .get_mut(dci as usize)?
            .as_mut()
    }

    pub(crate) fn ring_ep(&self, slot_id: u8, dci: u8) {
        self.db.ring_endpoint(slot_id, dci);
    }

    fn get_hub_port_count(&mut self, hub_slot: u8) -> Option<u8> {
        let (va, phys) = unsafe { alloc_dma_buffer(16) };
        let ring = self.transfer_rings[hub_slot as usize][1].as_mut()?;
        // Linux usa bmRequestType 0xA0 (IN | CLASS | DEVICE), no STANDARD.
        let handles = ring.enqueue_control_transfer(
            USB_DIR_IN | USB_TYPE_CLASS | USB_RECIP_DEVICE,
            USB_REQ_GET_DESCRIPTOR,
            (USB_DESC_HUB as u16) << 8,
            0,
            phys,
            16,
        );
        self.db.ring_endpoint(hub_slot, 1);
        let evt = self.wait_transfer_event(hub_slot, Some(handles.status_trb_phys))?;
        if evt.completion_code() != TRB_COMPLETION_SUCCESS
            && evt.completion_code() != TRB_COMPLETION_SHORT_PACKET
        {
            log::warn!(
                "xhci: GET hub descriptor slot={hub_slot} code={}",
                evt.completion_code()
            );
            return None;
        }
        let data = unsafe { read_dma_buffer(va, 16) };
        if data.len() >= 3 {
            log::info!("xhci: hub descriptor slot={hub_slot}: {} puertos", data[2]);
            return Some(data[2]);
        }
        None
    }

    fn hub_set_port_feature(&mut self, hub_slot: u8, port: u8, feature: u16) -> bool {
        let ring = match self.transfer_rings[hub_slot as usize][1].as_mut() {
            Some(r) => r,
            None => return false,
        };
        let handles = ring.enqueue_control_transfer(
            USB_DIR_OUT | USB_TYPE_CLASS | USB_RECIP_OTHER,
            HUB_REQ_SET_FEATURE,
            feature,
            port as u16,
            0,
            0,
        );
        self.db.ring_endpoint(hub_slot, 1);
        matches!(
            self.wait_transfer_event(hub_slot, Some(handles.status_trb_phys)),
            Some(evt) if evt.completion_code() == TRB_COMPLETION_SUCCESS
        )
    }

    fn hub_clear_port_feature(&mut self, hub_slot: u8, port: u8, feature: u16) -> bool {
        let ring = match self.transfer_rings[hub_slot as usize][1].as_mut() {
            Some(r) => r,
            None => return false,
        };
        let handles = ring.enqueue_control_transfer(
            USB_DIR_OUT | USB_TYPE_CLASS | USB_RECIP_OTHER,
            HUB_REQ_CLEAR_FEATURE,
            feature,
            port as u16,
            0,
            0,
        );
        self.db.ring_endpoint(hub_slot, 1);
        matches!(
            self.wait_transfer_event(hub_slot, Some(handles.status_trb_phys)),
            Some(evt) if evt.completion_code() == TRB_COMPLETION_SUCCESS
        )
    }

    fn hub_get_port_status(&mut self, hub_slot: u8, port: u8) -> Option<(u16, u16)> {
        let (va, phys) = unsafe { alloc_dma_buffer(4) };
        let ring = self.transfer_rings[hub_slot as usize][1].as_mut()?;
        let handles = ring.enqueue_control_transfer(
            USB_DIR_IN | USB_TYPE_CLASS | USB_RECIP_OTHER,
            USB_REQ_GET_STATUS,
            0,
            port as u16,
            phys,
            4,
        );
        self.db.ring_endpoint(hub_slot, 1);
        let evt = self.wait_transfer_event(hub_slot, Some(handles.status_trb_phys))?;
        if evt.completion_code() != TRB_COMPLETION_SUCCESS
            && evt.completion_code() != TRB_COMPLETION_SHORT_PACKET
        {
            return None;
        }
        let data = unsafe { read_dma_buffer(va, 4) };
        if data.len() < 4 {
            return None;
        }
        Some((
            u16::from_le_bytes([data[0], data[1]]),
            u16::from_le_bytes([data[2], data[3]]),
        ))
    }

    fn hub_wait_port_reset_complete(&mut self, hub_slot: u8, port: u8) -> bool {
        for _ in 0..500 {
            delay_us(1000);
            let Some((_status, change)) = self.hub_get_port_status(hub_slot, port) else {
                continue;
            };
            if change & HUB_PORT_CHANGE_RESET != 0 {
                if !self.hub_clear_port_feature(hub_slot, port, HUB_FEATURE_C_PORT_RESET) {
                    return false;
                }
                let Some((status, _)) = self.hub_get_port_status(hub_slot, port) else {
                    return false;
                };
                delay_us(trst_recovery_us(usb_speed_from_hub_status(status)));
                return status & HUB_PORT_CONNECTED != 0;
            }
        }
        false
    }

    fn hub_reset_child_port(&mut self, hub_slot: u8, port: u8) -> bool {
        if !self.hub_set_port_feature(hub_slot, port, HUB_FEATURE_PORT_RESET) {
            return false;
        }
        self.hub_wait_port_reset_complete(hub_slot, port)
    }

    fn enumerate_hub_children(&mut self, hub_slot: u8, root_port: u8, ports_hint: Option<u8>) {
        let n_ports = ports_hint.or_else(|| self.get_hub_port_count(hub_slot));
        let Some(n_ports) = n_ports else {
            log::warn!("xhci: hub slot={hub_slot} sin descriptor");
            return;
        };
        log::info!("xhci: hub slot={hub_slot} {n_ports} puertos downstream");
        for hp in 1..=n_ports {
            if !self.hub_set_port_feature(hub_slot, hp, HUB_FEATURE_PORT_POWER) {
                continue;
            }
            delay_us(HUB_POWER_ON_US);
            let (status, _change) = match self.hub_get_port_status(hub_slot, hp) {
                Some(s) => s,
                None => continue,
            };
            if status & HUB_PORT_CONNECTED == 0 {
                continue;
            }
            log::info!("xhci: hub puerto {hp} conectado status={status:#06x}");
            if !self.hub_set_port_feature(hub_slot, hp, HUB_FEATURE_PORT_RESET) {
                continue;
            }
            if !self.hub_wait_port_reset_complete(hub_slot, hp) {
                log::warn!("xhci: hub puerto {hp} reset timeout");
                continue;
            }
            let (status, _) = self
                .hub_get_port_status(hub_slot, hp)
                .unwrap_or((0, 0));
            let child_speed = usb_speed_from_hub_status(status);
            let path = DevPath::behind_hub(root_port, hp, hub_slot);
            if let Some(slot) = self.enable_slot() {
                self.initialize_device(slot, path, child_speed);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Diagnostics
    // -----------------------------------------------------------------------

}

/// Capacidad extendida ID 1: USB Legacy Support (handoff BIOS ↔ OS).
const XHCI_EXT_CAP_LEGACY: u8 = 1;
const USBLEGSUP_BIOS_OWNED: u32 = 1 << 16;
const USBLEGSUP_OS_OWNED: u32 = 1 << 24;
/// Offset USBLEGCTLSTS respecto a USBLEGSUP (xHCI §7.1.2 / Linux).
const XHCI_LEGACY_CONTROL_OFFSET: usize = 0x04;
/// Bits a preservar al apagar SMIs (Linux `XHCI_LEGACY_DISABLE_SMI`).
const XHCI_LEGACY_DISABLE_SMI: u32 = (0x7 << 1) + (0xff << 5) + (0x7 << 17);
/// Eventos SMI RW1C (Linux `XHCI_LEGACY_SMI_EVENTS`).
const XHCI_LEGACY_SMI_EVENTS: u32 = 0x7 << 29;

fn wait_root_port_resets_idle(op: &OperationalRegs, max_ports: u8) {
    for port in 1..=max_ports {
        for _ in 0..200 {
            let portsc = op.portsc(port);
            if portsc & (PORTSC_PR | PORTSC_WPR) == 0 {
                break;
            }
            delay_us(1000);
        }
    }
}

/// Linux `xhci_reset`: HCRST + esperar CNR=0 + PR/WPR idle.
fn do_hcrst(op: &OperationalRegs, max_ports: u8) {
    log::info!("xhci: HCRST");
    op.set_usbcmd(op.usbcmd() | USBCMD_HCRST);
    // Intel quirk (Linux udelay(1000)): no tocar regs ~1 ms tras HCRST.
    delay_us(1000);
    let mut timeout_ms = 0u32;
    while op.usbcmd() & USBCMD_HCRST != 0 {
        delay_us(100);
        timeout_ms += 1;
        if timeout_ms > 10_000 {
            panic!("xhci: HCRST did not clear");
        }
    }
    timeout_ms = 0;
    while !op.is_ready() {
        delay_us(100);
        timeout_ms += 1;
        if timeout_ms > 50_000 {
            panic!("xhci: controller not ready after reset");
        }
    }
    wait_root_port_resets_idle(op, max_ports);
    log::info!("xhci: controller reset complete");
}

/// Equivalente a Linux `quirk_usb_handoff_xhci`: ownership + SMI off + halt.
fn linux_bios_handoff(bar0: usize, cap: &CapabilityRegs, op: &OperationalRegs) {
    let xecp = hccparams1_xecp(cap.hccparams1());
    if xecp == 0 {
        log::info!("xhci: sin xECP; no hay USBLEGSUP");
    } else {
        let mut off = (xecp as usize) * 4;
        let mut found = false;
        for _ in 0..64 {
            if off == 0 || off >= 0x10000 {
                break;
            }
            let addr = bar0 + off;
            let val = unsafe { mmio_read32(addr) };
            let id = (val & 0xff) as u8;
            let next = ((val >> 8) & 0xff) as usize;
            if id == XHCI_EXT_CAP_LEGACY {
                found = true;
                let bios = val & USBLEGSUP_BIOS_OWNED != 0;
                let os = val & USBLEGSUP_OS_OWNED != 0;
                log::info!("xhci: USBLEGSUP bios_owned={bios} os_owned={os}");
                if bios {
                    unsafe { mmio_write32(addr, val | USBLEGSUP_OS_OWNED) };
                    // Linux: handshake 1 s, poll ~10 µs.
                    let mut ok = false;
                    for _ in 0..100_000 {
                        if unsafe { mmio_read32(addr) } & USBLEGSUP_BIOS_OWNED == 0 {
                            ok = true;
                            break;
                        }
                        delay_us(10);
                    }
                    if ok {
                        log::info!("xhci: handoff BIOS→OS OK");
                    } else {
                        let v = unsafe { mmio_read32(addr) };
                        unsafe {
                            mmio_write32(addr, (v | USBLEGSUP_OS_OWNED) & !USBLEGSUP_BIOS_OWNED);
                        }
                        log::warn!("xhci: handoff timeout; forzado OS owned");
                    }
                } else if !os {
                    unsafe { mmio_write32(addr, val | USBLEGSUP_OS_OWNED) };
                }

                // Apagar SMIs del firmware (USBLEGCTLSTS).
                let ctl_addr = addr + XHCI_LEGACY_CONTROL_OFFSET;
                let ctl = unsafe { mmio_read32(ctl_addr) };
                let ctl = (ctl & XHCI_LEGACY_DISABLE_SMI) | XHCI_LEGACY_SMI_EVENTS;
                unsafe { mmio_write32(ctl_addr, ctl) };
                log::info!("xhci: USBLEGCTLSTS SMI disabled");
                break;
            }
            if next == 0 {
                break;
            }
            off += next * 4;
        }
        if !found {
            log::info!("xhci: USBLEGSUP no presente en xECP");
        }
    }

    // CNR=0 (Linux handshake hasta 5 s).
    for _ in 0..50_000 {
        if op.is_ready() {
            break;
        }
        delay_us(100);
    }
    if !op.is_ready() {
        log::warn!(
            "xhci: CNR sigue activo tras handoff (USBSTS={:#x})",
            op.usbsts()
        );
    }

    // Halt + deshabilitar IRQs (Linux: clear RUN | IRQS).
    let cmd = op.usbcmd() & !(USBCMD_RS | USBCMD_IRQS);
    op.set_usbcmd(cmd);
    for _ in 0..320 {
        if op.is_halted() {
            break;
        }
        delay_us(125);
    }
    if !op.is_halted() {
        log::warn!(
            "xhci: no halt tras handoff (USBSTS={:#x})",
            op.usbsts()
        );
    } else {
        log::info!("xhci: halt post-handoff OK");
    }
}

impl XhciController {
    /// Print controller status to log.
    pub fn dump_status(&self) {
        let usbsts = self.op.usbsts();
        let usbcmd = self.op.usbcmd();

        log::info!("xhci: === Controller Status ===");
        log::info!("xhci: USBCMD={:#010x} USBSTS={:#010x}", usbcmd, usbsts);
        log::info!(
            "xhci:   RS={} HCH={} EINT={} PCD={} CNR={} HCE={}",
            usbcmd & USBCMD_RS != 0,
            usbsts & USBSTS_HCH != 0,
            usbsts & USBSTS_EINT != 0,
            usbsts & USBSTS_PCD != 0,
            usbsts & USBSTS_CNR != 0,
            usbsts & USBSTS_HCE != 0,
        );

        for port in 1..=self.max_ports {
            let portsc = self.op.portsc(port);
            if portsc & PORTSC_CCS != 0 {
                let speed = (portsc & PORTSC_SPEED_MASK) >> PORTSC_SPEED_SHIFT;
                let enabled = portsc & PORTSC_PED != 0;
                log::info!(
                    "xhci:   port {}: connected speed={} enabled={}",
                    port, speed, enabled
                );
            }
        }

        for (slot_id, dev) in self.devices.iter().enumerate() {
            if let Some(dev) = dev {
                log::info!(
                    "xhci:   slot {}: port={} speed={:?} configured={} keyboard={}",
                    slot_id,
                    dev.port,
                    dev.speed,
                    dev.configured,
                    dev.is_keyboard(),
                );
                if let Some(ref desc) = dev.device_desc {
                    log::info!(
                        "xhci:     VID={:#06x} PID={:#06x}",
                        desc.id_vendor, desc.id_product
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod event_dispatch_tests {
    use alloc::vec::Vec;

    use super::XhciController;
    use crate::ring::{
        Trb, TRB_COMPLETION_CODE_SHIFT, TRB_COMPLETION_SHORT_PACKET, TRB_COMPLETION_SUCCESS,
        TRB_ENDPOINT_ID_SHIFT, TRB_SLOT_ID_SHIFT, TRB_TYPE_SHIFT, TRB_TYPE_TRANSFER_EVENT,
    };

    fn transfer_evt(slot: u8, ep: u8, completion: u8, param: u64) -> Trb {
        Trb {
            parameter_lo: param as u32,
            parameter_hi: (param >> 32) as u32,
            status: (completion as u32) << TRB_COMPLETION_CODE_SHIFT,
            control: (TRB_TYPE_TRANSFER_EVENT << TRB_TYPE_SHIFT)
                | ((slot as u32) << TRB_SLOT_ID_SHIFT)
                | ((ep as u32) << TRB_ENDPOINT_ID_SHIFT),
        }
    }

    #[test]
    fn bulk_wait_matches_same_slot_only() {
        let disk = transfer_evt(1, 2, TRB_COMPLETION_SUCCESS, 0);
        let kbd = transfer_evt(3, 3, TRB_COMPLETION_SUCCESS, 0);
        assert!(XhciController::transfer_matches_wait(&disk, 1, 2, None));
        assert!(!XhciController::transfer_matches_wait(&kbd, 1, 2, None));
    }

    #[test]
    fn ep0_status_wait_matches_status_trb_only() {
        let status_phys = 0x20ab_c000u64;
        let status_evt = transfer_evt(2, 1, TRB_COMPLETION_SUCCESS, status_phys);
        let data_evt = transfer_evt(2, 1, TRB_COMPLETION_SHORT_PACKET, 0x1000);
        assert!(XhciController::transfer_matches_wait(
            &status_evt,
            2,
            1,
            Some(status_phys)
        ));
        assert!(!XhciController::transfer_matches_wait(
            &data_evt,
            2,
            1,
            Some(status_phys)
        ));
        assert!(XhciController::ep0_wait_continues(
            &data_evt,
            Some(status_phys)
        ));
    }

    #[test]
    fn bulk_wait_matches_endpoint_when_ptr_differs() {
        let trb_phys = 0x3000;
        let evt = transfer_evt(1, 2, TRB_COMPLETION_SUCCESS, 0x4000);
        assert!(XhciController::transfer_matches_wait(
            &evt,
            1,
            2,
            Some(trb_phys)
        ));
        assert!(!XhciController::transfer_matches_wait(
            &evt,
            1,
            3,
            Some(trb_phys)
        ));
    }

    #[test]
    fn pending_queue_preserves_alien_slot_for_later() {
        let mut pending = Vec::new();
        let kbd = transfer_evt(3, 3, TRB_COMPLETION_SUCCESS, 0);
        let disk = transfer_evt(1, 2, TRB_COMPLETION_SUCCESS, 0);
        pending.push(kbd);
        pending.push(disk);

        let take = |pending: &mut Vec<Trb>, slot: u8, dci: u8| -> Option<Trb> {
            let i = pending
                .iter()
                .position(|evt| XhciController::transfer_matches_wait(evt, slot, dci, None))?;
            Some(pending.remove(i))
        };

        assert_eq!(take(&mut pending, 1, 2).unwrap().slot_id(), 1);
        assert!(take(&mut pending, 1, 2).is_none());
        assert_eq!(take(&mut pending, 3, 3).unwrap().slot_id(), 3);
        assert!(pending.is_empty());
    }
}
