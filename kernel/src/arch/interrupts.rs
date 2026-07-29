//! IDT, PIC 8259 remapeado y handlers de excepciones e IRQs.

use crate::arch::{apic, gdt, pit};
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
        // Timer LAPIC de los APs (la BSP sigue con PIC+PIT/`InterruptIndex::Timer`):
        // ISR en asm desnudo, como el de arriba, pero con la pila/área xsave
        // de cada core vía GS en vez de un símbolo fijo (`task::ap_timer_isr`).
        idt[apic::TIMER_VECTOR].set_handler_addr(x86_64::VirtAddr::new(
            crate::task::ap_timer_isr as *const () as u64,
        ));
    }
    idt[apic::RESCHED_VECTOR].set_handler_fn(resched_handler);
    idt[apic::TLB_SHOOTDOWN_VECTOR].set_handler_fn(tlb_shootdown_handler);
    idt[InterruptIndex::Com1 as u8].set_handler_fn(com1_handler);
    crate::arch::irq::install_stubs(&mut idt);
    idt
});

/// IPI de replanificación: solo EOI. Despierta al core del `hlt` para que
/// el bucle del scheduler vuelva a mirar `PROCS`.
extern "x86-interrupt" fn resched_handler(_stack_frame: InterruptStackFrame) {
    apic::eoi();
}

/// IPI de shootdown TLB: recarga CR3 del proceso actual en este core.
extern "x86-interrupt" fn tlb_shootdown_handler(_stack_frame: InterruptStackFrame) {
    use x86_64::registers::control::Cr3;
    let (frame, flags) = Cr3::read();
    unsafe {
        Cr3::write(frame, flags);
    }
    apic::eoi();
}

/// Carga la IDT compartida en un AP (sin tocar el PIC, que es de la BSP).
pub fn load_idt_ap() {
    IDT.load();
}

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
        con_rsp_alineado(kill_shim, 2, 0);
    }
    panic!("EXCEPCIÓN: invalid opcode\n{stack_frame:#?}");
}

extern "x86-interrupt" fn gpf_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    if desde_usuario(&stack_frame) {
        con_rsp_alineado(kill_shim, 1, 0);
    }
    // rip/rsp como escalares primero: el Debug del frame puede fallar si el
    // contexto está corrupto y perderíamos el dato clave.
    panic!(
        "EXCEPCIÓN: general protection fault (error {error_code:#x}) rip={:#x} rsp={:#x}\n{stack_frame:#?}",
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64()
    );
}

/// La convención `x86-interrupt` con código de error deja `rsp % 16 == 8`
/// en los `call` del handler (LLVM); la ABI SysV exige `% 16 == 0`. Todo
/// código profundo llamado desde estos handlers (SSE: `movaps` sobre la
/// pila en memcpy/fmt/iteradores) debe entrar por este trampolín que
/// normaliza la alineación — mismo problema que `timer_isr` con la cripto
/// de net::poll. Sin esto: GPF esporádicos y panics truncados.
#[unsafe(naked)]
pub extern "sysv64" fn con_rsp_alineado(
    _f: extern "sysv64" fn(u64, u64) -> u64,
    _a: u64,
    _b: u64,
) -> u64 {
    core::arch::naked_asm!(
        "push rbp",
        "mov rbp, rsp",
        "and rsp, -16",
        "mov rax, rdi",
        "mov rdi, rsi",
        "mov rsi, rdx",
        "call rax",
        "mov rsp, rbp",
        "pop rbp",
        "ret",
    )
}

extern "sysv64" fn mmap_fault_shim(addr: u64, is_write: u64) -> u64 {
    // El fault interrumpe al usuario en una instrucción arbitraria: sus
    // XMM están vivos y el camino del kernel (memcpy/fs) los clobbea.
    let mut fpu = crate::arch::fpu::FpuArea::empty();
    unsafe { crate::arch::fpu::save(&mut fpu) };
    let r = crate::task::handle_mmap_fault(addr, is_write != 0) as u64;
    unsafe { crate::arch::fpu::restore(&fpu) };
    r
}

/// Mata el proceso actual con el mensaje indexado (no retorna).
extern "sysv64" fn kill_shim(motivo: u64, dato: u64) -> u64 {
    match motivo {
        0 => {
            crate::println!("task: page fault de usuario en {dato:#x}");
            crate::task::kill_current("page fault")
        }
        1 => crate::task::kill_current("general protection fault"),
        2 => crate::task::kill_current("invalid opcode"),
        _ => crate::task::kill_current("excepción"),
    }
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let addr = x86_64::registers::control::Cr2::read_raw();
    if desde_usuario(&stack_frame) {
        let is_write = error_code.contains(PageFaultErrorCode::CAUSED_BY_WRITE);
        if con_rsp_alineado(mmap_fault_shim, addr, is_write as u64) != 0 {
            return;
        }
        con_rsp_alineado(kill_shim, 0, addr);
    }
    // Escalares primero: el Debug del frame puede volver a fallar y perder
    // el diagnóstico. [rsp] delata quién hizo `call` a una dirección mala.
    let rip = stack_frame.instruction_pointer.as_u64();
    let rsp = stack_frame.stack_pointer.as_u64();
    let ret = if rsp % 8 == 0 && rsp != 0 {
        unsafe { core::ptr::read_volatile(rsp as *const u64) }
    } else {
        0
    };
    panic!(
        "EXCEPCIÓN: page fault accediendo a {addr:#x} ({error_code:?}) rip={rip:#x} rsp={rsp:#x} [rsp]={ret:#x} cs={:#x}\n{stack_frame:#?}",
        stack_frame.code_segment.0
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
