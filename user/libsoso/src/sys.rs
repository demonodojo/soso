//! Wrappers finos sobre la instrucción `syscall`.

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
            // El kernel usa SSE (cripto, memcpy): los registros vectoriales
            // NO sobreviven a una syscall. Sin esto, con el userspace
            // compilado con AVX2 el compilador asumiría que sí.
            clobber_abi("C"),
        );
    }
    ret
}

fn syscall1(nr: u64, a1: u64) -> i64 {
    syscall4(nr, a1, 0, 0, 0)
}

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

pub fn chdir(path: &str) -> i64 {
    syscall4(abi::SYS_CHDIR, path.as_ptr() as u64, path.len() as u64, 0, 0)
}

pub fn getcwd(buf: &mut [u8]) -> i64 {
    syscall4(
        abi::SYS_GETCWD,
        buf.as_mut_ptr() as u64,
        buf.len() as u64,
        0,
        0,
    )
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

pub fn spawn_io(path: &str, args: &str, stdin: u64, stdout: u64, stderr: u64) -> i64 {
    let opts = abi::SpawnIo {
        path_ptr: path.as_ptr() as u64,
        path_len: path.len() as u64,
        args_ptr: args.as_ptr() as u64,
        args_len: args.len() as u64,
        stdin_fd: stdin,
        stdout_fd: stdout,
        stderr_fd: stderr,
    };
    syscall4(
        abi::SYS_SPAWN_IO,
        &opts as *const abi::SpawnIo as u64,
        0,
        0,
        0,
    )
}

pub fn pipe() -> Result<(u64, u64), i64> {
    let v = syscall1(abi::SYS_PIPE, 0);
    if v < 0 {
        Err(v)
    } else {
        let read_fd = v as u64 & 0xffff_ffff;
        let write_fd = (v as u64) >> 32;
        Ok((read_fd, write_fd))
    }
}

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

pub fn halt() -> i64 {
    syscall1(abi::SYS_HALT, 0)
}

/// Mapea un fichero o región anónima. `fd == u64::MAX` para anónimo.
pub fn mmap(addr: u64, len: u64, fd: u64, offset: u64) -> i64 {
    syscall4(abi::SYS_MMAP, addr, len, fd, offset)
}

pub fn munmap(addr: u64, len: u64) -> i64 {
    syscall4(abi::SYS_MUNMAP, addr, len, 0, 0)
}

pub fn gpu_info(out: &mut abi::GpuInfo) -> i64 {
    syscall4(abi::SYS_GPU_INFO, out as *mut abi::GpuInfo as u64, 0, 0, 0)
}

pub fn gpu_alloc(size: u64) -> i64 {
    syscall1(abi::SYS_GPU_ALLOC, size)
}

pub fn gpu_map(handle: u64, ptr: u64, len: u64) -> i64 {
    syscall4(abi::SYS_GPU_MAP, handle, ptr, len, 0)
}

pub fn gpu_submit(cmd: &[u8]) -> i64 {
    syscall4(abi::SYS_GPU_SUBMIT, cmd.as_ptr() as u64, cmd.len() as u64, 0, 0)
}
