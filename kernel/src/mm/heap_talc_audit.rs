//! Recorre las listas libres de talc sin desreferenciar un `next` fuera del heap.
//!
//! `Talc` es `repr(Rust)`: el orden de sus campos no está garantizado (en
//! placa el tercer word era la máscara de disponibilidad, no `bins`). Se
//! localiza `bins` buscando entre sus palabras la que vale `TALC_BINS`, que
//! es donde `claim` deja la tabla (`HEAP_START + TAG_SIZE`).

use core::ptr::NonNull;
use talc::{ClaimOnOom, Talc};

use super::heap::{HEAP_START, TALC_BINS, imprimir_ultimos_accesos};

const BIN_COUNT: usize = 128;
const GAP_LOW_SIZE_OFFSET: usize = 16;
const BINS_TABLE_BYTES: u64 = (BIN_COUNT * core::mem::size_of::<Option<NonNull<LlistNode>>>()) as u64;

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

fn bins_en_heap(bins: *mut Option<NonNull<LlistNode>>, fin: u64) -> bool {
    if bins.is_null() {
        return false;
    }
    let b = bins as u64;
    b >= HEAP_START && b.saturating_add(BINS_TABLE_BYTES) <= fin && b % 8 == 0
}

/// Busca entre las palabras de `Talc` la que apunta a la tabla de bins.
/// Devuelve nulo si ninguna vale `TALC_BINS` (talc aún sin `claim`).
unsafe fn bins_ptr(talc: &Talc<ClaimOnOom>) -> *mut Option<NonNull<LlistNode>> {
    let palabras = core::mem::size_of::<Talc<ClaimOnOom>>() / core::mem::size_of::<u64>();
    let base = talc as *const Talc<ClaimOnOom> as *const u64;
    for i in 0..palabras {
        let v = unsafe { core::ptr::read_volatile(base.add(i)) };
        if v == TALC_BINS {
            return v as *mut Option<NonNull<LlistNode>>;
        }
    }
    core::ptr::null_mut()
}

unsafe fn bin_head(bins: *mut Option<NonNull<LlistNode>>, bin: usize) -> u64 {
    unsafe {
        let raw = core::ptr::read_volatile(bins.add(bin) as *const u64);
        if raw == 0 {
            0
        } else {
            raw
        }
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
    let bins = unsafe { bins_ptr(talc) };
    if !bins_en_heap(bins, fin) {
        static AVISADO: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        if !AVISADO.swap(true, core::sync::atomic::Ordering::Relaxed) {
            crate::println!(
                "heap: vigilar_huecos({contexto}) omitido (bins={bins:p} fuera de {HEAP_START:#x}..{fin:#x})"
            );
        }
        return;
    }
    for bin in 0..BIN_COUNT {
        let mut cur = unsafe { bin_head(bins, bin) };
        let mut pasos = 0usize;
        while cur != 0 {
            if pasos > 4096 {
                panic!("heap: lista libre bin {bin} demasiado larga (>{pasos})");
            }
            if !next_en_rango(cur, fin) {
                let pa = super::heap::bins_pa();
                let alias = super::heap::bins_alias();
                crate::println!(
                    "heap: cabecera bin {bin} fuera del heap ({contexto}) valor={cur:#x} pa={pa:#x} alias={alias:#x} (sin #DB: ni la VA ni el alias)"
                );
                super::heap::informar_dma_en_pf(pa);
                super::heap::informar_dma_en_pf(cur);
                imprimir_ultimos_accesos();
                panic!("heap: nodo libre fuera del heap en bin {bin}: {cur:#x}");
            }
            let next = unsafe { core::ptr::read_volatile(cur as *const u64) };
            if !next_en_rango(next, fin) {
                let size =
                    unsafe { core::ptr::read_volatile((cur + GAP_LOW_SIZE_OFFSET as u64) as *const usize) };
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
