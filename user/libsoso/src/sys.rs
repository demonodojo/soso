//! Wrappers finos sobre la instrucción `syscall`.
//!
//! El kernel preserva rsp y los callee-saved; todo lo demás se declara
//! clobber. rcx y r11 los pisa la propia instrucción.

use core::arch::asm;
use soso_abi as abi;

fn syscall4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") nr => ret,
            inlateout("rdi") a1 => _,
            inlateout("rsi") a2 => _,
            inlateout("rdx") a3 => _,
            inlateout("r10") a4 => _,
            lateout("rcx") _,
            lateout("r11") _,
            lateout("r8") _,
            lateout("r9") _,
        );
    }
    ret
}

fn syscall1(nr: u64, a1: u64) -> i64 {
    syscall4(nr, a1, 0, 0, 0)
}

// ---- las 14 ----

pub fn exit(code: u8) -> ! {
    syscall1(abi::SYS_EXIT, code as u64);
    unreachable!()
}

pub fn read(fd: u64, buf: &mut [u8]) -> i64 {
    syscall4(abi::SYS_READ, fd, buf.as_mut_ptr() as u64, buf.len() as u64, 0)
}

pub fn write(fd: u64, buf: &[u8]) -> i64 {
    syscall4(abi::SYS_WRITE, fd, buf.as_ptr() as u64, buf.len() as u64, 0)
}

pub fn open(path: &str, flags: u64) -> i64 {
    syscall4(abi::SYS_OPEN, path.as_ptr() as u64, path.len() as u64, flags, 0)
}

pub fn close(fd: u64) -> i64 {
    syscall1(abi::SYS_CLOSE, fd)
}

pub fn seek(fd: u64, off: i64, whence: u64) -> i64 {
    syscall4(abi::SYS_SEEK, fd, off as u64, whence, 0)
}

pub fn stat(path: &str, out: &mut abi::Stat) -> i64 {
    syscall4(
        abi::SYS_STAT,
        path.as_ptr() as u64,
        path.len() as u64,
        out as *mut abi::Stat as u64,
        0,
    )
}

pub fn getdents(fd: u64, buf: &mut [abi::Dirent]) -> i64 {
    syscall4(
        abi::SYS_GETDENTS,
        fd,
        buf.as_mut_ptr() as u64,
        (buf.len() * abi::DIRENT_SIZE) as u64,
        0,
    )
}

pub fn mkdir(path: &str) -> i64 {
    syscall4(abi::SYS_MKDIR, path.as_ptr() as u64, path.len() as u64, 0, 0)
}

pub fn unlink(path: &str) -> i64 {
    syscall4(abi::SYS_UNLINK, path.as_ptr() as u64, path.len() as u64, 0, 0)
}

pub fn spawn(path: &str, args: &str) -> i64 {
    syscall4(
        abi::SYS_SPAWN,
        path.as_ptr() as u64,
        path.len() as u64,
        args.as_ptr() as u64,
        args.len() as u64,
    )
}

/// Espera a cualquier hijo. Devuelve (pid, código) o el errno negativo.
pub fn wait() -> Result<(u64, u8), i64> {
    let v = syscall1(abi::SYS_WAIT, 0);
    if v < 0 { Err(v) } else { Ok(abi::wait_decode(v)) }
}

pub fn sbrk(delta: i64) -> i64 {
    syscall1(abi::SYS_SBRK, delta as u64)
}

pub fn sleep_ms(ms: u64) -> i64 {
    syscall1(abi::SYS_SLEEP_MS, ms)
}

/// Apaga la máquina. No retorna si tiene éxito.
pub fn halt() -> i64 {
    syscall1(abi::SYS_HALT, 0)
}
