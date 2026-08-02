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
pub const SYS_GPU_FREE: u64 = 34;
/// Contadores de memoria física (frames de 4 KiB).
pub const SYS_MEMINFO: u64 = 35;
pub const SYS_IOSTAT: u64 = 36;

/// Lista discos raw para instalación (`DiskInfo` en buffer de usuario).
pub const SYS_DISK_LIST: u64 = 37;
/// Lee sectores de 512 B: `(id, lba, buf, len)`.
pub const SYS_DISK_READ: u64 = 38;
/// Escribe sectores de 512 B en NVMe destino: `(id, lba, buf, len)`.
pub const SYS_DISK_WRITE: u64 = 39;
/// Resuelve un nombre DNS a IPv4 (`host_ptr`, `host_len`, `out: *mut u8[4]`).
pub const SYS_DNS_RESOLVE: u64 = 40;

/// Bits del valor que devuelve `SYS_GPU_SUBMIT` para SAXPY/MATVF.
///
/// Son DOS preguntas distintas y hacían falta las dos: `ON_GPU` es "lo calculó
/// el silicio de la GPU" (el criterio GO de L6, y no se pone por cortesía) y
/// `COMPUTED` es "el resultado ya está en el búfer, no lo recalcules". El camino
/// de NVIDIA cae en CPU dentro del kernel cuando el canal no está listo: eso es
/// `COMPUTED=1, ON_GPU=0`, y con un solo bit el userspace lo repetía por su
/// cuenta creyendo que no se había hecho nada.
pub const GPU_SUBMIT_ON_GPU: u64 = 1 << 32;
pub const GPU_SUBMIT_COMPUTED: u64 = 1 << 33;
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

/// Dominio de `SYS_GPU_ALLOC` (arg2; arg1 = bytes). Como nouveau: quien crea el
/// búfer elige VRAM (pesos residentes) o sysmem/GART (scratch x/y).
pub const GPU_ALLOC_GART: u64 = 0;
pub const GPU_ALLOC_VRAM: u64 = 1;

/// Contrato de retorno de las syscalls GPU, porque no estaba escrito y las dos
/// mitades del mismo par no coincidían:
///
/// - `SYS_GPU_ALLOC`  → handle (≥ 0).
/// - `SYS_GPU_MAP`    → **0**. Subir de más es EINVAL, no un recorte.
/// - `SYS_GPU_READ`   → **0**. Igual.
/// - `SYS_GPU_FREE`   → bytes devueltos a la cuenta de VRAM (los usa el llamante
///   para su contabilidad de residentes; ese sí es un número con dueño).
/// - `SYS_GPU_SUBMIT` → bits `GPU_SUBMIT_*`.
///
/// Negativo es siempre `-errno`.

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GpuInfo {
    pub present: u8,
    pub vendor: u8,
    /// 1 = `SYS_GPU_SUBMIT` **calcula el resultado** en este dispositivo (aunque
    /// sea con el bucle de CPU del kernel); 0 = acepta búferes pero no ejecuta
    /// nada. Sin este bit, un userspace que ve `present=1` sube la matriz entera
    /// por syscalls a una iGPU Intel que no va a lanzar ningún kernel, y luego
    /// la recalcula en CPU: coste doble sin ninguna señal de que algo va mal.
    /// Sale del hueco de `_pad`, así que el layout no cambia.
    pub compute: u8,
    pub _pad: [u8; 5],
    pub vram_total: u64,
    pub vram_free: u64,
    pub name: [u8; 32],
    /// Hasta dónde llegó el bring-up del dispositivo, en texto y con NUL
    /// (`bar0`, `booted`, `rm_ce`, `rm_compute`, `booted_soft`…). Va como cadena
    /// y no como código numérico a propósito: la tabla de nombres vive en un solo
    /// sitio —el port— y no en dos que divergirían, que es el vicio que ya dio un
    /// `INVALID_CLASS` con nombre inventado.
    ///
    /// Existe porque `on_gpu=0` no dice **dónde** paró, y averiguarlo obligaba a
    /// abrir cientos de líneas del log de serie. Vacía cuando no hay dispositivo
    /// con fases (el de software, o ninguno).
    pub phase: [u8; 16],
}

/// Umbral: ficheros mayores se abren en modo lazy (sin cargar todo).
pub const LAZY_FILE_THRESHOLD: u64 = 64 * 1024;

/// Respuesta de `SYS_MEMINFO`: frames de 4 KiB (multiplicar ×4096 para bytes).
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct MemInfo {
    pub total_frames: u64,
    pub free_frames: u64,
    /// Páginas mmap RO file-backed registradas y evictables por el kernel.
    pub reclaimable_frames: u64,
}

/// Respuesta de `SYS_IOSTAT`: contadores de E/S de bloque desde el arranque.
///
/// `peticiones` frente a `bloques` es la distinción que importa al agrupar
/// lecturas: bajar bloques es leer menos, bajar peticiones es leer lo mismo en
/// menos viajes al dispositivo.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct IoStat {
    pub peticiones: u64,
    pub bloques: u64,
    pub nanos: u64,
    pub escrituras: u64,
    /// Caché de bloques de sosomfs (0 si no hay disco de modelos montado).
    pub cache_aciertos: u64,
    pub cache_fallos: u64,
}

/// Tipos de disco para `DiskInfo.kind`.
pub const DISK_KIND_USB: u32 = 0;
pub const DISK_KIND_VIRTIO: u32 = 1;
pub const DISK_KIND_NVME: u32 = 2;

/// Flags de `DiskInfo.flags`.
pub const DISK_FLAG_READONLY: u32 = 1;
/// Disco de arranque live (origen de clonación).
pub const DISK_FLAG_BOOT: u32 = 2;

/// Entrada devuelta por `SYS_DISK_LIST`.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct DiskInfo {
    pub id: u32,
    pub kind: u32,
    pub slot: u32,
    pub sectors: u64,
    pub flags: u32,
    pub name: [u8; 16],
}

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
pub const EBUSY: i64 = 16;
pub const EROFS: i64 = 30;
pub const ENOTSUP: i64 = 95;
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
/// Con `O_WRONLY`: crea el fichero si no existe.
pub const O_CREAT: u64 = 4;

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
