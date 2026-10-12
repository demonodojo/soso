//! LAPIC con dos backends: x2APIC (MSRs; hardware real moderno) y xAPIC
//! clásico (MMIO en 0xFEE00000; QEMU TCG no emula x2APIC).

use core::sync::atomic::{AtomicBool, Ordering};
use x86_64::registers::model_specific::Msr;

const IA32_APIC_BASE: u32 = 0x1B;
const XAPIC_MMIO: u64 = 0xFEE0_0000;

// Offsets MMIO xAPIC (los MSR x2APIC son 0x800 + off/16)
const R_ID: u32 = 0x20;
const R_EOI: u32 = 0xB0;
const R_SPURIOUS: u32 = 0xF0;
const R_ICR_LO: u32 = 0x300;
const R_ICR_HI: u32 = 0x310;
const R_LVT_TIMER: u32 = 0x320;
const R_LVT_LINT0: u32 = 0x350;
const R_LVT_LINT1: u32 = 0x360;
const R_TIMER_INIT: u32 = 0x380;
const R_TIMER_CUR: u32 = 0x390;
const R_TIMER_DIV: u32 = 0x3E0;

/// Vector del timer LAPIC (los APs; la BSP sigue con PIC+PIT).
pub const TIMER_VECTOR: u8 = 0x40;
/// IPI de replanificación: despierta un core en `hlt` para que mire PROCS.
pub const RESCHED_VECTOR: u8 = 0x41;
/// IPI de shootdown TLB: recarga CR3 en el core destino.
pub const TLB_SHOOTDOWN_VECTOR: u8 = 0x42;
pub const SPURIOUS_VECTOR: u8 = 0xFF;

static MODO_X2: AtomicBool = AtomicBool::new(false);
/// `eoi()` toca MMIO xAPIC; antes de `init_bsp` esa página no está mapeada.
static LISTO: AtomicBool = AtomicBool::new(false);

fn x2() -> bool {
    MODO_X2.load(Ordering::Relaxed)
}

fn mmio_read(off: u32) -> u32 {
    unsafe { core::ptr::read_volatile(crate::mm::phys_to_virt(XAPIC_MMIO + off as u64).as_ptr()) }
}

fn mmio_write(off: u32, v: u32) {
    unsafe {
        core::ptr::write_volatile(
            crate::mm::phys_to_virt(XAPIC_MMIO + off as u64).as_mut_ptr(),
            v,
        );
    }
}

fn msr_read(off: u32) -> u64 {
    unsafe { Msr::new(0x800 + off / 16).read() }
}

fn msr_write(off: u32, v: u64) {
    unsafe { Msr::new(0x800 + off / 16).write(v) };
}

/// Detección + mapeo MMIO. Llamar UNA vez en la BSP antes de los APs.
pub fn init_bsp() {
    let f = core::arch::x86_64::__cpuid(1);
    let tiene_x2 = f.ecx & (1 << 21) != 0;
    MODO_X2.store(tiene_x2, Ordering::Relaxed);
    if !tiene_x2 {
        crate::mm::ensure_mmio_mapped(XAPIC_MMIO, 4096);
    }
    enable_cpu();
    // Virtual wire: el 8259 entra por LINT0. Sin ExtINT, al poner el LAPIC
    // (sobre todo x2APIC) el PIT deja de tic-tac y cualquier espera con
    // `uptime_ms()` se queda girando (placa: `smp: arrancando apic …`).
    lvt_write(R_LVT_LINT0, 0b111 << 8); // ExtINT, desenmascarado
    lvt_write(R_LVT_LINT1, 0b100 << 8); // NMI
    LISTO.store(true, Ordering::Release);
    crate::println!(
        "apic: {} (id {})",
        if tiene_x2 { "x2APIC" } else { "xAPIC mmio" },
        id()
    );
}

fn lvt_write(off: u32, v: u32) {
    if x2() {
        msr_write(off, v as u64);
    } else {
        mmio_write(off, v);
    }
}

/// Habilita el LAPIC de la CPU actual (BSP o AP).
pub fn enable_cpu() {
    unsafe {
        let mut base = Msr::new(IA32_APIC_BASE);
        let v = base.read();
        let bits = if x2() { (1 << 11) | (1 << 10) } else { 1 << 11 };
        base.write(v | bits);
    }
    let sv = 0x100 | SPURIOUS_VECTOR as u32;
    if x2() {
        msr_write(R_SPURIOUS, sv as u64);
    } else {
        mmio_write(R_SPURIOUS, sv);
    }
}

pub fn id() -> u32 {
    if x2() {
        msr_read(R_ID) as u32
    } else {
        mmio_read(R_ID) >> 24
    }
}

pub fn eoi() {
    if !LISTO.load(Ordering::Acquire) {
        return;
    }
    if x2() {
        msr_write(R_EOI, 0);
    } else {
        mmio_write(R_EOI, 0);
    }
}

/// IPI al APIC destino. En xAPIC espera a que se entregue (bit 12).
fn icr(dest_apic: u32, flags: u32) {
    if x2() {
        // En x2APIC los bits 12/14/15 del ICR están reservados (eran
        // delivery-status / level / trigger). Ponerlos a 1 puede #GP.
        let flags = flags & !((1 << 12) | (1 << 14) | (1 << 15));
        msr_write(R_ICR_LO, ((dest_apic as u64) << 32) | flags as u64);
    } else {
        mmio_write(R_ICR_HI, dest_apic << 24);
        mmio_write(R_ICR_LO, flags);
        while mmio_read(R_ICR_LO) & (1 << 12) != 0 {
            core::hint::spin_loop();
        }
    }
}

/// IPI de entrega fija (`Fixed`) con el vector indicado.
pub fn send_ipi(dest_apic: u32, vector: u8) {
    icr(dest_apic, vector as u32);
}

/// Despierta al resto de CPUs en línea (p. ej. tras hacer Runnable un
/// proceso) para que salgan del `hlt` y miren `PROCS`.
pub fn kick_idle_cpus() {
    broadcast_ipi_except_me(RESCHED_VECTOR);
}

/// Invalida entradas TLB obsoletas en todos los cores (recarga CR3).
pub fn tlb_shootdown_all() {
    broadcast_ipi_except_me(TLB_SHOOTDOWN_VECTOR);
}

fn broadcast_ipi_except_me(vector: u8) {
    let me = crate::arch::percpu::cpu_index();
    let n = crate::arch::smp::CPUS_ONLINE.load(Ordering::Relaxed) as usize;
    for cpu in 0..n.min(crate::arch::smp::MAX_CPUS) {
        if cpu == me {
            continue;
        }
        if let Some(apic_id) = crate::arch::smp::apic_id_of(cpu) {
            send_ipi(apic_id, vector);
        }
    }
}

/// INIT + 2×SIPI al AP indicado (protocolo clásico de arranque).
pub fn arrancar_ap(apic_id: u32, tramp_page: u8) {
    // INIT (assert de nivel) y dos SIPI con el vector de la página.
    // Delays por TSC, no PIT: ver `tsc::delay_ms`.
    icr(apic_id, 0b101 << 8);
    crate::arch::tsc::delay_ms(10);
    icr(apic_id, (0b110 << 8) | tramp_page as u32);
    crate::arch::tsc::delay_ms(1);
    icr(apic_id, (0b110 << 8) | tramp_page as u32);
}

/// Timer LAPIC periódico a ~`hz` en la CPU actual, calibrado contra el PIT.
pub fn timer_periodico(hz: u32) {
    let (wr, rd): (fn(u32, u32), fn(u32) -> u32) = if x2() {
        (|o, v| msr_write(o, v as u64), |o| msr_read(o) as u32)
    } else {
        (mmio_write, mmio_read)
    };
    wr(R_TIMER_DIV, 0b1011); // divisor 1
    wr(R_LVT_TIMER, 1 << 16); // enmascarado, one-shot para calibrar
    wr(R_TIMER_INIT, u32::MAX);
    crate::arch::tsc::delay_ms(50);
    let usados = u32::MAX - rd(R_TIMER_CUR);
    let por_periodo = (usados / 50) * (1000 / hz);
    wr(R_LVT_TIMER, (1 << 17) | TIMER_VECTOR as u32); // periódico
    wr(R_TIMER_INIT, por_periodo.max(1000));
}
