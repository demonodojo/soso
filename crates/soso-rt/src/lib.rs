//! ABI mínima para `library/std` en `x86_64-unknown-soso`.
//!
//! Expone símbolos al estilo `hermit-abi` que la PAL de std espera.

#![no_std]
#![allow(nonstandard_style)]

pub use soso_abi::*;

use core::arch::asm;

#[inline]
pub unsafe fn syscall0(n: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => ret,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack, preserves_flags)
        );
    }
    ret
}

#[inline]
pub unsafe fn syscall1(n: u64, a1: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => ret,
            in("rdi") a1,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack, preserves_flags)
        );
    }
    ret
}

#[inline]
pub unsafe fn syscall3(n: u64, a1: u64, a2: u64, a3: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => ret,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack, preserves_flags)
        );
    }
    ret
}

pub fn exit(code: i32) -> ! {
    unsafe {
        let _ = syscall1(SYS_EXIT, code as u64);
        loop {
            asm!("pause", options(nomem, nostack));
        }
    }
}

pub fn abort() -> ! {
    exit(134);
}

pub unsafe fn write(fd: u64, buf: *const u8, len: usize) -> isize {
    let n = syscall3(SYS_WRITE, fd, buf as u64, len as u64);
    if n < 0 { -1 } else { n as isize }
}

pub unsafe fn read(fd: u64, buf: *mut u8, len: usize) -> isize {
    let n = syscall3(SYS_READ, fd, buf as u64, len as u64);
    if n < 0 { -1 } else { n as isize }
}
