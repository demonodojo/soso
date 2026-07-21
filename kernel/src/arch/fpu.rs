//! Estado FPU/SSE/AVX por proceso vía xsave64/xrstor64 (x87 + XMM + YMM).
//!
//! fxsave NO basta: solo guarda los 128 bits bajos (XMM). Con dos procesos
//! de usuario usando AVX, al alternarlos cada uno acabaría con sus mitades
//! bajas y las mitades ALTAS (YMM) del otro — lo detecta el test `init
//! test` (hijos "fpu"). xsave con máscara x87|SSE|AVX guarda el estado
//! completo. Requiere XSAVE en la CPU (QEMU -cpu max y cualquier x86_64
//! moderno; `sse::enable` lo verifica en el arranque).
//!
//! Dónde se preserva:
//! - `timer_isr` hace xsave a `TIMER_FPU` antes de `net::poll` (cripto SSE)
//!   y xrstor al volver sin desalojo; si desaloja, `timer_tick` copia el
//!   estado a `Process.fpu` y `schedule_inner` lo restaura al reanudar.
//! - El page fault handler preserva alrededor de `handle_mmap_fault`.
//! - Las syscalls NO preservan: la ABI declara los registros vectoriales
//!   caller-saved y los wrappers de libsoso llevan `clobber_abi("C")`.

/// Máscara XCR0: x87 | SSE | AVX.
const XSAVE_MASK: u64 = 0b111;

/// Área xsave: 512 legacy + 64 header + 256 AVX = 832; se redondea a 1024.
/// Un área a CERO es válida para xrstor (XSTATE_BV=0 → estado init limpio,
/// MXCSR=0x1F80) y para el primer xsave (header sin compactación).
#[repr(C, align(64))]
#[derive(Clone)]
pub struct FpuArea(pub [u8; 1024]);

impl FpuArea {
    pub const fn empty() -> Self {
        Self([0; 1024])
    }

    /// Estado inicial de un proceso: init-state limpio.
    pub fn inicial() -> Self {
        Self::empty()
    }
}

/// # Safety
/// XSAVE habilitado (XCR0 con x87|SSE|AVX) y `area` alineada a 64
/// (garantizado por el tipo).
pub unsafe fn save(area: &mut FpuArea) {
    unsafe {
        core::arch::asm!(
            "xsave64 [{area}]",
            area = in(reg) area.0.as_mut_ptr(),
            in("rax") XSAVE_MASK,
            in("rdx") 0u64,
            options(nostack),
        );
    }
}

/// # Safety
/// `area` debe contener un estado xsave válido (o ceros = estado init).
pub unsafe fn restore(area: &FpuArea) {
    unsafe {
        core::arch::asm!(
            "xrstor64 [{area}]",
            area = in(reg) area.0.as_ptr(),
            in("rax") XSAVE_MASK,
            in("rdx") 0u64,
            options(nostack, readonly),
        );
    }
}

/// Buffer del timer: único porque soso es monocore y el handler corre con
/// IF=0 (no puede anidar).
pub static mut TIMER_FPU: FpuArea = FpuArea::empty();
