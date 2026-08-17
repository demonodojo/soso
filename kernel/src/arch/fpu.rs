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
/// Un área a CERO vale para el primer `xsave` (header sin compactación) y para
/// `xrstor` en cuanto al estado x87/SSE/AVX (XSTATE_BV=0 → init limpio), **pero
/// no para MXCSR**: ver [`FpuArea::inicial`].
#[repr(C, align(64))]
#[derive(Clone)]
pub struct FpuArea(pub [u8; 1024]);

impl FpuArea {
    pub const fn empty() -> Self {
        Self([0; 1024])
    }

    /// Estado inicial de un proceso: init-state limpio y **MXCSR enmascarado**.
    ///
    /// AVERÍA (2026-08-17, sólo en silicio real): esto devolvía el área a ceros
    /// dando por hecho que `xrstor` dejaría MXCSR en su valor init (0x1F80).
    /// No lo hace: `xrstor` **siempre** carga MXCSR desde la imagen en memoria
    /// si la máscara incluye SSE o AVX, mire lo que mire `XSTATE_BV`. Así que
    /// cada proceso arrancaba con **MXCSR = 0**, es decir con todas las
    /// excepciones SIMD DESENMASCARADAS.
    ///
    /// Con eso, la primera operación de coma flotante inexacta levanta #XM. Y
    /// lo es casi cualquiera: `"0.7".parse::<f32>()` compila a un `vdivss` en
    /// `core::num::dec2flt`, que es justo donde moría `ask` al leer
    /// `temp=0.7` de `/etc/llm.conf`. En QEMU no se ve —TCG no entrega #XM—,
    /// solo en la máquina de verdad.
    pub fn inicial() -> Self {
        let mut a = Self::empty();
        // MXCSR va en el offset 24 del área legacy; 0x1F80 = las seis
        // excepciones enmascaradas, redondeo al más cercano.
        a.0[24..28].copy_from_slice(&0x0000_1F80u32.to_le_bytes());
        // MXCSR_MASK (offset 28): 0 haría que `xrstor` rechazara bits válidos.
        a.0[28..32].copy_from_slice(&0x0000_FFBFu32.to_le_bytes());
        // XSTATE_BV (offset 512) con SSE marcado: sin el bit, `xrstor` deja el
        // estado SSE en init pero YA ha cargado el MXCSR de arriba, que es lo
        // que nos importa. Se marca para que la imagen sea autoconsistente.
        a.0[512..520].copy_from_slice(&0x2u64.to_le_bytes());
        a
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
