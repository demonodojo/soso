//! Diagnóstico de la entrada en modo gráfico.
//!
//! En una placa (portátil con teclado i8042 y HID USB) el teclado dejaba de
//! responder en cuanto un programa tomaba la pantalla, incluido
//! Ctrl+Alt+Retroceso, que el kernel atiende antes de entregar la tecla a
//! nadie. Estos contadores dicen en qué tramo se pierde: si llegan IRQ, si se
//! leen bytes del i8042, si entran informes HID, si los eventos pasan por la
//! cola y si el dueño de la pantalla la vacía. Con la pantalla en modo gráfico
//! se vuelca una línea al registro del kernel cada [`PERIODO_MS`].
//!
//! Los contadores son atómicos sueltos: se tocan desde IRQ y no toman ningún
//! candado. El volcado sólo se hace desde contexto de proceso.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::arch::smp::MAX_CPUS;

const PERIODO_MS: u64 = 5000;

pub static IRQ_KBD: AtomicU32 = AtomicU32::new(0);
pub static I8042_IRQ: AtomicU32 = AtomicU32::new(0);
pub static I8042_SONDEO: AtomicU32 = AtomicU32::new(0);
pub static I8042_AUX: AtomicU32 = AtomicU32::new(0);
pub static USB_IRQ: AtomicU32 = AtomicU32::new(0);
pub static USB_TECLA: AtomicU32 = AtomicU32::new(0);
pub static USB_PUNTERO: AtomicU32 = AtomicU32::new(0);
pub static CAD: AtomicU32 = AtomicU32::new(0);
pub static EVT_DENTRO: AtomicU32 = AtomicU32::new(0);
pub static EVT_FUERA: AtomicU32 = AtomicU32::new(0);
pub static INPUT_POLL: AtomicU32 = AtomicU32::new(0);
pub static KM_OCUPADO: AtomicU32 = AtomicU32::new(0);

static PASADAS: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];
static PASADAS_PREV: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];
static ULTIMO_VOLCADO: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn suma(c: &AtomicU32) {
    c.fetch_add(1, Ordering::Relaxed);
}

/// Una vuelta del planificador en la CPU `cpu`.
#[inline]
pub fn pasada(cpu: usize) {
    if let Some(c) = PASADAS.get(cpu) {
        c.fetch_add(1, Ordering::Relaxed);
    }
}

/// Vuelca una línea si la pantalla está en modo gráfico y ha pasado el
/// periodo. Lo llaman el planificador de la BSP y `input_poll`: si uno de los
/// dos caminos se atasca, el otro sigue dejando rastro (y el campo `cpu` dice
/// quién escribió).
pub fn quizas_volcar() {
    if !super::fb::graphics_mode() {
        return;
    }
    let ahora = crate::arch::pit::uptime_ms();
    let prev = ULTIMO_VOLCADO.load(Ordering::Relaxed);
    if ahora.saturating_sub(prev) < PERIODO_MS {
        return;
    }
    if ULTIMO_VOLCADO
        .compare_exchange(prev, ahora, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let (sc, ultimo, _, _) = super::kbd::latido();
    let dueno = super::fb::framebuffer_owner();
    let vida = ahora.saturating_sub(super::fb::keepalive_at());
    let plazo = super::fb::keepalive_ms();
    let r = |c: &AtomicU32| c.load(Ordering::Relaxed);
    crate::klogln!(
        "diag-entrada: cpu={} t={ahora} dueño={dueno} vida={vida}/{plazo}ms \
         irq1={} i8042 irq={} sondeo={} aux={} sc={sc} ult={ultimo:#04x} \
         usb irq={} tecla={} puntero={} cad={} km_ocupado={} \
         cola dentro={} fuera={} input_poll={}",
        crate::arch::percpu::cpu_index(),
        r(&IRQ_KBD),
        r(&I8042_IRQ),
        r(&I8042_SONDEO),
        r(&I8042_AUX),
        r(&USB_IRQ),
        r(&USB_TECLA),
        r(&USB_PUNTERO),
        r(&CAD),
        r(&KM_OCUPADO),
        r(&EVT_DENTRO),
        r(&EVT_FUERA),
        r(&INPUT_POLL),
    );
    // Qué ejecuta cada CPU y cuántas vueltas de planificador dio desde el
    // volcado anterior: una CPU con 0 vueltas y el mismo pid está atascada.
    let mut linea = alloc::string::String::new();
    for cpu in 0..MAX_CPUS {
        let n = PASADAS[cpu].load(Ordering::Relaxed);
        let antes = PASADAS_PREV[cpu].swap(n, Ordering::Relaxed);
        if n == 0 {
            continue;
        }
        let pid = crate::arch::percpu::pid_of_cpu(cpu);
        let _ = core::fmt::Write::write_fmt(
            &mut linea,
            format_args!(" {cpu}:pid{pid}/+{}", n.wrapping_sub(antes)),
        );
    }
    crate::klogln!("diag-entrada: cpus{linea}");
}
