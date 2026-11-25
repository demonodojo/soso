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

/// Pines que se recorren al limpiar (bancos 0–2 de `pinctrl-amd`). El
/// registro del pin 63 (0xfc) es `WAKE_INT_MASTER_REG`, no un pin.
const PINES: u16 = 184;

fn es_pin(p: u16, len: u32) -> bool {
    p != 63 && (p as u32 + 1) * 4 <= len
}
const CLR_INTR_STAT: u32 = 1;

static BASE: AtomicU64 = AtomicU64::new(0);
static PIN: AtomicU16 = AtomicU16::new(0xffff);
static LEN: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
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
    if !es_pin(pin, mmio_len) {
        return Err("pin fuera del bloque");
    }
    mm::ensure_mmio_mapped(mmio, mmio_len as u64);
    // La línea es una para todos los pines: uno que dejó habilitado el
    // firmware, con su estado pendiente, la tendría levantada para siempre.
    // Como Linux al arrancar, se apagan las interrupciones de los demás.
    let mut apagados = 0u32;
    for p in 0..PINES {
        if p == pin || !es_pin(p, mmio_len) {
            continue;
        }
        let r = rr(mmio, p as u64 * 4);
        let bits = (1 << INTERRUPT_ENABLE_OFF) | (1 << INTERRUPT_MASK_OFF);
        if r & bits != 0 {
            rw(mmio, p as u64 * 4, (r & !bits) | (CLR_INTR_STAT << INTERRUPT_STS_OFF));
            apagados += 1;
        }
    }
    if apagados != 0 {
        crate::klogln!("amd-gpio: {apagados} pin(es) con interrupción del firmware apagados");
    }
    LEN.store(mmio_len, Ordering::Release);
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
    // Todos los pendientes, no sólo el nuestro: si quedara uno, la línea no
    // bajaría nunca.
    let len = LEN.load(Ordering::Acquire);
    for p in 0..PINES {
        if !es_pin(p, len) {
            continue;
        }
        let r = rr(base, p as u64 * 4);
        if r & (1 << INTERRUPT_STS_OFF) != 0 {
            rw(base, p as u64 * 4, r | (CLR_INTR_STAT << INTERRUPT_STS_OFF));
        }
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
