//! Inventario ACPI: controladores AMDI0010 y touchpads PNP0C50.

use alloc::vec::Vec;

use super::acpi;

#[derive(Clone, Copy, Debug)]
pub struct HidI2cBinding {
    pub i2c_mmio: u64,
    pub i2c_len: u32,
    pub slave: u8,
}

pub fn discover() -> Vec<HidI2cBinding> {
    let mut mmios: Vec<(u64, u32)> = Vec::new();
    let mut slaves: Vec<u8> = Vec::new();
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
    for &slave in &slaves {
        out.push(HidI2cBinding {
            i2c_mmio,
            i2c_len,
            slave,
        });
        crate::println!(
            "hid-i2c: PNP0C50 esclavo {slave:#04x} bus mmio={i2c_mmio:#x} len={i2c_len}"
        );
    }
    out
}

fn scan_aml(aml: &[u8], mmios: &mut Vec<(u64, u32)>, slaves: &mut Vec<u8>) {
    for (off, _) in find_all(aml, b"AMDI0010") {
        if let Some(pair) = scan_memory32fixed(aml, off) {
            if !mmios.iter().any(|m| m.0 == pair.0) {
                mmios.push(pair);
            }
        }
    }
    for (off, _) in find_all(aml, b"PNP0C50") {
        if let Some(slave) = scan_i2c_slave(aml, off) {
            if !slaves.contains(&slave) {
                slaves.push(slave);
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
