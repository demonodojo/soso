//! Llamadas de libc que el código C enlazado (OpenSSL, libgit2) hace con
//! números de syscall de Linux, que soso no atiende.
//!
//! OpenSSL pide entropía con `getrandom` o `syscall(318, ...)`; sin ella
//! `libgit2_init` falla con «error retrieving entropy». `syscall` traduce los
//! pocos números que se ven y falla (-1) con el resto.

const SYS_LINUX_GETPID: i64 = 39;
const SYS_LINUX_GETTID: i64 = 186;
const SYS_LINUX_GETRANDOM: i64 = 318;

fn aleatorio(buf: *mut u8, len: usize, flags: u32) -> isize {
    unsafe {
        soso_rt::syscall3(soso_rt::SYS_GETRANDOM, buf.expose_provenance() as u64, len as u64, flags as u64)
            as isize
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getrandom(buf: *mut u8, len: usize, flags: u32) -> isize {
    let n = aleatorio(buf, len, flags);
    if n < 0 { -1 } else { n }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getentropy(buf: *mut u8, len: usize) -> i32 {
    if len > 256 {
        return -1;
    }
    if aleatorio(buf, len, 0) == len as isize { 0 } else { -1 }
}

#[unsafe(no_mangle)]
pub extern "C" fn getpid() -> i32 {
    unsafe { soso_rt::syscall0(soso_rt::SYS_GETPID) as i32 }
}

/// `syscall(long n, ...)` con seis argumentos fijos: en SysV los variádicos
/// viajan en los mismos registros.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall(n: i64, a1: u64, a2: u64, a3: u64, _a4: u64, _a5: u64, _a6: u64) -> i64 {
    match n {
        SYS_LINUX_GETRANDOM => {
            let r = aleatorio(crate::ptr::with_exposed_provenance_mut(a1 as usize), a2 as usize, a3 as u32);
            r as i64
        }
        SYS_LINUX_GETPID | SYS_LINUX_GETTID => getpid() as i64,
        _ => -1,
    }
}
