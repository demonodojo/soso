//! GPIO AMD (AMDI0030) mínimo: atención de un pin para HID-over-I2C.
//!
//! Registros según Linux `pinctrl-amd` (6.6.32).

use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};

use crate::arch::{apic, ioapic, irq};
use crate::mm;

const WAKE_INT_MASTER_REG: u64 = 0xfc;
const EOI_MASK: u32 = 1 << 29;

const LEVEL_TRIG_OFF: u32 = 8;
const ACTIVE_LEVEL_OFF: u32 = 9;
const INTERRUPT_ENABLE_OFF: u32 = 11;
const INTERRUPT_MASK_OFF: u32 = 12;
const INTERRUPT_STS_OFF: u32 = 28;

const BOTH_EDGE: u32 = 0x2;
const CLR_INTR_STAT: u32 = 1;

static BASE: AtomicU64 = AtomicU64::new(0);
static PIN: AtomicU16 = AtomicU16::new(0xffff);
static LISTO: AtomicBool = AtomicBool::new(false);

fn rr(base: u64, off: u64) -> u32 {
    unsafe { core::ptr::read_volatile(mm::phys_to_virt(base + off).as_ptr()) }
}

fn rw(base: u64, off: u64, v: u32) {
    unsafe {
        core::ptr::write_volatile(mm::phys_to_virt(base + off).as_mut_ptr(), v);
    }
}

/// Habilita IRQ de flanco en `pin` y enruta el GSI del bloque GPIO.
pub fn arm_pin(mmio: u64, mmio_len: u32, pin: u16, gsi: u32, gsi_flags: u16) -> Result<(), &'static str> {
    if mmio == 0 || mmio_len < 0x300 {
        return Err("mmio gpio corto");
    }
    mm::ensure_mmio_mapped(mmio, mmio_len as u64);
    let Some(vec) = irq::allocate(gpio_irq) else {
        return Err("sin vector IRQ");
    };
    ioapic::route(gsi, vec, apic::id(), gsi_flags)?;

    // Tipo: flanco ambos; enable + unmask; clear STS.
    let mut pin_reg = rr(mmio, pin as u64 * 4);
    pin_reg &= !(1 << LEVEL_TRIG_OFF);
    pin_reg &= !(0b11 << ACTIVE_LEVEL_OFF);
    pin_reg |= BOTH_EDGE << ACTIVE_LEVEL_OFF;
    pin_reg |= 1 << INTERRUPT_ENABLE_OFF;
    pin_reg |= 1 << INTERRUPT_MASK_OFF;
    pin_reg |= CLR_INTR_STAT << INTERRUPT_STS_OFF;
    rw(mmio, pin as u64 * 4, pin_reg);

    BASE.store(mmio, Ordering::Release);
    PIN.store(pin, Ordering::Release);
    LISTO.store(true, Ordering::Release);
    crate::println!("amd-gpio: pin={pin} gsi={gsi} vector={vec:#x}");
    Ok(())
}

fn gpio_irq() {
    let base = BASE.load(Ordering::Acquire);
    let pin = PIN.load(Ordering::Acquire);
    if base == 0 || pin == 0xffff {
        return;
    }
    let mut pin_reg = rr(base, pin as u64 * 4);
    if pin_reg & (1 << INTERRUPT_STS_OFF) != 0 {
        pin_reg |= CLR_INTR_STAT << INTERRUPT_STS_OFF;
        rw(base, pin as u64 * 4, pin_reg);
    }
    // EOI del bloque.
    let mut master = rr(base, WAKE_INT_MASTER_REG);
    master |= EOI_MASK;
    rw(base, WAKE_INT_MASTER_REG, master);
    crate::drivers::hid_i2c::marcar_irq_pendiente();
}

#[allow(dead_code)]
pub fn present() -> bool {
    LISTO.load(Ordering::Acquire)
}
