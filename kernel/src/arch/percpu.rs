//! Datos por-CPU accesibles vía GS base (`IA32_GS_BASE`, MSR 0xC0000101):
//! cada core apunta su GS a su propia instancia de `PerCpu`, así que
//! `gs:OFFSET` siempre lee/escribe el campo de "este" core sin tener que
//! calcular antes qué CPU es (a diferencia de una tabla indexada por APIC
//! id). Necesario para que el scheduler y la entrada de syscall dejen de
//! asumir un único core: hoy referencian `KSTACK`/`USER_RSP_SCRATCH` por
//! símbolo fijo desde ensamblador desnudo, lo que con SMP haría que dos
//! cores pisaran la misma pila.
//!
//! Los offsets son fijos y los usa directamente ensamblador desnudo
//! (`task::ap_timer_isr`, `task::syscall::ap_syscall_entry`): si se
//! reordena esta struct hay que actualizar esos sitios a la vez.

use crate::arch::fpu::FpuArea;
use crate::arch::smp::MAX_CPUS;
use x86_64::registers::model_specific::Msr;

const IA32_GS_BASE: u32 = 0xC000_0101;

#[repr(C)]
struct PerCpu {
    kstack_top: u64,
    current_pid: u64,
    cpu_index: u64,
    fpu_scratch: u64,
    syscall_scratch: u64,
}

pub const OFF_KSTACK_TOP: usize = 0;
pub const OFF_CURRENT_PID: usize = 8;
pub const OFF_CPU_INDEX: usize = 16;
pub const OFF_FPU_SCRATCH: usize = 24;
pub const OFF_SYSCALL_SCRATCH: usize = 32;

static mut PERCPU: [PerCpu; MAX_CPUS] = [const {
    PerCpu {
        kstack_top: 0,
        current_pid: 0,
        cpu_index: 0,
        fpu_scratch: 0,
        syscall_scratch: 0,
    }
}; MAX_CPUS];

/// Área xsave por-CPU para las ISR de timer (BSP y AP la necesitan cada una
/// la suya): fuera de `PerCpu` porque `FpuArea` exige alineación de 64 B.
#[repr(align(64))]
struct FpuSlot(FpuArea);
static mut FPU_SCRATCH: [FpuSlot; MAX_CPUS] = [const { FpuSlot(FpuArea::empty()) }; MAX_CPUS];

/// Inicializa y activa el bloque per-CPU de la CPU actual (`cpu`: 0 = BSP).
/// Llamar UNA vez por core, con `kstack_top` ya decidido para ese core.
pub fn init(cpu: usize, kstack_top: u64) {
    unsafe {
        let p = &raw mut PERCPU[cpu];
        (*p).kstack_top = kstack_top;
        (*p).current_pid = 0;
        (*p).cpu_index = cpu as u64;
        (*p).fpu_scratch = (&raw mut FPU_SCRATCH[cpu]) as u64;
        (*p).syscall_scratch = 0;
        Msr::new(IA32_GS_BASE).write(p as u64);
    }
}

#[inline]
pub fn current_pid() -> u64 {
    let v: u64;
    unsafe {
        core::arch::asm!(
            "mov {v}, gs:[{off}]",
            v = out(reg) v,
            off = const OFF_CURRENT_PID,
            options(nostack, preserves_flags, readonly),
        );
    }
    v
}

#[inline]
pub fn set_current_pid(pid: u64) {
    unsafe {
        core::arch::asm!(
            "mov gs:[{off}], {pid}",
            pid = in(reg) pid,
            off = const OFF_CURRENT_PID,
            options(nostack, preserves_flags),
        );
    }
}

/// Puntero al `FpuArea` xsave de este core (para la ISR de timer de un AP,
/// que necesita leerlo/escribirlo con una dirección en tiempo de ejecución
/// en vez de un símbolo fijo).
#[inline]
pub fn fpu_scratch_ptr() -> u64 {
    let v: u64;
    unsafe {
        core::arch::asm!(
            "mov {v}, gs:[{off}]",
            v = out(reg) v,
            off = const OFF_FPU_SCRATCH,
            options(nostack, preserves_flags, readonly),
        );
    }
    v
}

#[inline]
pub fn cpu_index() -> usize {
    let v: u64;
    unsafe {
        core::arch::asm!(
            "mov {v}, gs:[{off}]",
            v = out(reg) v,
            off = const OFF_CPU_INDEX,
            options(nostack, preserves_flags, readonly),
        );
    }
    v as usize
}
