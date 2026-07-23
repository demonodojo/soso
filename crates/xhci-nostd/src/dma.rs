//! Asignación DMA con dirección física real (no identity map).

use alloc::alloc::{alloc_zeroed, Layout};

static mut ALLOC: Option<fn(usize, usize) -> (*mut u8, u64)> = None;

/// Registra un allocator `(size, align) -> (virt, phys)` antes de `XhciController::init`.
pub fn set_allocator(f: fn(usize, usize) -> (*mut u8, u64)) {
    unsafe { ALLOC = Some(f); }
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
