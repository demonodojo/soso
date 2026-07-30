//! Arranque de APs (SMP): trampolín real→protegido→largo copiado a
//! TRAMP_PHYS (<1 MiB, reservado en el frame allocator) + INIT-SIPI-SIPI
//! por x2APIC. En L3a los APs quedan en un idle-loop; el scheduler
//! multicore llega en L3b.

use crate::arch::{apic, interrupts, sse};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Página física del trampolín (el vector SIPI es page>>12 = 0x08).
pub const TRAMP_PHYS: u64 = 0x8000;

pub const MAX_CPUS: usize = 16;
const AP_STACK_SIZE: usize = 32 * 1024;

#[repr(C, align(16))]
struct ApStack([u8; AP_STACK_SIZE]);
static mut AP_STACKS: [ApStack; MAX_CPUS - 1] =
    [const { ApStack([0; AP_STACK_SIZE]) }; MAX_CPUS - 1];

/// CPUs en línea (incluida la BSP).
pub static CPUS_ONLINE: AtomicU32 = AtomicU32::new(1);
static AP_APIC_ID: AtomicU64 = AtomicU64::new(0);
/// APIC id por índice de CPU (`u32::MAX` = desconocido).
static APIC_IDS: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(u32::MAX) }; MAX_CPUS];

pub fn set_apic_id(cpu: usize, id: u32) {
    if cpu < MAX_CPUS {
        APIC_IDS[cpu].store(id, Ordering::Relaxed);
    }
}

pub fn apic_id_of(cpu: usize) -> Option<u32> {
    if cpu >= MAX_CPUS {
        return None;
    }
    let v = APIC_IDS[cpu].load(Ordering::Relaxed);
    if v == u32::MAX {
        None
    } else {
        Some(v)
    }
}

// Direcciones fijas DENTRO de la página del trampolín (los operandos de
// memoria del ensamblador no admiten aritmética de símbolos, así que el
// código usa estas constantes y Rust escribe ahí GDT y datos parcheados).
const GDT_OFF: usize = 0xF80; // 4 descriptores
const GDTR_OFF: usize = 0xFE0; // limit u16 + base u32
const CPUIDX_OFF: usize = 0xFB8; // índice de CPU (1..MAX_CPUS-1) de este AP
const CR3_OFF: usize = 0xFC0;
const STACK_OFF: usize = 0xFC8;
const ENTRY_OFF: usize = 0xFD0;

core::arch::global_asm!(
    r#"
.section .text
.code16
.global ap_tramp_start, ap_tramp_end
ap_tramp_start:
    cli
    xor ax, ax
    mov ds, ax
    // marca 'A': el AP ha despertado en modo real
    mov dx, 0x3F8
    mov al, 0x41
    out dx, al
    lgdt [0x8FE0]
    mov eax, cr0
    or eax, 1
    mov cr0, eax
    // far jmp a 32 bits: EA off16 seg16
    .byte 0xEA
    .word 0x8000 + (ap_tramp32 - ap_tramp_start)
    .word 0x08

.code32
ap_tramp32:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    // marca 'B': modo protegido
    mov dx, 0x3F8
    mov al, 0x42
    out dx, al
    // PAE
    mov eax, cr4
    or eax, 0x20
    mov cr4, eax
    // CR3 del kernel (parcheado por la BSP)
    mov eax, [0x8FC0]
    mov cr3, eax
    // EFER: LME + NXE (las tablas del kernel usan NX: sin NXE el bit 63
    // es reservado y la activación de paginación hace triple fault) + SCE
    mov ecx, 0xC0000080
    rdmsr
    or eax, 0x901
    wrmsr
    // paginación
    mov eax, cr0
    or eax, 0x80000001
    mov cr0, eax
    // far jmp a 64 bits: EA off32 seg16
    .byte 0xEA
    .long 0x8000 + (ap_tramp64 - ap_tramp_start)
    .word 0x18

.code64
ap_tramp64:
    // marca 'C': modo largo con paginación
    mov dx, 0x3F8
    mov al, 0x43
    out dx, al
    mov rsp, [0x8FC8]
    mov rax, [0x8FD0]
    jmp rax
ap_tramp_end:
"#
);

unsafe extern "C" {
    static ap_tramp_start: u8;
    static ap_tramp_end: u8;
}

/// Frecuencia del timer LAPIC de los APs: es su único mecanismo de
/// preempción (no hay PIC/PIT por core), así que fija el timeslice real.
const AP_TIMER_HZ: u32 = 100;

/// Punto de entrada Rust de cada AP (pila propia, paginación del kernel).
extern "C" fn ap_entry() -> ! {
    // Índice de CPU que la BSP parcheó en la página del trampolín antes del
    // SIPI (1..MAX_CPUS-1; 0 es la BSP). Selecciona la TSS/RSP0 de este AP.
    let cpu = unsafe {
        core::ptr::read_volatile((TRAMP_PHYS as usize + CPUIDX_OFF) as *const u64) as usize
    };
    // SSE/AVX, GDT/TSS y GS (datos per-CPU) propios son estado por-CPU.
    sse::enable();
    crate::arch::gdt::init_cpu(cpu);
    crate::arch::percpu::init(cpu, crate::arch::gdt::kstack_top_for(cpu).as_u64());
    apic::enable_cpu();
    interrupts::load_idt_ap();
    // MSRs de syscall propias (LSTAR → ap_syscall_entry, no el de la BSP):
    // sin esto, un proceso ejecutando `syscall` en este core saltaría a la
    // pila de la BSP (o a lo que hubiera en LSTAR, sin inicializar = #GP).
    crate::task::syscall::init_msrs_ap();
    let id = apic::id();
    set_apic_id(cpu, id);
    crate::println!("smp: cpu apic {id} en línea");
    CPUS_ONLINE.fetch_add(1, Ordering::SeqCst);
    AP_APIC_ID.store(id as u64 | (1 << 63), Ordering::SeqCst);
    apic::timer_periodico(AP_TIMER_HZ);
    x86_64::instructions::interrupts::enable();
    crate::task::ap_enter_scheduler();
}

/// Arranca todos los APs del MADT. Llamar en la BSP con PIT en marcha y
/// antes de lanzar procesos.
pub fn init(rsdp_phys: u64) {
    apic::init_bsp();
    let bsp = apic::id();
    set_apic_id(0, bsp);
    let ids = crate::arch::acpi::apic_ids(rsdp_phys);
    crate::println!("smp: MADT con {} CPUs (bsp apic {bsp})", ids.len());
    if ids.len() <= 1 {
        return;
    }

    // El AP activa CR3+paginación mientras su RIP sigue en esta página física
    // baja (0x8000): sin identidad VA==PA aquí, el fetch justo tras `mov cr0`
    // hace page fault (el mapeo del bootloader es solo phys_to_virt, con
    // offset) y el AP triple-faultea antes de llegar a modo largo.
    crate::mm::ensure_identity_mapped(TRAMP_PHYS, 4096);

    // Copiar el trampolín a la página reservada y montar su GDT/GDTR.
    let tramp_len =
        (&raw const ap_tramp_end) as usize - (&raw const ap_tramp_start) as usize;
    assert!(tramp_len < GDT_OFF, "trampolín demasiado grande");
    let dst = crate::mm::phys_to_virt(TRAMP_PHYS).as_mut_ptr::<u8>();
    unsafe {
        core::ptr::copy_nonoverlapping(&raw const ap_tramp_start, dst, tramp_len);
        // GDT: null, código32, datos32, código64
        let gdt = dst.add(GDT_OFF) as *mut u64;
        gdt.write_unaligned(0);
        gdt.add(1).write_unaligned(0x00CF9A000000FFFF);
        gdt.add(2).write_unaligned(0x00CF92000000FFFF);
        gdt.add(3).write_unaligned(0x00AF9A000000FFFF);
        // GDTR: limit + base física
        (dst.add(GDTR_OFF) as *mut u16).write_unaligned(4 * 8 - 1);
        (dst.add(GDTR_OFF + 2) as *mut u32).write_unaligned((TRAMP_PHYS as u32) + GDT_OFF as u32);
    }
    let cr3 = x86_64::registers::control::Cr3::read().0.start_address().as_u64();

    let mut arrancadas = 1u32;
    for (i, &apic_id) in ids.iter().filter(|&&id| id != bsp).enumerate() {
        if i >= MAX_CPUS - 1 {
            crate::println!("smp: limitado a {MAX_CPUS} CPUs");
            break;
        }
        unsafe {
            // parchear cr3 / pila / entry / índice de CPU para ESTE AP
            let stack_top = (&raw const AP_STACKS[i]) as u64 + AP_STACK_SIZE as u64;
            (dst.add(CR3_OFF) as *mut u64).write_unaligned(cr3);
            // convención post-call (rsp%16==8)
            (dst.add(STACK_OFF) as *mut u64).write_unaligned(stack_top - 8);
            (dst.add(ENTRY_OFF) as *mut u64).write_unaligned(ap_entry as *const () as usize as u64);
            (dst.add(CPUIDX_OFF) as *mut u64).write_unaligned((i + 1) as u64);
        }
        let antes = CPUS_ONLINE.load(Ordering::SeqCst);
        crate::println!("smp: arrancando apic {apic_id}…");
        apic::arrancar_ap(apic_id, (TRAMP_PHYS >> 12) as u8);
        // esperar a que se anuncie (con timeout)
        let fin = crate::arch::pit::uptime_ms() + 200;
        while CPUS_ONLINE.load(Ordering::SeqCst) == antes
            && crate::arch::pit::uptime_ms() < fin
        {
            core::hint::spin_loop();
        }
        if CPUS_ONLINE.load(Ordering::SeqCst) == antes {
            crate::println!("smp: apic {apic_id} no respondió");
        } else {
            arrancadas += 1;
        }
    }
    crate::println!("smp: {arrancadas}/{} CPUs en línea", ids.len());
}
