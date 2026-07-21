//! Pool de hilos userspace para `RowParallel`.
//!
//! Los workers hacen spin-wait sobre un contador de generación (sin futex
//! en el camino caliente del matvec): más simple y evita lost-wakeups
//! al arrancar. El futex sigue usándose en libsoso::thread::Barrier y
//! en la suite de init.

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
}

static mut SHARED: Shared = Shared {
    generation: AtomicU32::new(0),
    rows: AtomicUsize::new(0),
    work0: AtomicUsize::new(0),
    work1: AtomicUsize::new(0),
    done: AtomicU32::new(0),
    n_workers: 0,
    shutdown: AtomicU32::new(0),
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
    let mut last = 0u32;
    loop {
        while shared.generation.load(Ordering::Acquire) == last {
            if shared.shutdown.load(Ordering::Acquire) != 0 {
                sys::exit(0);
            }
            core::hint::spin_loop();
        }
        last = shared.generation.load(Ordering::Acquire);
        if shared.shutdown.load(Ordering::Acquire) != 0 {
            sys::exit(0);
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

/// Pool con hasta `ncpu - 1` workers (el llamante también trabaja).
pub struct ThreadPool {
    n_total: usize,
}

impl ThreadPool {
    pub fn new() -> Self {
        let ncpu = sys::ncpu().max(1) as usize;
        let want = ncpu.saturating_sub(1);
        let shared = unsafe { &mut *(&raw mut SHARED) };
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
        // Publicar el job: los workers en spin ven el nuevo generation.
        let _ = shared.generation.fetch_add(1, Ordering::AcqRel);

        let (r0, r1) = row_strip(rows, 0, self.n_total);
        f(r0, r1);

        while shared.done.load(Ordering::Acquire) < shared.n_workers {
            core::hint::spin_loop();
        }
        shared.work0.store(0, Ordering::Relaxed);
        shared.work1.store(0, Ordering::Release);
    }
}
