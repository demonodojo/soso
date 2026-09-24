//! Recorre las listas libres de talc sin desreferenciar un `next` fuera del heap.
//!
//! Layout de `Talc<O>` 4.4.3 en 64 bits: `availability_low`, `availability_high`,
//! `bins` (deuda: campos privados del crate talc).

use core::ptr::NonNull;
use talc::{ClaimOnOom, Talc};

use super::heap::{HEAP_START, imprimir_ultimos_accesos};

const BIN_COUNT: usize = 128;
const GAP_LOW_SIZE_OFFSET: usize = 16;

#[repr(C)]
struct TalcBinsHead {
    availability_low: usize,
    availability_high: usize,
    bins: *mut Option<NonNull<LlistNode>>,
}

#[repr(C)]
struct LlistNode {
    next: Option<NonNull<LlistNode>>,
    next_of_prev: *mut Option<NonNull<LlistNode>>,
}

fn heap_fin() -> u64 {
    let bytes = super::heap::heap_mapeado_bytes();
    if bytes == 0 {
        return HEAP_START;
    }
    HEAP_START + bytes
}

fn next_en_rango(next: u64, fin: u64) -> bool {
    next == 0 || (next >= HEAP_START && next < fin)
}

unsafe fn bin_head(talc: &Talc<ClaimOnOom>, bin: usize) -> u64 {
    unsafe {
        let head = talc as *const _ as *const TalcBinsHead;
        let bins = (*head).bins;
        if bins.is_null() {
            return 0;
        }
        let slot = bins.add(bin).read();
        slot.map(|n| n.as_ptr() as u64).unwrap_or(0)
    }
}

/// Comprueba cada `next` de hueco libre. Si alguno apunta fuera del heap,
/// imprime contexto y panica (sin seguir ese puntero).
pub fn vigilar_huecos(contexto: &str) {
    let fin = heap_fin();
    if fin <= HEAP_START {
        return;
    }
    super::heap::with_talc_audit(|talc| {
        vigilar_huecos_locked(contexto, talc, fin);
    });
}

fn vigilar_huecos_locked(contexto: &str, talc: &Talc<ClaimOnOom>, fin: u64) {
    for bin in 0..BIN_COUNT {
        let mut cur = unsafe { bin_head(talc, bin) };
        let mut pasos = 0usize;
        while cur != 0 {
            if pasos > 4096 {
                panic!("heap: lista libre bin {bin} demasiado larga (>{pasos})");
            }
            if !next_en_rango(cur, fin) {
                panic!("heap: nodo libre fuera del heap en bin {bin}: {cur:#x}");
            }
            let next = unsafe { core::ptr::read_volatile(cur as *const u64) };
            if !next_en_rango(next, fin) {
                let size = unsafe { core::ptr::read_volatile((cur + GAP_LOW_SIZE_OFFSET as u64) as *const usize) };
                let acme = cur.saturating_add(size as u64);
                let tag_bajo = cur.saturating_sub(8);
                let bajo = if tag_bajo >= HEAP_START {
                    unsafe { core::ptr::read_volatile(tag_bajo as *const u64) }
                } else {
                    0
                };
                crate::println!(
                    "heap: HUECO ROTO ({contexto}) bin={bin} nodo={cur:#x} next={next:#x} size={size} acme={acme:#x} tag_bajo={bajo:#018x}"
                );
                imprimir_ultimos_accesos();
                panic!(
                    "heap: enlace next inválido en hueco libre ({contexto}) nodo={cur:#x} next={next:#x}"
                );
            }
            cur = next;
            pasos += 1;
        }
    }
}
