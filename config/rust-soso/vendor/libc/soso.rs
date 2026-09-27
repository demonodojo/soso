//! Tipos que `rustc_codegen_llvm` pide y la rama vacía de `libc` no publica.
//!
//! En x86_64 `size_t` es `usize`. `free` no llama al asignador: soso no tiene
//! la libc de C, y el check sólo necesita el símbolo.

use core::ffi::c_void;

#[allow(non_camel_case_types)]
pub type size_t = usize;

/// No libera. El puntero se queda vivo hasta que el proceso termina.
pub unsafe fn free(_p: *mut c_void) {}
