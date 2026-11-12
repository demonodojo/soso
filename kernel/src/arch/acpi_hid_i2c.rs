//! Inventario ACPI: controladores AMDI0010/AMDI0030 y touchpads PNP0C50.

use alloc::vec::Vec;

use super::acpi;

#[derive(Clone, Copy, Debug)]
pub struct HidI2cBinding {
    pub i2c_mmio: u64,
    pub i2c_len: u32,
    pub slave: u8,
    /// GSI directo del dispositivo (Interrupt extendido), si existe.
    pub gsi: Option<u32>,
    pub gsi_flags: u16,
    /// Pin GpioInt (AMDI0030), si no hay GSI directo.
    pub gpio_pin: Option<u16>,
}

#[derive(Clone, Copy, Debug)]
pub struct AmdGpioBlock {
    pub mmio: u64,
    pub len: u32,
    pub gsi: Option<u32>,
    pub gsi_flags: u16,
}

pub fn discover() -> Vec<HidI2cBinding> {
    let mut mmios: Vec<(u64, u32)> = Vec::new();
    let mut slaves: Vec<(u8, Option<u32>, u16, Option<u16>)> = Vec::new();
    acpi::each_table(b"DSDT", |aml| scan_aml(aml, &mut mmios, &mut slaves));
    acpi::each_table(b"SSDT", |aml| scan_aml(aml, &mut mmios, &mut slaves));
    if mmios.is_empty() {
        crate::klogln!("hid-i2c: sin AMDI0010 en ACPI");
        return Vec::new();
    }
    if slaves.is_empty() {
        crate::klogln!("hid-i2c: sin PNP0C50 en ACPI");
        return Vec::new();
    }
    let (i2c_mmio, i2c_len) = mmios[0];
    let mut out = Vec::new();
    for &(slave, gsi, gsi_flags, gpio_pin) in &slaves {
        out.push(HidI2cBinding {
            i2c_mmio,
            i2c_len,
            slave,
            gsi,
            gsi_flags,
            gpio_pin,
        });
        crate::println!(
            "hid-i2c: PNP0C50 esclavo {slave:#04x} bus mmio={i2c_mmio:#x} len={i2c_len} gsi={gsi:?} gpio={gpio_pin:?}"
        );
    }
    out
}

pub fn discover_amd_gpio() -> Option<AmdGpioBlock> {
    let mut found = None;
    acpi::each_table(b"DSDT", |aml| {
        if found.is_none() {
            found = scan_amd_gpio(aml);
        }
    });
    acpi::each_table(b"SSDT", |aml| {
        if found.is_none() {
            found = scan_amd_gpio(aml);
        }
    });
    if let Some(b) = found {
        crate::klogln!(
            "amd-gpio: AMDI0030 mmio={:#x} len={} gsi={:?}",
            b.mmio,
            b.len,
            b.gsi
        );
    }
    found
}

fn scan_amd_gpio(aml: &[u8]) -> Option<AmdGpioBlock> {
    for (off, _) in find_all(aml, b"AMDI0030") {
        let (mmio, len) = scan_memory32fixed(aml, off)?;
        let (gsi, gsi_flags) = scan_extended_interrupt(aml, off)
            .map(|(g, f)| (Some(g), f))
            .unwrap_or((None, 0));
        return Some(AmdGpioBlock {
            mmio,
            len,
            gsi,
            gsi_flags,
        });
    }
    None
}

fn scan_aml(
    aml: &[u8],
    mmios: &mut Vec<(u64, u32)>,
    slaves: &mut Vec<(u8, Option<u32>, u16, Option<u16>)>,
) {
    for (off, _) in find_all(aml, b"AMDI0010") {
        if let Some(pair) = scan_memory32fixed(aml, off) {
            if !mmios.iter().any(|m| m.0 == pair.0) {
                mmios.push(pair);
            }
        }
    }
    for (off, _) in find_all(aml, b"PNP0C50") {
        if let Some(slave) = scan_i2c_slave(aml, off) {
            let (gsi, gsi_flags) = scan_extended_interrupt(aml, off)
                .map(|(g, f)| (Some(g), f))
                .unwrap_or((None, 0));
            let gpio_pin = scan_gpio_int_pin(aml, off);
            if !slaves.iter().any(|s| s.0 == slave) {
                slaves.push((slave, gsi, gsi_flags, gpio_pin));
            }
        }
    }
}

fn find_all(hay: &[u8], needle: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if &hay[i..i + needle.len()] == needle {
            out.push((i, i));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

fn scan_memory32fixed(aml: &[u8], from: usize) -> Option<(u64, u32)> {
    let end = (from + 800).min(aml.len());
    let mut i = from;
    while i + 11 <= end {
        if aml[i] == 0x86 && aml[i + 1] == 0x09 {
            let addr = u32::from_le_bytes(aml[i + 3..i + 7].try_into().ok()?);
            let len = u32::from_le_bytes(aml[i + 7..i + 11].try_into().ok()?);
            if addr != 0 && len >= 0x1000 {
                return Some((u64::from(addr), len));
            }
        }
        i += 1;
    }
    None
}

/// Interrupt extendido (0x89): primer GSI de la tabla.
fn scan_extended_interrupt(aml: &[u8], from: usize) -> Option<(u32, u16)> {
    let end = (from + 800).min(aml.len());
    let mut i = from;
    while i + 9 <= end {
        if aml[i] == 0x89 {
            let len = u16::from_le_bytes([aml[i + 1], aml[i + 2]]) as usize;
            if len >= 6 && i + 3 + len <= aml.len() {
                let flags = aml[i + 3] as u16;
                let table_len = aml[i + 4] as usize;
                if table_len >= 1 && len >= 5 + 4 {
                    let gsi = u32::from_le_bytes(aml[i + 5..i + 9].try_into().ok()?);
                    if gsi != 0 && gsi < 4096 {
                        return Some((gsi, flags));
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// GpioInt (0x8C): primer pin de la tabla de pines.
fn scan_gpio_int_pin(aml: &[u8], from: usize) -> Option<u16> {
    let end = (from + 800).min(aml.len());
    let mut i = from;
    while i + 16 <= end {
        if aml[i] == 0x8c {
            let len = u16::from_le_bytes([aml[i + 1], aml[i + 2]]) as usize;
            if len < 12 || i + 3 + len > aml.len() {
                i += 1;
                continue;
            }
            // Byte 4: Connection Type — 0 = Interrupt.
            if aml[i + 4] != 0 {
                i += 1;
                continue;
            }
            let pin_table_off = u16::from_le_bytes([aml[i + 10], aml[i + 11]]) as usize;
            let pin_at = i + pin_table_off;
            if pin_at + 2 <= aml.len() && pin_table_off >= 12 {
                let pin = u16::from_le_bytes([aml[pin_at], aml[pin_at + 1]]);
                if pin < 512 {
                    return Some(pin);
                }
            }
        }
        i += 1;
    }
    None
}

/// I2CSerialBusV2 en `_CRS` (tipo I2C = 0, esclavo 7 bits).
fn scan_i2c_slave(aml: &[u8], from: usize) -> Option<u8> {
    let end = (from + 800).min(aml.len());
    let mut i = from;
    while i + 8 <= end {
        if aml[i] & 0x80 != 0 {
            let len = (aml[i] & 0x7f) as usize;
            if i + 1 < end && aml[i + 1] == 0x01 && len >= 8 && i + len + 1 <= end {
                if aml[i + 4] == 0 {
                    let raw = u16::from_le_bytes([aml[i + 6], aml[i + 7]]);
                    return Some((raw >> 1) as u8);
                }
            }
            i += len.saturating_add(1).max(1);
        } else {
            i += 1;
        }
    }
    None
}
