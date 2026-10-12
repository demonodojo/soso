//! Timers jiffies (HZ=100, alineado con PIT de soso).

use crate::arch::pit;
use alloc::vec::Vec;
use spin::Mutex;

pub const HZ: u64 = pit::HZ;

struct TimerEntry {
    deadline_ms: u64,
    fired: bool,
}

static TIMERS: Mutex<Vec<TimerEntry>> = Mutex::new(Vec::new());

pub fn init() {}

#[unsafe(no_mangle)]
pub extern "C" fn lx_jiffies() -> u64 {
    pit::ticks()
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_msecs_to_jiffies(ms: u32) -> u64 {
    (ms as u64 * HZ / 1000).max(1)
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_jiffies_to_msecs(j: u64) -> u32 {
    (j * 1000 / HZ) as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_ktime_get_ns() -> u64 {
    // TSC, no tick: con el PIT a 100 Hz esto devolvía múltiplos de 10 ms y las
    // medidas de los lanzamientos de GPU no medían el lanzamiento sino el tick
    // («~10000 us por QMD» en el log del 2026-08-02, que era exactamente uno).
    crate::arch::tsc::now_ns()
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_udelay(us: u64) {
    // Con el reloj fino, un microsegundo es un microsegundo. Antes esto hacía
    // `end = start + us/1000 + 1` sobre el contador de ticks, o sea que
    // `lx_udelay(1)` esperaba **hasta 10 ms**: el bring-up está lleno de bucles
    // de sondeo que llamaban aquí.
    if crate::arch::tsc::ready() {
        crate::arch::tsc::spin_us(us);
        return;
    }
    let start = pit::uptime_ms();
    let end = start + us / 1000 + 1;
    while pit::uptime_ms() < end {
        core::hint::spin_loop();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_mdelay(ms: u32) {
    sleep_ms(ms);
}

/// Espera `ms` de reloj de verdad, haya o no otra fibra a la que ceder.
///
/// La versión anterior registraba un timer y llamaba a `block_current()`, pero
/// `yield_now()` **retorna en el acto cuando solo hay una fibra**: durante el
/// bring-up de la GPU (fibra única) el retardo se evaporaba y los bucles de
/// sondeo daban miles de vueltas en microsegundos. Así se comió los 4 s de espera
/// del lockdown del GSP el 2026-07-25 y también el `gsp_mmio_poll_ready(2000)`.
pub fn sleep_ms(ms: u32) {
    use core::sync::atomic::{AtomicBool, Ordering};
    /// Si el PIT no avanza (x2APIC en placa) y aún no hay TSC, esperar por
    /// reloj colgaría. Tras calibrar el TSC, `lx_mdelay` usa eso: si no, el
    /// atajo CLOCK_DEAD convertía la espera ALIVE del AX211 (~5 s) en un
    /// sondeo vacío y el firmware nunca llegaba a UCODE_ALIVE_NTFY.
    static CLOCK_DEAD: AtomicBool = AtomicBool::new(false);
    const STUCK_SPINS: u64 = 5_000_000;

    if ms == 0 {
        return;
    }
    if crate::arch::tsc::ready() {
        crate::arch::tsc::delay_ms(ms as u64);
        return;
    }
    if CLOCK_DEAD.load(Ordering::Relaxed) {
        return;
    }
    let start = pit::uptime_ms();
    let deadline = start + ms as u64;
    let mut spins = 0u64;
    loop {
        let now = pit::uptime_ms();
        if now >= deadline {
            return;
        }
        if now == start && spins > STUCK_SPINS {
            CLOCK_DEAD.store(true, Ordering::Relaxed);
            return;
        }
        super::fiber::yield_now();
        core::hint::spin_loop();
        spins += 1;
    }
}

pub fn tick() {
    let now = pit::uptime_ms();
    let mut wake = false;
    {
        let mut t = TIMERS.lock();
        for e in t.iter_mut() {
            if !e.fired && now >= e.deadline_ms {
                e.fired = true;
                wake = true;
            }
        }
        t.retain(|e| !e.fired);
    }
    if wake {
        super::fiber::unblock_all();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_wait_for_completion_timeout(
    c: *mut super::completion::LxCompletion,
    timeout_jiffies: u64,
) -> u64 {
    let timeout_ms = lx_jiffies_to_msecs(timeout_jiffies).max(1) as u64;
    let start = pit::uptime_ms();
    loop {
        if super::completion::is_complete(c) {
            return 1;
        }
        if pit::uptime_ms() - start >= timeout_ms {
            return 0;
        }
        super::fiber::yield_now();
    }
}
