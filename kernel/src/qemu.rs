//! Salida controlada de QEMU vía isa-debug-exit (puerto 0xf4).
//! QEMU termina con código (valor << 1) | 1.

use x86_64::instructions::port::Port;

/// ¿Hay un VMM encima (KVM/QEMU)? CPUID.1:ECX[31]. En placa desnuda va a 0.
///
/// El fini del GSP solo es obligatorio cuando el host va a resetear la GPU
/// al salir (VFIO). En metal el `hlt` no suelta la tarjeta.
pub fn bajo_hypervisor() -> bool {
    core::arch::x86_64::__cpuid(1).ecx & (1 << 31) != 0
}

#[derive(Clone, Copy)]
#[repr(u32)]
#[allow(dead_code)] // Success lo usará la batería de tests
pub enum ExitCode {
    Success = 0x10, // exit code 33
    Failed = 0x11,  // exit code 35
}

pub fn exit(code: ExitCode) -> ! {
    unsafe {
        Port::new(0xf4).write(code as u32);
    }
    // Si el dispositivo no existe (hardware real), detenerse.
    loop {
        x86_64::instructions::hlt();
    }
}
