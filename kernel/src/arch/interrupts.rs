//! IDT, PIC 8259 remapeado y handlers de excepciones e IRQs.

use crate::arch::{apic, gdt, pit};
use crate::println;
use core::sync::atomic::{AtomicU64, Ordering};
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
    // IRQ1 teclado (PIC): debe existir antes de desenmascarar; si falta,
    // un flanco en placa real → #GP en vector 33 → panic desalineado → #DF.
    idt[PIC_1_OFFSET + 1].set_handler_fn(kbd_pic_handler);
    instalar_excepciones_restantes(&mut idt);
    install_pic_fallback_handlers(&mut idt);
    crate::arch::irq::install_stubs(&mut idt);
    idt
});

/// Handlers para el resto de excepciones de CPU (0..31).
///
/// AVERÍA (2026-08-17, placa real): la IDT solo tenía breakpoint, #UD, #GP, #PF
/// y #DF. Cualquier otra excepción encontraba su entrada **no presente**, y
/// entregarla se convertía en `EXCEPTION: double fault` sin más pista que el
/// `rip`. Fue exactamente lo que pasó al parsear `temp=0.7` de `/etc/llm.conf`:
/// el `vdivss` de `dec2flt` levantó una excepción SIMD (#XM, vector 19) que no
/// tenía dónde ir. Es la misma lección que ya está apuntada dos líneas más
/// arriba para la IRQ1 del teclado — con la IDT incompleta, todo error acaba
/// disfrazado del mismo double fault.
///
/// Desde ring 3 matan al proceso; desde ring 0 hacen panic diciendo **cuál**
/// fue la excepción.
fn instalar_excepciones_restantes(idt: &mut InterruptDescriptorTable) {
    idt.divide_error.set_handler_fn(exc_divide);
    idt.debug.set_handler_fn(exc_debug);
    idt.overflow.set_handler_fn(exc_overflow);
    idt.bound_range_exceeded.set_handler_fn(exc_bound);
    idt.device_not_available.set_handler_fn(exc_nm);
    idt.x87_floating_point.set_handler_fn(exc_x87);
    idt.simd_floating_point.set_handler_fn(exc_simd);
    idt.virtualization.set_handler_fn(exc_virt);
    idt.invalid_tss.set_handler_fn(exc_tss);
    idt.segment_not_present.set_handler_fn(exc_np);
    idt.stack_segment_fault.set_handler_fn(exc_ss);
    idt.alignment_check.set_handler_fn(exc_ac);
}

macro_rules! excepciones {
    ($(($nombre:ident, $vector:expr)),+ $(,)?) => {
        $(
            extern "x86-interrupt" fn $nombre(stack_frame: InterruptStackFrame) {
                excepcion_generica(&stack_frame, $vector, 0);
            }
        )+
    };
}

macro_rules! excepciones_con_error {
    ($(($nombre:ident, $vector:expr)),+ $(,)?) => {
        $(
            extern "x86-interrupt" fn $nombre(stack_frame: InterruptStackFrame, error_code: u64) {
                excepcion_generica(&stack_frame, $vector, error_code);
            }
        )+
    };
}

excepciones! {
    (exc_divide, 0), (exc_debug, 1), (exc_overflow, 4), (exc_bound, 5),
    (exc_nm, 7), (exc_x87, 16), (exc_simd, 19), (exc_virt, 20),
}

excepciones_con_error! {
    (exc_tss, 10), (exc_np, 11), (exc_ss, 12), (exc_ac, 17),
}

fn nombre_excepcion(vector: u8) -> &'static str {
    match vector {
        0 => "divide error (#DE)",
        1 => "debug (#DB)",
        4 => "overflow (#OF)",
        5 => "bound range (#BR)",
        7 => "device not available (#NM)",
        10 => "invalid TSS (#TS)",
        11 => "segment not present (#NP)",
        12 => "stack fault (#SS)",
        16 => "x87 floating point (#MF)",
        17 => "alignment check (#AC)",
        19 => "SIMD floating point (#XM)",
        20 => "virtualization (#VE)",
        _ => "desconocida",
    }
}

static EXC_VECTOR: AtomicU64 = AtomicU64::new(0);

fn excepcion_generica(stack_frame: &InterruptStackFrame, vector: u8, error_code: u64) {
    if desde_usuario(stack_frame) {
        EXC_VECTOR.store(vector as u64, Ordering::Relaxed);
        con_rsp_alineado(kill_vector_shim, vector as u64, 0);
    }
    stash_exc(
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64(),
        error_code,
    );
    EXC_VECTOR.store(vector as u64, Ordering::Relaxed);
    let _ = con_rsp_alineado(exception_vector_panic_shim, 0, 0);
    loop {}
}

extern "sysv64" fn kill_vector_shim(vector: u64, _: u64) -> u64 {
    crate::println!(
        "task: excepción de usuario: {}",
        nombre_excepcion(vector as u8)
    );
    crate::task::kill_current("excepción de CPU")
}

extern "sysv64" fn exception_vector_panic_shim(_: u64, _b: u64) -> u64 {
    let rip = EXC_RIP.load(Ordering::Relaxed);
    let rsp = EXC_RSP.load(Ordering::Relaxed);
    let extra = EXC_EXTRA.load(Ordering::Relaxed);
    let v = EXC_VECTOR.load(Ordering::Relaxed) as u8;
    panic!(
        "EXCEPTION: {} (vector {v}, error {extra:#x}) rip={rip:#x} rsp={rsp:#x}",
        nombre_excepcion(v)
    );
}

/// EOI genérico del 8259. Vectores PIC sin handler → #GP al entregar la IRQ
/// → double fault (rip suele quedar en el `ret` tras `sti` de without_interrupts).
fn pic_eoi(vector: u8) {
    unsafe {
        PICS.lock().notify_end_of_interrupt(vector);
    }
}

macro_rules! pic_fallback_handlers {
    ($($vec:literal => $name:ident),+ $(,)?) => {
        $(
            extern "x86-interrupt" fn $name(_stack_frame: InterruptStackFrame) {
                pic_eoi($vec);
            }
        )+
        fn install_pic_fallback_handlers(idt: &mut InterruptDescriptorTable) {
            $( idt[$vec].set_handler_fn($name); )+
        }
    };
}

pic_fallback_handlers! {
    34 => pic_vec_34, // IRQ2 cascada del esclavo
    35 => pic_vec_35,
    37 => pic_vec_37,
    38 => pic_vec_38,
    39 => pic_vec_39,
    40 => pic_vec_40,
    41 => pic_vec_41,
    42 => pic_vec_42,
    43 => pic_vec_43,
    44 => pic_vec_44,
    45 => pic_vec_45,
    46 => pic_vec_46,
    47 => pic_vec_47,
}

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
        // Timer (IRQ0) y COM1 (IRQ4). IRQ1 en kbd::init; cascada/esclavo con
        // handlers de respaldo pero enmascarados hasta haga falta el PIC legacy.
        pics.write_masks(!0b0001_0001, 0xFF);
    }
    pit::init();
    x86_64::instructions::interrupts::enable();
}

// ---- excepciones ----

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    // Recuperable: prueba de vida de la IDT. Solo ASCII (el FB es 7-bit) y
    // println vía trampolín: x86-interrupt deja rsp % 16 == 8 y el fmt/SSE
    // del print GPF-ea en placa real (panic truncado a «EXCEPCI»).
    let rip = stack_frame.instruction_pointer.as_u64();
    let _ = con_rsp_alineado(breakpoint_print_shim, rip, 0);
}

extern "sysv64" fn breakpoint_print_shim(rip: u64, _b: u64) -> u64 {
    println!("idt: breakpoint ok rip={rip:#x}");
    0
}

/// Una excepción llegando de ring 3 mata al proceso, no al kernel.
fn desde_usuario(stack_frame: &InterruptStackFrame) -> bool {
    stack_frame.code_segment.rpl() == x86_64::PrivilegeLevel::Ring3
}

static EXC_RIP: AtomicU64 = AtomicU64::new(0);
static EXC_RSP: AtomicU64 = AtomicU64::new(0);
static EXC_EXTRA: AtomicU64 = AtomicU64::new(0);

fn stash_exc(rip: u64, rsp: u64, extra: u64) {
    EXC_RIP.store(rip, Ordering::Relaxed);
    EXC_RSP.store(rsp, Ordering::Relaxed);
    EXC_EXTRA.store(extra, Ordering::Relaxed);
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    if desde_usuario(&stack_frame) {
        con_rsp_alineado(kill_shim, 2, 0);
    }
    stash_exc(
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64(),
        0,
    );
    let _ = con_rsp_alineado(exception_panic_shim, 0, 0);
    loop {}
}

extern "x86-interrupt" fn gpf_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    if desde_usuario(&stack_frame) {
        con_rsp_alineado(kill_shim, 1, 0);
    }
    stash_exc(
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64(),
        error_code,
    );
    let _ = con_rsp_alineado(exception_panic_shim, 1, 0);
    loop {}
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
    stash_exc(rip, rsp, ret);
    let _ = con_rsp_alineado(kernel_pf_panic_shim, addr, 0);
    loop {}
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    stash_exc(
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64(),
        0,
    );
    let _ = con_rsp_alineado(exception_panic_shim, 2, 0);
    loop {}
}

extern "sysv64" fn exception_panic_shim(kind: u64, _: u64) -> u64 {
    let rip = EXC_RIP.load(Ordering::Relaxed);
    let rsp = EXC_RSP.load(Ordering::Relaxed);
    let extra = EXC_EXTRA.load(Ordering::Relaxed);
    match kind {
        0 => panic!("EXCEPTION: invalid opcode rip={rip:#x} rsp={rsp:#x}"),
        1 => panic!(
            "EXCEPTION: general protection fault (error {extra:#x}) rip={rip:#x} rsp={rsp:#x}"
        ),
        // Un #DF casi siempre significa que la excepción ORIGINAL no tenía
        // entrada en la IDT (o que la pila de kernel no servía para
        // entregarla). El `rip` es el de la instrucción que la provocó.
        2 => panic!(
            "EXCEPTION: double fault rip={rip:#x} rsp={rsp:#x} \
             (¿excepción sin handler? mira qué instrucción hay en ese rip)"
        ),
        _ => panic!("EXCEPTION: rip={rip:#x} rsp={rsp:#x}"),
    }
}

extern "sysv64" fn kernel_pf_panic_shim(addr: u64, _: u64) -> u64 {
    let rip = EXC_RIP.load(Ordering::Relaxed);
    let rsp = EXC_RSP.load(Ordering::Relaxed);
    let ret = EXC_EXTRA.load(Ordering::Relaxed);
    panic!("EXCEPTION: page fault at {addr:#x} rip={rip:#x} rsp={rsp:#x} [rsp]={ret:#x}");
}

/// IRQ 1 (teclado) por el **PIC legacy**. En placa la IRQ 1 va por el IOAPIC
/// (`irq::dispatch`, vector 0x42) y este handler no llega a usarse; la regla de
/// «solo sondear desde ring 3» está en `kbd::handle_irq`, común a ambos.
extern "x86-interrupt" fn kbd_pic_handler(stack_frame: InterruptStackFrame) {
    // La guarda vive dentro de `kbd::handle_irq`, común a este camino y al del
    // IOAPIC; aquí solo se le pasa el privilegio del contexto interrumpido.
    crate::arch::irq::con_contexto(desde_usuario(&stack_frame), || {
        crate::drivers::kbd::handle_irq()
    });
    unsafe {
        PICS.lock().notify_end_of_interrupt(PIC_1_OFFSET + 1);
    }
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
