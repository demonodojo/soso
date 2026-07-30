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
        habilitar_avx();
    }
}

/// AVX para el userspace (el kernel compila sin AVX): CR4.OSXSAVE +
/// XCR0 = x87|SSE|AVX. Sin esto, vmovups en ring 3 da #UD.
unsafe fn habilitar_avx() {
    use core::arch::x86_64::__cpuid;
    let f = __cpuid(1);
    let xsave = f.ecx & (1 << 26) != 0;
    let avx = f.ecx & (1 << 28) != 0;
    if !(xsave && avx) {
        // El cambio de contexto usa xsave/xrstor incondicionalmente y el
        // userspace compila con AVX2: sin soporte no se puede continuar.
        panic!("sse: CPU sin XSAVE/AVX (usa QEMU -cpu max o un x86_64 moderno)");
    }
    unsafe {
        Cr4::update(|cr4| cr4.insert(Cr4Flags::OSXSAVE));
        use x86_64::registers::xcontrol::{XCr0, XCr0Flags};
        XCr0::update(|x| {
            x.insert(XCr0Flags::X87);
            x.insert(XCr0Flags::SSE);
            x.insert(XCr0Flags::AVX);
        });
    }
}
