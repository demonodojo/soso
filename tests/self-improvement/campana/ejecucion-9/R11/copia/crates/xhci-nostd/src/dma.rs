//! Asignación DMA con dirección física real (no identity map).

use alloc::alloc::{alloc_zeroed, Layout};
use core::hint::spin_loop;

static mut ALLOC: Option<fn(usize, usize) -> (*mut u8, u64)> = None;
static mut DELAY_US: Option<fn(u32)> = None;

/// Registra un allocator `(size, align) -> (virt, phys)` antes de `XhciController::init`.
pub fn set_allocator(f: fn(usize, usize) -> (*mut u8, u64)) {
    unsafe { ALLOC = Some(f); }
}

/// Registra espera en microsegundos (p. ej. `tsc::spin_us`) para handoff/HCRST.
pub fn set_delay_us(f: fn(u32)) {
    unsafe { DELAY_US = Some(f); }
}

/// Espera activa; con hook del kernel ≈ Linux `udelay`/`handshake`.
pub fn delay_us(us: u32) {
    if let Some(f) = unsafe { DELAY_US } {
        f(us);
        return;
    }
    // Fallback burdo si nadie registró TSC (~unos µs por 1000 spins).
    for _ in 0..(us as u64).saturating_mul(2000) {
        spin_loop();
    }
}

/// Reserva memoria alineada; usa el hook del kernel si está registrado.
pub unsafe fn alloc_aligned(size: usize, align: usize) -> (*mut u8, u64) {
    if let Some(f) = ALLOC {
        return f(size, align);
    }
    let layout = Layout::from_size_align(size, align).expect("xhci dma layout");
    let ptr = alloc_zeroed(layout);
    assert!(!ptr.is_null(), "xhci dma oom");
    (ptr, ptr as u64)
}
