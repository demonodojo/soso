//! Reclaim de páginas mmap RO file-backed bajo presión de memoria.
//!
//! Las páginas se registran al resolver un page fault; si `free_frames` cae
//! bajo la marca de agua, se desmapean las más antiguas **sin** borrar la
//! `MmapRegion`, de modo que un acceso posterior vuelve a faultar y recarga
//! desde sosomfs.

use alloc::collections::VecDeque;
use crate::task::addrspace::AddrSpace;
use spin::Mutex;

/// Reserva mínima de frames libres (4 MiB).
const WATERMARK_FRAMES: usize = 1024;
/// Cuántas páginas intentar evictar por ronda de presión.
const EVICT_BATCH: usize = 16;

struct CachedPage {
    space: AddrSpace,
    va: u64,
    /// `true` si el mapeo es de 2 MiB.
    is_2m: bool,
}

struct ReclaimState {
    queue: VecDeque<CachedPage>,
}

static RECLAIM: Mutex<ReclaimState> = Mutex::new(ReclaimState {
    queue: VecDeque::new(),
});

/// Frames registrados como evictables (4 KiB cada entrada; 2 MiB cuenta 512).
pub fn reclaimable_frames() -> usize {
    RECLAIM
        .lock()
        .queue
        .iter()
        .map(|e| if e.is_2m { 512 } else { 1 })
        .sum()
}

/// Registra una página file-backed RO recién mapeada.
pub fn register(space: &AddrSpace, va: u64, is_2m: bool) {
    RECLAIM.lock().queue.push_back(CachedPage {
        space: space.clone(),
        va,
        is_2m,
    });
}

/// Libera frames hasta que haya al menos `watermark + need` libres.
pub fn ensure_free_frames(need: usize) -> bool {
    let target = WATERMARK_FRAMES.saturating_add(need);
    loop {
        let free = crate::mm::FRAME_ALLOC
            .get()
            .map(|a| a.lock().free_frames())
            .unwrap_or(0);
        if free >= target {
            return true;
        }
        let entry = RECLAIM.lock().queue.pop_front();
        let Some(entry) = entry else {
            return free >= need;
        };
        entry.space.evict_page(entry.va, entry.is_2m);
        crate::arch::apic::tlb_shootdown_all();
    }
}

/// Evicta un lote explícito (p. ej. antes de un bloque 2 MiB grande).
pub fn evict_batch(max_pages: usize) {
    for _ in 0..max_pages {
        let entry = RECLAIM.lock().queue.pop_front();
        let Some(entry) = entry else {
            break;
        };
        entry.space.evict_page(entry.va, entry.is_2m);
    }
    crate::arch::apic::tlb_shootdown_all();
}
