//! SMEP/SMAP y línea de arranque.

use core::sync::atomic::{AtomicBool, Ordering};
use x86_64::registers::control::{Cr4, Cr4Flags};

static CLAC_EN_ISR: AtomicBool = AtomicBool::new(false);

pub struct Hardening {
    pub smep: bool,
    pub smap: bool,
}

fn leaf7_ebx() -> u32 {
    let eax = 7u32;
    let ecx = 0u32;
    let ebx_out: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {ebx_out:e}, ebx",
            "pop rbx",
            ebx_out = out(reg) ebx_out,
            inout("eax") eax => _,
            inout("ecx") ecx => _,
            lateout("edx") _,
            options(nostack, preserves_flags),
        );
    }
    ebx_out
}

pub fn detect() -> Hardening {
    let ebx = leaf7_ebx();
    Hardening {
        smep: ebx & (1 << 7) != 0,
        smap: ebx & (1 << 20) != 0,
    }
}

pub fn enable_smep() {
    if detect().smep {
        unsafe {
            Cr4::update(|cr4| cr4.insert(Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION));
        }
    }
}

pub fn enable_smap() {
    if detect().smap {
        unsafe {
            Cr4::update(|cr4| cr4.insert(Cr4Flags::SUPERVISOR_MODE_ACCESS_PREVENTION));
        }
        CLAC_EN_ISR.store(true, Ordering::Relaxed);
    }
}

/// Hereda AC del contexto interrumpido; en ISR/excepciones hay que limpiarlo.
#[inline]
pub fn clac_en_entrada_isr() {
    if CLAC_EN_ISR.load(Ordering::Relaxed) {
        unsafe {
            core::arch::asm!("clac", options(nostack, preserves_flags));
        }
    }
}

pub fn log_line() {
    let h = detect();
    let cr4 = Cr4::read();
    crate::println!(
        "cpu: SMEP={} SMAP={}",
        if h.smep && cr4.contains(Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION) {
            "on"
        } else if h.smep {
            "off"
        } else {
            "no"
        },
        if h.smap && cr4.contains(Cr4Flags::SUPERVISOR_MODE_ACCESS_PREVENTION) {
            "on"
        } else if h.smap {
            "off"
        } else {
            "no"
        }
    );
}
