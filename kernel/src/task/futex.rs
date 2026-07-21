//! Futex-lite: wait/wake sobre una dirección de usuario (word de 32 bits).
//!
//! Clave = (pml4 físico, dirección virtual): los hilos del mismo proceso
//! comparten PML4 y ven la misma dirección.
//!
//! El wait registra al waiter y comprueba `*addr == expected` bajo el
//! mismo lock que el wake, para no perder el wakeup entre el check y el
//! bloqueo.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use spin::Mutex;

use super::{Context, State, PROCS};

/// (pml4_phys, uaddr) → pids esperando.
static WAITERS: Mutex<BTreeMap<(u64, u64), Vec<u64>>> = Mutex::new(BTreeMap::new());

/// Si `*uaddr == expected`, bloquea al proceso actual en `WaitingFutex`.
/// No vuelve. Si el valor ya cambió, restaura `Runnable` con rax=0 y
/// replanifica (equivalente a "wait no-op").
pub fn wait_or_resume(pml4: u64, uaddr: u64, expected: u32, ctx: Context) -> ! {
    x86_64::instructions::interrupts::disable();
    let pid = super::current_pid();
    {
        let mut waiters = WAITERS.lock();
        let mut procs = PROCS.lock();
        let actual = unsafe { core::ptr::read_volatile(uaddr as *const u32) };
        if actual != expected {
            if let Some(p) = procs.iter_mut().find(|p| p.pid == pid) {
                p.ctx = ctx;
                p.ctx.rax = 0;
                p.state = State::Runnable;
            }
            drop(procs);
            drop(waiters);
            super::schedule();
        }
        waiters.entry((pml4, uaddr)).or_default().push(pid);
        if let Some(p) = procs.iter_mut().find(|p| p.pid == pid) {
            p.ctx = ctx;
            p.state = State::WaitingFutex { pml4, uaddr };
        }
        crate::arch::percpu::set_current_pid(0);
    }
    super::schedule();
}

/// Despierta hasta `n` procesos esperando en `(pml4, uaddr)`.
/// Devuelve cuántos despertó.
pub fn wake(pml4: u64, uaddr: u64, n: u64) -> u64 {
    let mut waiters = WAITERS.lock();
    let Some(list) = waiters.get_mut(&(pml4, uaddr)) else {
        return 0;
    };
    let mut woken = 0u64;
    let mut procs = PROCS.lock();
    while woken < n && !list.is_empty() {
        let pid = list.remove(0);
        if let Some(p) = procs.iter_mut().find(|p| p.pid == pid) {
            if matches!(p.state, State::WaitingFutex { .. }) {
                p.state = State::Runnable;
                p.ctx.rax = 0;
                woken += 1;
            }
        }
    }
    if list.is_empty() {
        waiters.remove(&(pml4, uaddr));
    }
    drop(procs);
    drop(waiters);
    if woken > 0 {
        crate::arch::apic::kick_idle_cpus();
    }
    woken
}

/// Quita al pid de todas las colas (al morir).
pub fn forget_pid(pid: u64) {
    let mut waiters = WAITERS.lock();
    waiters.retain(|_, list| {
        list.retain(|&p| p != pid);
        !list.is_empty()
    });
}
