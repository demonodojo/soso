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

// ---- mmap ----

pub const PROT_READ: u64 = 1;
pub const PROT_WRITE: u64 = 2;
pub const MAP_PRIVATE: u64 = 1;
pub const MAP_SHARED: u64 = 2;
pub const MAP_ANONYMOUS: u64 = 4;

/// Región reservada para mmap de ficheros (por encima del brk habitual).
pub const MMAP_BASE: u64 = 0x2000_0000;
pub const MMAP_LIMIT: u64 = 0x5f00_0000;

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

// ---- open ----

pub const O_RDONLY: u64 = 0;
/// Escritura: crea (o trunca) el fichero; el contenido se publica en close().
pub const O_WRONLY: u64 = 1;

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

/// wait() devuelve (pid << 8) | (código de salida & 0xff).
pub fn wait_decode(v: i64) -> (u64, u8) {
    ((v as u64) >> 8, v as u8)
}
