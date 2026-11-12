//! Touchpad interno vía HID-over-I2C (PNP0C50 + AMDI0010).

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use spin::Mutex;
use xhci_nostd::hid::{PointerReportLayout, PointerState};

use super::i2c_dw::DesignWareI2c;
use crate::arch::acpi_hid_i2c::{self, HidI2cBinding};

static IRQ_PENDIENTE: AtomicBool = AtomicBool::new(false);

pub fn marcar_irq_pendiente() {
    IRQ_PENDIENTE.store(true, Ordering::Release);
    crate::net::marcar_trabajo_pendiente();
}

pub fn tomar_irq_pendiente() -> bool {
    IRQ_PENDIENTE.swap(false, Ordering::AcqRel)
}

struct HidI2cDevice {
    bus: DesignWareI2c,
    slave: u8,
    input_reg: u16,
    max_input: u16,
    state: PointerState,
}

static DEVICES: Mutex<Vec<HidI2cDevice>> = Mutex::new(Vec::new());

#[repr(C, packed)]
struct HidDesc {
    len: u16,
    bcd: u16,
    rdesc_len: u16,
    rdesc_reg: u16,
    input_reg: u16,
    max_input: u16,
}

pub fn init() {
    let bindings = acpi_hid_i2c::discover();
    let mut devs = DEVICES.lock();
    for b in bindings.iter().copied() {
        if let Some(d) = probe_binding(b) {
            arm_attention(b);
            devs.push(d);
        }
    }
    if !devs.is_empty() {
        crate::println!("hid-i2c: {} touchpad(s) listo(s)", devs.len());
    }
}

fn hid_gsi_irq() {
    marcar_irq_pendiente();
}

fn arm_attention(b: HidI2cBinding) {
    if let Some(gsi) = b.gsi {
        if let Some(vec) = crate::arch::irq::allocate(hid_gsi_irq) {
            match crate::arch::ioapic::route(gsi, vec, crate::arch::apic::id(), b.gsi_flags) {
                Ok(()) => {
                    crate::println!("hid-i2c: irq gsi={gsi} vector={vec:#x}");
                    return;
                }
                Err(e) => crate::klogln!("hid-i2c: route gsi={gsi} fallo ({e})"),
            }
        }
    }
    let Some(pin) = b.gpio_pin else {
        crate::klogln!("hid-i2c: sin GSI ni GpioInt; solo sondeo");
        return;
    };
    let Some(gpio) = acpi_hid_i2c::discover_amd_gpio() else {
        crate::klogln!("hid-i2c: GpioInt pin={pin} sin AMDI0030");
        return;
    };
    let Some(gsi) = gpio.gsi else {
        crate::klogln!("hid-i2c: AMDI0030 sin Interrupt");
        return;
    };
    if let Err(e) = super::amd_gpio::arm_pin(gpio.mmio, gpio.len, pin, gsi, gpio.gsi_flags) {
        crate::klogln!("hid-i2c: amd-gpio pin={pin} fallo ({e})");
    }
}

pub fn poll() {
    let mut devs = DEVICES.lock();
    for dev in devs.iter_mut() {
        let mut buf = [0u8; 64];
        let len = dev.max_input.min(64) as usize;
        if !dev
            .bus
            .read_reg16(dev.slave, dev.input_reg, &mut buf[..len])
        {
            continue;
        }
        dev.state.process_raw_report(&buf[..len]);
        while let Some(evt) = dev.state.next_event() {
            super::mouse::apply_hid_pointer(evt);
        }
    }
}

fn probe_binding(b: HidI2cBinding) -> Option<HidI2cDevice> {
    let bus = DesignWareI2c::new(b.i2c_mmio, b.i2c_len);
    let mut hdr = [0u8; 6];
    if !bus.read_reg16(b.slave, 0, &mut hdr) {
        crate::klogln!("hid-i2c: esclavo {:#04x} sin HID descriptor", b.slave);
        return None;
    }
    let desc_len = u16::from_le_bytes([hdr[0], hdr[1]]) as usize;
    if desc_len < core::mem::size_of::<HidDesc>() || desc_len > 256 {
        crate::klogln!("hid-i2c: desc_len raro {desc_len}");
        return None;
    }
    let mut raw = vec![0u8; desc_len];
    if !bus.read_reg16(b.slave, 0, &mut raw) {
        return None;
    }
    let hd = unsafe { &*(raw.as_ptr() as *const HidDesc) };
    let rdesc_len = hd.rdesc_len as usize;
    if rdesc_len == 0 || rdesc_len > 4096 {
        return None;
    }
    let mut rdesc = vec![0u8; rdesc_len];
    if !bus.read_reg16(b.slave, hd.rdesc_reg, &mut rdesc) {
        crate::klogln!("hid-i2c: report descriptor ilegible");
        return None;
    }
    let layout = PointerReportLayout::from_descriptor(&rdesc).unwrap_or_else(|| {
        crate::klogln!("hid-i2c: descriptor sin puntero; boot fallback");
        PointerReportLayout::boot()
    });
    Some(HidI2cDevice {
        bus,
        slave: b.slave,
        input_reg: hd.input_reg,
        max_input: hd.max_input.max(8),
        state: PointerState::new(layout),
    })
}
