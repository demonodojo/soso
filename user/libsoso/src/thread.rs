//! Hilos de usuario mínimos sobre `thread_spawn` + futex.

use crate::sys;
use core::sync::atomic::{AtomicU32, Ordering};

/// Tamaño por defecto de la pila de un hilo (mmap anónimo).
pub const DEFAULT_STACK: usize = 256 * 1024;

/// Arranca `f(arg)` en un hilo nuevo. Devuelve el tid o errno negativo.
///
/// `f` debe ser `extern "C" fn(*mut u8) -> !` o terminar con `sys::exit`.
pub fn spawn(entry: extern "C" fn(u64) -> !, arg: u64) -> Result<u64, i64> {
    let stack_len = DEFAULT_STACK as u64;
    let base = sys::mmap(0, stack_len, u64::MAX, 0);
    if base < 0 {
        return Err(base);
    }
    // Tope alineado a 16; el kernel resta 8 para ABI SysV (rsp%16==8).
    let top = (base as u64 + stack_len) & !0xFu64;
    let tid = sys::thread_spawn(entry as *const () as u64, arg, top);
    if tid < 0 {
        let _ = sys::munmap(base as u64, stack_len);
        return Err(tid);
    }
    Ok(tid as u64)
}

/// Barrera de generación: el líder incrementa `gen` y hace wake; los
/// workers esperan a que `gen` cambie.
pub struct Barrier {
    pub generation: AtomicU32,
    pub arrived: AtomicU32,
    pub n: u32,
}

impl Barrier {
    pub const fn new(n: u32) -> Self {
        Self {
            generation: AtomicU32::new(0),
            arrived: AtomicU32::new(0),
            n,
        }
    }

    /// Espera a que los `n` participantes lleguen; el último despierta al resto.
    pub fn wait(&self) {
        let g = self.generation.load(Ordering::Acquire);
        let a = self.arrived.fetch_add(1, Ordering::AcqRel) + 1;
        if a >= self.n {
            self.arrived.store(0, Ordering::Release);
            self.generation.fetch_add(1, Ordering::Release);
            let _ = sys::futex_wake(
                &self.generation as *const AtomicU32 as *const u32,
                u64::MAX,
            );
        } else {
            while self.generation.load(Ordering::Acquire) == g {
                let _ = sys::futex_wait(
                    &self.generation as *const AtomicU32 as *const u32,
                    g,
                );
            }
        }
    }
}
