//! Wrappers finos sobre la instrucción `syscall`.

use core::arch::asm;
use soso_abi as abi;

/// Syscall cruda, para poder pasar punteros que un `&[u8]` no permite construir.
/// La usa la suite de regresión para comprobar que el kernel contesta EFAULT en vez
/// de tocar una dirección que no es del proceso.
pub fn raw4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
    syscall4(nr, a1, a2, a3, a4)
}

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

/// Lee hasta `len` bytes con límite de espera (`timeout_ms`; 0 = bloqueante).
pub fn read_timeout(fd: u64, buf: &mut [u8], timeout_ms: u64) -> i64 {
    syscall4(
        abi::SYS_READ_TIMEOUT,
        fd,
        buf.as_mut_ptr() as u64,
        buf.len() as u64,
        timeout_ms,
    )
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
    syscall4(abi::SYS_GPU_ALLOC, size, abi::GPU_ALLOC_GART, 0, 0)
}

/// Reserva en VRAM del dispositivo (pesos residentes G6). Si no hay GSP, cae a
/// heap del kernel como TTM fallback.
pub fn gpu_alloc_vram(size: u64) -> i64 {
    syscall4(abi::SYS_GPU_ALLOC, size, abi::GPU_ALLOC_VRAM, 0, 0)
}

pub fn gpu_map(handle: u64, ptr: u64, len: u64) -> i64 {
    syscall4(abi::SYS_GPU_MAP, handle, ptr, len, 0)
}

pub fn gpu_read(handle: u64, ptr: u64, len: u64) -> i64 {
    syscall4(abi::SYS_GPU_READ, handle, ptr, len, 0)
}

/// Libera un búfer del dispositivo. Devuelve sus bytes, que vuelven a la VRAM
/// contable: sin esto, cachear pesos en el dispositivo es una fuga garantizada.
pub fn gpu_free(handle: u64) -> i64 {
    syscall1(abi::SYS_GPU_FREE, handle)
}

pub fn gpu_submit(cmd: &[u8]) -> i64 {
    syscall4(abi::SYS_GPU_SUBMIT, cmd.as_ptr() as u64, cmd.len() as u64, 0, 0)
}

/// Crea un hilo: `entry(arg)` con pila en `stack_top` (tope, alineado).
pub fn thread_spawn(entry: u64, arg: u64, stack_top: u64) -> i64 {
    syscall4(abi::SYS_THREAD_SPAWN, entry, arg, stack_top, 0)
}

pub fn futex_wait(addr: *const u32, expected: u32) -> i64 {
    syscall4(
        abi::SYS_FUTEX,
        abi::FUTEX_WAIT,
        addr as u64,
        expected as u64,
        0,
    )
}

pub fn futex_wake(addr: *const u32, n: u64) -> i64 {
    syscall4(abi::SYS_FUTEX, abi::FUTEX_WAKE, addr as u64, 0, n)
}

pub fn ncpu() -> i64 {
    syscall1(abi::SYS_NCPU, 0)
}

pub fn uptime_ms() -> i64 {
    syscall1(abi::SYS_UPTIME_MS, 0)
}

pub fn meminfo(out: &mut abi::MemInfo) -> i64 {
    syscall4(
        abi::SYS_MEMINFO,
        out as *mut abi::MemInfo as u64,
        0,
        0,
        0,
    )
}

pub fn iostat(out: &mut abi::IoStat) -> i64 {
    syscall4(abi::SYS_IOSTAT, out as *mut abi::IoStat as u64, 0, 0, 0)
}

pub fn disk_list(out: &mut [abi::DiskInfo]) -> i64 {
    syscall4(
        abi::SYS_DISK_LIST,
        out.as_mut_ptr() as u64,
        out.len() as u64,
        0,
        0,
    )
}

pub fn disk_read(id: u32, lba: u64, buf: &mut [u8]) -> i64 {
    syscall4(
        abi::SYS_DISK_READ,
        id as u64,
        lba,
        buf.as_mut_ptr() as u64,
        buf.len() as u64,
    )
}

pub fn disk_write(id: u32, lba: u64, buf: &[u8]) -> i64 {
    syscall4(
        abi::SYS_DISK_WRITE,
        id as u64,
        lba,
        buf.as_ptr() as u64,
        buf.len() as u64,
    )
}

/// Deja la petición de entrada de arranque en `SOSOBOOT.TXT` de la ESP live,
/// para que el shim UEFI la registre en el siguiente arranque.
pub fn bootreq_write(buf: &[u8]) -> i64 {
    syscall4(
        abi::SYS_BOOTREQ_WRITE,
        buf.as_ptr() as u64,
        buf.len() as u64,
        0,
        0,
    )
}

/// Lee `SOSOBOOT.TXT`; `buf.len()` debe ser múltiplo de 512.
pub fn bootreq_read(buf: &mut [u8]) -> i64 {
    syscall4(
        abi::SYS_BOOTREQ_READ,
        buf.as_mut_ptr() as u64,
        buf.len() as u64,
        0,
        0,
    )
}

pub fn som_begin(name: &str) -> i64 {
    syscall4(
        abi::SYS_SOM_BEGIN,
        name.as_ptr() as u64,
        name.len() as u64,
        0,
        0,
    )
}

pub fn som_put(rel: &str, data: &[u8]) -> i64 {
    syscall4(
        abi::SYS_SOM_PUT,
        rel.as_ptr() as u64,
        rel.len() as u64,
        data.as_ptr() as u64,
        data.len() as u64,
    )
}

pub fn som_commit() -> i64 {
    syscall4(abi::SYS_SOM_COMMIT, 0, 0, 0, 0)
}

pub fn som_abort() -> i64 {
    syscall4(abi::SYS_SOM_ABORT, 0, 0, 0, 0)
}

pub fn som_scratch_alloc(size: u64) -> i64 {
    syscall4(abi::SYS_SOM_SCRATCH_ALLOC, size, 0, 0, 0)
}

pub fn som_scratch_write(start_lba: u64, offset: u64, data: &[u8]) -> i64 {
    syscall4(
        abi::SYS_SOM_SCRATCH_WRITE,
        start_lba,
        offset,
        data.as_ptr() as u64,
        data.len() as u64,
    )
}

pub fn som_scratch_read(start_lba: u64, offset: u64, buf: &mut [u8]) -> i64 {
    syscall4(
        abi::SYS_SOM_SCRATCH_READ,
        start_lba,
        offset,
        buf.as_mut_ptr() as u64,
        buf.len() as u64,
    )
}

pub fn som_scratch_free() -> i64 {
    syscall4(abi::SYS_SOM_SCRATCH_FREE, 0, 0, 0, 0)
}

pub fn tcp_connect(addr: &abi::SockAddr, timeout_ms: u64) -> i64 {
    syscall4(
        abi::SYS_TCP_CONNECT,
        addr as *const abi::SockAddr as u64,
        timeout_ms,
        0,
        0,
    )
}

pub fn tcp_listen(port: u16) -> i64 {
    syscall4(abi::SYS_TCP_LISTEN, port as u64, 0, 0, 0)
}

pub fn tcp_accept(listener_fd: u64, timeout_ms: u64) -> i64 {
    syscall4(abi::SYS_TCP_ACCEPT, listener_fd, timeout_ms, 0, 0)
}

/// Lee exactamente `buf.len()` bytes o falla.
pub fn read_exact(fd: u64, buf: &mut [u8]) -> Result<(), i64> {
    let mut off = 0usize;
    while off < buf.len() {
        let n = read(fd, &mut buf[off..]);
        if n < 0 {
            return Err(n);
        }
        if n == 0 {
            return Err(-abi::EIO);
        }
        off += n as usize;
    }
    Ok(())
}

/// Escribe exactamente `buf` o falla.
pub fn write_all(fd: u64, buf: &[u8]) -> Result<(), i64> {
    let mut off = 0usize;
    while off < buf.len() {
        let n = write(fd, &buf[off..]);
        if n < 0 {
            return Err(n);
        }
        if n == 0 {
            return Err(-abi::EIO);
        }
        off += n as usize;
    }
    Ok(())
}

pub fn dns_resolve(host: &str, out: &mut [u8; 4]) -> Result<(), i64> {
    let r = syscall4(
        abi::SYS_DNS_RESOLVE,
        host.as_ptr() as u64,
        host.len() as u64,
        out.as_mut_ptr() as u64,
        0,
    );
    if r < 0 {
        Err(r)
    } else {
        Ok(())
    }
}

/// IPv4 + puerto en formato de red.
pub fn sock_addr(a: u8, b: u8, c: u8, d: u8, port: u16) -> abi::SockAddr {
    abi::SockAddr {
        addr: [a, b, c, d],
        port,
        _pad: 0,
    }
}
