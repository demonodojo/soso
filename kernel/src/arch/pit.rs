//! PIT (8253/8254) canal 0 como timer del sistema a 100 Hz.

use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::port::Port;

pub const HZ: u64 = 100;
const PIT_FREQ: u32 = 1_193_182;

static TICKS: AtomicU64 = AtomicU64::new(0);

pub fn init() {
    let divisor = (PIT_FREQ / HZ as u32) as u16;
    let mut cmd: Port<u8> = Port::new(0x43);
    let mut ch0: Port<u8> = Port::new(0x40);
    unsafe {
        // Canal 0, acceso lo/hi, modo 3 (onda cuadrada), binario.
        cmd.write(0x36);
        ch0.write(divisor as u8);
        ch0.write((divisor >> 8) as u8);
    }
}

/// Solo desde el handler de IRQ0.
pub fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub fn uptime_ms() -> u64 {
    // En placa x2APIC el IRQ0 no llega (PIT/IOAPIC mudos). El TSC sí:
    // sin esto `sleep_ms`, connect y read_timeout nunca vencen y `ask` se
    // queda colgado. `tsc::now_ns` no llama aquí cuando el TSC está listo.
    if crate::arch::tsc::ready() {
        crate::arch::tsc::now_ns() / 1_000_000
    } else {
        ticks() * (1000 / HZ)
    }
}
