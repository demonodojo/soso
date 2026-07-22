//! Asignador de vectores IRQ (0x42..=0x61) y registro vector → handler.
//!
//! Los stubs se instalan en la IDT al arrancar; el dispatch hace EOI de
//! LAPIC tras invocar el handler del driver.

use crate::arch::apic;
use core::sync::atomic::{AtomicU8, Ordering};
use spin::Mutex;
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

/// Reserva un vector libre e instala `handler`. Devuelve el vector o None.
pub fn allocate(handler: IrqHandler) -> Option<u8> {
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
pub fn free(vector: u8) {
    if let Some(idx) = index(vector) {
        SLOTS.lock()[idx].handler = None;
    }
}

fn index(vector: u8) -> Option<usize> {
    if (VECTOR_BASE..=VECTOR_LAST).contains(&vector) {
        Some((vector - VECTOR_BASE) as usize)
    } else {
        None
    }
}

pub(crate) fn dispatch(vector: u8) {
    if let Some(idx) = index(vector) {
        let handler = SLOTS.lock()[idx].handler;
        if let Some(h) = handler {
            h();
        }
    }
    apic::eoi();
}

macro_rules! irq_stubs {
    ($(($vec:expr, $name:ident)),+ $(,)?) => {
        $(
            extern "x86-interrupt" fn $name(_frame: InterruptStackFrame) {
                dispatch($vec);
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
