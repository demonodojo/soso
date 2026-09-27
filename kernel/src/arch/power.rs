//! Apagado (ACPI S5) y reinicio en placa. En QEMU el apagado sigue saliendo
//! por isa-debug-exit para que `xtask test` vea el código 33.

use x86_64::instructions::interrupts;
use x86_64::instructions::port::Port;

use super::acpi::{self, RegAcpi};

/// Vuelca logs, suelta la GPU y apaga (`false`) o reinicia (`true`).
pub fn solicitar(reiniciar: bool) -> ! {
    if reiniciar {
        crate::println!("reboot: reiniciando soso");
    } else {
        crate::println!("halt: apagando soso");
    }
    #[cfg(feature = "drv-live-disk")]
    let _ = crate::drivers::fatlog::flush();
    crate::drivers::logfs::drenar_todo();
    #[cfg(feature = "drv-gpu-nvidia")]
    crate::drivers::gpu::shutdown();
    if reiniciar {
        reiniciar_hw();
    } else {
        apagar_hw();
    }
}

fn apagar_hw() -> ! {
    // El bit de hypervisor está puesto con `-cpu max`. Si apagáramos por ACPI
    // antes, QEMU saldría con código 0 y el test de halt dejaría de ver 33.
    if crate::qemu::bajo_hypervisor() {
        crate::qemu::exit(crate::qemu::ExitCode::Success);
    }
    if let Some(e) = acpi::energia() {
        if let (Some(pm1a), Some(typa)) = (e.pm1a, e.slp_typa) {
            crate::println!("halt: S5 typ={typa} pm1a={:#x}", pm1a.addr);
            interrupts::disable();
            activar_acpi(e.smi_cmd, e.acpi_enable, pm1a);
            let _ = escribir_sueno(pm1a, typa);
            if let Some(pm1b) = e.pm1b {
                let _ = escribir_sueno(pm1b, e.slp_typb.unwrap_or(typa));
            }
            crate::arch::tsc::delay_ms(300);
        }
    }
    interrupts::disable();
    crate::println!("halt: no se pudo apagar");
    loop {
        x86_64::instructions::hlt();
    }
}

fn reiniciar_hw() -> ! {
    interrupts::disable();
    if let Some(e) = acpi::energia() {
        if let Some(reg) = e.reset_reg {
            let _ = escribir_reg(reg, e.reset_val as u32);
            crate::arch::tsc::delay_ms(50);
        }
    }
    reiniciar_teclado();
    reiniciar_cf9();
    triple_fault()
}

/// Pone el chipset en modo ACPI si el firmware arrancó en modo legacy.
fn activar_acpi(smi_cmd: u32, acpi_enable: u8, pm1a: RegAcpi) {
    if smi_cmd == 0 || acpi_enable == 0 {
        return;
    }
    let Some(cur) = leer_reg(pm1a) else {
        return;
    };
    if cur & 1 != 0 {
        return;
    }
    let Ok(port) = u16::try_from(smi_cmd) else {
        return;
    };
    outb(port, acpi_enable);
    for _ in 0..1000 {
        crate::arch::tsc::delay_ms(1);
        if leer_reg(pm1a).unwrap_or(0) & 1 != 0 {
            return;
        }
    }
    crate::println!("acpi: SCI_EN no se activó");
}

/// SLP_TYPx en los bits 10-12 y SLP_EN (bit 13) en el mismo write.
fn escribir_sueno(reg: RegAcpi, typ: u8) -> bool {
    let Some(cur) = leer_reg(reg) else {
        return false;
    };
    let mut v = cur;
    v &= !((7 << 10) | (1 << 13));
    v |= (u32::from(typ) & 7) << 10;
    v |= 1 << 13;
    escribir_reg(reg, v)
}

fn leer_reg(reg: RegAcpi) -> Option<u32> {
    match reg.space {
        1 => {
            let port = u16::try_from(reg.addr).ok()?;
            Some(match reg.width {
                8 => u32::from(inb(port)),
                32 => inl(port),
                _ => u32::from(inw(port)),
            })
        }
        0 => {
            crate::mm::ensure_mmio_mapped(reg.addr, 4);
            let p = crate::mm::phys_to_virt(reg.addr);
            unsafe {
                Some(match reg.width {
                    8 => u32::from(core::ptr::read_volatile(p.as_ptr::<u8>())),
                    32 => core::ptr::read_volatile(p.as_ptr::<u32>()),
                    _ => u32::from(core::ptr::read_volatile(p.as_ptr::<u16>())),
                })
            }
        }
        _ => None,
    }
}

fn escribir_reg(reg: RegAcpi, val: u32) -> bool {
    match reg.space {
        1 => {
            let Ok(port) = u16::try_from(reg.addr) else {
                return false;
            };
            match reg.width {
                8 => outb(port, val as u8),
                32 => outl(port, val),
                _ => outw(port, val as u16),
            }
            true
        }
        0 => {
            crate::mm::ensure_mmio_mapped(reg.addr, 4);
            let p = crate::mm::phys_to_virt(reg.addr);
            unsafe {
                match reg.width {
                    8 => core::ptr::write_volatile(p.as_mut_ptr::<u8>(), val as u8),
                    32 => core::ptr::write_volatile(p.as_mut_ptr::<u32>(), val),
                    _ => core::ptr::write_volatile(p.as_mut_ptr::<u16>(), val as u16),
                }
            }
            true
        }
        _ => false,
    }
}

fn reiniciar_teclado() {
    // Sin i8042 el puerto 0x64 lee 0xff. Esperarlo hasta el tope retrasa el CF9.
    if inb(0x64) == 0xff {
        return;
    }
    for _ in 0..10 {
        for _ in 0..10_000 {
            if inb(0x64) & 0x02 == 0 {
                break;
            }
            crate::arch::tsc::delay_us(2);
        }
        crate::arch::tsc::delay_us(50);
        outb(0x64, 0xfe);
        crate::arch::tsc::delay_us(50);
    }
}

fn reiniciar_cf9() {
    let cf9 = inb(0xcf9) & !0x06;
    outb(0xcf9, cf9 | 0x02);
    crate::arch::tsc::delay_us(50);
    outb(0xcf9, cf9 | 0x06);
    crate::arch::tsc::delay_ms(50);
}

fn triple_fault() -> ! {
    #[repr(C, packed)]
    struct Idtr {
        limit: u16,
        base: u64,
    }
    let idtr = Idtr { limit: 0, base: 0 };
    unsafe {
        core::arch::asm!(
            "lidt [{idtr}]",
            "int3",
            idtr = in(reg) &idtr,
            options(noreturn),
        );
    }
}

fn outb(port: u16, val: u8) {
    unsafe { Port::new(port).write(val) }
}

fn inb(port: u16) -> u8 {
    unsafe { Port::new(port).read() }
}

fn outw(port: u16, val: u16) {
    unsafe { Port::new(port).write(val) }
}

fn inw(port: u16) -> u16 {
    unsafe { Port::new(port).read() }
}

fn outl(port: u16, val: u32) {
    unsafe { Port::new(port).write(val) }
}

fn inl(port: u16) -> u32 {
    unsafe { Port::new(port).read() }
}
