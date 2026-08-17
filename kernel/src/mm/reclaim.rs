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

/// Registra una página file-backed RO recién mapeada. **O(1)**.
///
/// AVERÍA (2026-08-02): esto recorría la cola ENTERA en cada fallo de página
/// para deduplicar, y la cola no se purgaba ni al `munmap` ni al morir el
/// proceso. Con `soso-llm` haciendo streaming —mapea shards, los usa, los
/// desmapea y vuelve a mapearlos en el token siguiente— la cola se llenaba de
/// entradas muertas y cada fallo costaba más que el anterior: coste cuadrático
/// en el número de páginas. Con el modelo `tiny` (2,3 MiB) ya eran ~4600
/// entradas y ~10 M comparaciones por token; con un modelo de 512 MiB serían
/// 131 072 páginas y no se acaba nunca.
///
/// El escaneo estaba para no meter duplicados. Se puede quitar porque la fuente
/// de los duplicados ya no existe: `handle_mmap_fault` sale antes si la página
/// está mapeada, así que un fallo significa que NO lo está, y las entradas
/// obsoletas se dan de baja en `forget_range` (munmap) y `forget_space` (muerte
/// del proceso). Sin esas dos bajas el escaneo sólo tapaba la fuga.
pub fn register(space: &AddrSpace, va: u64, is_2m: bool) {
    let va = if is_2m {
        va & !(2 * 1024 * 1024 - 1)
    } else {
        va & !0xfff
    };
    let mut st = RECLAIM.lock();
    st.queue.push_back(CachedPage {
        space: space.clone(),
        va,
        is_2m,
        referenced: true,
    });
}

/// Da de baja las páginas de `[start, start+len)` de este espacio.
///
/// La llama `munmap`: sin esto, desmapear un shard dejaba sus páginas en la cola
/// apuntando a VAs que ya no existen —y peor, esa VA puede reutilizarse para otra
/// región (el asignador de mmap es first-fit), con lo que un desalojo posterior
/// tiraría la página de OTRA cosa.
pub fn forget_range(space: &AddrSpace, start: u64, len: u64) {
    let fin = start.saturating_add(len);
    let pml4 = space.pml4_phys();
    let mut st = RECLAIM.lock();
    st.queue
        .retain(|e| !(e.space.pml4_phys() == pml4 && e.va >= start && e.va < fin));
}

/// Da de baja todo lo de un espacio que se muere.
///
/// Cada entrada guarda un `AddrSpace`, que es un `Arc`: mientras la cola tenga
/// una, el espacio del proceso muerto NO se libera y sus tablas de páginas siguen
/// ocupando memoria. Un proceso por token de `soso-llm` y la cuenta sale sola.
pub fn forget_space(space: &AddrSpace) {
    let pml4 = space.pml4_phys();
    let mut st = RECLAIM.lock();
    st.queue.retain(|e| e.space.pml4_phys() != pml4);
}

/// Ventanas intocables mientras un dispositivo lee de ellas por DMA.
///
/// Hace falta porque las páginas mmap RO de pesos son justo las evictables: otro
/// core que faltee puede desmapearlas y devolver sus frames al asignador **en
/// medio** de la copia, y con un DMA leyendo directamente de ellas eso no es un
/// fallo de página, es leer lo que ya haya escrito otro. Es una lista, no un bit
/// por entrada: la cola llega a cientos de miles de páginas y recorrerla en cada
/// subida sería O(n); ventanas hay una por subida en vuelo.
static PINCHADAS: Mutex<alloc::vec::Vec<(u64, u64, u64)>> = Mutex::new(alloc::vec::Vec::new());

/// Fija `[va, va+len)` de este espacio hasta el `unpin`. Reentrante: dos subidas
/// solapadas añaden dos ventanas y cada `unpin` quita la suya.
#[cfg_attr(not(feature = "lxdde"), allow(dead_code))]
pub fn pin_range(space: &AddrSpace, va: u64, len: u64) {
    PINCHADAS
        .lock()
        .push((space.pml4_phys(), va, va.saturating_add(len)));
}

#[cfg_attr(not(feature = "lxdde"), allow(dead_code))]
pub fn unpin_range(space: &AddrSpace, va: u64, len: u64) {
    let clave = (space.pml4_phys(), va, va.saturating_add(len));
    let mut st = PINCHADAS.lock();
    if let Some(i) = st.iter().position(|e| *e == clave) {
        st.remove(i);
    }
}

/// `true` si esta página cae en alguna ventana fijada. Con la lista vacía —el
/// caso normal— es una comparación y fuera.
fn pinchada(e: &CachedPage) -> bool {
    let st = PINCHADAS.lock();
    if st.is_empty() {
        return false;
    }
    let fin = e.va + if e.is_2m { 2 * 1024 * 1024 } else { 4096 };
    let pml4 = e.space.pml4_phys();
    st.iter()
        .any(|&(p, ini, f)| p == pml4 && e.va < f && fin > ini)
}

/// Saca de la cola la primera candidata que no esté fijada, rotando las que sí.
fn primera_no_pinchada(st: &mut ReclaimState) -> Option<CachedPage> {
    for _ in 0..st.queue.len() {
        let e = st.queue.pop_front()?;
        if pinchada(&e) {
            st.queue.push_back(e);
        } else {
            return Some(e);
        }
    }
    None
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
                    if pinchada(&e) {
                        // Al fondo y con la segunda oportunidad intacta: mientras el
                        // DMA la esté leyendo no es candidata a nada.
                        e.referenced = true;
                        st.queue.push_back(e);
                    } else if e.referenced {
                        e.referenced = false;
                        st.queue.push_back(e);
                    } else {
                        victims.push(e);
                    }
                }
            }
            // Si todo el scan tenía referenced, forzar la más antigua **no fijada**:
            // forzar a ciegas se saltaría el pin, que es lo único que protege a un
            // DMA en vuelo.
            if victims.is_empty() {
                if let Some(e) = primera_no_pinchada(&mut st) {
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
            match primera_no_pinchada(&mut st) {
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
