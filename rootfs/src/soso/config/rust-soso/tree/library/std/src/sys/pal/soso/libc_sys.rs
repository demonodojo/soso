//! Llamadas de libc que el código C enlazado (OpenSSL, libgit2) hace con
//! números de syscall de Linux, que soso no atiende.
//!
//! OpenSSL pide entropía con `getrandom` o `syscall(318, ...)`; sin ella
//! `libgit2_init` falla con «error retrieving entropy». `syscall` traduce los
//! pocos números que se ven y falla (-1) con el resto.

const SYS_LINUX_GETPID: i64 = 39;
const SYS_LINUX_GETTID: i64 = 186;
const SYS_LINUX_GETRANDOM: i64 = 318;

/// `sys/ioctl.h` de x86_64 Linux.
const TIOCGWINSZ: u64 = 0x5413;

/// `struct winsize` de Linux: cuatro `unsigned short`.
#[repr(C)]
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

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

/// `ioctl` de libc sobre la consola del proceso.
///
/// Sólo se atiende `TIOCGWINSZ`, traducido a [`soso_rt::SYS_TTY_SIZE`]
/// (columnas en los 32 bits bajos, filas en los altos). Cualquier otra
/// petición falla con -1: soso no tiene `errno`, así que el llamador no debe
/// fiarse de él.
///
/// Es débil a propósito: si el binario enlaza una libc estática (glibc de
/// las herramientas C), su `ioctl` fuerte gana. soso no tiene enlazador
/// dinámico, así que no se puede interponer sin duplicar el símbolo; las
/// herramientas C de soso usan el shim de `tests/self-improvement/native/llvm`,
/// que sí redefine `ioctl`.
#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ioctl(fd: i32, req: u64, arg: u64) -> i32 {
    if req != TIOCGWINSZ || arg == 0 || fd < 0 {
        return -1;
    }
    if unsafe { soso_rt::syscall1(soso_rt::SYS_ISATTY, fd as u64) } <= 0 {
        return -1;
    }
    let packed = unsafe { soso_rt::syscall1(soso_rt::SYS_TTY_SIZE, 0) };
    if packed < 0 {
        return -1;
    }
    let packed = packed as u64;
    let ws = crate::ptr::with_exposed_provenance_mut::<Winsize>(arg as usize);
    unsafe {
        ws.write(Winsize {
            ws_row: (packed >> 32) as u16,
            ws_col: (packed & 0xffff) as u16,
            ws_xpixel: 0,
            ws_ypixel: 0,
        });
    }
    0
}

/// Pregunta al kernel si el fd es la consola del proceso o un maestro de
/// PTY: un descriptor redirigido a fichero, pipe o socket da 0.
#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isatty(fd: i32) -> i32 {
    if fd < 0 {
        return 0;
    }
    let r = unsafe { soso_rt::syscall1(soso_rt::SYS_ISATTY, fd as u64) };
    if r > 0 { 1 } else { 0 }
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
