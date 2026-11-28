//! Asignación DMA con dirección física real (no identity map).

use alloc::alloc::{alloc_zeroed, Layout};
use alloc::vec::Vec;
use core::hint::spin_loop;
use core::sync::atomic::{AtomicBool, Ordering};

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

/// Un bloque DMA ya entregado: se puede devolver a la reserva.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Block {
    size: usize,
    align: usize,
    va: *mut u8,
    phys: u64,
}

/// Reciclado de bloques DMA para la conexión en caliente.
///
/// El asignador del kernel no libera (y mapea cada bloque sin caché en un VA
/// propio), así que cada dispositivo enchufado se llevaba ~15 páginas para
/// siempre. Lo que se pide mientras se inicializa un slot queda apuntado en
/// el registro abierto; al desenchufar, el controlador devuelve esos bloques
/// aquí y la siguiente inicialización los reutiliza.
///
/// El registro es global: sólo se abre durante una enumeración, y las
/// enumeraciones no se solapan (arranque secuencial, hotplug desde la BSP).
/// Lo que vive más que un slot (búfer de rebote, GET_PORT_STATUS de hub)
/// se pide con `alloc_persistent` y no se apunta.
struct Pool {
    free: Vec<Block>,
    track: Vec<Vec<Block>>,
}

static POOL_LOCK: AtomicBool = AtomicBool::new(false);
static mut POOL: Pool = Pool {
    free: Vec::new(),
    track: Vec::new(),
};

fn with_pool<T>(f: impl FnOnce(&mut Pool) -> T) -> T {
    while POOL_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        spin_loop();
    }
    // SAFETY: exclusión por POOL_LOCK.
    let r = f(unsafe { &mut *core::ptr::addr_of_mut!(POOL) });
    POOL_LOCK.store(false, Ordering::Release);
    r
}

/// Abre un registro: los bloques que se pidan hasta `track_end` son suyos.
pub(crate) fn track_begin() {
    with_pool(|p| p.track.push(Vec::new()));
}

/// Cierra el registro más interno y devuelve sus bloques.
pub(crate) fn track_end() -> Vec<Block> {
    with_pool(|p| p.track.pop().unwrap_or_default())
}

/// Devuelve bloques a la reserva. El HC ya no debe poder tocarlos (slot
/// deshabilitado).
pub(crate) fn release(blocks: Vec<Block>) {
    if blocks.is_empty() {
        return;
    }
    with_pool(|p| p.free.extend(blocks));
}

/// Bloques en la reserva (diagnóstico y pruebas).
pub fn pooled_blocks() -> usize {
    with_pool(|p| p.free.len())
}

fn raw_alloc(size: usize, align: usize) -> (*mut u8, u64) {
    if let Some(f) = unsafe { ALLOC } {
        return f(size, align);
    }
    let layout = Layout::from_size_align(size, align).expect("xhci dma layout");
    let ptr = unsafe { alloc_zeroed(layout) };
    assert!(!ptr.is_null(), "xhci dma oom");
    (ptr, ptr as u64)
}

/// Reserva memoria alineada; usa el hook del kernel si está registrado.
///
/// Reutiliza un bloque reciclado del mismo tamaño si lo hay, y apunta el
/// resultado en el registro abierto (si hay uno).
pub unsafe fn alloc_aligned(size: usize, align: usize) -> (*mut u8, u64) {
    let reused = with_pool(|p| {
        let i = p
            .free
            .iter()
            .position(|b| b.size == size && b.align >= align)?;
        Some(p.free.swap_remove(i))
    });
    let block = match reused {
        Some(b) => {
            // El hook entrega memoria a cero; la reciclada también debe.
            for i in 0..size {
                core::ptr::write_volatile(b.va.add(i), 0);
            }
            b
        }
        None => {
            let (va, phys) = raw_alloc(size, align);
            Block {
                size,
                align,
                va,
                phys,
            }
        }
    };
    with_pool(|p| {
        if let Some(t) = p.track.last_mut() {
            t.push(block);
        }
    });
    (block.va, block.phys)
}

/// Como `alloc_aligned`, pero fuera de todo registro: para búferes que
/// sobreviven al slot que los pidió.
pub(crate) unsafe fn alloc_persistent(size: usize, align: usize) -> (*mut u8, u64) {
    raw_alloc(size, align)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloques_de_un_slot_se_reutilizan_a_cero() {
        track_begin();
        let (va, phys) = unsafe { alloc_aligned(192, 64) };
        unsafe { core::ptr::write_bytes(va, 0xAB, 192) };
        // Persistente: no entra en el registro.
        let _ = unsafe { alloc_persistent(192, 64) };
        let blocks = track_end();
        assert_eq!(blocks.len(), 1);
        release(blocks);
        let (va2, phys2) = unsafe { alloc_aligned(192, 64) };
        assert_eq!((va2, phys2), (va, phys));
        let s = unsafe { core::slice::from_raw_parts(va2, 192) };
        assert!(s.iter().all(|&b| b == 0));
    }
}
