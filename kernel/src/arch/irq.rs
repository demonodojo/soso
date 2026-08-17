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

/// ¿El contexto que interrumpió esta IRQ estaba en ring 3?
///
/// `dispatch` ya lo sabe, pero los handlers son `fn()` y no lo recibían, así que
/// no podían aplicar la misma regla que `timer_tick`: **desde ring 0 el kernel
/// puede tener cogido cualquier candado** y un handler que vuelva a pedirlo
/// clava el core. Lo necesita el teclado, que en placa entra por aquí (IOAPIC,
/// vector 0x42) y no por `kbd_pic_handler`.
static DESDE_RING3: [AtomicBool; MAX_CPUS] = [const { AtomicBool::new(false) }; MAX_CPUS];

pub fn desde_ring3() -> bool {
    DESDE_RING3[crate::arch::percpu::cpu_index()].load(Ordering::Relaxed)
}

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
    if desde_ring3 && crate::net::trabajo_pendiente() {
        crate::net::poll();
    }
}

macro_rules! irq_stubs {
    ($(($vec:expr, $name:ident)),+ $(,)?) => {
        $(
            extern "x86-interrupt" fn $name(frame: InterruptStackFrame) {
                // El privilegio del contexto interrumpido decide si el trabajo
                // diferido puede correr al salir (ver `dispatch`).
                let desde_ring3 =
                    frame.code_segment.rpl() == x86_64::PrivilegeLevel::Ring3;
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
