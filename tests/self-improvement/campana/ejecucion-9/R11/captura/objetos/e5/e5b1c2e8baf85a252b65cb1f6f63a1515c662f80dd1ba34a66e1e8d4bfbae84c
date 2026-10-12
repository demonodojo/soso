//! HID (Human Interface Device) Keyboard Support
//!
//! Implements the USB HID boot protocol for keyboards. The boot protocol is a
//! simplified 8-byte report format that does not require parsing HID report
//! descriptors, making it ideal for early-boot keyboard support.
//!
//! Boot protocol keyboard report (8 bytes):
//! - Byte 0: Modifier keys (bitmap)
//! - Byte 1: Reserved (always 0)
//! - Bytes 2-7: Up to 6 simultaneous keycodes (USB HID Usage IDs)

use alloc::collections::VecDeque;

// ---------------------------------------------------------------------------
// HID Class-specific requests
// ---------------------------------------------------------------------------

/// HID class request: GET_REPORT
pub const HID_REQ_GET_REPORT: u8 = 0x01;
/// HID class request: GET_IDLE
pub const HID_REQ_GET_IDLE: u8 = 0x02;
/// HID class request: GET_PROTOCOL
pub const HID_REQ_GET_PROTOCOL: u8 = 0x03;
/// HID class request: SET_REPORT
pub const HID_REQ_SET_REPORT: u8 = 0x09;
/// HID class request: SET_IDLE
pub const HID_REQ_SET_IDLE: u8 = 0x0A;
/// HID class request: SET_PROTOCOL
pub const HID_REQ_SET_PROTOCOL: u8 = 0x0B;

/// Protocol values for SET_PROTOCOL
pub const HID_PROTOCOL_BOOT: u16 = 0;
pub const HID_PROTOCOL_REPORT: u16 = 1;

// ---------------------------------------------------------------------------
// Modifier key bits (byte 0 of boot protocol report)
// ---------------------------------------------------------------------------

pub const MOD_LEFT_CTRL: u8 = 1 << 0;
pub const MOD_LEFT_SHIFT: u8 = 1 << 1;
pub const MOD_LEFT_ALT: u8 = 1 << 2;
pub const MOD_LEFT_GUI: u8 = 1 << 3;
pub const MOD_RIGHT_CTRL: u8 = 1 << 4;
pub const MOD_RIGHT_SHIFT: u8 = 1 << 5;
pub const MOD_RIGHT_ALT: u8 = 1 << 6;
pub const MOD_RIGHT_GUI: u8 = 1 << 7;

// ---------------------------------------------------------------------------
// USB HID Usage ID to PS/2-style scancode mapping
// ---------------------------------------------------------------------------

/// A keyboard event produced by parsing a HID boot protocol report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// The USB HID Usage ID (keycode)
    pub usage_id: u8,
    /// PS/2 scancode equivalent (for compatibility with pc-keyboard crate)
    pub scancode: u8,
    /// true = key pressed, false = key released
    pub pressed: bool,
    /// Modifier state at the time of the event
    pub modifiers: u8,
}

/// USB HID Usage ID to PS/2 Set 1 scancode lookup table.
/// Index = USB HID Usage ID, Value = PS/2 Make code (0 = unmapped).
///
/// Based on USB HID Usage Tables 1.4, Section 10 "Keyboard/Keypad Page (0x07)"
/// and PS/2 Scan Code Set 1.
static HID_TO_SCANCODE: [u8; 128] = [
    // 0x00-0x03: No Event, Error Roll Over, POST Fail, Error Undefined
    0x00, 0x00, 0x00, 0x00,
    // 0x04: A, 0x05: B, 0x06: C, 0x07: D
    0x1E, 0x30, 0x2E, 0x20,
    // 0x08: E, 0x09: F, 0x0A: G, 0x0B: H
    0x12, 0x21, 0x22, 0x23,
    // 0x0C: I, 0x0D: J, 0x0E: K, 0x0F: L
    0x17, 0x24, 0x25, 0x26,
    // 0x10: M, 0x11: N, 0x12: O, 0x13: P
    0x32, 0x31, 0x18, 0x19,
    // 0x14: Q, 0x15: R, 0x16: S, 0x17: T
    0x10, 0x13, 0x1F, 0x14,
    // 0x18: U, 0x19: V, 0x1A: W, 0x1B: X
    0x16, 0x2F, 0x11, 0x2D,
    // 0x1C: Y, 0x1D: Z, 0x1E: 1, 0x1F: 2
    0x15, 0x2C, 0x02, 0x03,
    // 0x20: 3, 0x21: 4, 0x22: 5, 0x23: 6
    0x04, 0x05, 0x06, 0x07,
    // 0x24: 7, 0x25: 8, 0x26: 9, 0x27: 0
    0x08, 0x09, 0x0A, 0x0B,
    // 0x28: Enter, 0x29: Escape, 0x2A: Backspace, 0x2B: Tab
    0x1C, 0x01, 0x0E, 0x0F,
    // 0x2C: Space, 0x2D: Minus, 0x2E: Equal, 0x2F: Left Bracket
    0x39, 0x0C, 0x0D, 0x1A,
    // 0x30: Right Bracket, 0x31: Backslash, 0x32: Non-US #, 0x33: Semicolon
    0x1B, 0x2B, 0x2B, 0x27,
    // 0x34: Apostrophe, 0x35: Grave Accent, 0x36: Comma, 0x37: Period
    0x28, 0x29, 0x33, 0x34,
    // 0x38: Slash, 0x39: Caps Lock, 0x3A: F1, 0x3B: F2
    0x35, 0x3A, 0x3B, 0x3C,
    // 0x3C: F3, 0x3D: F4, 0x3E: F5, 0x3F: F6
    0x3D, 0x3E, 0x3F, 0x40,
    // 0x40: F7, 0x41: F8, 0x42: F9, 0x43: F10
    0x41, 0x42, 0x43, 0x44,
    // 0x44: F11, 0x45: F12, 0x46: Print Screen, 0x47: Scroll Lock
    0x57, 0x58, 0x00, 0x46,
    // 0x48: Pause, 0x49: Insert, 0x4A: Home, 0x4B: Page Up
    0x00, 0x52, 0x47, 0x49,
    // 0x4C: Delete, 0x4D: End, 0x4E: Page Down, 0x4F: Right Arrow
    0x53, 0x4F, 0x51, 0x4D,
    // 0x50: Left Arrow, 0x51: Down Arrow, 0x52: Up Arrow, 0x53: Num Lock
    0x4B, 0x50, 0x48, 0x45,
    // 0x54: KP /, 0x55: KP *, 0x56: KP -, 0x57: KP +
    0x35, 0x37, 0x4A, 0x4E,
    // 0x58: KP Enter, 0x59: KP 1, 0x5A: KP 2, 0x5B: KP 3
    0x1C, 0x4F, 0x50, 0x51,
    // 0x5C: KP 4, 0x5D: KP 5, 0x5E: KP 6, 0x5F: KP 7
    0x4B, 0x4C, 0x4D, 0x47,
    // 0x60: KP 8, 0x61: KP 9, 0x62: KP 0, 0x63: KP .
    0x48, 0x49, 0x52, 0x53,
    // 0x64: Non-US \, 0x65: Application, 0x66: Power, 0x67: KP =
    0x56, 0x00, 0x00, 0x00,
    // 0x68-0x6B: F13-F16
    0x00, 0x00, 0x00, 0x00,
    // 0x6C-0x6F: F17-F20
    0x00, 0x00, 0x00, 0x00,
    // 0x70-0x73: F21-F24
    0x00, 0x00, 0x00, 0x00,
    // 0x74-0x77: Execute, Help, Menu, Select
    0x00, 0x00, 0x00, 0x00,
    // 0x78-0x7B: Stop, Again, Undo, Cut
    0x00, 0x00, 0x00, 0x00,
    // 0x7C-0x7F: Copy, Paste, Find, Mute
    0x00, 0x00, 0x00, 0x00,
];

/// Convert a USB HID Usage ID to a PS/2 scancode.
pub fn hid_usage_to_scancode(usage_id: u8) -> u8 {
    if (usage_id as usize) < HID_TO_SCANCODE.len() {
        HID_TO_SCANCODE[usage_id as usize]
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Boot protocol report parser
// ---------------------------------------------------------------------------

/// Boot protocol keyboard report: 8 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootKeyboardReport {
    /// Modifier keys bitmap
    pub modifiers: u8,
    /// Reserved byte (should be 0)
    pub reserved: u8,
    /// Up to 6 keycodes currently pressed
    pub keycodes: [u8; 6],
}

impl BootKeyboardReport {
    /// Parse from an 8-byte buffer.
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 8 {
            log::warn!("xhci: HID report too short ({} bytes, need 8)", buf.len());
            return None;
        }

        Some(Self {
            modifiers: buf[0],
            reserved: buf[1],
            keycodes: [buf[2], buf[3], buf[4], buf[5], buf[6], buf[7]],
        })
    }

    /// Check for phantom/rollover condition (all keycodes = 0x01)
    pub fn is_rollover(&self) -> bool {
        self.keycodes.iter().all(|&k| k == 0x01)
    }

    /// Check if a specific keycode is present in this report
    pub fn has_keycode(&self, code: u8) -> bool {
        self.keycodes.iter().any(|&k| k == code)
    }
}

// ---------------------------------------------------------------------------
// Report descriptor layout (report protocol)
// ---------------------------------------------------------------------------

/// Layout de un informe HID de teclado (boot o report protocol).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardReportLayout {
    /// Report ID si el descriptor lo declara.
    pub report_id: Option<u8>,
    /// Longitud total del informe en el bus (incluye byte de ID).
    pub report_len: usize,
    pub modifier_offset: usize,
    pub reserved_offset: usize,
    pub keys_offset: usize,
    pub key_count: usize,
}

impl KeyboardReportLayout {
    /// Layout boot de 8 bytes sin Report ID.
    pub fn boot() -> Self {
        Self {
            report_id: None,
            report_len: 8,
            modifier_offset: 0,
            reserved_offset: 1,
            keys_offset: 2,
            key_count: 6,
        }
    }

    /// Parsea el report descriptor HID y localiza la colección teclado.
    pub fn from_descriptor(rdesc: &[u8], vendor_id: u16, product_id: u16) -> Self {
        let mut fixed = rdesc.to_vec();
        fix_asus_nkey_descriptor(&mut fixed, vendor_id, product_id);
        if let Some(layout) = parse_keyboard_layout(&fixed) {
            log::info!(
                "xhci: keyboard report layout id={:?} len={} keys={}",
                layout.report_id,
                layout.report_len,
                layout.key_count
            );
            return layout;
        }
        log::debug!("xhci: keyboard layout fallback to boot protocol");
        Self::boot()
    }

    /// Extrae un informe boot-compatible desde datos crudos (con o sin Report ID).
    pub fn extract_report(&self, data: &[u8]) -> Option<BootKeyboardReport> {
        let payload = if let Some(id) = self.report_id {
            if data.first().copied()? != id {
                return None;
            }
            &data[1..]
        } else {
            data
        };

        if payload.len() < self.report_len.saturating_sub(self.report_id.map(|_| 1).unwrap_or(0)) {
            return None;
        }

        let modifiers = payload.get(self.modifier_offset).copied()?;
        let reserved = payload.get(self.reserved_offset).copied().unwrap_or(0);
        let mut keycodes = [0u8; 6];
        for (i, slot) in keycodes.iter_mut().enumerate() {
            *slot = payload.get(self.keys_offset + i).copied().unwrap_or(0);
        }
        if self.key_count < 6 {
            for slot in &mut keycodes[self.key_count..] {
                *slot = 0;
            }
        }

        Some(BootKeyboardReport {
            modifiers,
            reserved,
            keycodes,
        })
    }
}

/// Quirk Linux `hid-asus`: Report Count erróneo en la parte 0x5a del descriptor N-Key.
pub fn fix_asus_nkey_descriptor(rdesc: &mut [u8], vendor_id: u16, product_id: u16) {
    if vendor_id != 0x0b05 {
        return;
    }
    let nkey = product_id == 0x1866;
    if nkey && rdesc.len() == 331 && rdesc.len() > 205 && rdesc[190] == 0x85 && rdesc[191] == 0x5a
        && rdesc[204] == 0x95 && rdesc[205] == 0x05
    {
        log::debug!("xhci: fixing ASUS N-KEY report descriptor (331 B)");
        rdesc[205] = 0x01;
    }
    if nkey && rdesc.len() > 15 {
        for i in 0..rdesc.len().saturating_sub(15) {
            if rdesc[i] == 0x85
                && rdesc[i + 1] == 0x5a
                && rdesc[i + 14] == 0x95
                && rdesc[i + 15] == 0x05
            {
                log::debug!("xhci: fixing ASUS N-KEY report descriptor at offset {}", i);
                rdesc[i + 15] = 0x01;
                break;
            }
        }
    }
}

const HID_USAGE_PAGE_GENERIC_DESKTOP: u16 = 0x01;
const HID_USAGE_PAGE_KEYBOARD: u16 = 0x07;
const HID_USAGE_KEYBOARD: u16 = 0x06;

#[derive(Clone, Copy, Debug, Default)]
struct FieldLayout {
    modifier_offset: usize,
    modifier_bytes: usize,
    reserved_offset: Option<usize>,
    keys_offset: usize,
    key_count: usize,
    report_len: usize,
}

fn parse_keyboard_layout(rdesc: &[u8]) -> Option<KeyboardReportLayout> {
    let mut best: Option<(u8, FieldLayout)> = None;
    let mut i = 0usize;
    let mut report_id: Option<u8> = None;
    let mut usage_page: u16 = 0;
    let mut report_size: u32 = 0;
    let mut report_count: u32 = 0;
    let mut usage_min: u32 = 0;
    let mut usage_max: u32 = 0;
    let mut collection_depth: u32 = 0;
    let mut in_keyboard_app = false;
    let mut bit_offset: usize = 0;
    let mut field = FieldLayout::default();

    while i < rdesc.len() {
        let head = rdesc[i];
        i += 1;
        let data_size = match head & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 4,
            _ => 0,
        };
        if i + data_size > rdesc.len() {
            break;
        }
        let mut data = 0u32;
        for b in 0..data_size {
            data |= (rdesc[i + b] as u32) << (8 * b);
        }
        i += data_size;

        let item_type = (head >> 2) & 0x03;
        let tag = head >> 4;

        match (item_type, tag) {
            (1, 0) => usage_page = data as u16,                 // Usage Page
            (1, 8) => {
                report_id = Some(data as u8);
                bit_offset = 0;
                field = FieldLayout::default();
            }
            (1, 7) => report_size = data, // Report Size
            (1, 9) => report_count = data, // Report Count
            (2, 0) => {
                usage_min = data;
                usage_max = data;
            }
            (2, 1) => usage_min = data,                          // Usage Minimum
            (2, 2) => usage_max = data,                          // Usage Maximum
            (0, 10) => {
                // Collection
                collection_depth += 1;
                if collection_depth == 1
                    && usage_page == HID_USAGE_PAGE_GENERIC_DESKTOP
                    && usage_min == HID_USAGE_KEYBOARD as u32
                {
                    in_keyboard_app = true;
                    bit_offset = 0;
                    field = FieldLayout::default();
                }
            }
            (0, 12) => {
                // End Collection
                if in_keyboard_app && collection_depth == 1 {
                    let report_len = (bit_offset + 7) / 8;
                    if field.key_count > 0 && report_len > 0 {
                        let id = report_id.unwrap_or(0);
                        let score = field.key_count;
                        let replace = best
                            .as_ref()
                            .map(|(_, f)| score > f.key_count)
                            .unwrap_or(true);
                        if replace {
                            best = Some((
                                id,
                                FieldLayout {
                                    report_len,
                                    ..field
                                },
                            ));
                        }
                    }
                    in_keyboard_app = false;
                    field = FieldLayout::default();
                    bit_offset = 0;
                }
                collection_depth = collection_depth.saturating_sub(1);
            }
            (0, 8) if in_keyboard_app => {
                // Input
                let byte_off = bit_offset / 8;
                if usage_page == HID_USAGE_PAGE_KEYBOARD {
                    if report_size == 1
                        && report_count == 8
                        && usage_min == 0xE0
                        && usage_max == 0xE7
                    {
                        field.modifier_offset = byte_off;
                        field.modifier_bytes = 1;
                    } else if report_size == 8 && report_count == 1 && (data & 0x01) != 0 {
                        field.reserved_offset = Some(byte_off);
                    } else if report_size == 8 && report_count >= 1 && (data & 0x01) == 0 {
                        field.keys_offset = byte_off;
                        field.key_count = report_count as usize;
                    }
                }
                bit_offset += (report_size * report_count) as usize;
            }
            _ => {}
        }
    }

    let (id, f) = best?;
    Some(KeyboardReportLayout {
        report_id: if report_id.is_some() { Some(id) } else { None },
        report_len: f.report_len + if report_id.is_some() { 1 } else { 0 },
        modifier_offset: f.modifier_offset,
        reserved_offset: f.reserved_offset.unwrap_or(f.modifier_offset + 1),
        keys_offset: f.keys_offset,
        key_count: f.key_count,
    })
}

// ---------------------------------------------------------------------------
// Keyboard state tracker
// ---------------------------------------------------------------------------

/// Tracks keyboard state across reports and generates press/release events
/// by diffing consecutive reports.
pub struct KeyboardState {
    /// Previous report (for diffing)
    prev_report: BootKeyboardReport,
    /// Queue of events to be consumed by the kernel
    event_queue: VecDeque<KeyEvent>,
    /// Layout parseado del report descriptor (boot o report protocol).
    layout: KeyboardReportLayout,
}

impl KeyboardState {
    /// Create a new keyboard state tracker.
    pub fn new(layout: KeyboardReportLayout) -> Self {
        log::debug!("xhci: keyboard state tracker initialized");
        Self {
            prev_report: BootKeyboardReport {
                modifiers: 0,
                reserved: 0,
                keycodes: [0; 6],
            },
            event_queue: VecDeque::new(),
            layout,
        }
    }

    /// Procesa un informe crudo según el layout parseado del descriptor.
    pub fn process_raw_report(&mut self, data: &[u8]) {
        if let Some(report) = self.layout.extract_report(data) {
            self.process_report(&report);
        }
    }

    /// Process a new HID boot protocol report and generate key events.
    pub fn process_report(&mut self, report: &BootKeyboardReport) {
        if report.is_rollover() {
            log::trace!("xhci: keyboard rollover detected, ignoring report");
            return;
        }

        // Detect modifier changes
        self.process_modifier_changes(report.modifiers);

        // Detect released keys (in prev but not in new)
        for &prev_key in &self.prev_report.keycodes {
            if prev_key != 0 && !report.has_keycode(prev_key) {
                let scancode = hid_usage_to_scancode(prev_key);
                log::trace!(
                    "xhci: key released: usage={:#x} scancode={:#x}",
                    prev_key, scancode
                );
                self.event_queue.push_back(KeyEvent {
                    usage_id: prev_key,
                    scancode,
                    pressed: false,
                    modifiers: report.modifiers,
                });
            }
        }

        // Detect pressed keys (in new but not in prev)
        for &new_key in &report.keycodes {
            if new_key != 0 && !self.prev_report.has_keycode(new_key) {
                let scancode = hid_usage_to_scancode(new_key);
                log::trace!(
                    "xhci: key pressed: usage={:#x} scancode={:#x}",
                    new_key, scancode
                );
                self.event_queue.push_back(KeyEvent {
                    usage_id: new_key,
                    scancode,
                    pressed: true,
                    modifiers: report.modifiers,
                });
            }
        }

        self.prev_report = *report;
    }

    /// Process modifier key changes as individual events.
    fn process_modifier_changes(&mut self, new_mods: u8) {
        let old_mods = self.prev_report.modifiers;
        let changed = old_mods ^ new_mods;

        if changed == 0 {
            return;
        }

        // Map each modifier bit to a USB HID usage ID
        // Left Ctrl=0xE0, Left Shift=0xE1, Left Alt=0xE2, Left GUI=0xE3
        // Right Ctrl=0xE4, Right Shift=0xE5, Right Alt=0xE6, Right GUI=0xE7
        let mod_bits = [
            (MOD_LEFT_CTRL, 0xE0u8),
            (MOD_LEFT_SHIFT, 0xE1),
            (MOD_LEFT_ALT, 0xE2),
            (MOD_LEFT_GUI, 0xE3),
            (MOD_RIGHT_CTRL, 0xE4),
            (MOD_RIGHT_SHIFT, 0xE5),
            (MOD_RIGHT_ALT, 0xE6),
            (MOD_RIGHT_GUI, 0xE7),
        ];

        // PS/2 scancodes for modifier keys
        let mod_scancodes: [u8; 8] = [
            0x1D, // Left Ctrl
            0x2A, // Left Shift
            0x38, // Left Alt
            0x00, // Left GUI (no PS/2 equivalent in set 1 base)
            0x1D, // Right Ctrl (extended)
            0x36, // Right Shift
            0x38, // Right Alt (extended)
            0x00, // Right GUI
        ];

        for (i, &(bit, usage)) in mod_bits.iter().enumerate() {
            if changed & bit != 0 {
                let pressed = new_mods & bit != 0;
                let scancode = mod_scancodes[i];
                log::trace!(
                    "xhci: modifier {}: usage={:#x} scancode={:#x} pressed={}",
                    i, usage, scancode, pressed
                );
                self.event_queue.push_back(KeyEvent {
                    usage_id: usage,
                    scancode,
                    pressed,
                    modifiers: new_mods,
                });
            }
        }
    }

    /// Dequeue the next key event, if any.
    pub fn next_event(&mut self) -> Option<KeyEvent> {
        self.event_queue.pop_front()
    }

    /// Check if there are pending key events.
    pub fn has_events(&self) -> bool {
        !self.event_queue.is_empty()
    }

    /// Get the current modifier state.
    pub fn modifiers(&self) -> u8 {
        self.prev_report.modifiers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Descriptor mínimo con Report ID 0x04 (System Control en 0b05:1866, no teclado).
    fn sample_report_id4_descriptor() -> Vec<u8> {
        vec![
            0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x04, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7,
            0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x75, 0x08, 0x95, 0x01,
            0x81, 0x01, 0x75, 0x08, 0x95, 0x06, 0x81, 0x00, 0xc0,
        ]
    }

    /// Report descriptor real del teclado ROG 0b05:18c6 (231 B, SOSODRV run5).
    fn sample_18c6_descriptor() -> Vec<u8> {
        vec![
            0x06, 0x89, 0xff, 0x09, 0x10, 0xa1, 0x01, 0x85, 0xa5, 0x09, 0x01, 0x15, 0x00, 0x26,
            0xff, 0x00, 0x75, 0x08, 0x95, 0x10, 0xb1, 0x00, 0xc0, 0x06, 0x82, 0xff, 0x09, 0xcf,
            0xa1, 0x01, 0x85, 0xc1, 0x09, 0x62, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95,
            0x3c, 0xb1, 0x02, 0x09, 0x66, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x10,
            0x81, 0x02, 0x09, 0x61, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x3c, 0x91,
            0x02, 0x85, 0xc2, 0x09, 0x8a, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x10,
            0x81, 0x02, 0x09, 0x8e, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x10, 0x91,
            0x02, 0xc0, 0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x01, 0x75, 0x01, 0x95, 0x08,
            0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00, 0x25, 0x01, 0x81, 0x02, 0x95, 0x01,
            0x75, 0x08, 0x81, 0x03, 0x95, 0x05, 0x75, 0x01, 0x05, 0x08, 0x19, 0x01, 0x29, 0x05,
            0x91, 0x02, 0x95, 0x01, 0x75, 0x03, 0x91, 0x03, 0x05, 0x07, 0x19, 0x00, 0x29, 0xff,
            0x15, 0x00, 0x25, 0x00, 0x95, 0x1e, 0x75, 0x08, 0x81, 0x00, 0x05, 0x07, 0x19, 0x00,
            0x29, 0xdf, 0x15, 0x00, 0x25, 0x01, 0x95, 0xe0, 0x75, 0x01, 0x81, 0x02, 0xc0, 0x05,
            0x0c, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x02, 0x19, 0x00, 0x2a, 0x3c, 0x02, 0x15, 0x00,
            0x26, 0x3c, 0x02, 0x75, 0x10, 0x95, 0x01, 0x81, 0x00, 0xc0, 0x05, 0x01, 0x09, 0x80,
            0xa1, 0x01, 0x85, 0x04, 0x19, 0x81, 0x29, 0x83, 0x15, 0x00, 0x25, 0x01, 0x95, 0x08,
            0x75, 0x01, 0x81, 0x02, 0xc0,
        ]
    }

    #[test]
    fn report_id4_1866_no_es_coleccion_teclado() {
        // System Control (Usage 0x80) con Report ID 0x04 — como en el 1866 real, no teclado.
        let rdesc = vec![
            0x05, 0x01, 0x09, 0x80, 0xa1, 0x01, 0x85, 0x04, 0x19, 0x81, 0x29, 0x83, 0x15, 0x00,
            0x25, 0x01, 0x95, 0x08, 0x75, 0x01, 0x81, 0x02, 0xc0,
        ];
        let layout = KeyboardReportLayout::from_descriptor(&rdesc, 0x0b05, 0x1866);
        assert_eq!(layout, KeyboardReportLayout::boot());
    }

    #[test]
    fn rog_18c6_descriptor_layout() {
        let layout =
            KeyboardReportLayout::from_descriptor(&sample_18c6_descriptor(), 0x0b05, 0x18c6);
        assert_eq!(layout.report_id, Some(0x01));
        // Offsets relativos al payload tras el byte de Report ID.
        assert_eq!(layout.modifier_offset, 0);
        assert_eq!(layout.keys_offset, 2);
        assert_eq!(layout.key_count, 30);
    }

    #[test]
    fn rog_18c6_shift_a_report_protocol() {
        let layout =
            KeyboardReportLayout::from_descriptor(&sample_18c6_descriptor(), 0x0b05, 0x18c6);
        let mut report = vec![0u8; layout.report_len];
        report[0] = 0x01;
        report[1] = 0x02;
        report[3] = 0x04;
        let boot = layout.extract_report(&report).expect("extract");
        assert_eq!(boot.modifiers, 0x02);
        assert_eq!(boot.keycodes[0], 0x04);
        assert_eq!(hid_usage_to_scancode(0x04), 0x1e);
    }

    #[test]
    fn rog_18c6_boot_protocol_shift_a() {
        let layout = KeyboardReportLayout::boot();
        let report = [0x02, 0x00, 0x04, 0, 0, 0, 0, 0];
        let mut state = KeyboardState::new(layout);
        state.process_raw_report(&report);
        let mut key_a = None;
        while let Some(evt) = state.next_event() {
            if evt.usage_id == 0x04 && evt.pressed {
                key_a = Some(evt);
                break;
            }
        }
        let evt = key_a.expect("key a pressed");
        assert_eq!(evt.scancode, 0x1e);
    }

    #[test]
    fn boot_layout_sin_report_id() {
        let layout = KeyboardReportLayout::boot();
        let report = [0x00, 0x00, 0x1e, 0, 0, 0, 0, 0];
        let boot = layout.extract_report(&report).expect("extract");
        assert_eq!(boot.keycodes[0], 0x1e);
    }

    #[test]
    fn asus_nkey_quirk_fixes_0x5a_count() {
        let mut rdesc = sample_report_id4_descriptor();
        let base = rdesc.len();
        rdesc.extend_from_slice(&[
            0x85, 0x5a, 0x09, 0x00, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x81, 0x00, 0x00,
            0x95, 0x05,
        ]);
        assert_eq!(rdesc[base + 14], 0x95);
        assert_eq!(rdesc[base + 15], 0x05);
        fix_asus_nkey_descriptor(&mut rdesc, 0x0b05, 0x1866);
        assert_eq!(rdesc[base + 15], 0x01);
    }
}
