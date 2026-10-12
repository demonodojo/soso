//! IOAPIC: redirección de IRQs legacy/INTx a vectores LAPIC.
//!
//! Usa la base del primer IOAPIC del MADT y aplica Interrupt Source Override.

use crate::arch::acpi;
use crate::mm;
use spin::Once;

const IOREGSEL: u64 = 0x00;
const IOWIN: u64 = 0x10;
const IOAPICVER: u32 = 0x01;
const IOREDTBL: u32 = 0x10;

struct IoApic {
    base: u64,
    gsi_base: u32,
    max_entry: u32,
}

static IOAPIC: Once<IoApic> = Once::new();

fn sel_write(base: u64, reg: u32) {
    unsafe {
        core::ptr::write_volatile(mm::phys_to_virt(base + IOREGSEL).as_mut_ptr::<u32>(), reg);
    }
}

fn win_read(base: u64) -> u32 {
    unsafe { core::ptr::read_volatile(mm::phys_to_virt(base + IOWIN).as_ptr::<u32>()) }
}

fn win_write(base: u64, val: u32) {
    unsafe {
        core::ptr::write_volatile(mm::phys_to_virt(base + IOWIN).as_mut_ptr::<u32>(), val);
    }
}

fn read_reg(base: u64, reg: u32) -> u32 {
    sel_write(base, reg);
    win_read(base)
}

fn write_reg(base: u64, reg: u32, val: u32) {
    sel_write(base, reg);
    win_write(base, val);
}

fn write_rte_raw(base: u64, index: u32, low: u32, high: u32) {
    write_reg(base, IOREDTBL + index * 2, low);
    write_reg(base, IOREDTBL + index * 2 + 1, high);
}

/// Inicializa el primer IOAPIC del MADT (si existe). Enmascara todas las RTE.
pub fn init() {
    let madt = acpi::madt();
    let Some(info) = madt.ioapics.first() else {
        crate::println!("ioapic: ninguno en MADT");
        return;
    };
    let base = info.address as u64;
    mm::ensure_mmio_mapped(base, 0x20);
    let ver = read_reg(base, IOAPICVER);
    let max_entry = (ver >> 16) & 0xff;
    for i in 0..=max_entry {
        write_rte_raw(base, i, 1 << 16, 0);
    }
    IOAPIC.call_once(|| IoApic {
        base,
        gsi_base: info.gsi_base,
        max_entry,
    });
    crate::println!(
        "ioapic: base={:#x} gsi_base={} max_entry={}",
        base,
        info.gsi_base,
        max_entry
    );
}

/// Traduce IRQ ISA → (GSI, flags) aplicando overrides del MADT.
pub fn irq_to_gsi(irq: u8) -> (u32, u16) {
    acpi::madt()
        .overrides
        .iter()
        .find(|o| o.irq == irq)
        .map(|o| (o.gsi, o.flags))
        .unwrap_or((irq as u32, 0))
}

/// Programa una RTE: GSI → vector en el LAPIC destino.
///
/// `flags`: bits ACPI MPS (polarity 0-1, trigger 2-3).
pub fn route(gsi: u32, vector: u8, dest_apic: u32, flags: u16) -> Result<(), &'static str> {
    let io = IOAPIC.get().ok_or("ioapic no inicializado")?;
    if gsi < io.gsi_base {
        return Err("gsi por debajo de gsi_base");
    }
    let index = gsi - io.gsi_base;
    if index > io.max_entry {
        return Err("gsi fuera de rango del IOAPIC");
    }
    let active_low = (flags & 0b11) == 0b11;
    let level = ((flags >> 2) & 0b11) == 0b11;
    let mut low = vector as u32;
    if active_low {
        low |= 1 << 13;
    }
    if level {
        low |= 1 << 15;
    }
    let high = dest_apic << 24;
    write_rte_raw(io.base, index, low, high);
    Ok(())
}

/// Enruta un IRQ ISA (aplicando ISO) al vector dado.
pub fn route_isa(irq: u8, vector: u8, dest_apic: u32) -> Result<(), &'static str> {
    let (gsi, flags) = irq_to_gsi(irq);
    route(gsi, vector, dest_apic, flags)
}

/// Enmascara la RTE del GSI.
#[allow(dead_code)]
pub fn mask(gsi: u32) -> Result<(), &'static str> {
    let io = IOAPIC.get().ok_or("ioapic no inicializado")?;
    if gsi < io.gsi_base {
        return Err("gsi por debajo de gsi_base");
    }
    let index = gsi - io.gsi_base;
    if index > io.max_entry {
        return Err("gsi fuera de rango");
    }
    let low = read_reg(io.base, IOREDTBL + index * 2);
    write_reg(io.base, IOREDTBL + index * 2, low | (1 << 16));
    Ok(())
}
