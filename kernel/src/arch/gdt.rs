//! GDT + TSS. La TSS aporta la pila alternativa (IST) para el handler de
//! double fault y la pila de ring 0 (RSP0) usada al entrar desde ring 3.
//!
//! El ORDEN de los segmentos importa: sysret carga CS = STAR.sysret+16 y
//! SS = STAR.sysret+8, así que user_data debe ir justo antes de user_code
//! (x86_64::Star::write lo verifica).
//!
//! Cada CPU tiene su propia TSS (RSP0 + pila IST de double fault): sin esto,
//! dos cores atendiendo una excepción o interrupción a la vez pisarían la
//! misma pila. La CPU 0 (BSP) sigue usando `KSTACK` (el scheduler y la
//! entrada de syscall, en ensamblador desnudo, la referencian por símbolo);
//! las demás (APs) tienen pilas propias reservadas aquí — hoy solo las usa
//! el timer LAPIC (`arch::smp`), sin scheduler multicore todavía (L3b).

use crate::arch::smp::MAX_CPUS;
use spin::Lazy;
use x86_64::VirtAddr;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

/// Pila de kernel para entradas desde ring 3 (syscall e interrupciones) de
/// la BSP (CPU 0).
pub const KSTACK_SIZE: usize = 64 * 1024;

#[repr(C, align(16))]
pub struct KStack(pub [u8; KSTACK_SIZE]);

pub static mut KSTACK: KStack = KStack([0; KSTACK_SIZE]);

pub fn kstack_top() -> VirtAddr {
    VirtAddr::from_ptr(&raw const KSTACK) + KSTACK_SIZE as u64
}

/// RSP0 de los APs (CPU 1..MAX_CPUS-1): hoy solo atienden el timer LAPIC (sin
/// anidar), no necesitan el fondo de la pila de la BSP.
const AP_KSTACK_SIZE: usize = 16 * 1024;
#[repr(C, align(16))]
struct ApKStack([u8; AP_KSTACK_SIZE]);
static mut AP_KSTACKS: [ApKStack; MAX_CPUS - 1] =
    [const { ApKStack([0; AP_KSTACK_SIZE]) }; MAX_CPUS - 1];

fn rsp0_for(cpu: usize) -> VirtAddr {
    if cpu == 0 {
        kstack_top()
    } else {
        unsafe { VirtAddr::from_ptr(&raw const AP_KSTACKS[cpu - 1]) + AP_KSTACK_SIZE as u64 }
    }
}

/// Pila IST del double fault, una por CPU.
const IST_SIZE: usize = 4096 * 5;
#[repr(C, align(16))]
struct IstStack([u8; IST_SIZE]);
static mut IST_STACKS: [IstStack; MAX_CPUS] = [const { IstStack([0; IST_SIZE]) }; MAX_CPUS];

fn ist_top_for(cpu: usize) -> VirtAddr {
    unsafe { VirtAddr::from_ptr(&raw const IST_STACKS[cpu]) + IST_SIZE as u64 }
}

static TSS: Lazy<[TaskStateSegment; MAX_CPUS]> = Lazy::new(|| {
    core::array::from_fn(|cpu| {
        let mut tss = TaskStateSegment::new();
        // La pila crece hacia abajo: se apunta al final.
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = ist_top_for(cpu);
        // RSP0: adónde salta la CPU en una interrupción llegando de ring 3.
        tss.privilege_stack_table[0] = rsp0_for(cpu);
        tss
    })
});

pub struct Selectors {
    pub kcode: SegmentSelector,
    pub kdata: SegmentSelector,
    /// Con RPL 3, listos para STAR/iretq.
    pub udata: SegmentSelector,
    pub ucode: SegmentSelector,
    pub tss: [SegmentSelector; MAX_CPUS],
}

/// El slot 0 es el descriptor nulo implícito + 4 descriptores planos + una
/// TSS (2 slots cada una, son descriptores de sistema) por CPU posible.
const GDT_CAP: usize = 1 + 4 + 2 * MAX_CPUS;

static GDT: Lazy<(GlobalDescriptorTable<GDT_CAP>, Selectors)> = Lazy::new(|| {
    use x86_64::PrivilegeLevel::Ring3;
    let mut gdt = GlobalDescriptorTable::<GDT_CAP>::empty();
    let kcode = gdt.append(Descriptor::kernel_code_segment()); // 0x08
    let kdata = gdt.append(Descriptor::kernel_data_segment()); // 0x10
    let mut udata = gdt.append(Descriptor::user_data_segment()); // 0x18
    let mut ucode = gdt.append(Descriptor::user_code_segment()); // 0x20
    udata.set_rpl(Ring3);
    ucode.set_rpl(Ring3);
    let tss = core::array::from_fn(|cpu| gdt.append(Descriptor::tss_segment(&TSS[cpu]))); // 0x28...
    (gdt, Selectors { kcode, kdata, udata, ucode, tss })
});

pub fn selectors() -> &'static Selectors {
    &GDT.1
}

/// Inicializa GDT+TSS en la CPU actual. `cpu` es su índice (0 = BSP).
pub fn init_cpu(cpu: usize) {
    use x86_64::instructions::segmentation::{CS, DS, ES, SS, Segment};
    use x86_64::instructions::tables::load_tss;

    GDT.0.load();
    unsafe {
        CS::set_reg(GDT.1.kcode);
        SS::set_reg(GDT.1.kdata);
        DS::set_reg(GDT.1.kdata);
        ES::set_reg(GDT.1.kdata);
        load_tss(GDT.1.tss[cpu]);
    }
}

pub fn init() {
    init_cpu(0);
    // El código de cambio de contexto (iretq) los lleva en duro.
    assert_eq!(GDT.1.udata.0, 0x18 | 3, "selector udata inesperado");
    assert_eq!(GDT.1.ucode.0, 0x20 | 3, "selector ucode inesperado");
}
