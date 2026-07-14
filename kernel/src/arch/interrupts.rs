//! IDT, PIC 8259 remapeado y handlers de excepciones e IRQs.

use crate::arch::{gdt, pit};
use crate::println;
use pic8259::ChainedPics;
use spin::{Lazy, Mutex};
use x86_64::structures::idt::{
    InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode,
};

/// Las excepciones de CPU ocupan los vectores 0-31; los IRQs van después.
pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,     // IRQ0
    Com1 = PIC_1_OFFSET + 4,  // IRQ4
}

static IDT: Lazy<InterruptDescriptorTable> = Lazy::new(|| {
    let mut idt = InterruptDescriptorTable::new();
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
    idt.general_protection_fault.set_handler_fn(gpf_handler);
    idt.page_fault.set_handler_fn(page_fault_handler);
    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        // ISR en asm: captura todos los registros para poder desalojar
        // al proceso de usuario interrumpido (fase 6).
        idt[InterruptIndex::Timer as u8]
            .set_handler_addr(x86_64::VirtAddr::new(
                crate::task::timer_isr as *const () as u64,
            ));
    }
    idt[InterruptIndex::Com1 as u8].set_handler_fn(com1_handler);
    idt
});

pub fn init() {
    IDT.load();
    unsafe {
        let mut pics = PICS.lock();
        pics.initialize();
        // Solo timer (IRQ0), cascada (IRQ2) y COM1 (IRQ4); el resto enmascarado.
        pics.write_masks(!0b0001_0101, 0xFF);
    }
    pit::init();
    x86_64::instructions::interrupts::enable();
}

// ---- excepciones ----

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    // Recuperable: se usa como prueba de vida de la IDT.
    println!("EXCEPCIÓN: breakpoint en {:?}", stack_frame.instruction_pointer);
}

/// Una excepción llegando de ring 3 mata al proceso, no al kernel.
fn desde_usuario(stack_frame: &InterruptStackFrame) -> bool {
    stack_frame.code_segment.rpl() == x86_64::PrivilegeLevel::Ring3
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    if desde_usuario(&stack_frame) {
        crate::task::kill_current("invalid opcode");
    }
    panic!("EXCEPCIÓN: invalid opcode\n{stack_frame:#?}");
}

extern "x86-interrupt" fn gpf_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    if desde_usuario(&stack_frame) {
        crate::task::kill_current("general protection fault");
    }
    panic!("EXCEPCIÓN: general protection fault (error {error_code:#x})\n{stack_frame:#?}");
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let addr = x86_64::registers::control::Cr2::read_raw();
    if desde_usuario(&stack_frame) {
        crate::println!(
            "task: page fault de usuario en {addr:#x} (rip {:#x}, {error_code:?})",
            stack_frame.instruction_pointer.as_u64()
        );
        crate::task::kill_current("page fault");
    }
    panic!(
        "EXCEPCIÓN: page fault accediendo a {addr:#x} ({error_code:?})\n{stack_frame:#?}"
    );
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    panic!("EXCEPCIÓN: double fault\n{stack_frame:#?}");
}

// ---- IRQs ----
// El timer (IRQ0) vive en task::timer_isr: necesita capturar el contexto
// completo para la preempción.

extern "x86-interrupt" fn com1_handler(_stack_frame: InterruptStackFrame) {
    crate::drivers::serial::handle_irq();
    unsafe {
        PICS.lock().notify_end_of_interrupt(InterruptIndex::Com1 as u8);
    }
}
