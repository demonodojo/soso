//! Reloj fino de CPU (TSC), calibrado contra el PIT.
//!
//! POR QUÉ EXISTE. El único reloj del kernel era `pit::uptime_ms()`, que avanza
//! con el tick — y el tick va a `HZ = 100`, o sea **de 10 en 10 ms**. Con eso:
//!
//!   - `lx_ktime_get_ns()` devolvía múltiplos de 10 ms, así que las medidas de
//!     los lanzamientos de GPU no medían el lanzamiento: medían el tick. El
//!     «~10000 us por QMD» del log del 2026-08-02 era exactamente eso.
//!   - `lx_udelay(1)` y `lx_mdelay(1)` esperaban **hasta un tick entero**. El
//!     bucle de espera del QMD hacía `mdelay(1)` por vuelta, así que un matvec
//!     que la GPU resuelve en microsegundos costaba 10 ms de reloj, y el modelo
//!     salía a 137 ms/capa: más lento que calcularlo en la CPU.
//!
//! El TSC es monótono y constante en cualquier x86-64 de esta época (invariant
//! TSC); aquí sólo hace falta para medir y para esperar poco rato, no para
//! mantener la hora.

use core::sync::atomic::{AtomicU64, Ordering};

/// Ciclos por microsegundo. 0 = sin calibrar (nadie ha llamado a `calibrate`).
static CYCLES_PER_US: AtomicU64 = AtomicU64::new(0);
/// TSC en el momento de calibrar: el origen de `now_ns`.
static BASE: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn rdtsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// Mide la frecuencia del TSC contra el PIT. Se llama una vez al arrancar, con
/// el PIT ya en marcha y las interrupciones activas.
///
/// Espera 5 ticks (~50 ms) porque calibrar contra uno solo deja el error del
/// propio tick dentro de la medida: con cinco, el sesgo baja al 20 % de un tick.
pub fn calibrate() {
    let t0_tick = crate::arch::pit::ticks();
    // Alinearse con el borde del siguiente tick para no contar un trozo suelto.
    while crate::arch::pit::ticks() == t0_tick {
        core::hint::spin_loop();
    }
    let inicio_tick = crate::arch::pit::ticks();
    let inicio_tsc = rdtsc();
    while crate::arch::pit::ticks() < inicio_tick + 5 {
        core::hint::spin_loop();
    }
    let ciclos = rdtsc() - inicio_tsc;
    let us = 5u64 * (1000 / crate::arch::pit::HZ) * 1000; // 5 ticks en µs
    let por_us = ciclos / us;

    if por_us == 0 {
        return; // TSC parado o PIT que no avanza: mejor no mentir
    }
    CYCLES_PER_US.store(por_us, Ordering::Relaxed);
    BASE.store(rdtsc(), Ordering::Relaxed);
    crate::println!(
        "tsc: {} MHz (calibrado contra el PIT en {} ms)",
        por_us,
        5 * (1000 / crate::arch::pit::HZ)
    );
}

/// ¿Hay reloj fino?
#[inline]
pub fn ready() -> bool {
    CYCLES_PER_US.load(Ordering::Relaxed) != 0
}

/// Nanosegundos desde la calibración. Sin calibrar cae al PIT, que es lo
/// honesto: devolver ceros dejaría los bucles de espera girando para siempre.
pub fn now_ns() -> u64 {
    let por_us = CYCLES_PER_US.load(Ordering::Relaxed);
    if por_us == 0 {
        return crate::arch::pit::uptime_ms() * 1_000_000;
    }
    let d = rdtsc().wrapping_sub(BASE.load(Ordering::Relaxed));
    // `d` cabe de sobra: a 5 GHz hacen falta ~117 años para desbordar el ×1000.
    d.wrapping_mul(1000) / por_us
}

/// Espera activa de `us` microsegundos con resolución de verdad. Para esperas
/// cortas dentro del kernel; las largas siguen cediendo con el tick.
pub fn spin_us(us: u64) {
    let por_us = CYCLES_PER_US.load(Ordering::Relaxed);
    if por_us == 0 {
        return;
    }
    let fin = rdtsc() + us * por_us;
    while rdtsc() < fin {
        core::hint::spin_loop();
    }
}
