//! GDT + TSS. La TSS aporta la pila alternativa (IST) para el handler de
//! double fault y la pila de ring 0 (RSP0) usada al entrar desde ring 3.
//!
//! El ORDEN de los segmentos importa: sysret carga CS = STAR.sysret+16 y
//! SS = STAR.sysret+8, así que user_data debe ir justo antes de user_code
//! (x86_64::Star::write lo verifica).

use spin::Lazy;
use x86_64::VirtAddr;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

/// Pila de kernel para entradas desde ring 3 (syscall e interrupciones).
/// Única: monocore y un solo proceso dentro del kernel a la vez.
pub const KSTACK_SIZE: usize = 64 * 1024;

#[repr(C, align(16))]
pub struct KStack(pub [u8; KSTACK_SIZE]);

pub static mut KSTACK: KStack = KStack([0; KSTACK_SIZE]);

pub fn kstack_top() -> VirtAddr {
    VirtAddr::from_ptr(&raw const KSTACK) + KSTACK_SIZE as u64
}

static TSS: Lazy<TaskStateSegment> = Lazy::new(|| {
    let mut tss = TaskStateSegment::new();
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
        const STACK_SIZE: usize = 4096 * 5;
        static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
        // La pila crece hacia abajo: se apunta al final.
        VirtAddr::from_ptr(&raw const STACK) + STACK_SIZE as u64
    };
    // RSP0: adónde salta la CPU en una interrupción llegando de ring 3.
    tss.privilege_stack_table[0] = kstack_top();
    tss
});

pub struct Selectors {
    pub kcode: SegmentSelector,
    pub kdata: SegmentSelector,
    /// Con RPL 3, listos para STAR/iretq.
    pub udata: SegmentSelector,
    pub ucode: SegmentSelector,
    pub tss: SegmentSelector,
}

static GDT: Lazy<(GlobalDescriptorTable, Selectors)> = Lazy::new(|| {
    use x86_64::PrivilegeLevel::Ring3;
    let mut gdt = GlobalDescriptorTable::new();
    let kcode = gdt.append(Descriptor::kernel_code_segment()); // 0x08
    let kdata = gdt.append(Descriptor::kernel_data_segment()); // 0x10
    let mut udata = gdt.append(Descriptor::user_data_segment()); // 0x18
    let mut ucode = gdt.append(Descriptor::user_code_segment()); // 0x20
    let tss = gdt.append(Descriptor::tss_segment(&TSS)); // 0x28 (+0x30)
    udata.set_rpl(Ring3);
    ucode.set_rpl(Ring3);
    (gdt, Selectors { kcode, kdata, udata, ucode, tss })
});

pub fn selectors() -> &'static Selectors {
    &GDT.1
}

pub fn init() {
    use x86_64::instructions::segmentation::{CS, DS, ES, SS, Segment};
    use x86_64::instructions::tables::load_tss;

    GDT.0.load();
    unsafe {
        CS::set_reg(GDT.1.kcode);
        SS::set_reg(GDT.1.kdata);
        DS::set_reg(GDT.1.kdata);
        ES::set_reg(GDT.1.kdata);
        load_tss(GDT.1.tss);
    }
    // El código de cambio de contexto (iretq) los lleva en duro.
    assert_eq!(GDT.1.udata.0, 0x18 | 3, "selector udata inesperado");
    assert_eq!(GDT.1.ucode.0, 0x20 | 3, "selector ucode inesperado");
}
