//! Claves TLS de pthread sobre el TLS de soso.
//!
//! libgit2 y OpenSSL (dentro del bin `cargo`) llaman a `pthread_getspecific`.
//! La de glibc lee `%fs:0x10` (su `THREAD_SELF`), que en soso es 0: page fault
//! en 0x318. Las claves son globales y los valores, por hilo; los destructores
//! no se llaman (el proceso `cargo` no los necesita).

use crate::sync::atomic::{AtomicUsize, Ordering};

const CLAVES: usize = 128;
const EINVAL: i32 = 22;
const EAGAIN: i32 = 11;

static SIGUIENTE: AtomicUsize = AtomicUsize::new(0);

#[thread_local]
static mut VALORES: [usize; CLAVES] = [0; CLAVES];

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_key_create(
    key: *mut u32,
    _destructor: Option<unsafe extern "C" fn(*mut u8)>,
) -> i32 {
    let k = SIGUIENTE.fetch_add(1, Ordering::Relaxed);
    if k >= CLAVES {
        return EAGAIN;
    }
    unsafe { key.write(k as u32) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_key_delete(key: u32) -> i32 {
    if key as usize >= CLAVES { EINVAL } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_getspecific(key: u32) -> *mut u8 {
    if key as usize >= CLAVES {
        return crate::ptr::null_mut();
    }
    unsafe { crate::ptr::with_exposed_provenance_mut((*(&raw const VALORES))[key as usize]) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_setspecific(key: u32, valor: *const u8) -> i32 {
    if key as usize >= CLAVES {
        return EINVAL;
    }
    unsafe { (*(&raw mut VALORES))[key as usize] = valor.expose_provenance() };
    0
}
