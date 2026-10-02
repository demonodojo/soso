//! Backend de soso: una llamada a `SYS_GETRANDOM` (78).
//!
//! Misma convención que `soso-rt`: número en `rax`, argumentos en
//! `rdi`/`rsi`/`rdx`, retorno en `rax` (negativo = `-errno`). El kernel usa
//! SSE, así que el `asm` declara `clobber_abi("C")` y no promete banderas ni
//! pila. El kernel llena el buffer entero o falla; un corto es error.

use crate::Error;
use core::mem::MaybeUninit;

pub use crate::util::{inner_u32, inner_u64};

const SYS_GETRANDOM: u64 = 78;

unsafe fn syscall3(n: u64, a1: u64, a2: u64, a3: u64) -> i64 {
    let ret: i64;
    // El `check` de rustc niega `unsafe_op_in_unsafe_fn` (edición 2024).
    unsafe {
        core::arch::asm!(
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

/// Llena `dest` entero con `SYS_GETRANDOM`. Vacío es éxito y no llama al kernel.
#[inline]
pub fn fill_inner(dest: &mut [MaybeUninit<u8>]) -> Result<(), Error> {
    if dest.is_empty() {
        return Ok(());
    }
    let addr = u64::try_from(dest.as_mut_ptr().expose_provenance()).map_err(|_| Error::UNEXPECTED)?;
    let len = u64::try_from(dest.len()).map_err(|_| Error::UNEXPECTED)?;
    let n = unsafe { syscall3(SYS_GETRANDOM, addr, len, 0) };
    if n < 0 {
        let code = i32::try_from(n).map_err(|_| Error::UNEXPECTED)?;
        return Err(Error::from_neg_error_code(code));
    }
    let n = usize::try_from(n).map_err(|_| Error::UNEXPECTED)?;
    if n != dest.len() {
        return Err(Error::UNEXPECTED);
    }
    Ok(())
}
