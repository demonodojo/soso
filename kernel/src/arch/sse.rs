//! Activación de SSE por hardware. El bootloader deja SSE deshabilitado
//! (CR0.EM=1), pero compilamos con +sse2 (la cripto de sunset lo exige),
//! así que hay que habilitarlo ANTES de ejecutar cualquier instrucción
//! SSE — es lo primero de `kernel_main`.

use x86_64::registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags};

/// # Safety
/// Debe llamarse una sola vez, al principio del arranque, antes de que se
/// ejecute código compilado con SSE (registros XMM).
pub fn enable() {
    unsafe {
        Cr0::update(|cr0| {
            cr0.remove(Cr0Flags::EMULATE_COPROCESSOR); // EM=0: SSE real, no emulado
            cr0.insert(Cr0Flags::MONITOR_COPROCESSOR); // MP=1
        });
        Cr4::update(|cr4| {
            // OSFXSR: el SO gestiona el estado FPU/SSE con fxsave/fxrstor.
            cr4.insert(Cr4Flags::OSFXSR);
            // OSXMMEXCPT: excepciones SIMD por #XM en vez de #UD.
            cr4.insert(Cr4Flags::OSXMMEXCPT_ENABLE);
        });
    }
}
