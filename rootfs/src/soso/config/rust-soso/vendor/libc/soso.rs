//! Tipos que `rustc_codegen_llvm` pide y la rama vacía de `libc` no publica.
//!
//! En x86_64 `size_t` es `usize`. `free` no llama al asignador: soso no tiene
//! la libc de C, y el check sólo necesita el símbolo.

use core::ffi::c_void;

#[allow(non_camel_case_types)]
pub type size_t = usize;

/// En x86_64 Linux-like, `off_t` es 64 bits. Lo piden `libz-sys` y similares.
#[allow(non_camel_case_types)]
pub type off_t = i64;

/// x86_64 GNU/Linux usa `time_t` de 64 bits.
#[allow(non_camel_case_types)]
pub type time_t = i64;

/// Tamaño de `fd_set` en x86_64 Linux (`FD_SETSIZE` 1024, 64 bits por slot).
#[repr(C)]
pub struct fd_set {
    fds_bits: [crate::c_ulong; 16],
}

#[allow(non_camel_case_types)]
pub type sa_family_t = u16;

#[repr(C)]
pub struct sockaddr {
    pub sa_family: sa_family_t,
    pub sa_data: [crate::c_char; 14],
}

/// No libera. El puntero se queda vivo hasta que el proceso termina.
pub unsafe fn free(_p: *mut c_void) {}
