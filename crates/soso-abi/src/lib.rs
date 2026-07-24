//! ABI kernel ↔ userspace de soso: números de syscall, errnos y structs
//! compartidas. Todo x86_64 little-endian, repr(C).
//!
//! Convención de llamada: `syscall` con nº en rax, argumentos en
//! rdi/rsi/rdx/r10, retorno en rax (i64; negativo = -errno). El kernel
//! preserva rsp y los registros callee-saved (rbx, rbp, r12-r15); el resto
//! se consideran clobber de la instrucción.

#![no_std]

// ---- números de syscall ----

pub const SYS_EXIT: u64 = 0;
pub const SYS_READ: u64 = 1;
pub const SYS_WRITE: u64 = 2;
pub const SYS_OPEN: u64 = 3;
pub const SYS_CLOSE: u64 = 4;
pub const SYS_SEEK: u64 = 5;
pub const SYS_STAT: u64 = 6;
pub const SYS_GETDENTS: u64 = 7;
pub const SYS_MKDIR: u64 = 8;
pub const SYS_UNLINK: u64 = 9;
pub const SYS_SPAWN: u64 = 10;
pub const SYS_WAIT: u64 = 11;
pub const SYS_SBRK: u64 = 12;
pub const SYS_SLEEP_MS: u64 = 13;
pub const SYS_HALT: u64 = 14;
pub const SYS_MMAP: u64 = 15;
pub const SYS_MUNMAP: u64 = 16;
pub const SYS_GPU_INFO: u64 = 17;
pub const SYS_GPU_ALLOC: u64 = 18;
pub const SYS_GPU_MAP: u64 = 19;
pub const SYS_GPU_SUBMIT: u64 = 20;
pub const SYS_GPU_READ: u64 = 33;
pub const SYS_PIPE: u64 = 21;
pub const SYS_SPAWN_IO: u64 = 22;
pub const SYS_CHDIR: u64 = 23;
pub const SYS_GETCWD: u64 = 24;
pub const SYS_THREAD_SPAWN: u64 = 25;
pub const SYS_FUTEX: u64 = 26;
pub const SYS_NCPU: u64 = 27;
pub const SYS_UPTIME_MS: u64 = 28;
pub const SYS_TCP_CONNECT: u64 = 29;
pub const SYS_TCP_LISTEN: u64 = 30;
pub const SYS_TCP_ACCEPT: u64 = 31;
/// `read(fd, buf, len)` con límite de espera en ms (0 = bloqueante indefinido).
pub const SYS_READ_TIMEOUT: u64 = 32;

/// Operaciones de `SYS_FUTEX` (arg `op`).
pub const FUTEX_WAIT: u64 = 0;
pub const FUTEX_WAKE: u64 = 1;

// ---- mmap ----

pub const PROT_READ: u64 = 1;
pub const PROT_WRITE: u64 = 2;
pub const MAP_PRIVATE: u64 = 1;
pub const MAP_SHARED: u64 = 2;
pub const MAP_ANONYMOUS: u64 = 4;

/// Región reservada para mmap (ficheros y anónimo), por encima de la pila y
/// dentro de la entrada L4[0] del usuario (< 512 GiB): ventana de ~416 GiB
/// para mapear modelos grandes completos.
pub const MMAP_BASE: u64 = 0x10_0000_0000; // 64 GiB
pub const MMAP_LIMIT: u64 = 0x78_0000_0000; // 480 GiB

// ---- GPU ----

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GpuInfo {
    pub present: u8,
    pub vendor: u8,
    pub _pad: [u8; 6],
    pub vram_total: u64,
    pub vram_free: u64,
    pub name: [u8; 32],
}

/// Umbral: ficheros mayores se abren en modo lazy (sin cargar todo).
pub const LAZY_FILE_THRESHOLD: u64 = 64 * 1024;

// ---- errnos (el kernel devuelve -errno) ----

pub const ENOENT: i64 = 2;
pub const EIO: i64 = 5;
pub const EBADF: i64 = 9;
pub const ECHILD: i64 = 10;
pub const ENOMEM: i64 = 12;
pub const EFAULT: i64 = 14;
pub const EEXIST: i64 = 17;
pub const ENOTDIR: i64 = 20;
pub const EISDIR: i64 = 21;
pub const EINVAL: i64 = 22;
pub const EMFILE: i64 = 24;
pub const ENOSPC: i64 = 28;
pub const ESPIPE: i64 = 29;
pub const ENAMETOOLONG: i64 = 36;
pub const ENOSYS: i64 = 38;
pub const ENOTEMPTY: i64 = 39;
pub const EPIPE: i64 = 32;
pub const EAGAIN: i64 = 11;
pub const ECONNREFUSED: i64 = 61;
pub const ENOTCONN: i64 = 107;

/// Dirección IPv4 + puerto para syscalls TCP.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct SockAddr {
    /// IPv4 en orden de bytes de red (p. ej. 10.0.2.15 → [10,0,2,15]).
    pub addr: [u8; 4],
    pub port: u16,
    pub _pad: u16,
}

// ---- open ----

pub const O_RDONLY: u64 = 0;
/// Escritura: crea (o trunca) el fichero; el contenido se publica en close().
pub const O_WRONLY: u64 = 1;
/// Con `O_WRONLY`: conserva el contenido existente y escribe al final.
pub const O_APPEND: u64 = 2;

/// Valor de stdio en `SpawnIo` para usar la tty del proceso (fd 0/1/2).
pub const FD_INHERIT_TTY: u64 = u64::MAX;

// ---- seek ----

pub const SEEK_SET: u64 = 0;
pub const SEEK_CUR: u64 = 1;
pub const SEEK_END: u64 = 2;

// ---- tipos de fichero (coinciden con sosofs) ----

pub const FT_FILE: u8 = 1;
pub const FT_DIR: u8 = 2;

pub const NAME_MAX: usize = 55;

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct Stat {
    pub ino: u64,
    pub size: u64,
    /// Segundos desde epoch (en soso: uptime al escribir).
    pub mtime: u64,
    pub file_type: u8,
    pub _pad: [u8; 7],
}

/// Registro de getdents(): tamaño fijo, `n` completos por llamada.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Dirent {
    pub ino: u64,
    pub file_type: u8,
    pub name_len: u8,
    pub name: [u8; NAME_MAX],
    pub _pad: u8,
}

impl Dirent {
    pub fn name_bytes(&self) -> &[u8] {
        &self.name[..(self.name_len as usize).min(NAME_MAX)]
    }
}

impl Default for Dirent {
    fn default() -> Self {
        Self { ino: 0, file_type: 0, name_len: 0, name: [0; NAME_MAX], _pad: 0 }
    }
}

pub const DIRENT_SIZE: usize = core::mem::size_of::<Dirent>();

/// Argumentos de `SYS_SPAWN_IO`: igual que spawn, más stdio opcional.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct SpawnIo {
    pub path_ptr: u64,
    pub path_len: u64,
    pub args_ptr: u64,
    pub args_len: u64,
    pub stdin_fd: u64,
    pub stdout_fd: u64,
    pub stderr_fd: u64,
}

/// wait() devuelve (pid << 8) | (código de salida & 0xff).
pub fn wait_decode(v: i64) -> (u64, u8) {
    ((v as u64) >> 8, v as u8)
}
