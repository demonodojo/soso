//! Salida controlada de QEMU vía isa-debug-exit (puerto 0xf4).
//! QEMU termina con código (valor << 1) | 1.

use x86_64::instructions::port::Port;

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
