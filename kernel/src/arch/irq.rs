//! Asignador de vectores IRQ (0x42..=0x61) y registro vector → handler.
//!
//! Los stubs se instalan en la IDT al arrancar; el dispatch hace EOI de
//! LAPIC tras invocar el handler del driver.

use crate::arch::apic;
use crate::arch::smp::MAX_CPUS;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use spin::Mutex;
use x86_64::instructions::interrupts::without_interrupts;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

/// Primer vector asignable (tras timer LAPIC 0x40 y resched 0x41).
pub const VECTOR_BASE: u8 = 0x42;
/// Número de stubs preinstalados.
pub const VECTOR_COUNT: u8 = 32;
/// Último vector inclusive.
pub const VECTOR_LAST: u8 = VECTOR_BASE + VECTOR_COUNT - 1;

pub type IrqHandler = fn();

struct Slot {
    handler: Option<IrqHandler>,
}

static SLOTS: Mutex<[Slot; VECTOR_COUNT as usize]> = Mutex::new(
    [const { Slot { handler: None } }; VECTOR_COUNT as usize],
);

static NEXT: AtomicU8 = AtomicU8::new(0);

/// Profundidad de IRQ dura por CPU (el `hardirq_count()` de Linux).
///
/// AVERÍA (2026-08-01): el handler MSI-X de virtio-net llamaba a `net::poll()`,
/// o sea que ejecutaba smoltcp + sunset + `ssh::drive` con IF=0 (estos stubs son
/// puertas de INTERRUPCIÓN). Ese camino toma PROCS, las colas RX/TX de ssh, el
/// VFS (lee /bin/sosh y /etc/motd) y el heap del kernel — los mismos
/// `spin::Mutex` que una syscall sostiene en ring 0 con las interrupciones
/// ABIERTAS (el `sti` de `syscall_entry`). En monocore la IRQ se quedaba girando
/// sobre un candado cuyo dueño era el contexto que ella misma había
/// interrumpido: máquina muerta, con la sesión SSH cortada justo después del
/// prompt y el puerto serie mudo desde ese instante.
///
/// Este contador existe para poder AFIRMAR la regla «la pila de red no corre
/// desde IRQ dura» en vez de suponerla — que es exactamente como se perdió: el
/// comentario de `net/ssh.rs` la daba por cierta razonando sólo sobre el tick
/// del timer, y el handler de la NIC la incumplía.
static PROF_IRQ: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];

/// Privilegio del contexto interrumpido (lo anota `con_contexto`).
/// Los handlers son `fn()`: no reciben el frame; el teclado usa `try_lock`.
static DESDE_RING3: [AtomicBool; MAX_CPUS] = [const { AtomicBool::new(false) }; MAX_CPUS];

/// Ejecuta el handler dejando anotado el privilegio del contexto interrumpido.
pub(crate) fn con_contexto<R>(desde_ring3: bool, f: impl FnOnce() -> R) -> R {
    let flag = &DESDE_RING3[crate::arch::percpu::cpu_index()];
    let previo = flag.swap(desde_ring3, Ordering::Relaxed);
    let r = f();
    flag.store(previo, Ordering::Relaxed);
    r
}

/// ¿Está esta CPU dentro de un handler de IRQ dura?
pub fn en_irq_dura() -> bool {
    PROF_IRQ[crate::arch::percpu::cpu_index()].load(Ordering::Relaxed) > 0
}

/// Reserva un vector libre e instala `handler`. Devuelve el vector o None.
pub fn allocate(handler: IrqHandler) -> Option<u8> {
    // `dispatch` toma SLOTS desde IRQ dura, así que en contexto de proceso hay
    // que enmascarar: es la disciplina `spin_lock_irqsave` de Linux. Aquí sólo
    // se llama al registrar drivers, pero un candado compartido con la IRQ no
    // admite excepciones «porque casi nunca coincide».
    without_interrupts(|| allocate_locked(handler))
}

fn allocate_locked(handler: IrqHandler) -> Option<u8> {
    let mut slots = SLOTS.lock();
    let start = NEXT.load(Ordering::Relaxed) as usize;
    for i in 0..VECTOR_COUNT as usize {
        let idx = (start + i) % VECTOR_COUNT as usize;
        if slots[idx].handler.is_none() {
            slots[idx].handler = Some(handler);
            NEXT.store(((idx + 1) % VECTOR_COUNT as usize) as u8, Ordering::Relaxed);
            return Some(VECTOR_BASE + idx as u8);
        }
    }
    None
}

/// Libera un vector previamente asignado.
#[allow(dead_code)]
pub fn free(vector: u8) {
    if let Some(idx) = index(vector) {
        without_interrupts(|| SLOTS.lock()[idx].handler = None);
    }
}

fn index(vector: u8) -> Option<usize> {
    if (VECTOR_BASE..=VECTOR_LAST).contains(&vector) {
        Some((vector - VECTOR_BASE) as usize)
    } else {
        None
    }
}

/// Invoca el handler del vector y hace EOI.
///
/// `desde_ring3` dice si el contexto interrumpido estaba en usuario: **sólo en
/// ese caso** este core no sostiene ningún candado del kernel, y por tanto sólo
/// entonces se puede procesar aquí el trabajo de red que la IRQ haya agendado.
/// Es el `irq_exit`/softirq de Linux, y el mismo invariante que `task::timer_tick`
/// ya usa (`if f.cs & 3 != 3 { return 0; }`). Si interrumpimos al kernel, el
/// trabajo se queda agendado y lo recoge el bucle del scheduler o el próximo tick
/// que venga de usuario.
pub(crate) fn dispatch(vector: u8, desde_ring3: bool) {
    // La CPU no limpia DF al entrar por una puerta de interrupción, y el
    // prólogo `x86-interrupt` de LLVM tampoco (`push`/`movaps`, sin `cld`:
    // comprobado en el ELF). `timer_isr` sí lo hace, en asm, porque es
    // `naked`. Este camino no: la MSI de la NIC llama a `net::poll` aquí
    // mismo, y si el userspace interrumpido iba a medias de un `memmove`
    // (DF=1, el TLS de rustls), el `rep movs` copia hacia atrás y pisa el
    // hueco libre de `talc` que hay delante del búfer. El centinela de
    // `lx_kmalloc` no se entera: está detrás del bloque. `iret` restaura
    // RFLAGS, así que este `cld` no rompe el `memmove` interrumpido.
    unsafe { core::arch::asm!("cld") };
    let prof = &PROF_IRQ[crate::arch::percpu::cpu_index()];
    prof.fetch_add(1, Ordering::Relaxed);
    if let Some(idx) = index(vector) {
        let handler = SLOTS.lock()[idx].handler;
        if let Some(h) = handler {
            con_contexto(desde_ring3, h);
        }
    }
    apic::eoi();
    prof.fetch_sub(1, Ordering::Relaxed);
    // Ya fuera del contexto de IRQ dura: aquí sí se puede tocar la pila de red.
    // No llamar a `net::poll` a pelo: sunset (chacha/curve25519) clobbea XMM
    // y MXCSR, y este stub `x86-interrupt` no es `timer_isr` — aquel hace
    // xsave a `TIMER_FPU` en asm. Sin preservar, el proceso ring 3
    // interrumpido hereda el estado SIMD de la cripto. Misma disciplina que
    // `mmap_fault_shim`: área local (no `TIMER_FPU`: un AP y la BSP no
    // pueden compartirla) y trampolín de alineación (cripto SSE).
    if desde_ring3 && crate::net::trabajo_pendiente() {
        let _ = crate::arch::interrupts::con_rsp_alineado(net_poll_shim, 0, 0);
    }
}

extern "sysv64" fn net_poll_shim(_a: u64, _b: u64) -> u64 {
    let mut fpu = crate::arch::fpu::FpuArea::empty();
    unsafe { crate::arch::fpu::save(&mut fpu) };
    crate::net::poll();
    unsafe { crate::arch::fpu::restore(&fpu) };
    0
}

/// Primer contexto visto con DF puesto al entrar por una IRQ.
///
/// La CPU no limpia DF al entrar por una puerta de interrupción, así que el
/// kernel hereda la bandera del contexto interrumpido y todo `rep movs` copia
/// hacia atrás. El `cld` de arriba lo corrige; esto sirve para saber **quién**
/// lo traía, que es lo que separa «el síntoma se fue» de «sé por qué».
///
/// **No imprime aquí**: un `println!` dentro de un handler de IRQ toma el
/// candado de la consola y clava la máquina. Sólo anota; lo saca
/// `schedule_inner`, que corre en contexto normal.
pub static DF_VISTO_RIP: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
pub static DF_VISTO_CS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static DF_ANOTADO: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

fn anotar_df(frame: &InterruptStackFrame) {
    use x86_64::registers::rflags::RFlags;
    if !frame.cpu_flags.contains(RFlags::DIRECTION_FLAG) {
        return;
    }
    if DF_ANOTADO.swap(true, Ordering::Relaxed) {
        return;
    }
    DF_VISTO_RIP.store(frame.instruction_pointer.as_u64(), Ordering::Relaxed);
    DF_VISTO_CS.store(frame.code_segment.0 as u64, Ordering::Relaxed);
}

/// Saca el aviso pendiente, si lo hay. La llama `schedule_inner`.
pub fn df_pendiente() -> Option<(u64, u64)> {
    let rip = DF_VISTO_RIP.swap(0, Ordering::Relaxed);
    if rip == 0 {
        return None;
    }
    Some((rip, DF_VISTO_CS.load(Ordering::Relaxed)))
}

macro_rules! irq_stubs {
    ($(($vec:expr, $name:ident)),+ $(,)?) => {
        $(
            extern "x86-interrupt" fn $name(frame: InterruptStackFrame) {
                // El privilegio del contexto interrumpido decide si el trabajo
                // diferido puede correr al salir (ver `dispatch`).
                let desde_ring3 =
                    frame.code_segment.rpl() == x86_64::PrivilegeLevel::Ring3;
                anotar_df(&frame);
                dispatch($vec, desde_ring3);
            }
        )+

        /// Instala stubs para todos los vectores asignables.
        pub fn install_stubs(idt: &mut InterruptDescriptorTable) {
            $(
                idt[$vec].set_handler_fn($name);
            )+
        }
    };
}

irq_stubs! {
    (0x42, irq_42), (0x43, irq_43), (0x44, irq_44), (0x45, irq_45),
    (0x46, irq_46), (0x47, irq_47), (0x48, irq_48), (0x49, irq_49),
    (0x4a, irq_4a), (0x4b, irq_4b), (0x4c, irq_4c), (0x4d, irq_4d),
    (0x4e, irq_4e), (0x4f, irq_4f), (0x50, irq_50), (0x51, irq_51),
    (0x52, irq_52), (0x53, irq_53), (0x54, irq_54), (0x55, irq_55),
    (0x56, irq_56), (0x57, irq_57), (0x58, irq_58), (0x59, irq_59),
    (0x5a, irq_5a), (0x5b, irq_5b), (0x5c, irq_5c), (0x5d, irq_5d),
    (0x5e, irq_5e), (0x5f, irq_5f), (0x60, irq_60), (0x61, irq_61),
}
