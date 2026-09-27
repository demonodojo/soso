//! ABI mínima para `library/std` en `x86_64-unknown-soso`.
//!
//! Expone símbolos al estilo `hermit-abi` que la PAL de std espera.

#![no_std]
#![allow(nonstandard_style)]

pub use soso_abi::*;
pub use soso_alloc::{alloc, alloc_zeroed, dealloc, heap_init, realloc};

use core::arch::asm;

/// # La convención, y por qué el `clobber_abi("C")`
///
/// Número en `rax`, argumentos en `rdi/rsi/rdx/r10` —`rcx` no, porque
/// `syscall` lo machaca con la dirección de retorno— y retorno en `rax`
/// (negativo = `-errno`).
///
/// **El kernel de soso usa SSE** (criptografía, `memcpy`), así que los
/// registros vectoriales **no sobreviven a una syscall**. Y el target
/// `x86_64-unknown-soso` compila con `+avx,+avx2,+fma`. Sin declarar el
/// clobber, el compilador daría por hecho que sí sobreviven y generaría
/// código que lee basura. `user/libsoso` ya lo aprendió; estas envolturas
/// **no lo declaraban**, y lo habría heredado la libstd entera.
///
/// Por lo mismo se va `options(nostack, preserves_flags)`: el kernel no
/// promete ninguna de las dos.

#[inline]
pub unsafe fn syscall0(n: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => ret,
            lateout("rcx") _,
            lateout("r11") _,
            clobber_abi("C"),
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
            inlateout("rdi") a1 => _,
            lateout("rcx") _,
            lateout("r11") _,
            clobber_abi("C"),
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
            inlateout("rdi") a1 => _,
            inlateout("rsi") a2 => _,
            inlateout("rdx") a3 => _,
            lateout("rcx") _,
            lateout("r11") _,
            clobber_abi("C"),
        );
    }
    ret
}

#[inline]
pub unsafe fn syscall4(n: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => ret,
            inlateout("rdi") a1 => _,
            inlateout("rsi") a2 => _,
            inlateout("rdx") a3 => _,
            inlateout("r10") a4 => _,
            lateout("rcx") _,
            lateout("r11") _,
            clobber_abi("C"),
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
    unsafe {
        let n = syscall3(SYS_WRITE, fd, buf as u64, len as u64);
        if n < 0 { -1 } else { n as isize }
    }
}

pub unsafe fn read(fd: u64, buf: *mut u8, len: usize) -> isize {
    unsafe {
        let n = syscall3(SYS_READ, fd, buf as u64, len as u64);
        if n < 0 { -1 } else { n as isize }
    }
}
