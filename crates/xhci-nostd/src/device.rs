//! USB Device Management
//!
//! Handles the xHCI device lifecycle: slot allocation, address assignment,
//! descriptor retrieval, and configuration. This module issues commands on the
//! command ring and parses standard USB descriptors.

use alloc::vec::Vec;
use core::ptr;

use crate::dma;

use crate::context::{
    EP_TYPE_CONTROL, EP_TYPE_INTERRUPT_IN,
};

// ---------------------------------------------------------------------------
// USB Standard Request codes (bRequest)
// ---------------------------------------------------------------------------

pub const USB_REQ_GET_STATUS: u8 = 0x00;
pub const USB_REQ_CLEAR_FEATURE: u8 = 0x01;
pub const USB_REQ_SET_FEATURE: u8 = 0x03;
pub const USB_REQ_SET_ADDRESS: u8 = 0x05;
pub const USB_REQ_GET_DESCRIPTOR: u8 = 0x06;
pub const USB_REQ_SET_DESCRIPTOR: u8 = 0x07;
pub const USB_REQ_GET_CONFIGURATION: u8 = 0x08;
pub const USB_REQ_SET_CONFIGURATION: u8 = 0x09;
pub const USB_REQ_GET_INTERFACE: u8 = 0x0A;
pub const USB_REQ_SET_INTERFACE: u8 = 0x0B;

// ---------------------------------------------------------------------------
// USB Descriptor types
// ---------------------------------------------------------------------------

pub const USB_DESC_DEVICE: u8 = 1;
pub const USB_DESC_CONFIGURATION: u8 = 2;
pub const USB_DESC_STRING: u8 = 3;
pub const USB_DESC_INTERFACE: u8 = 4;
pub const USB_DESC_ENDPOINT: u8 = 5;
pub const USB_DESC_DEVICE_QUALIFIER: u8 = 6;
pub const USB_DESC_HID: u8 = 0x21;
pub const USB_DESC_HID_REPORT: u8 = 0x22;

// ---------------------------------------------------------------------------
// USB bmRequestType building
// ---------------------------------------------------------------------------

/// Direction: Device to Host
pub const USB_DIR_IN: u8 = 0x80;
/// Direction: Host to Device
pub const USB_DIR_OUT: u8 = 0x00;
/// Type: Standard
pub const USB_TYPE_STANDARD: u8 = 0x00;
/// Type: Class
pub const USB_TYPE_CLASS: u8 = 0x20;
/// Recipient: Device
pub const USB_RECIP_DEVICE: u8 = 0x00;
/// Recipient: Interface
pub const USB_RECIP_INTERFACE: u8 = 0x01;
/// Recipient: Endpoint
pub const USB_RECIP_ENDPOINT: u8 = 0x02;

// ---------------------------------------------------------------------------
// USB Class codes
// ---------------------------------------------------------------------------

pub const USB_CLASS_HID: u8 = 0x03;
pub const USB_CLASS_HUB: u8 = 0x09;

pub const USB_DESC_HUB: u8 = 0x29;

/// `fls()` de Linux: posición 1-indexada del bit más alto puesto (0 si x==0).
const fn fls(x: u32) -> u32 {
    32 - x.leading_zeros()
}

// ---------------------------------------------------------------------------
// USB Device Descriptor (18 bytes)
// ---------------------------------------------------------------------------

/// Parsed USB Device Descriptor (per USB 3.2 spec Table 9-11)
#[derive(Debug, Clone)]
pub struct DeviceDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_sub_class: u8,
    pub b_device_protocol: u8,
    pub b_max_packet_size0: u8,
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial_number: u8,
    pub b_num_configurations: u8,
}

impl DeviceDescriptor {
    pub const SIZE: usize = 18;

    /// Parse from a raw byte buffer. Returns None if buffer is too small.
    pub fn is_hub(&self) -> bool {
        self.b_device_class == USB_CLASS_HUB
    }

    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE {
            log::warn!("xhci: device descriptor too short ({} bytes)", buf.len());
            return None;
        }
        if buf[1] != USB_DESC_DEVICE {
            log::warn!("xhci: expected device descriptor type 1, got {}", buf[1]);
            return None;
        }

        let desc = Self {
            b_length: buf[0],
            b_descriptor_type: buf[1],
            bcd_usb: u16::from_le_bytes([buf[2], buf[3]]),
            b_device_class: buf[4],
            b_device_sub_class: buf[5],
            b_device_protocol: buf[6],
            b_max_packet_size0: buf[7],
            id_vendor: u16::from_le_bytes([buf[8], buf[9]]),
            id_product: u16::from_le_bytes([buf[10], buf[11]]),
            bcd_device: u16::from_le_bytes([buf[12], buf[13]]),
            i_manufacturer: buf[14],
            i_product: buf[15],
            i_serial_number: buf[16],
            b_num_configurations: buf[17],
        };

        log::info!(
            "xhci: device descriptor: USB {}.{}, VID={:#06x} PID={:#06x}, class={}, max_pkt0={}, configs={}",
            desc.bcd_usb >> 8,
            (desc.bcd_usb >> 4) & 0xF,
            desc.id_vendor,
            desc.id_product,
            desc.b_device_class,
            desc.b_max_packet_size0,
            desc.b_num_configurations,
        );

        Some(desc)
    }
}

// ---------------------------------------------------------------------------
// USB Configuration Descriptor (9 bytes header)
// ---------------------------------------------------------------------------

/// Parsed USB Configuration Descriptor header (per USB 3.2 spec Table 9-13)
#[derive(Debug, Clone)]
pub struct ConfigurationDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub w_total_length: u16,
    pub b_num_interfaces: u8,
    pub b_configuration_value: u8,
    pub i_configuration: u8,
    pub bm_attributes: u8,
    pub b_max_power: u8,
}

impl ConfigurationDescriptor {
    pub const SIZE: usize = 9;

    /// Parse from a raw byte buffer. Returns None if buffer is too small.
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE {
            log::warn!("xhci: config descriptor too short ({} bytes)", buf.len());
            return None;
        }
        if buf[1] != USB_DESC_CONFIGURATION {
            log::warn!("xhci: expected config descriptor type 2, got {}", buf[1]);
            return None;
        }

        let desc = Self {
            b_length: buf[0],
            b_descriptor_type: buf[1],
            w_total_length: u16::from_le_bytes([buf[2], buf[3]]),
            b_num_interfaces: buf[4],
            b_configuration_value: buf[5],
            i_configuration: buf[6],
            bm_attributes: buf[7],
            b_max_power: buf[8],
        };

        log::info!(
            "xhci: config descriptor: total_len={}, interfaces={}, config_val={}, max_power={}mA",
            desc.w_total_length,
            desc.b_num_interfaces,
            desc.b_configuration_value,
            desc.b_max_power as u32 * 2,
        );

        Some(desc)
    }
}

// ---------------------------------------------------------------------------
// USB Interface Descriptor (9 bytes)
// ---------------------------------------------------------------------------

/// Parsed USB Interface Descriptor (per USB 3.2 spec Table 9-17)
#[derive(Debug, Clone)]
pub struct InterfaceDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub b_interface_number: u8,
    pub b_alternate_setting: u8,
    pub b_num_endpoints: u8,
    pub b_interface_class: u8,
    pub b_interface_sub_class: u8,
    pub b_interface_protocol: u8,
    pub i_interface: u8,
}

impl InterfaceDescriptor {
    pub const SIZE: usize = 9;

    /// Parse from a raw byte buffer.
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE || buf[1] != USB_DESC_INTERFACE {
            return None;
        }

        let desc = Self {
            b_length: buf[0],
            b_descriptor_type: buf[1],
            b_interface_number: buf[2],
            b_alternate_setting: buf[3],
            b_num_endpoints: buf[4],
            b_interface_class: buf[5],
            b_interface_sub_class: buf[6],
            b_interface_protocol: buf[7],
            i_interface: buf[8],
        };

        // A nivel debug: en placa real sin serie la pantalla son ~40 filas y un
        // dispositivo compuesto (8 interfaces, 17 endpoints) se las come todas.
        log::debug!(
            "xhci: interface descriptor: num={}, alt={}, eps={}, class={:#x} sub={:#x} proto={:#x}",
            desc.b_interface_number,
            desc.b_alternate_setting,
            desc.b_num_endpoints,
            desc.b_interface_class,
            desc.b_interface_sub_class,
            desc.b_interface_protocol,
        );

        Some(desc)
    }

    /// Boot HID keyboard (class=3, subclass=1, protocol=1).
    pub fn is_hid_boot_keyboard(&self) -> bool {
        self.b_interface_class == USB_CLASS_HID
            && self.b_interface_sub_class == 1
            && self.b_interface_protocol == 1
    }

    /// Candidato a teclado: HID que no es ratón (p. ej. ASUS NKEY en report protocol).
    pub fn is_hid_keyboard_candidate(&self) -> bool {
        self.b_interface_class == USB_CLASS_HID && self.b_interface_protocol != 2
    }

    /// Is this a HID interface?
    pub fn is_hid(&self) -> bool {
        self.b_interface_class == USB_CLASS_HID
    }
}

// ---------------------------------------------------------------------------
// USB Endpoint Descriptor (7 bytes)
// ---------------------------------------------------------------------------

/// Parsed USB Endpoint Descriptor (per USB 3.2 spec Table 9-20)
#[derive(Debug, Clone)]
pub struct EndpointDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub b_endpoint_address: u8,
    pub bm_attributes: u8,
    pub w_max_packet_size: u16,
    pub b_interval: u8,
}

impl EndpointDescriptor {
    pub const SIZE: usize = 7;

    /// Parse from a raw byte buffer.
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE || buf[1] != USB_DESC_ENDPOINT {
            return None;
        }

        let desc = Self {
            b_length: buf[0],
            b_descriptor_type: buf[1],
            b_endpoint_address: buf[2],
            bm_attributes: buf[3],
            w_max_packet_size: u16::from_le_bytes([buf[4], buf[5]]),
            b_interval: buf[6],
        };

        log::debug!(
            "xhci: endpoint descriptor: addr={:#x}, attr={:#x}, max_pkt={}, interval={}",
            desc.b_endpoint_address,
            desc.bm_attributes,
            desc.w_max_packet_size,
            desc.b_interval,
        );

        Some(desc)
    }

    /// Endpoint number (bits 3:0 of bEndpointAddress)
    pub fn endpoint_number(&self) -> u8 {
        self.b_endpoint_address & 0x0F
    }

    /// Direction: true = IN (device to host), false = OUT
    pub fn is_in(&self) -> bool {
        self.b_endpoint_address & 0x80 != 0
    }

    /// Transfer type (bits 1:0 of bmAttributes)
    pub fn transfer_type(&self) -> u8 {
        self.bm_attributes & 0x03
    }

    /// Is this an interrupt endpoint?
    pub fn is_interrupt(&self) -> bool {
        self.transfer_type() == 3
    }

    /// Is this a bulk endpoint?
    pub fn is_bulk(&self) -> bool {
        self.transfer_type() == 2
    }

    /// Convert to xHCI Device Context Index (DCI).
    /// DCI = endpoint_number * 2 + direction (0=OUT, 1=IN).
    /// EP0 is DCI=1. EP1 OUT=2, EP1 IN=3, etc.
    pub fn dci(&self) -> u8 {
        let ep_num = self.endpoint_number();
        if ep_num == 0 {
            1 // Control endpoint is always DCI 1
        } else {
            ep_num * 2 + if self.is_in() { 1 } else { 0 }
        }
    }

    /// Max packet size para Endpoint Context (bits 10:0 de wMaxPacketSize).
    pub fn xhci_max_packet_size(&self) -> u16 {
        self.w_max_packet_size & 0x7FF
    }

    /// Mult (bits 9:8 EP context DW0) — solo HS periódico.
    pub fn xhci_mult(&self, speed: UsbSpeed) -> u8 {
        if speed == UsbSpeed::High && (self.is_interrupt() || self.transfer_type() == 1) {
            ((self.w_max_packet_size >> 11) & 0x03) as u8
        } else {
            0
        }
    }

    /// ¿Endpoint isócrono? (bmAttributes bits 1:0 = 1)
    pub fn is_isoch(&self) -> bool {
        self.transfer_type() == 1
    }

    /// Intervalo codificado para el Endpoint Context, calcando
    /// `xhci_get_endpoint_interval()` de Linux (drivers/usb/host/xhci-mem.c).
    ///
    /// Ojo: en Low/Full speed `bInterval` son **milisegundos** (1..255), no un
    /// exponente. El xHCI spec 6.2.3.6 sólo admite 3..10 para interrupt FS/LS y
    /// 3..18 para isoch FS; escribir el valor crudo saca el campo de rango y el
    /// Configure Endpoint responde Parameter Error (código 17).
    pub fn xhci_interval(&self, speed: UsbSpeed) -> u8 {
        let b = self.b_interval as u32;
        match (self.transfer_type(), speed) {
            // Control y bulk no tienen periodo.
            (0, _) | (2, _) => 0,
            // HS/SS periódicos: bInterval ya es un exponente de microframes.
            (1 | 3, UsbSpeed::High | UsbSpeed::Super | UsbSpeed::SuperPlus) => {
                (b.clamp(1, 16) - 1) as u8
            }
            // Isoch Full speed: exponente en frames; +3 para pasarlo a microframes.
            (1, _) => (b.clamp(1, 16) - 1 + 3) as u8,
            // Interrupt Full/Low speed: fls(bInterval * 8) - 1, acotado a 3..10.
            (3, _) => (fls(b.max(1) * 8) - 1).clamp(3, 10) as u8,
            _ => 0,
        }
    }

    /// Convert to xHCI endpoint type value for Endpoint Context.
    pub fn xhci_ep_type(&self) -> u32 {
        match (self.transfer_type(), self.is_in()) {
            (0, _) => EP_TYPE_CONTROL,           // Control
            (1, false) => 1,                      // Isoch OUT
            (1, true) => 5,                       // Isoch IN
            (2, false) => 2,                      // Bulk OUT
            (2, true) => 6,                       // Bulk IN
            (3, false) => 3,                      // Interrupt OUT
            (3, true) => EP_TYPE_INTERRUPT_IN,    // Interrupt IN
            _ => 0,                               // Not valid
        }
    }
}

// ---------------------------------------------------------------------------
// HID Descriptor (embedded in configuration descriptor set)
// ---------------------------------------------------------------------------

/// Parsed HID Descriptor (per HID spec 1.11, section 6.2.1)
#[derive(Debug, Clone)]
pub struct HidDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub bcd_hid: u16,
    pub b_country_code: u8,
    pub b_num_descriptors: u8,
    /// Descriptor type of the first (usually only) report descriptor
    pub report_descriptor_type: u8,
    /// Length of the first report descriptor
    pub report_descriptor_length: u16,
}

impl HidDescriptor {
    pub const MIN_SIZE: usize = 9;

    /// Parse from a raw byte buffer.
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::MIN_SIZE || buf[1] != USB_DESC_HID {
            return None;
        }

        let desc = Self {
            b_length: buf[0],
            b_descriptor_type: buf[1],
            bcd_hid: u16::from_le_bytes([buf[2], buf[3]]),
            b_country_code: buf[4],
            b_num_descriptors: buf[5],
            report_descriptor_type: buf[6],
            report_descriptor_length: u16::from_le_bytes([buf[7], buf[8]]),
        };

        log::info!(
            "xhci: HID descriptor: ver={}.{}, country={}, num_desc={}, report_len={}",
            desc.bcd_hid >> 8,
            (desc.bcd_hid >> 4) & 0xF,
            desc.b_country_code,
            desc.b_num_descriptors,
            desc.report_descriptor_length,
        );

        Some(desc)
    }
}

// ---------------------------------------------------------------------------
// Parsed configuration (all descriptors from one GET_DESCRIPTOR(Configuration))
// ---------------------------------------------------------------------------

/// All descriptors parsed from a full configuration descriptor set.
#[derive(Debug, Clone)]
pub struct ParsedConfiguration {
    pub config: ConfigurationDescriptor,
    pub interfaces: Vec<InterfaceDescriptor>,
    /// (interface_number, alternate_setting, endpoint)
    pub endpoints: Vec<(u8, u8, EndpointDescriptor)>,
    pub hid_descriptors: Vec<(u8, HidDescriptor)>, // (interface_number, hid)
    /// bNbrPorts del hub descriptor embebido (solo hubs).
    pub hub_num_ports: Option<u8>,
}

impl ParsedConfiguration {
    /// Parse a complete configuration descriptor set (header + all subordinate descriptors).
    pub fn parse(buf: &[u8]) -> Option<Self> {
        let config = ConfigurationDescriptor::parse(buf)?;
        let total_len = config.w_total_length as usize;

        if buf.len() < total_len {
            log::warn!(
                "xhci: config descriptor claims {} bytes but buffer has {}",
                total_len,
                buf.len()
            );
        }

        let parse_len = buf.len().min(total_len);
        let mut interfaces = Vec::new();
        let mut endpoints = Vec::new();
        let mut hid_descriptors = Vec::new();
        let mut current_iface: u8 = 0;
        let mut current_alt: u8 = 0;
        let mut hub_num_ports: Option<u8> = None;

        let mut offset = config.b_length as usize;
        while offset + 2 <= parse_len {
            let desc_len = buf[offset] as usize;
            let desc_type = buf[offset + 1];

            if desc_len < 2 || offset + desc_len > parse_len {
                log::warn!(
                    "xhci: malformed descriptor at offset {}: len={} type={:#x}",
                    offset, desc_len, desc_type
                );
                break;
            }

            match desc_type {
                USB_DESC_INTERFACE => {
                    if let Some(iface) = InterfaceDescriptor::parse(&buf[offset..]) {
                        current_iface = iface.b_interface_number;
                        current_alt = iface.b_alternate_setting;
                        interfaces.push(iface);
                    }
                }
                USB_DESC_ENDPOINT => {
                    if let Some(ep) = EndpointDescriptor::parse(&buf[offset..]) {
                        endpoints.push((current_iface, current_alt, ep));
                    }
                }
                USB_DESC_HID => {
                    if let Some(hid) = HidDescriptor::parse(&buf[offset..]) {
                        hid_descriptors.push((current_iface, hid));
                    }
                }
                USB_DESC_HUB => {
                    if desc_len >= 3 {
                        hub_num_ports = Some(buf[offset + 2]);
                        log::info!(
                            "xhci: hub descriptor: {} puertos downstream",
                            buf[offset + 2]
                        );
                    }
                }
                _ => {
                    log::trace!(
                        "xhci: skipping descriptor type {:#x} len={} at offset {}",
                        desc_type, desc_len, offset
                    );
                }
            }

            offset += desc_len;
        }

        let active_n = endpoints.iter().filter(|(_, alt, _)| *alt == 0).count();
        log::info!(
            "xhci: parsed config: {} interfaces, {} endpoints ({} alt0), {} HID descriptors",
            interfaces.len(),
            endpoints.len(),
            active_n,
            hid_descriptors.len(),
        );

        Some(Self {
            config,
            interfaces,
            endpoints,
            hid_descriptors,
            hub_num_ports,
        })
    }

    /// ¿Necesita SET_CONFIGURATION + Configure Endpoint? (hub, teclado, mass storage).
    pub fn needs_full_config(&self, dev_desc: &DeviceDescriptor, has_keyboard: bool) -> bool {
        if dev_desc.is_hub() {
            return true;
        }
        if has_keyboard {
            return true;
        }
        self.is_mass_storage()
    }

    /// Interfaz BOT mass storage en alternate 0.
    pub fn is_mass_storage(&self) -> bool {
        const MS_CLASS: u8 = 0x08;
        const MS_SUBCLASS: u8 = 0x06;
        const MS_PROTO: u8 = 0x50;
        self.interfaces.iter().any(|i| {
            i.b_alternate_setting == 0
                && i.b_interface_class == MS_CLASS
                && i.b_interface_sub_class == MS_SUBCLASS
                && i.b_interface_protocol == MS_PROTO
        })
    }

    /// Endpoints del alternate activo tras SET_CONFIGURATION (Linux: alt 0 por defecto).
    pub fn active_endpoints(&self) -> impl Iterator<Item = (u8, &EndpointDescriptor)> {
        self.endpoints
            .iter()
            .filter(|(_, alt, _)| *alt == 0)
            .map(|(iface, _, ep)| (*iface, ep))
    }

    /// Find the first HID keyboard interface (class=3, subclass=1, protocol=1).
    /// Returns (interface_number, interrupt IN endpoint).
    /// Primera interfaz HID que puede dar teclas, con su endpoint interrupt IN.
    ///
    /// Devuelve además si declara la subclase Boot: quien la configure debe
    /// saberlo para no mandar SET_PROTOCOL a un HID que sólo habla report
    /// protocol.
    pub fn find_hid_keyboard(&self) -> Option<(u8, bool, &EndpointDescriptor)> {
        for iface in &self.interfaces {
            if iface.b_alternate_setting != 0 || !iface.is_hid_keyboard_candidate() {
                continue;
            }
            let kind = if iface.is_hid_boot_keyboard() {
                "boot"
            } else {
                "generic HID"
            };
            log::info!(
                "xhci: found HID keyboard ({}) on interface {}",
                kind,
                iface.b_interface_number
            );

            for (iface_num, alt, ep) in &self.endpoints {
                if *iface_num == iface.b_interface_number
                    && *alt == 0
                    && ep.is_interrupt()
                    && ep.is_in()
                {
                    log::info!(
                        "xhci: keyboard interrupt IN endpoint: addr={:#x} max_pkt={} interval={}",
                        ep.b_endpoint_address,
                        ep.w_max_packet_size,
                        ep.b_interval,
                    );
                    return Some((
                        iface.b_interface_number,
                        iface.is_hid_boot_keyboard(),
                        ep,
                    ));
                }
            }

            log::warn!(
                "xhci: HID keyboard interface {} has no interrupt IN endpoint",
                iface.b_interface_number
            );
        }
        None
    }
}

// ---------------------------------------------------------------------------
// DMA buffer helpers
// ---------------------------------------------------------------------------

/// Allocate a DMA-safe buffer (physically contiguous, cache-line aligned).
/// Returns (virtual_address, physical_address). In an identity-mapped environment these are the same.
///
/// # Safety
/// Caller must ensure the buffer is not freed while DMA is in progress.
pub unsafe fn alloc_dma_buffer(size: usize) -> (*mut u8, u64) {
    let align = if size >= 4096 { 4096 } else { 64 };
    let len = size.next_multiple_of(align);
    dma::alloc_aligned(len, align)
}

/// Read bytes from a DMA buffer.
pub unsafe fn read_dma_buffer(va: *const u8, len: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(len);
    for i in 0..len {
        buf.push(ptr::read_volatile(va.add(i)));
    }
    buf
}

// ---------------------------------------------------------------------------
// USB device state tracking
// ---------------------------------------------------------------------------

/// Speed of a USB device (from port status or slot context)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbSpeed {
    Low,       // 1.5 Mb/s
    Full,      // 12 Mb/s
    High,      // 480 Mb/s
    Super,     // 5 Gb/s
    SuperPlus, // 10 Gb/s
    Unknown,
}

impl UsbSpeed {
    /// Convert from xHCI port speed value
    pub fn from_port_speed(speed: u32) -> Self {
        match speed {
            1 => Self::Full,
            2 => Self::Low,
            3 => Self::High,
            4 => Self::Super,
            5 => Self::SuperPlus,
            _ => Self::Unknown,
        }
    }

    /// Convert to xHCI slot context speed value
    pub fn to_slot_speed(self) -> u32 {
        match self {
            Self::Full => 1,
            Self::Low => 2,
            Self::High => 3,
            Self::Super => 4,
            Self::SuperPlus => 5,
            Self::Unknown => 0,
        }
    }

    /// Default max packet size for EP0 based on speed
    pub fn default_max_packet_size0(self) -> u16 {
        match self {
            Self::Low => 8,
            Self::Full => 8,  // Could be 8, 16, 32, or 64; start with 8
            Self::High => 64,
            Self::Super | Self::SuperPlus => 512,
            Self::Unknown => 8,
        }
    }
}

/// Tracks the state of an enumerated USB device.
#[derive(Debug)]
pub struct UsbDevice {
    /// xHCI slot ID (1-indexed)
    pub slot_id: u8,
    /// Root hub port number (1-indexed)
    pub port: u8,
    /// Device speed
    pub speed: UsbSpeed,
    /// Device descriptor (after GET_DESCRIPTOR)
    pub device_desc: Option<DeviceDescriptor>,
    /// Parsed configuration (after GET_DESCRIPTOR(Config))
    pub config: Option<ParsedConfiguration>,
    /// Whether the device is configured (SET_CONFIGURATION issued)
    pub configured: bool,
    /// If this is a HID keyboard: the interface number
    pub keyboard_interface: Option<u8>,
    /// If this is a HID keyboard: the interrupt IN endpoint DCI
    pub keyboard_endpoint_dci: Option<u8>,
    /// Route string xHCI (0 = dispositivo en puerto root directo).
    pub route_string: u32,
    /// TT hub slot / port (USB2 full/low detrás de hub).
    pub tt_hub_slot: u8,
    pub tt_port: u8,
}

impl UsbDevice {
    /// Create a new device in the initial state.
    pub fn new(
        slot_id: u8,
        port: u8,
        speed: UsbSpeed,
        route_string: u32,
        tt_hub_slot: u8,
        tt_port: u8,
    ) -> Self {
        log::info!(
            "xhci: new USB device slot={} port={} speed={:?} route={route_string:#x}",
            slot_id, port, speed
        );
        Self {
            slot_id,
            port,
            speed,
            device_desc: None,
            config: None,
            configured: false,
            keyboard_interface: None,
            keyboard_endpoint_dci: None,
            route_string,
            tt_hub_slot,
            tt_port,
        }
    }

    /// Check if this device is (or could be) a keyboard.
    pub fn is_keyboard(&self) -> bool {
        self.keyboard_interface.is_some()
    }
}
