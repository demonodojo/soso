//! Hilos de usuario mínimos sobre `thread_spawn` + futex.

use crate::sys;
use core::sync::atomic::{AtomicU32, Ordering};

/// Tamaño por defecto de la pila de un hilo (mmap anónimo).
pub const DEFAULT_STACK: usize = 256 * 1024;
const GUARD: u64 = 4096;

/// Arranca `f(arg)` en un hilo nuevo. Devuelve el handle o errno negativo.
pub fn spawn(entry: extern "C" fn(u64) -> !, arg: u64) -> Result<JoinHandle, i64> {
    let stack_len = DEFAULT_STACK as u64;
    let base = sys::mmap(0, stack_len, u64::MAX, 0);
    if base < 0 {
        return Err(base);
    }
    let join = alloc::boxed::Box::new(AtomicU32::new(0));
    let join_ptr = alloc::boxed::Box::into_raw(join);
    let top = (base as u64 + stack_len) & !0xFu64;
    let tid = sys::thread_spawn(
        entry as *const () as u64,
        arg,
        top,
        join_ptr as u64,
    );
    if tid < 0 {
        unsafe {
            let _ = alloc::boxed::Box::from_raw(join_ptr);
        }
        let _ = sys::munmap(base as u64, stack_len);
        return Err(tid);
    }
    let _ = sys::mprotect(base as u64, GUARD, 0);
    Ok(JoinHandle {
        tid: tid as u64,
        join: join_ptr,
    })
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

/// Handle para join de hilos (futex en el kernel al salir del hilo).
pub struct JoinHandle {
    pub tid: u64,
    join: *const AtomicU32,
}

impl JoinHandle {
    pub fn join(self) -> Result<(), i64> {
        unsafe {
            while (*self.join).load(Ordering::Acquire) == 0 {
                let _ = sys::futex_wait(self.join as *const u32, 0);
            }
            let _ = alloc::boxed::Box::from_raw(self.join as *mut AtomicU32);
        }
        Ok(())
    }
}
