//! Pool de hilos userspace para `RowParallel`.
//!
//! Entre trabajos los workers duermen en futex (si no, ncpu-1 cores al 100 %
//! y `Drop` en askd no volvía al accept). Dentro del matvec siguen en spin
//! sobre `done`, sin syscall.

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use libsoso::sys;
use libsoso::thread;
use soso_llm_core::parallel::{row_strip, RowParallel};

type RowFn = dyn Fn(usize, usize) + Sync;

struct Shared {
    generation: AtomicU32,
    rows: AtomicUsize,
    work0: AtomicUsize,
    work1: AtomicUsize,
    done: AtomicU32,
    n_workers: u32,
    shutdown: AtomicU32,
    /// Workers realmente vivos. Sin este contador nadie sabe cuándo han
    /// salido, y `Drop` no puede esperarlos.
    vivos: AtomicU32,
}

static mut SHARED: Shared = Shared {
    generation: AtomicU32::new(0),
    rows: AtomicUsize::new(0),
    work0: AtomicUsize::new(0),
    work1: AtomicUsize::new(0),
    done: AtomicU32::new(0),
    n_workers: 0,
    shutdown: AtomicU32::new(0),
    vivos: AtomicU32::new(0),
};

fn store_fn<'a>(shared: &Shared, f: &'a (dyn Fn(usize, usize) + Sync + 'a)) {
    let ptr: *const (dyn Fn(usize, usize) + Sync) = f as *const _;
    let parts: [usize; 2] = unsafe { core::mem::transmute_copy(&ptr) };
    shared.work0.store(parts[0], Ordering::Relaxed);
    shared.work1.store(parts[1], Ordering::Release);
}

fn load_fn(shared: &Shared) -> Option<*const RowFn> {
    let parts = [
        shared.work0.load(Ordering::Relaxed),
        shared.work1.load(Ordering::Acquire),
    ];
    if parts[0] == 0 && parts[1] == 0 {
        return None;
    }
    let ptr: *const RowFn = unsafe { core::mem::transmute_copy(&parts) };
    Some(ptr)
}

extern "C" fn worker_entry(arg: u64) -> ! {
    let idx = arg as usize;
    let shared = unsafe { &*(&raw const SHARED) };
    shared.vivos.fetch_add(1, Ordering::AcqRel);
    let mut last = 0u32;
    loop {
        while shared.generation.load(Ordering::Acquire) == last {
            if shared.shutdown.load(Ordering::Acquire) != 0 {
                salir(shared);
            }
            // Dormir entre trabajos. El spin de antes quemaba ncpu-1 cores
            // en vacío: Drop de askd no llegaba al accept y la placa
            // parecía colgada (2026-08-31). El camino caliente (esperar
            // `done` dentro del matvec) sigue en spin, sin syscall.
            sys::futex_wait(
                &shared.generation as *const AtomicU32 as *const u32,
                last,
            );
        }
        last = shared.generation.load(Ordering::Acquire);
        if shared.shutdown.load(Ordering::Acquire) != 0 {
            salir(shared);
        }
        let rows = shared.rows.load(Ordering::Acquire);
        let n = shared.n_workers as usize + 1;
        let (r0, r1) = row_strip(rows, idx, n);
        if let Some(fptr) = load_fn(shared)
            && r0 < r1
        {
            unsafe { (*fptr)(r0, r1) };
        }
        shared.done.fetch_add(1, Ordering::AcqRel);
    }
}

fn salir(shared: &Shared) -> ! {
    shared.vivos.fetch_sub(1, Ordering::AcqRel);
    sys::exit(0)
}

/// Pool con hasta `ncpu - 1` workers (el llamante también trabaja).
pub struct ThreadPool {
    n_total: usize,
}

impl ThreadPool {
    pub fn new() -> Self {
        let ncpu = sys::ncpu().max(1) as usize;
        let want = ncpu.saturating_sub(1);
        let shared = unsafe { &mut *(&raw mut SHARED) };
        // Si un pool anterior de este proceso dejó workers a medio salir,
        // esperarlos antes de tocar `shutdown`: ponerlo a 0 con hilos vivos
        // los resucita y acabaríamos con dos juegos sobre el mismo `SHARED`.
        for _ in 0..1_000_000 {
            if shared.vivos.load(Ordering::Acquire) == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        shared.shutdown.store(0, Ordering::Release);
        shared.generation.store(0, Ordering::Release);
        let mut spawned = 0usize;
        for i in 0..want {
            match thread::spawn(worker_entry, (i + 1) as u64) {
                Ok(_) => spawned += 1,
                Err(_) => break,
            }
        }
        shared.n_workers = spawned as u32;
        Self {
            n_total: spawned + 1,
        }
    }

    pub fn workers(&self) -> usize {
        self.n_total
    }
}

impl Drop for ThreadPool {
    /// Apaga los workers y **espera a que hayan salido**.
    ///
    /// AVERÍA (2026-08-16, placa real de 8 cores): no existía este `Drop` y
    /// nadie ponía `shutdown` a 1 jamás. Los workers eran procesos del
    /// scheduler que comparten el AddrSpace: **sobrevivían al proceso que
    /// los creó**. Entre trabajos ahora duermen en futex; el spin queda
    /// solo dentro del matvec (esperar `done`). En QEMU con
    /// `SOSO_QEMU_SMP=1`, `want == 0` y este camino no se ejecuta.
    fn drop(&mut self) {
        if self.n_total <= 1 {
            return;
        }
        let shared = unsafe { &mut *(&raw mut SHARED) };
        shared.shutdown.store(1, Ordering::Release);
        // Un cambio de generación por si alguno estuviera entre las dos
        // comprobaciones de `shutdown`. Están en futex: hay que despertarlos.
        let _ = shared.generation.fetch_add(1, Ordering::AcqRel);
        sys::futex_wake(
            &shared.generation as *const AtomicU32 as *const u32,
            u32::MAX as u64,
        );
        // El tope evita quedarse clavado si uno muriera de otra forma.
        for _ in 0..10_000 {
            if shared.vivos.load(Ordering::Acquire) == 0 {
                break;
            }
            let _ = sys::sleep_ms(1);
        }
        shared.n_workers = 0;
        self.n_total = 1;
    }
}

impl RowParallel for ThreadPool {
    fn for_rows(&self, rows: usize, f: &(dyn Fn(usize, usize) + Sync)) {
        if self.n_total <= 1 || rows == 0 {
            f(0, rows);
            return;
        }
        let shared = unsafe { &*(&raw const SHARED) };
        shared.rows.store(rows, Ordering::Release);
        store_fn(shared, f);
        shared.done.store(0, Ordering::Release);
        // Publicar el job y despertar a quien duerme en el futex.
        let _ = shared.generation.fetch_add(1, Ordering::AcqRel);
        sys::futex_wake(
            &shared.generation as *const AtomicU32 as *const u32,
            shared.n_workers as u64,
        );

        let (r0, r1) = row_strip(rows, 0, self.n_total);
        f(r0, r1);

        while shared.done.load(Ordering::Acquire) < shared.n_workers {
            core::hint::spin_loop();
        }
        shared.work0.store(0, Ordering::Relaxed);
        shared.work1.store(0, Ordering::Release);
    }
}
