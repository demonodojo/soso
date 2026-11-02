//! Alocador de `std` en soso: el mismo estado que libsoso (`soso-alloc`).
//!
//! `sys/alloc/mod.rs` reexporta `alloc` / `dealloc` / `realloc` desde este
//! módulo. `alloc_zeroed` lo pone el `cfg_select` de ese fichero (reserva y
//! luego pone ceros), así que aquí no se duplica.

use crate::alloc::Layout;

#[inline]
pub unsafe fn alloc(layout: Layout) -> *mut u8 {
    unsafe { soso_rt::alloc(layout) }
}

#[inline]
pub unsafe fn dealloc(ptr: *mut u8, layout: Layout) {
    unsafe { soso_rt::dealloc(ptr, layout) }
}

#[inline]
pub unsafe fn realloc(ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    unsafe { soso_rt::realloc(ptr, layout, new_size) }
}
