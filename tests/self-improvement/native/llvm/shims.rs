//! Capa de libc para herramientas C++ (LLVM) enlazadas como programas de soso (T42).
//!
//! La glibc estática hace sus llamadas con **números de syscall de Linux** (`open` = 2,
//! `mmap` = 9, `rt_sigprocmask` = 14…); en soso el 2 es `write`, el 9 `unlink` y el 14
//! `halt`. Aquí se redefinen con los nombres públicos las funciones que LLVM usa, sobre
//! las syscalls de soso (`crates/soso-abi`). Lo que no se redefine y llega a un syscall
//! crudo es un riesgo: ante un fallo raro, buscar qué función de glibc se coló.
#![allow(non_camel_case_types, clippy::missing_safety_doc)]

use std::arch::asm;
use std::ffi::{c_char, c_int, c_long, c_void, CStr};

// Números de syscall de soso (`crates/soso-abi`).
const SYS_EXIT: u64 = 0;
const SYS_READ: u64 = 1;
const SYS_WRITE: u64 = 2;
const SYS_OPEN: u64 = 3;
const SYS_CLOSE: u64 = 4;
const SYS_SEEK: u64 = 5;
const SYS_STAT: u64 = 6;
const SYS_GETDENTS: u64 = 7;
const SYS_MKDIR: u64 = 8;
const SYS_UNLINK: u64 = 9;
const SYS_SLEEP_MS: u64 = 13;
const SYS_MMAP: u64 = 15;
const SYS_MUNMAP: u64 = 16;
const SYS_PIPE: u64 = 21;
const SYS_CHDIR: u64 = 23;
const SYS_GETCWD: u64 = 24;
const SYS_NCPU: u64 = 27;
const SYS_GETPID: u64 = 51;
const SYS_KILL: u64 = 52;
const SYS_RENAME: u64 = 70;
const SYS_DUP2: u64 = 73;
const SYS_FSTAT: u64 = 74;
const SYS_FSYNC: u64 = 76;
const SYS_SCHED_YIELD: u64 = 77;
const SYS_MPROTECT: u64 = 80;
const SYS_FTRUNCATE: u64 = 94;
const SYS_EXIT_GROUP: u64 = 95;

const ENOENT: i32 = 2;
const EBADF: i32 = 9;
const ENOMEM: i32 = 12;
const EINVAL: i32 = 22;
const ENOTTY: i32 = 25;
const ENOSYS: i32 = 38;
const EAGAIN: i32 = 11;

#[inline(always)]
unsafe fn sys(n: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
    let ret: i64;
    // Como `soso_rt::syscall4`: el kernel no conserva los registros de argumentos.
    unsafe {
        asm!("syscall", inlateout("rax") n as i64 => ret, inlateout("rdi") a1 => _,
             inlateout("rsi") a2 => _, inlateout("rdx") a3 => _, inlateout("r10") a4 => _,
             lateout("rcx") _, lateout("r11") _, lateout("r8") _, lateout("r9") _,
             options(nostack));
    }
    ret
}

unsafe extern "C" {
    fn __errno_location() -> *mut c_int;
}

/// `ret < 0` es `-errno`: se fija `errno` y se devuelve -1.
unsafe fn r(ret: i64) -> i64 {
    if ret < 0 {
        unsafe { *__errno_location() = (-ret) as c_int };
        -1
    } else {
        ret
    }
}

unsafe fn fallo(e: i32) -> i64 {
    unsafe { *__errno_location() = e };
    -1
}

unsafe fn ruta<'a>(p: *const c_char) -> &'a [u8] {
    if p.is_null() { &[] } else { unsafe { CStr::from_ptr(p).to_bytes() } }
}

// ---- E/S de ficheros ----

const LO_WRONLY: c_int = 1;
const LO_RDWR: c_int = 2;
const LO_CREAT: c_int = 0x40;
const LO_EXCL: c_int = 0x80;
const LO_TRUNC: c_int = 0x200;
const LO_APPEND: c_int = 0x400;

unsafe fn abrir(path: *const c_char, flags: c_int) -> i64 {
    let escribe = flags & (LO_WRONLY | LO_RDWR) != 0 || flags & (LO_CREAT | LO_TRUNC | LO_APPEND) != 0;
    // O_RDWR sobre un fichero existente sin truncar se abre sólo para lectura: soso no
    // tiene lectura+escritura sin truncar (LLVM sólo lo usa para mapear en memoria).
    let escribe = escribe && !(flags & LO_RDWR != 0 && flags & (LO_CREAT | LO_TRUNC | LO_APPEND) == 0);
    let mut f = 0u64;
    if escribe {
        f |= 1;
        if flags & LO_APPEND != 0 { f |= 2; }
        if flags & LO_CREAT != 0 { f |= 4; }
        if flags & LO_TRUNC != 0 { f |= 8; }
        if flags & LO_EXCL != 0 { f |= 16; }
    }
    let p = unsafe { ruta(path) };
    unsafe { r(sys(SYS_OPEN, p.as_ptr() as u64, p.len() as u64, f, 0)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, _mode: c_int) -> c_int {
    unsafe { abrir(path, flags) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn open64(path: *const c_char, flags: c_int, mode: c_int) -> c_int {
    unsafe { open(path, flags, mode) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openat(_dirfd: c_int, path: *const c_char, flags: c_int, mode: c_int) -> c_int {
    unsafe { open(path, flags, mode) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openat64(d: c_int, path: *const c_char, flags: c_int, mode: c_int) -> c_int {
    unsafe { openat(d, path, flags, mode) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn creat(path: *const c_char, mode: c_int) -> c_int {
    unsafe { open(path, LO_WRONLY | LO_CREAT | LO_TRUNC, mode) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn close(fd: c_int) -> c_int {
    unsafe { r(sys(SYS_CLOSE, fd as u64, 0, 0, 0)) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize {
    unsafe { r(sys(SYS_READ, fd as u64, buf as u64, n as u64, 0)) as isize }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lseek(fd: c_int, off: i64, whence: c_int) -> i64 {
    unsafe { r(sys(SYS_SEEK, fd as u64, off as u64, whence as u64, 0)) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lseek64(fd: c_int, off: i64, whence: c_int) -> i64 {
    unsafe { lseek(fd, off, whence) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pread(fd: c_int, buf: *mut c_void, n: usize, off: i64) -> isize {
    unsafe {
        let cur = lseek(fd, 0, 1);
        if cur < 0 || lseek(fd, off, 0) < 0 { return -1; }
        let k = read(fd, buf, n);
        lseek(fd, cur, 0);
        k
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pread64(fd: c_int, buf: *mut c_void, n: usize, off: i64) -> isize {
    unsafe { pread(fd, buf, n, off) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pwrite(fd: c_int, buf: *const c_void, n: usize, off: i64) -> isize {
    unsafe {
        let cur = lseek(fd, 0, 1);
        if cur < 0 || lseek(fd, off, 0) < 0 { return -1; }
        let k = r(sys(SYS_WRITE, fd as u64, buf as u64, n as u64, 0)) as isize;
        lseek(fd, cur, 0);
        k
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pwrite64(fd: c_int, buf: *const c_void, n: usize, off: i64) -> isize {
    unsafe { pwrite(fd, buf, n, off) }
}

#[repr(C)]
pub struct iovec {
    base: *mut c_void,
    len: usize,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn writev(fd: c_int, v: *const iovec, n: c_int) -> isize {
    let mut total = 0isize;
    for i in 0..n as usize {
        let e = unsafe { &*v.add(i) };
        if e.len == 0 { continue; }
        let k = unsafe { r(sys(SYS_WRITE, fd as u64, e.base as u64, e.len as u64, 0)) } as isize;
        if k < 0 { return if total > 0 { total } else { -1 }; }
        total += k;
        if (k as usize) < e.len { break; }
    }
    total
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn readv(fd: c_int, v: *const iovec, n: c_int) -> isize {
    let mut total = 0isize;
    for i in 0..n as usize {
        let e = unsafe { &*v.add(i) };
        if e.len == 0 { continue; }
        let k = unsafe { read(fd, e.base, e.len) };
        if k < 0 { return if total > 0 { total } else { -1 }; }
        total += k;
        if (k as usize) < e.len { break; }
    }
    total
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fsync(fd: c_int) -> c_int {
    unsafe { r(sys(SYS_FSYNC, fd as u64, 0, 0, 0)) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fdatasync(fd: c_int) -> c_int {
    unsafe { fsync(fd) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ftruncate(fd: c_int, len: i64) -> c_int {
    unsafe { r(sys(SYS_FTRUNCATE, fd as u64, len as u64, 0, 0)) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ftruncate64(fd: c_int, len: i64) -> c_int {
    unsafe { ftruncate(fd, len) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dup2(a: c_int, b: c_int) -> c_int {
    unsafe { r(sys(SYS_DUP2, a as u64, b as u64, 0, 0)) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dup(_a: c_int) -> c_int {
    unsafe { fallo(ENOSYS) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pipe(fds: *mut c_int) -> c_int {
    let v = unsafe { r(sys(SYS_PIPE, 0, 0, 0, 0)) };
    if v < 0 { return -1; }
    unsafe {
        *fds = (v as u64 & 0xffff_ffff) as c_int;
        *fds.add(1) = ((v as u64) >> 32) as c_int;
    }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pipe2(fds: *mut c_int, _flags: c_int) -> c_int {
    unsafe { pipe(fds) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fcntl(_fd: c_int, cmd: c_int, _arg: c_long) -> c_int {
    // F_GETFD/F_SETFD/F_GETFL/F_SETFL: sin efecto. Lo demás (F_DUPFD…) no existe.
    match cmd {
        1 | 2 | 3 | 4 => 0,
        _ => unsafe { fallo(EINVAL) as c_int },
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ioctl(_fd: c_int, _req: u64, _arg: u64) -> c_int {
    unsafe { fallo(ENOTTY) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isatty(_fd: c_int) -> c_int {
    unsafe { fallo(ENOTTY) as c_int; }
    0
}

// ---- nombres, metadatos ----

/// `struct stat` de x86-64 Linux (144 bytes).
#[repr(C)]
pub struct lstat_t {
    st_dev: u64,
    st_ino: u64,
    st_nlink: u64,
    st_mode: u32,
    st_uid: u32,
    st_gid: u32,
    _pad0: i32,
    st_rdev: u64,
    st_size: i64,
    st_blksize: i64,
    st_blocks: i64,
    st_atime: i64,
    st_atime_ns: i64,
    st_mtime: i64,
    st_mtime_ns: i64,
    st_ctime: i64,
    st_ctime_ns: i64,
    _unused: [i64; 3],
}

#[repr(C)]
#[derive(Default)]
struct SosoStat {
    ino: u64,
    size: u64,
    mtime: u64,
    file_type: u8,
    _pad: [u8; 7],
}

fn traducir(s: &SosoStat, out: &mut lstat_t) {
    out.st_dev = 1;
    out.st_ino = s.ino;
    out.st_nlink = 1;
    // FT_FILE = 1, FT_DIR = 2.
    out.st_mode = if s.file_type == 2 { 0o040755 } else { 0o100644 };
    out.st_size = s.size as i64;
    out.st_blksize = 4096;
    out.st_blocks = ((s.size + 511) / 512) as i64;
    out.st_atime = s.mtime as i64;
    out.st_mtime = s.mtime as i64;
    out.st_ctime = s.mtime as i64;
}

unsafe fn estado(path: *const c_char, out: *mut lstat_t) -> c_int {
    let p = unsafe { ruta(path) };
    let mut s = SosoStat::default();
    let rc = unsafe { r(sys(SYS_STAT, p.as_ptr() as u64, p.len() as u64, (&raw mut s) as u64, 0)) };
    if rc < 0 { return -1; }
    unsafe { traducir(&s, &mut *out) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stat(path: *const c_char, out: *mut lstat_t) -> c_int { unsafe { estado(path, out) } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stat64(path: *const c_char, out: *mut lstat_t) -> c_int { unsafe { estado(path, out) } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lstat(path: *const c_char, out: *mut lstat_t) -> c_int { unsafe { estado(path, out) } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lstat64(path: *const c_char, out: *mut lstat_t) -> c_int { unsafe { estado(path, out) } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstatat(_d: c_int, path: *const c_char, out: *mut lstat_t, _f: c_int) -> c_int {
    unsafe { estado(path, out) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstatat64(d: c_int, path: *const c_char, out: *mut lstat_t, f: c_int) -> c_int {
    unsafe { fstatat(d, path, out, f) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn newfstatat(d: c_int, path: *const c_char, out: *mut lstat_t, f: c_int) -> c_int {
    unsafe { fstatat(d, path, out, f) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstat(fd: c_int, out: *mut lstat_t) -> c_int {
    let mut s = SosoStat::default();
    let rc = unsafe { sys(SYS_FSTAT, fd as u64, (&raw mut s) as u64, 0, 0) };
    // stdin/stdout/stderr (consola, tubo) son «dispositivos de caracteres»: así los ve LLVM
    // (`FixupStandardFileDescriptors` aborta el driver si `fstat(0..2)` falla).
    if fd <= 2 {
        unsafe {
            std::ptr::write_bytes(out as *mut u8, 0, std::mem::size_of::<lstat_t>());
            (*out).st_mode = 0o020620;
            (*out).st_blksize = 4096;
        }
        return 0;
    }
    if rc < 0 { return unsafe { r(rc) as c_int }; }
    unsafe { traducir(&s, &mut *out) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstat64(fd: c_int, out: *mut lstat_t) -> c_int { unsafe { fstat(fd, out) } }

#[unsafe(no_mangle)]
pub unsafe extern "C" fn access(path: *const c_char, _mode: c_int) -> c_int {
    let mut st = std::mem::MaybeUninit::<lstat_t>::zeroed();
    unsafe { estado(path, st.as_mut_ptr()) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn faccessat(_d: c_int, path: *const c_char, mode: c_int, _f: c_int) -> c_int {
    unsafe { access(path, mode) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unlink(path: *const c_char) -> c_int {
    let p = unsafe { ruta(path) };
    unsafe { r(sys(SYS_UNLINK, p.as_ptr() as u64, p.len() as u64, 0, 0)) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rmdir(path: *const c_char) -> c_int { unsafe { unlink(path) } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mkdir(path: *const c_char, _mode: u32) -> c_int {
    let p = unsafe { ruta(path) };
    unsafe { r(sys(SYS_MKDIR, p.as_ptr() as u64, p.len() as u64, 0, 0)) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rename(a: *const c_char, b: *const c_char) -> c_int {
    let (pa, pb) = unsafe { (ruta(a), ruta(b)) };
    unsafe { r(sys(SYS_RENAME, pa.as_ptr() as u64, pa.len() as u64, pb.as_ptr() as u64, pb.len() as u64)) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn chdir(path: *const c_char) -> c_int {
    let p = unsafe { ruta(path) };
    unsafe { r(sys(SYS_CHDIR, p.as_ptr() as u64, p.len() as u64, 0, 0)) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getcwd(buf: *mut c_char, size: usize) -> *mut c_char {
    let n = unsafe { r(sys(SYS_GETCWD, buf as u64, size as u64, 0, 0)) };
    if n < 0 { return std::ptr::null_mut(); }
    // Termina en NUL si cabe.
    if (n as usize) < size { unsafe { *buf.add(n as usize) = 0 }; }
    buf
}
/// Camino absoluto del ejecutable (lo fija el lanzador). `/proc/self/exe` no existe en soso y
/// LLVM lo usa para hallar su directorio (`<bin>/../lib/clang/N` = cabeceras del compilador).
pub static EXE: Mutex<Vec<u8>> = Mutex::new(Vec::new());

#[unsafe(no_mangle)]
pub unsafe extern "C" fn readlink(p: *const c_char, b: *mut c_char, n: usize) -> isize {
    if unsafe { ruta(p) } == b"/proc/self/exe" {
        let exe = EXE.lock().unwrap();
        if !exe.is_empty() {
            let k = exe.len().min(n);
            unsafe { std::ptr::copy_nonoverlapping(exe.as_ptr(), b as *mut u8, k) };
            return k as isize;
        }
    }
    unsafe { fallo(EINVAL) as isize }
}
unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
}

/// `realpath` de glibc recorre el camino con `readlink` crudo: syscall 89 de Linux, que en
/// soso es `ping` (bloquea esperando un eco ICMP: ~8 s por componente, 21 minutos para
/// `immintrin.h`). Aquí no hay enlaces simbólicos: se normaliza el camino y se comprueba
/// que exista.
unsafe fn camino_real(path: *const c_char, out: *mut c_char, cap: usize) -> *mut c_char {
    if path.is_null() {
        unsafe { *__errno_location() = EINVAL };
        return std::ptr::null_mut();
    }
    let p = unsafe { ruta(path) };
    let mut abs: Vec<u8> = Vec::new();
    if p.first() != Some(&b'/') {
        let mut cwd = vec![0u8; 4096];
        let n = unsafe { sys(SYS_GETCWD, cwd.as_mut_ptr() as u64, cwd.len() as u64, 0, 0) };
        if n < 0 {
            unsafe { *__errno_location() = (-n) as c_int };
            return std::ptr::null_mut();
        }
        cwd.truncate(n as usize);
        abs.extend_from_slice(&cwd);
        abs.push(b'/');
    }
    abs.extend_from_slice(p);
    let mut partes: Vec<&[u8]> = Vec::new();
    for c in abs.split(|&b| b == b'/') {
        match c {
            b"" | b"." => {}
            b".." => {
                partes.pop();
            }
            _ => partes.push(c),
        }
    }
    let mut res: Vec<u8> = Vec::new();
    for c in &partes {
        res.push(b'/');
        res.extend_from_slice(c);
    }
    if res.is_empty() {
        res.push(b'/');
    }
    res.push(0);
    // Existe?
    let mut st = std::mem::MaybeUninit::<lstat_t>::zeroed();
    if unsafe { estado(res.as_ptr().cast(), st.as_mut_ptr()) } < 0 {
        return std::ptr::null_mut();
    }
    let destino = if out.is_null() {
        let b = unsafe { malloc(res.len()) } as *mut c_char;
        if b.is_null() {
            unsafe { *__errno_location() = ENOMEM };
            return b;
        }
        b
    } else {
        if cap != 0 && res.len() > cap {
            unsafe { *__errno_location() = 36 };
            return std::ptr::null_mut();
        }
        out
    };
    unsafe { std::ptr::copy_nonoverlapping(res.as_ptr(), destino as *mut u8, res.len()) };
    destino
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn realpath(path: *const c_char, out: *mut c_char) -> *mut c_char {
    unsafe { camino_real(path, out, 0) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __realpath_chk(path: *const c_char, out: *mut c_char, cap: usize) -> *mut c_char {
    unsafe { camino_real(path, out, cap) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn canonicalize_file_name(path: *const c_char) -> *mut c_char {
    unsafe { camino_real(path, std::ptr::null_mut(), 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn chmod(_p: *const c_char, _m: u32) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fchmod(_fd: c_int, _m: u32) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn utimes(_p: *const c_char, _t: *const c_void) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn futimens(_fd: c_int, _t: *const c_void) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn utimensat(_d: c_int, _p: *const c_char, _t: *const c_void, _f: c_int) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn umask(_m: u32) -> u32 { 0o022 }

// ---- directorios ----

/// `struct dirent` de Linux x86-64.
#[repr(C)]
pub struct dirent {
    d_ino: u64,
    d_off: i64,
    d_reclen: u16,
    d_type: u8,
    d_name: [c_char; 256],
}
pub struct DIR {
    fd: c_int,
    cola: Vec<(u64, u8, Vec<u8>)>,
    pos: usize,
    leido: bool,
    actual: dirent,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opendir(path: *const c_char) -> *mut DIR {
    let fd = unsafe { abrir(path, 0) };
    if fd < 0 { return std::ptr::null_mut(); }
    Box::into_raw(Box::new(DIR {
        fd: fd as c_int,
        cola: Vec::new(),
        pos: 0,
        leido: false,
        actual: unsafe { std::mem::zeroed() },
    }))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn readdir(d: *mut DIR) -> *mut dirent {
    let d = unsafe { &mut *d };
    if !d.leido {
        d.leido = true;
        // `Dirent` de soso: ino u64, file_type u8, name_len u8, name[NAME_MAX = 255].
        const SZ: usize = 8 + 1 + 1 + 255;
        let mut buf = vec![0u8; SZ * 64];
        loop {
            let n = unsafe { r(sys(SYS_GETDENTS, d.fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64, 0)) };
            if n <= 0 { break; }
            for i in 0..n as usize / SZ {
                let e = &buf[i * SZ..(i + 1) * SZ];
                let ino = u64::from_le_bytes(e[0..8].try_into().unwrap());
                let nlen = e[9] as usize;
                d.cola.push((ino, e[8], e[10..10 + nlen].to_vec()));
            }
        }
    }
    if d.pos >= d.cola.len() { return std::ptr::null_mut(); }
    let (ino, ft, nombre) = &d.cola[d.pos];
    d.pos += 1;
    d.actual.d_ino = *ino;
    d.actual.d_off = d.pos as i64;
    d.actual.d_reclen = std::mem::size_of::<dirent>() as u16;
    d.actual.d_type = if *ft == 2 { 4 } else { 8 };
    d.actual.d_name = [0; 256];
    for (i, b) in nombre.iter().take(255).enumerate() { d.actual.d_name[i] = *b as c_char; }
    &raw mut d.actual
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn readdir64(d: *mut DIR) -> *mut dirent { unsafe { readdir(d) } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn closedir(d: *mut DIR) -> c_int {
    let b = unsafe { Box::from_raw(d) };
    unsafe { sys(SYS_CLOSE, b.fd as u64, 0, 0, 0) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dirfd(d: *mut DIR) -> c_int { unsafe { (*d).fd } }

// ---- memoria ----

const MAP_ANON: c_int = 0x20;
const MAP_FIXED: c_int = 0x10;
const PROT_WRITE: c_int = 2;
const MAP_FAILED: *mut c_void = !0usize as *mut c_void;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: i64) -> *mut c_void {
    if flags & MAP_FIXED != 0 {
        unsafe { *__errno_location() = ENOSYS };
        return MAP_FAILED;
    }
    let v = if flags & MAP_ANON != 0 || fd < 0 {
        unsafe { sys(SYS_MMAP, 0, len as u64, u64::MAX, 0) }
    } else {
        // Un fichero mapeado con escritura (`FileOutputBuffer`) no existe en soso: que LLVM
        // use su búfer en memoria.
        if prot & PROT_WRITE != 0 && flags & 1 == 0 {
            unsafe { *__errno_location() = ENOSYS };
            return MAP_FAILED;
        }
        unsafe { sys(SYS_MMAP, 0, len as u64, fd as u64, off as u64) }
    };
    let _ = addr;
    if v < 0 {
        unsafe { *__errno_location() = (-v) as c_int };
        MAP_FAILED
    } else {
        v as *mut c_void
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mmap64(a: *mut c_void, l: usize, p: c_int, f: c_int, fd: c_int, o: i64) -> *mut c_void {
    unsafe { mmap(a, l, p, f, fd, o) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn munmap(addr: *mut c_void, len: usize) -> c_int {
    unsafe { r(sys(SYS_MUNMAP, addr as u64, len as u64, 0, 0)) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int {
    unsafe { r(sys(SYS_MPROTECT, addr as u64, len as u64, prot as u64, 0)) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn madvise(_a: *mut c_void, _l: usize, _adv: c_int) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn msync(_a: *mut c_void, _l: usize, _f: c_int) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getpagesize() -> c_int { 4096 }

// ---- otros ----

/// `getauxval` lee `_dl_auxv`, que `__libc_start_main` rellena y aquí es NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getauxval(tipo: u64) -> u64 {
    match tipo {
        6 => 4096,  // AT_PAGESZ
        17 => 100,  // AT_CLKTCK
        _ => {
            unsafe { *__errno_location() = ENOENT };
            0
        }
    }
}

const SYS_GETRANDOM: u64 = 78;

/// `arc4random` de glibc 2.39 pide entropía con un syscall de Linux y, si falla, `__libc_fatal`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn arc4random_buf(buf: *mut u8, n: usize) {
    unsafe { sys(SYS_GETRANDOM, buf as u64, n as u64, 0, 0) };
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn arc4random() -> u32 {
    let mut v = 0u32;
    unsafe { arc4random_buf((&raw mut v).cast(), 4) };
    v
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn arc4random_uniform(limite: u32) -> u32 {
    if limite < 2 { return 0; }
    let min = limite.wrapping_neg() % limite;
    loop {
        let r = unsafe { arc4random() };
        if r >= min { return r % limite; }
    }
}

// ---- señales, procesos, límites ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigaction(_s: c_int, _a: *const c_void, _o: *mut c_void) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn signal(_s: c_int, _h: usize) -> usize { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigprocmask(_h: c_int, _s: *const c_void, _o: *mut c_void) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_sigmask(_h: c_int, _s: *const c_void, _o: *mut c_void) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigaltstack(_s: *const c_void, _o: *mut c_void) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn raise(sig: c_int) -> c_int {
    unsafe { sys(SYS_EXIT_GROUP, (128 + sig) as u64, 0, 0, 0) as c_int }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kill(pid: c_int, sig: c_int) -> c_int {
    unsafe { r(sys(SYS_KILL, pid as i64 as u64, sig as u64, 0, 0)) as c_int }
}
// `abort`: dice quién la llamó (glibc la usa para aserciones y errores fatales sin más
// mensaje). El rastro son las palabras de la pila que parecen direcciones del texto.
std::arch::global_asm!(
    ".globl abort",
    "abort:",
    "mov rdi, rsp",
    "and rsp, -16",
    "call {rust}",
    "ud2",
    rust = sym abort_rust,
);

unsafe extern "C" fn abort_rust(rsp: *const u64) -> ! {
    let mut m = String::from("abort() llamado; rastro:");
    for i in 0..160 {
        let v = unsafe { *rsp.add(i) };
        if (0x400000..0x2000000).contains(&v) {
            m.push_str(&format!(" {v:#x}"));
        }
    }
    m.push('\n');
    unsafe { sys(SYS_WRITE, 2, m.as_ptr() as u64, m.len() as u64, 0) };
    unsafe { sys(SYS_EXIT_GROUP, 134, 0, 0, 0) };
    loop {}
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _exit(code: c_int) -> ! {
    unsafe { sys(SYS_EXIT_GROUP, code as u64, 0, 0, 0) };
    loop {}
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Exit(code: c_int) -> ! { unsafe { _exit(code) } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getuid() -> u32 { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geteuid() -> u32 { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getgid() -> u32 { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getegid() -> u32 { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getppid() -> c_int { 1 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sched_yield() -> c_int {
    unsafe { sys(SYS_SCHED_YIELD, 0, 0, 0, 0) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sched_getaffinity(_p: c_int, size: usize, mask: *mut u8) -> c_int {
    let n = unsafe { sys(SYS_NCPU, 0, 0, 0, 0) }.clamp(1, 64) as u32;
    unsafe {
        std::ptr::write_bytes(mask, 0, size);
        if size >= 8 { *(mask as *mut u64) = if n >= 64 { u64::MAX } else { (1u64 << n) - 1 }; }
    }
    0
}
#[repr(C)]
pub struct timespec {
    sec: i64,
    nsec: i64,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nanosleep(req: *const timespec, _rem: *mut timespec) -> c_int {
    let t = unsafe { &*req };
    let ms = (t.sec * 1000 + t.nsec / 1_000_000).max(0) as u64;
    unsafe { sys(SYS_SLEEP_MS, ms, 0, 0, 0) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn usleep(us: u32) -> c_int {
    unsafe { sys(SYS_SLEEP_MS, (us / 1000) as u64, 0, 0, 0) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sleep(s: u32) -> u32 {
    unsafe { sys(SYS_SLEEP_MS, s as u64 * 1000, 0, 0, 0) };
    0
}
#[repr(C)]
pub struct rlimit {
    cur: u64,
    max: u64,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getrlimit(_res: c_int, out: *mut rlimit) -> c_int {
    unsafe { *out = rlimit { cur: !0, max: !0 } };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setrlimit(_res: c_int, _l: *const rlimit) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getrusage(_who: c_int, out: *mut u8) -> c_int {
    unsafe { std::ptr::write_bytes(out, 0, 144) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uname(out: *mut u8) -> c_int {
    unsafe { std::ptr::write_bytes(out, 0, 65 * 6) };
    for (i, s) in ["soso", "soso", "0.1", "0.1", "x86_64"].iter().enumerate() {
        unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), out.add(i * 65), s.len()) };
    }
    0
}
// ---- hilos POSIX ----
//
// `lld` crea su pool de hilos aunque se le pida `--threads=1` (`ThreadPoolExecutor` lanza siempre
// un hilo de arranque) y libstdc++ usa `pthread_*` de glibc, que sincroniza con `futex` de Linux
// (syscall 202: en soso no existe). Se redefine lo mínimo sobre `std::thread`: los hilos son hilos
// de soso (`SYS_THREAD_SPAWN` por el PAL) y las esperas son **de sondeo** (`sched_yield`), más
// simples que un futex y suficientes: es un solo núcleo y pocas colisiones. Un `pthread_mutex_t` /
// `pthread_cond_t` / `pthread_rwlock_t` de glibc se deja en cero (inicializador estático) y se
// usan sus primeras palabras como estado; `kind` está en el desplazamiento 16, como en glibc.

use std::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::thread::JoinHandle;

unsafe extern "C" {
    static mut __libc_single_threaded: i8;
}

static HILOS: Mutex<Option<HashMap<u64, JoinHandle<()>>>> = Mutex::new(None);
static SIG_HILO: AtomicU64 = AtomicU64::new(1);

struct Enviar(usize, usize);
unsafe impl Send for Enviar {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_create(t: *mut u64, _a: *const c_void, f: usize, arg: *mut c_void) -> c_int {
    // A partir de aquí glibc y libstdc++ han de usar sus caminos multihilo.
    unsafe { __libc_single_threaded = 0 };
    let datos = Enviar(f, arg as usize);
    let h = std::thread::Builder::new().stack_size(8 << 20).spawn(move || {
        let d = datos;
        let f: extern "C" fn(*mut c_void) -> *mut c_void = unsafe { std::mem::transmute(d.0) };
        f(d.1 as *mut c_void);
    });
    match h {
        Ok(h) => {
            let id = SIG_HILO.fetch_add(1, Ordering::Relaxed);
            HILOS.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, h);
            unsafe { *t = id };
            0
        }
        Err(_) => EAGAIN,
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_join(t: u64, ret: *mut *mut c_void) -> c_int {
    let h = HILOS.lock().unwrap().get_or_insert_with(HashMap::new).remove(&t);
    if !ret.is_null() { unsafe { *ret = std::ptr::null_mut() } };
    match h {
        Some(h) => { let _ = h.join(); 0 }
        None => 3, // ESRCH
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_detach(t: u64) -> c_int {
    HILOS.lock().unwrap().get_or_insert_with(HashMap::new).remove(&t);
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_equal(a: u64, b: u64) -> c_int { (a == b) as c_int }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_setname_np(_t: u64, _n: *const c_char) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_getname_np(_t: u64, n: *mut c_char, l: usize) -> c_int {
    if l > 0 { unsafe { *n = 0 } };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_attr_init(_a: *mut c_void) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_attr_destroy(_a: *mut c_void) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_attr_setstacksize(_a: *mut c_void, _s: usize) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_attr_setdetachstate(_a: *mut c_void, _s: c_int) -> c_int { 0 }

/// Identidad del hilo: su TCB (`%fs:0x10`), como `pthread_self` de glibc.
fn yo() -> u32 {
    let v: usize;
    unsafe { std::arch::asm!("mov {}, fs:0x10", out(reg) v, options(nostack, readonly)) };
    (v >> 4) as u32 | 1
}
fn ceder() {
    unsafe { sys(SYS_SCHED_YIELD, 0, 0, 0, 0) };
}

// `pthread_mutex_t`: [0..4) dueño (0 = libre), [4..8) recursión, [16..20) tipo (1 = recursivo).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_init(m: *mut u8, attr: *const u8) -> c_int {
    unsafe {
        std::ptr::write_bytes(m, 0, 40);
        if !attr.is_null() { *(m.add(16) as *mut i32) = *(attr as *const i32); }
    }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_destroy(_m: *mut u8) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_trylock(m: *mut u8) -> c_int {
    let dueño = unsafe { &*(m as *const AtomicU32) };
    let rec = unsafe { &mut *(m.add(4) as *mut u32) };
    let tipo = unsafe { *(m.add(16) as *const i32) } & 3;
    let yo = yo();
    if tipo == 1 && dueño.load(Ordering::Acquire) == yo {
        *rec += 1;
        return 0;
    }
    if dueño.compare_exchange(0, yo, Ordering::Acquire, Ordering::Relaxed).is_ok() {
        *rec = 1;
        0
    } else {
        16 // EBUSY
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_lock(m: *mut u8) -> c_int {
    while unsafe { pthread_mutex_trylock(m) } != 0 { ceder(); }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_unlock(m: *mut u8) -> c_int {
    let dueño = unsafe { &*(m as *const AtomicU32) };
    let rec = unsafe { &mut *(m.add(4) as *mut u32) };
    if *rec > 1 { *rec -= 1; return 0; }
    *rec = 0;
    dueño.store(0, Ordering::Release);
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_init(a: *mut i32) -> c_int { unsafe { *a = 0 }; 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_destroy(_a: *mut i32) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_settype(a: *mut i32, t: c_int) -> c_int { unsafe { *a = t }; 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_gettype(a: *const i32, t: *mut c_int) -> c_int { unsafe { *t = *a }; 0 }

// `pthread_cond_t`: [0..8) contador de señales.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_init(c: *mut u8, _a: *const u8) -> c_int {
    unsafe { std::ptr::write_bytes(c, 0, 48) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_destroy(_c: *mut u8) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_signal(c: *mut u8) -> c_int {
    unsafe { &*(c as *const AtomicU64) }.fetch_add(1, Ordering::Release);
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_broadcast(c: *mut u8) -> c_int { unsafe { pthread_cond_signal(c) } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_wait(c: *mut u8, m: *mut u8) -> c_int {
    let seq = unsafe { &*(c as *const AtomicU64) };
    let visto = seq.load(Ordering::Acquire);
    unsafe { pthread_mutex_unlock(m) };
    while seq.load(Ordering::Acquire) == visto { ceder(); }
    unsafe { pthread_mutex_lock(m) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_timedwait(c: *mut u8, m: *mut u8, abs: *const timespec) -> c_int {
    let seq = unsafe { &*(c as *const AtomicU64) };
    let visto = seq.load(Ordering::Acquire);
    // El plazo es absoluto (CLOCK_REALTIME por defecto): se compara con el reloj de pared.
    let limite = unsafe { ((*abs).sec as i128) * 1_000_000_000 + (*abs).nsec as i128 };
    unsafe { pthread_mutex_unlock(m) };
    let mut vencio = false;
    while seq.load(Ordering::Acquire) == visto {
        let ahora = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as i128);
        if ahora >= limite { vencio = true; break; }
        ceder();
    }
    unsafe { pthread_mutex_lock(m) };
    if vencio { 110 } else { 0 } // ETIMEDOUT
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_condattr_init(_a: *mut u8) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_condattr_destroy(_a: *mut u8) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_condattr_setclock(_a: *mut u8, _c: c_int) -> c_int { 0 }

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_once(o: *mut AtomicI32, f: extern "C" fn()) -> c_int {
    let o = unsafe { &*o };
    loop {
        match o.compare_exchange(0, 1, Ordering::Acquire, Ordering::Acquire) {
            Ok(_) => { f(); o.store(2, Ordering::Release); return 0; }
            Err(2) => return 0,
            Err(_) => ceder(),
        }
    }
}

// `pthread_rwlock_t`: [0..4) lectores (>0) o -1 (escritor).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_init(l: *mut u8, _a: *const u8) -> c_int {
    unsafe { std::ptr::write_bytes(l, 0, 56) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_destroy(_l: *mut u8) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_tryrdlock(l: *mut u8) -> c_int {
    let a = unsafe { &*(l as *const AtomicI32) };
    let v = a.load(Ordering::Acquire);
    if v >= 0 && a.compare_exchange(v, v + 1, Ordering::Acquire, Ordering::Relaxed).is_ok() { 0 } else { 16 }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_rdlock(l: *mut u8) -> c_int {
    while unsafe { pthread_rwlock_tryrdlock(l) } != 0 { ceder(); }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_trywrlock(l: *mut u8) -> c_int {
    let a = unsafe { &*(l as *const AtomicI32) };
    if a.compare_exchange(0, -1, Ordering::Acquire, Ordering::Relaxed).is_ok() { 0 } else { 16 }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_wrlock(l: *mut u8) -> c_int {
    while unsafe { pthread_rwlock_trywrlock(l) } != 0 { ceder(); }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_unlock(l: *mut u8) -> c_int {
    let a = unsafe { &*(l as *const AtomicI32) };
    if a.load(Ordering::Acquire) < 0 { a.store(0, Ordering::Release) } else { a.fetch_sub(1, Ordering::Release); }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fork() -> c_int { unsafe { fallo(ENOSYS) as c_int } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vfork() -> c_int { unsafe { fallo(ENOSYS) as c_int } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execv(_p: *const c_char, _a: *const *const c_char) -> c_int { unsafe { fallo(ENOSYS) as c_int } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execve(_p: *const c_char, _a: *const *const c_char, _e: *const *const c_char) -> c_int {
    unsafe { fallo(ENOSYS) as c_int }
}
// ---- procesos hijo ----
//
// `llvm::sys::ExecuteAndWait` (el driver de clang lanza `clang -cc1as` para ensamblar) usa
// `posix_spawn` + `waitpid`. Se sirven con `std::process::Command`, que en soso ya hace
// `SYS_SPAWN_IO`; los hijos se guardan por pid para `waitpid`/`wait4`.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::soso::ffi::OsStrExt;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

static HIJOS: Mutex<Option<HashMap<i32, Child>>> = Mutex::new(None);

enum Accion {
    Abrir(c_int, Vec<u8>, c_int),
    Dup2(c_int, c_int),
    Cerrar(c_int),
}
type Acciones = Vec<Accion>;

unsafe fn acciones<'a>(fa: *mut c_void) -> &'a mut Acciones {
    unsafe { &mut **(fa as *mut *mut Acciones) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawn_file_actions_init(fa: *mut c_void) -> c_int {
    unsafe { *(fa as *mut *mut Acciones) = Box::into_raw(Box::new(Vec::new())) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawn_file_actions_destroy(fa: *mut c_void) -> c_int {
    let p = unsafe { *(fa as *mut *mut Acciones) };
    if !p.is_null() {
        drop(unsafe { Box::from_raw(p) });
        unsafe { *(fa as *mut *mut Acciones) = std::ptr::null_mut() };
    }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawn_file_actions_addopen(fa: *mut c_void, fd: c_int, path: *const c_char, flags: c_int, _mode: u32) -> c_int {
    unsafe { acciones(fa).push(Accion::Abrir(fd, ruta(path).to_vec(), flags)) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawn_file_actions_adddup2(fa: *mut c_void, a: c_int, b: c_int) -> c_int {
    unsafe { acciones(fa).push(Accion::Dup2(a, b)) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawn_file_actions_addclose(fa: *mut c_void, fd: c_int) -> c_int {
    unsafe { acciones(fa).push(Accion::Cerrar(fd)) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawnattr_init(_a: *mut c_void) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawnattr_destroy(_a: *mut c_void) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawnattr_setflags(_a: *mut c_void, _f: i16) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawnattr_setsigmask(_a: *mut c_void, _m: *const c_void) -> c_int { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawnattr_setsigdefault(_a: *mut c_void, _m: *const c_void) -> c_int { 0 }

unsafe fn lanzar(pid: *mut c_int, path: *const c_char, fa: *mut c_void, argv: *const *const c_char, envp: *const *const c_char) -> c_int {
    let prog = unsafe { ruta(path) };
    let mut cmd = Command::new(OsStr::from_bytes(prog));
    // argv[0] se descarta: `Command` pone el programa.
    let mut i = 1;
    while !argv.is_null() && !unsafe { *argv.add(i) }.is_null() {
        cmd.arg(OsStr::from_bytes(unsafe { ruta(*argv.add(i)) }));
        i += 1;
    }
    cmd.env_clear();
    let mut i = 0;
    while !envp.is_null() && !unsafe { *envp.add(i) }.is_null() {
        let kv = unsafe { ruta(*envp.add(i)) };
        if let Some(p) = kv.iter().position(|&b| b == b'=') {
            cmd.env(OsStr::from_bytes(&kv[..p]), OsStr::from_bytes(&kv[p + 1..]));
        }
        i += 1;
    }
    if !fa.is_null() && !unsafe { *(fa as *mut *mut Acciones) }.is_null() {
        for a in unsafe { acciones(fa) }.iter() {
            if let Accion::Abrir(fd, ruta_f, flags) = a {
                let nombre = OsStr::from_bytes(ruta_f);
                let mut o = std::fs::OpenOptions::new();
                if flags & LO_WRONLY != 0 || flags & LO_RDWR != 0 {
                    o.write(true).create(flags & LO_CREAT != 0).truncate(flags & LO_TRUNC != 0);
                } else {
                    o.read(true);
                }
                if let Ok(f) = o.open(nombre) {
                    match fd {
                        0 => { cmd.stdin(Stdio::from(f)); }
                        1 => { cmd.stdout(Stdio::from(f)); }
                        2 => { cmd.stderr(Stdio::from(f)); }
                        _ => {}
                    }
                }
            }
        }
    }
    match cmd.spawn() {
        Ok(h) => {
            let id = h.id() as i32;
            HIJOS.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, h);
            if !pid.is_null() { unsafe { *pid = id } };
            0
        }
        Err(e) => e.raw_os_error().unwrap_or(ENOENT),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawn(pid: *mut c_int, path: *const c_char, fa: *mut c_void, _at: *mut c_void,
    argv: *const *const c_char, envp: *const *const c_char) -> c_int {
    unsafe { lanzar(pid, path, fa, argv, envp) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawnp(pid: *mut c_int, file: *const c_char, fa: *mut c_void, at: *mut c_void,
    argv: *const *const c_char, envp: *const *const c_char) -> c_int {
    // Con `/` es un camino; sin él se busca en PATH.
    let f = unsafe { ruta(file) };
    if f.contains(&b'/') {
        return unsafe { posix_spawn(pid, file, fa, at, argv, envp) };
    }
    let path = std::env::var_os("PATH").unwrap_or_else(|| "/bin".into());
    for dir in path.as_bytes().split(|&b| b == b':') {
        let mut cand = dir.to_vec();
        cand.push(b'/');
        cand.extend_from_slice(f);
        cand.push(0);
        let mut st = std::mem::MaybeUninit::<lstat_t>::zeroed();
        if unsafe { estado(cand.as_ptr().cast(), st.as_mut_ptr()) } == 0 {
            return unsafe { posix_spawn(pid, cand.as_ptr().cast(), fa, at, argv, envp) };
        }
    }
    ENOENT
}

unsafe fn esperar(pid: c_int, estado_out: *mut c_int) -> c_int {
    let mut g = HIJOS.lock().unwrap();
    let mapa = g.get_or_insert_with(HashMap::new);
    let id = if pid > 0 { pid } else { match mapa.keys().next() { Some(&k) => k, None => return unsafe { fallo(10) as c_int } } };
    let Some(mut h) = mapa.remove(&id) else { return unsafe { fallo(10) as c_int } };
    drop(g);
    match h.wait() {
        Ok(st) => {
            if !estado_out.is_null() {
                unsafe { *estado_out = (st.code().unwrap_or(255) & 0xff) << 8 };
            }
            id
        }
        Err(_) => unsafe { fallo(10) as c_int },
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waitpid(pid: c_int, st: *mut c_int, _opts: c_int) -> c_int {
    unsafe { esperar(pid, st) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wait4(pid: c_int, st: *mut c_int, _opts: c_int, ru: *mut u8) -> c_int {
    if !ru.is_null() { unsafe { std::ptr::write_bytes(ru, 0, 144) }; }
    unsafe { esperar(pid, st) }
}

#[allow(dead_code)]
const _UNUSED: (u64, i32, i32, i32) = (SYS_EXIT, ENOENT, EBADF, ENOMEM);
const _GETPID: u64 = SYS_GETPID;
const _WRITE: u64 = SYS_WRITE;
