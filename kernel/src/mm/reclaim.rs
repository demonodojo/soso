//! Reclaim de páginas mmap RO file-backed bajo presión de memoria.
//!
//! Las páginas se registran al resolver un page fault; si `free_frames` cae
//! bajo la marca de agua, se desmapean las más antiguas **sin** borrar la
//! `MmapRegion`, de modo que un acceso posterior vuelve a faultar y recarga
//! desde sosomfs.
//!
//! Algoritmo **clock** (segunda oportunidad): un re-acceso reciente (touch
//! vía `register` de la misma VA) pone el bit; al evictar se salta una vez.
//! El TLB shootdown se agrupa por lote para no saturar el IPI en cada página.

use alloc::collections::VecDeque;
use crate::task::addrspace::AddrSpace;
use spin::Mutex;

/// Reserva mínima de frames libres (4 MiB).
const WATERMARK_FRAMES: usize = 1024;
/// Páginas a considerar por ronda de clock antes de forzar.
const CLOCK_SCAN: usize = 32;

struct CachedPage {
    space: AddrSpace,
    va: u64,
    /// `true` si el mapeo es de 2 MiB.
    is_2m: bool,
    /// Segunda oportunidad (clock).
    referenced: bool,
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
/// Si ya estaba en la cola (mismo espacio+VA), marca `referenced`.
pub fn register(space: &AddrSpace, va: u64, is_2m: bool) {
    let va = if is_2m {
        va & !(2 * 1024 * 1024 - 1)
    } else {
        va & !0xfff
    };
    let mut st = RECLAIM.lock();
    let pml4 = space.pml4_phys();
    for e in st.queue.iter_mut() {
        if e.va == va && e.space.pml4_phys() == pml4 {
            e.referenced = true;
            e.is_2m = is_2m;
            return;
        }
    }
    st.queue.push_back(CachedPage {
        space: space.clone(),
        va,
        is_2m,
        referenced: true,
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
        let mut victims = alloc::vec::Vec::new();
        {
            let mut st = RECLAIM.lock();
            if st.queue.is_empty() {
                return free >= need;
            }
            let mut scanned = 0;
            while scanned < CLOCK_SCAN && !st.queue.is_empty() && victims.is_empty() {
                scanned += 1;
                if let Some(mut e) = st.queue.pop_front() {
                    if e.referenced {
                        e.referenced = false;
                        st.queue.push_back(e);
                    } else {
                        victims.push(e);
                    }
                }
            }
            // Si todo el scan tenía referenced, forzar la más antigua.
            if victims.is_empty() {
                if let Some(e) = st.queue.pop_front() {
                    victims.push(e);
                }
            }
        }
        if victims.is_empty() {
            return free >= need;
        }
        for e in victims {
            e.space.evict_page(e.va, e.is_2m);
        }
        crate::arch::apic::tlb_shootdown_all();
    }
}

/// Evicta un lote explícito (p. ej. antes de un bloque 2 MiB grande).
#[allow(dead_code)]
pub fn evict_batch(max_pages: usize) {
    let mut victims = alloc::vec::Vec::new();
    {
        let mut st = RECLAIM.lock();
        for _ in 0..max_pages {
            match st.queue.pop_front() {
                Some(e) => victims.push(e),
                None => break,
            }
        }
    }
    if victims.is_empty() {
        return;
    }
    for e in victims {
        e.space.evict_page(e.va, e.is_2m);
    }
    crate::arch::apic::tlb_shootdown_all();
}
