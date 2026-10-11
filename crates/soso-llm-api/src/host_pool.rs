//! Pool de hilos del host para repartir filas de matvec, atención y FFN (`RowParallel`).
//!
//! Los hilos esperan un rato activos (los trabajos de un prefill llegan cada pocos cientos de
//! microsegundos) y después duermen en una `Condvar`: un servidor ocioso no quema CPU.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use soso_llm_core::parallel::RowParallel;

/// Vueltas de espera activa antes de dormir.
const GIROS_ANTES_DE_DORMIR: u32 = 20_000;

struct Inner {
    epoca: AtomicU32,
    hechos: AtomicUsize,
    filas: AtomicUsize,
    /// Dirección de un `&dyn Fn(usize, usize) + Sync` que vive en la pila de `for_rows`.
    trabajo: AtomicUsize,
    parar: AtomicBool,
    candado: Mutex<()>,
    despertar: Condvar,
}

pub struct HostPool {
    n: usize,
    inner: Arc<Inner>,
    hilos: Vec<JoinHandle<()>>,
}

fn franja(filas: usize, i: usize, n: usize) -> (usize, usize) {
    (filas * i / n, filas * (i + 1) / n)
}

impl HostPool {
    /// `n` hilos en total (el llamante trabaja como uno más). `n <= 1` = secuencial.
    pub fn new(n: usize) -> Self {
        let n = n.max(1);
        let inner = Arc::new(Inner {
            epoca: AtomicU32::new(0),
            hechos: AtomicUsize::new(0),
            filas: AtomicUsize::new(0),
            trabajo: AtomicUsize::new(0),
            parar: AtomicBool::new(false),
            candado: Mutex::new(()),
            despertar: Condvar::new(),
        });
        let mut hilos = Vec::new();
        for i in 1..n {
            let inner = inner.clone();
            hilos.push(std::thread::spawn(move || {
                let mut ultima = 0u32;
                loop {
                    let mut giros = 0u32;
                    while inner.epoca.load(Ordering::Acquire) == ultima {
                        if inner.parar.load(Ordering::Acquire) {
                            return;
                        }
                        giros += 1;
                        if giros < GIROS_ANTES_DE_DORMIR {
                            std::hint::spin_loop();
                        } else {
                            let g = inner.candado.lock().unwrap();
                            if inner.epoca.load(Ordering::Acquire) == ultima && !inner.parar.load(Ordering::Acquire) {
                                let _g = inner.despertar.wait(g).unwrap();
                            }
                            giros = 0;
                        }
                    }
                    ultima = inner.epoca.load(Ordering::Acquire);
                    let (r0, r1) = franja(inner.filas.load(Ordering::Acquire), i, n);
                    if r0 < r1 {
                        let f = unsafe { &*(inner.trabajo.load(Ordering::Acquire) as *const &(dyn Fn(usize, usize) + Sync)) };
                        f(r0, r1);
                    }
                    inner.hechos.fetch_add(1, Ordering::AcqRel);
                }
            }));
        }
        HostPool { n, inner, hilos }
    }

    pub fn hilos(&self) -> usize {
        self.n
    }
}

impl RowParallel for HostPool {
    fn for_rows(&self, rows: usize, f: &(dyn Fn(usize, usize) + Sync)) {
        if self.n <= 1 || rows == 0 {
            f(0, rows);
            return;
        }
        let fref: &(dyn Fn(usize, usize) + Sync) = f;
        self.inner.filas.store(rows, Ordering::Release);
        self.inner.trabajo.store(&fref as *const _ as usize, Ordering::Release);
        self.inner.hechos.store(0, Ordering::Release);
        self.inner.epoca.fetch_add(1, Ordering::AcqRel);
        {
            // Con el candado tomado un hilo que va a dormir ya ha mirado la época o la verá.
            let _g = self.inner.candado.lock().unwrap();
            self.inner.despertar.notify_all();
        }
        let (r0, r1) = franja(rows, 0, self.n);
        f(r0, r1);
        while self.inner.hechos.load(Ordering::Acquire) < self.n - 1 {
            std::hint::spin_loop();
        }
    }
}

impl Drop for HostPool {
    fn drop(&mut self) {
        self.inner.parar.store(true, Ordering::Release);
        {
            let _g = self.inner.candado.lock().unwrap();
            self.inner.despertar.notify_all();
        }
        for h in self.hilos.drain(..) {
            let _ = h.join();
        }
    }
}
