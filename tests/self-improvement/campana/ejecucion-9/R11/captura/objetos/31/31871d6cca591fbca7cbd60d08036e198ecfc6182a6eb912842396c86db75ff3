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

/// Mide la frecuencia del TSC contra el PIT. Se llama una vez al arrancar.
///
/// Espera 5 ticks (~50 ms) porque calibrar contra uno solo deja el error del
/// propio tick dentro de la medida. Si el PIT no avanza (x2APIC en placa),
/// intenta IRQ0 por IOAPIC y, si sigue mudo, CPUID 0x16.
pub fn calibrate() {
    if !pit_avanza(40) {
        let bsp = crate::arch::apic::id();
        match crate::arch::ioapic::route_isa(
            0,
            crate::arch::interrupts::InterruptIndex::Timer as u8,
            bsp,
        ) {
            Ok(()) => {
                crate::println!("tsc: IRQ0 vía IOAPIC (8259 mudo)");
                // IRQ0 ya no por PIC: evita ticks dobles si LINT0 ExtINT revive.
                unsafe {
                    let mut pics = crate::arch::interrupts::PICS.lock();
                    pics.write_masks(!0b0001_0000, 0xFF);
                }
            }
            Err(e) => crate::println!("tsc: IOAPIC IRQ0 no enrutado ({e})"),
        }
    }

    if !pit_avanza(50) {
        aplicar_freq_cpuid();
        crate::println!(
            "tsc: {} MHz (CPUID 0x16; PIT no avanza)",
            CYCLES_PER_US.load(Ordering::Relaxed)
        );
        return;
    }

    let t0_tick = crate::arch::pit::ticks();
    let limite = rdtsc().wrapping_add(200_000 * ciclos_por_us());
    while crate::arch::pit::ticks() == t0_tick {
        if rdtsc().wrapping_sub(limite) < u64::MAX / 2 {
            aplicar_freq_cpuid();
            crate::println!(
                "tsc: {} MHz (CPUID; timeout alineando PIT)",
                CYCLES_PER_US.load(Ordering::Relaxed)
            );
            return;
        }
        core::hint::spin_loop();
    }
    let inicio_tick = crate::arch::pit::ticks();
    let inicio_tsc = rdtsc();
    let limite = rdtsc().wrapping_add(200_000 * ciclos_por_us());
    while crate::arch::pit::ticks() < inicio_tick + 5 {
        if rdtsc().wrapping_sub(limite) < u64::MAX / 2 {
            break;
        }
        core::hint::spin_loop();
    }
    let dticks = crate::arch::pit::ticks().saturating_sub(inicio_tick).max(1);
    let ciclos = rdtsc().wrapping_sub(inicio_tsc);
    let us = dticks * (1000 / crate::arch::pit::HZ) * 1000;
    let por_us = if us == 0 { 0 } else { ciclos / us };

    if por_us == 0 {
        aplicar_freq_cpuid();
        crate::println!(
            "tsc: {} MHz (CPUID; calibración PIT nula)",
            CYCLES_PER_US.load(Ordering::Relaxed)
        );
        return;
    }
    CYCLES_PER_US.store(por_us, Ordering::Relaxed);
    BASE.store(rdtsc(), Ordering::Relaxed);
    crate::println!(
        "tsc: {} MHz (calibrado contra el PIT en {} ms)",
        por_us,
        dticks * (1000 / crate::arch::pit::HZ)
    );
}

fn pit_avanza(ms: u64) -> bool {
    let t0 = crate::arch::pit::ticks();
    delay_ms(ms);
    crate::arch::pit::ticks() != t0
}

fn aplicar_freq_cpuid() {
    let mhz = ciclos_por_us();
    CYCLES_PER_US.store(mhz.max(1), Ordering::Relaxed);
    BASE.store(rdtsc(), Ordering::Relaxed);
}

/// ¿Hay reloj fino?
#[inline]
pub fn ready() -> bool {
    CYCLES_PER_US.load(Ordering::Relaxed) != 0
}

/// Nanosegundos desde la calibración. Sin PIT usa TSC (CPUID o calibrado).
pub fn now_ns() -> u64 {
    let por_us = ciclos_por_us();
    if CYCLES_PER_US.load(Ordering::Relaxed) == 0 {
        return crate::arch::pit::uptime_ms() * 1_000_000;
    }
    let d = rdtsc().wrapping_sub(BASE.load(Ordering::Relaxed));
    d.wrapping_mul(1000) / por_us
}

/// Milisegundos monótonos: TSC si está listo, si no el PIT.
pub fn uptime_ms() -> u64 {
    if ready() {
        now_ns() / 1_000_000
    } else {
        crate::arch::pit::uptime_ms()
    }
}

/// Espera activa de `us` microsegundos con resolución de verdad. Para esperas
/// cortas dentro del kernel; las largas siguen cediendo con el tick.
pub fn spin_us(us: u64) {
    delay_us(us);
}

/// Ciclos de TSC por µs: calibrados si los hay; si no, CPUID 0x16 o 3 GHz.
/// Sirve para delays del bring-up SMP, cuando el PIT aún no entrega IRQ0.
fn ciclos_por_us() -> u64 {
    let c = CYCLES_PER_US.load(Ordering::Relaxed);
    if c != 0 {
        return c;
    }
    let r = core::arch::x86_64::__cpuid(0x16);
    let mhz = r.eax & 0xffff;
    if mhz >= 100 {
        mhz as u64
    } else {
        3000
    }
}

/// Espera activa que **no** usa el PIT. Tras encender el LAPIC, IRQ0 del 8259
/// deja de llegar en silicio moderno; un `while uptime_ms() < fin` se cuelga.
pub fn delay_us(us: u64) {
    if us == 0 {
        return;
    }
    let start = rdtsc();
    let need = us.saturating_mul(ciclos_por_us());
    while rdtsc().wrapping_sub(start) < need {
        core::hint::spin_loop();
    }
}

pub fn delay_ms(ms: u64) {
    delay_us(ms.saturating_mul(1000));
}
