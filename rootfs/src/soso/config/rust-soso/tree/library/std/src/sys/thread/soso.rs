//! Hilos de soso: `SYS_THREAD_SPAWN` y espera con `SYS_FUTEX`.
//!
//! Cada hilo instala su propia imagen TLS antes de `ThreadInit::init`.
//! Si no, `%fs` es el del padre y `set_current` aborta.

use crate::io;
use crate::ptr;
use crate::sync::atomic::{AtomicU32, Ordering};
use crate::thread::ThreadInit;

pub struct Thread {
    // C-112: puntero a un `AtomicU32` filtrado a propósito. El kernel escribe un
    // `1` ahí cuando el hilo sale; si el `JoinHandle` se suelta sin `join`, esa
    // dirección ya no puede ser memoria libre (el `1` caía sobre el primer
    // campo de lo que ocupara después el montón: `T41_LARGO` pasaba a
    // `\u{1}\0\0\0LARGO`). Sólo `join` la libera.
    join: *mut AtomicU32,
    stack: *mut u8,
    stack_len: usize,
}

unsafe impl Send for Thread {}
unsafe impl Sync for Thread {}

pub const DEFAULT_MIN_STACK_SIZE: usize = 1024 * 1024;

impl Thread {
    // unsafe: ver `thread::Builder::spawn_unchecked`.
    pub unsafe fn new(stack: usize, init: Box<ThreadInit>) -> io::Result<Thread> {
        let len = stack.max(DEFAULT_MIN_STACK_SIZE).next_multiple_of(4096);
        // SAFETY: mmap anónimo. `u64::MAX` es el fd de anónimo en soso.
        let base = unsafe { soso_rt::syscall4(soso_rt::SYS_MMAP, 0, len as u64, u64::MAX, 0) };
        if base < 0 {
            return Err(io::Error::from_raw_os_error((-base) as i32));
        }
        let stack_ptr = ptr::with_exposed_provenance_mut::<u8>(base as usize);
        let join: *mut AtomicU32 = Box::into_raw(Box::new(AtomicU32::new(0)));
        let data = Box::into_raw(init);
        let top = (base as u64).saturating_add(len as u64) & !0xF;
        let entry = thread_start as *const ();
        // SAFETY: `data` y `join` viven hasta que el hilo sale y `join` vuelve.
        let tid = unsafe {
            soso_rt::syscall4(
                soso_rt::SYS_THREAD_SPAWN,
                entry.expose_provenance() as u64,
                data.expose_provenance() as u64,
                top,
                join.expose_provenance() as u64,
            )
        };
        if tid < 0 {
            // SAFETY: el hilo no arrancó, así que nadie más usa `data` ni `join`.
            drop(unsafe { Box::from_raw(data) });
            drop(unsafe { Box::from_raw(join) });
            unsafe {
                let _ = soso_rt::syscall3(soso_rt::SYS_MUNMAP, base as u64, len as u64, 0);
            }
            return Err(io::Error::from_raw_os_error((-tid) as i32));
        }
        Ok(Thread { join, stack: stack_ptr, stack_len: len })
    }

    pub fn join(self) {
        // SAFETY: `join` vive hasta que se libera aquí.
        let cell = unsafe { &*self.join };
        while cell.load(Ordering::Acquire) == 0 {
            // SAFETY: `join` sigue vivo. El valor esperado es 0.
            unsafe {
                let _ = soso_rt::syscall4(
                    soso_rt::SYS_FUTEX,
                    soso_rt::FUTEX_WAIT,
                    self.join.expose_provenance() as u64,
                    0,
                    0,
                );
            }
        }
        // SAFETY: el hilo ya salió; la pila no se usa.
        unsafe {
            let _ = soso_rt::syscall3(
                soso_rt::SYS_MUNMAP,
                self.stack.expose_provenance() as u64,
                self.stack_len as u64,
                0,
            );
            drop(Box::from_raw(self.join));
        }
    }
}

extern "C" fn thread_start(data: u64) -> ! {
    crate::sys::pal::tls_hilo_nuevo();
    // SAFETY: el padre filtró este `Box` y nadie más lo lee.
    let init = unsafe { Box::from_raw(ptr::with_exposed_provenance_mut::<ThreadInit>(data as usize)) };
    let rust_start = init.init();
    rust_start();
    soso_rt::exit(0);
}

/// C-105: `SYS_NCPU`. Sin esto cargo se negaba a arrancar («the number of
/// hardware threads is not known for the target platform»).
pub fn available_parallelism() -> io::Result<crate::num::NonZero<usize>> {
    let n = unsafe { soso_rt::syscall0(soso_rt::SYS_NCPU) };
    crate::num::NonZero::new(if n > 0 { n as usize } else { 1 }).ok_or(io::Error::UNKNOWN_THREAD_COUNT)
}
