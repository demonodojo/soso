//! Backend de soso: `SYS_GETRANDOM` (78), misma convención que `soso-rt`.

use crate::Error;
use core::convert::TryFrom;
use core::mem::MaybeUninit;
use core::num::NonZeroU32;

const SYS_GETRANDOM: u64 = 78;

unsafe fn syscall3(n: u64, a1: u64, a2: u64, a3: u64) -> i64 {
    let ret: i64;
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

pub fn getrandom_inner(dest: &mut [MaybeUninit<u8>]) -> Result<(), Error> {
    if dest.is_empty() {
        return Ok(());
    }
    let addr = dest.as_mut_ptr() as usize as u64;
    let len = u64::try_from(dest.len()).map_err(|_| Error::UNEXPECTED)?;
    let n = unsafe { syscall3(SYS_GETRANDOM, addr, len, 0) };
    if n < 0 {
        let errno = u32::try_from(-n).map_err(|_| Error::UNEXPECTED)?;
        return Err(Error::from(
            NonZeroU32::new(errno).ok_or(Error::ERRNO_NOT_POSITIVE)?,
        ));
    }
    let n = usize::try_from(n).map_err(|_| Error::UNEXPECTED)?;
    if n != dest.len() {
        return Err(Error::UNEXPECTED);
    }
    Ok(())
}
