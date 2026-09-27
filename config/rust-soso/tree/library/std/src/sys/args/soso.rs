//! Argumentos de la línea de órdenes en soso.
//!
//! El kernel entrega un blob `SOSA` en el arranque; `_start` de la PAL lo
//! decodifica en `argc`/`argv` antes de `main` ([T62] y T39). Desde
//! [T62](../../../../../../docs/self-improvement/T62-argv-en-los-programas.md)
//! los productores pasan un vector de argumentos, no una línea de shell. Por
//! eso aquí se leen de verdad en vez de devolver la lista vacía de
//! `unsupported`, que compilaría igual y mentiría.

pub use super::common::Args;
use crate::ffi::{CStr, OsString};
use crate::os::soso::ffi::OsStringExt;
use crate::ptr;
use crate::sync::atomic::{AtomicIsize, AtomicPtr, Ordering};

static ARGC: AtomicIsize = AtomicIsize::new(0);
static ARGV: AtomicPtr<*const u8> = AtomicPtr::new(ptr::null_mut());

/// La llama `runtime_entry` de la PAL antes de `main`. Guarda los punteros que
/// da el kernel; no copia nada todavía.
///
/// # Safety
/// `argv` tiene que apuntar a `argc` punteros a cadenas terminadas en NUL que
/// vivan durante todo el proceso, que es lo que garantiza el cargador de soso.
pub unsafe fn init(argc: isize, argv: *const *const u8) {
    ARGC.store(argc, Ordering::Relaxed);
    ARGV.store(argv as *mut _, Ordering::Relaxed);
}

pub fn args() -> Args {
    let argc = ARGC.load(Ordering::Relaxed);
    let argv = ARGV.load(Ordering::Relaxed);
    if argc <= 0 || argv.is_null() {
        // Antes de `init`, o si el cargador no dejó nada. Lista vacía, que es
        // lo que hay; no un panic, porque `args()` se puede llamar pronto.
        return Args::new(Vec::new());
    }
    let mut v = Vec::with_capacity(argc as usize);
    for i in 0..argc {
        // SAFETY: `init` recibió `argc` punteros válidos y vivos.
        let p = unsafe { *argv.offset(i) };
        if p.is_null() {
            break;
        }
        let bytes = unsafe { CStr::from_ptr(p as *const _) }.to_bytes().to_vec();
        v.push(OsString::from_vec(bytes));
    }
    Args::new(v)
}
