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
/// Escribe la petición de arranque en `SOSOBOOT.TXT` de la ESP live:
/// `(buf, len)`. Es el único camino por el que userspace toca el disco de
/// arranque, y solo ese fichero pre-creado. Lo lee el shim UEFI en el
/// siguiente arranque para registrar la entrada `Boot####`.
pub const SYS_BOOTREQ_WRITE: u64 = 41;
/// Lee ese mismo fichero: `(buf, len)`. Devuelve los bytes leídos.
pub const SYS_BOOTREQ_READ: u64 = 42;
/// Import atómico de modelos en sosomfs (ver `som_import`).
pub const SYS_SOM_BEGIN: u64 = 43;
pub const SYS_SOM_PUT: u64 = 44;
pub const SYS_SOM_COMMIT: u64 = 45;
pub const SYS_SOM_ABORT: u64 = 46;
pub const SYS_SOM_SCRATCH_ALLOC: u64 = 47;
pub const SYS_SOM_SCRATCH_WRITE: u64 = 48;
pub const SYS_SOM_SCRATCH_READ: u64 = 49;
pub const SYS_SOM_SCRATCH_FREE: u64 = 50;
pub const SYS_GETPID: u64 = 51;
pub const SYS_KILL: u64 = 52;
pub const SYS_SETPGID: u64 = 53;
pub const SYS_SETSID: u64 = 54;
pub const SYS_TCSETPGRP: u64 = 55;
/// Escanea redes WiFi (`out: *mut WifiBss`, `max`). Devuelve el número de BSS.
pub const SYS_WIFI_SCAN: u64 = 56;
/// Estado del adaptador WiFi (`out: *mut WifiStatus`).
pub const SYS_WIFI_STATUS: u64 = 57;
/// Asocia a una red: `(ssid_ptr, ssid_len, psk_ptr, psk_len)`. `psk_len=0` = abierta.
pub const SYS_WIFI_CONNECT: u64 = 58;
/// Abre captura de audio: `(format_ptr)` → fd lógico 0 si ok.
pub const SYS_AUDIO_OPEN: u64 = 59;
/// Lee PCM: `(buf, len, overrun_ptr)` → bytes leídos; `overrun_ptr` recibe 0/1.
pub const SYS_AUDIO_READ: u64 = 60;
/// Cierra captura de audio.
pub const SYS_AUDIO_CLOSE: u64 = 61;
/// Información del framebuffer GOP.
pub const SYS_FB_INFO: u64 = 62;
/// Modo consola (0) o gráfico userspace (1): cede la consola de texto.
pub const SYS_FB_SET_MODE: u64 = 63;
/// Copia un búfer de píxeles al framebuffer físico.
pub const SYS_FB_PRESENT: u64 = 64;
/// Lee eventos de ratón/teclado pendientes.
pub const SYS_INPUT_POLL: u64 = 65;
/// Espera a que termine un QMD encolado con `GPU_SUBMIT_ASYNC` (arg1 = fence).
pub const SYS_GPU_WAIT: u64 = 66;
/// Copia la versión del kernel a un buffer de usuario (`buf`, `len`). Devuelve bytes escritos.
pub const SYS_VERSION: u64 = 67;
/// Escribe en el hueco de actualización de la ESP: `(which, offset, buf, len)`.
/// `which`: `UPD_WHICH_MAILBOX` o `UPD_WHICH_KERNEL`. Offset múltiplo de 512.
pub const SYS_UPD_WRITE: u64 = 68;
/// Lee del hueco: `(which, offset, buf, len)`. Devuelve bytes leídos.
pub const SYS_UPD_READ: u64 = 69;

/// Renombra o mueve: `(old_ptr, old_len, new_ptr, new_len)`.
pub const SYS_RENAME: u64 = 70;
/// Trunca un fichero: `(path_ptr, path_len, size)`.
pub const SYS_TRUNCATE: u64 = 71;
/// Reloj de pared: `(clock_id, out: *mut Timespec)`.
pub const SYS_CLOCK_GETTIME: u64 = 72;
/// Duplica un descriptor: `(oldfd, newfd)`.
pub const SYS_DUP2: u64 = 73;
/// Stat por fd: `(fd, out: *mut Stat)`.
pub const SYS_FSTAT: u64 = 74;
/// Toca mtime: `(path_ptr, path_len, mtime_secs)`.
pub const SYS_UTIME: u64 = 75;
/// Sincroniza un fd abierto para escritura.
pub const SYS_FSYNC: u64 = 76;
/// Cede la CPU al scheduler.
pub const SYS_SCHED_YIELD: u64 = 77;
/// Rellena bytes aleatorios (`buf`, `len`) vía RDRAND.
pub const SYS_GETRANDOM: u64 = 78;
/// Establece FS_BASE del hilo actual (`tls_base`).
pub const SYS_SET_TLS: u64 = 79;
/// Cambia permisos de un rango mmap: `(addr, len, prot)`.
pub const SYS_MPROTECT: u64 = 80;
/// Redimensiona un mmap: `(addr, old_len, new_len, flags)`.
pub const SYS_MREMAP: u64 = 81;
/// Escribe en fd a offset fijo sin mover el cursor: `(fd, buf, len, offset)`.
pub const SYS_PWRITE: u64 = 82;
/// Lee variable de entorno del proceso: `(key_ptr, key_len, val_ptr, val_len)`.
pub const SYS_GETENV: u64 = 83;
/// Redimensiona sosofs/sosomfs en disco GPT live/instalado.
/// `(op, arg, out_ptr)`: `FS_RESIZE_GROW_ROOT` + bloques 4K, o `FS_RESIZE_QUERY` + `FsSpaceInfo`.
pub const SYS_FS_RESIZE: u64 = 84;

pub const FS_RESIZE_GROW_ROOT: u64 = 0;
pub const FS_RESIZE_QUERY: u64 = 1;

/// Informe de espacio para `FS_RESIZE_QUERY`.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct FsSpaceInfo {
    pub root_fs_blocks: u64,
    pub root_free_blocks: u64,
    pub root_part_blocks: u64,
    pub models_fs_blocks: u64,
    pub models_used_blocks: u64,
    pub models_part_blocks: u64,
    /// Cuántos bloques 4K se pueden robar del final de modelos.
    pub max_grow_blocks: u64,
}

pub const UPD_WHICH_MAILBOX: u64 = 0;
pub const UPD_WHICH_KERNEL: u64 = 1;
pub const UPD_WHICH_META: u64 = 2;
pub const UPD_MAILBOX_SIZE: usize = 4096;
pub const UPD_KERNEL_META_SIZE: usize = 512;
pub const UPD_KERNEL_SLOT_SIZE: u64 = 64 * 1024 * 1024;
/// Tamaño fijo de `SOSOBOOT.TXT`.
pub const BOOTREQ_SIZE: usize = 4096;

pub const WIFI_SSID_MAX: usize = 32;
pub const WIFI_PSK_MAX: usize = 63;
pub const WIFI_SCAN_MAX: usize = 32;
pub const WIFI_PHASE_MAX: usize = 48;
pub const WIFI_FLAG_PRESENT: u32 = 1;
pub const WIFI_FLAG_ALIVE: u32 = 2;
pub const WIFI_FLAG_CONNECTED: u32 = 4;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct WifiBss {
    pub ssid: [u8; WIFI_SSID_MAX],
    pub ssid_len: u8,
    pub bssid: [u8; 6],
    pub rssi: i8,
    pub channel: u8,
    pub open: u8,
    pub _pad: u8,
}

impl Default for WifiBss {
    fn default() -> Self {
        Self {
            ssid: [0; WIFI_SSID_MAX],
            ssid_len: 0,
            bssid: [0; 6],
            rssi: 0,
            channel: 0,
            open: 0,
            _pad: 0,
        }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct WifiStatus {
    pub flags: u32,
    pub mac: [u8; 6],
    pub _pad: u16,
    pub phase: [u8; WIFI_PHASE_MAX],
}

impl Default for WifiStatus {
    fn default() -> Self {
        Self {
            flags: 0,
            mac: [0; 6],
            _pad: 0,
            phase: [0; WIFI_PHASE_MAX],
        }
    }
}

// ---- framebuffer ----

pub const FB_FMT_RGB: u8 = 0;
pub const FB_FMT_BGR: u8 = 1;
pub const FB_FMT_U8: u8 = 2;

pub const FB_MODE_CONSOLE: u64 = 0;
pub const FB_MODE_GRAPHICS: u64 = 1;

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct FbInfo {
    pub present: u8,
    pub pixel_format: u8,
    pub bytes_per_pixel: u8,
    pub _pad: u8,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub byte_len: u64,
}

// ---- entrada (ratón / teclado) ----

pub const INPUT_MOUSE_MOVE: u32 = 0;
pub const INPUT_MOUSE_BTN: u32 = 1;
pub const INPUT_KEY: u32 = 2;

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct InputEvent {
    pub kind: u32,
    pub x: i32,
    pub y: i32,
    pub button: u32,
    pub key: u32,
    pub pressed: u8,
    pub _pad: [u8; 3],
}

/// Formato de captura para `SYS_AUDIO_OPEN`.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub _pad: u32,
}

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
pub const GPU_SUBMIT_ASYNC: u64 = 1 << 34;
pub const GPU_SUBMIT_FENCE_MASK: u64 = 0xffff;
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

// ---- señales (modelo mínimo) ----

pub const SIGINT: u64 = 2;
pub const SIGKILL: u64 = 9;
pub const SIGTERM: u64 = 15;

/// Código de salida por señal: 128 + número de señal.
pub const fn exit_by_signal(sig: u8) -> u8 {
    128 + sig
}

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
/// - `SYS_GPU_ALLOC`  → handle (≥ 0). `GPU_ALLOC_VRAM` en la NVIDIA sale del pool
///   del dispositivo o falla (ENOTSUP sin pool, ENOMEM si está lleno): nunca del
///   heap del kernel disfrazado. Ver `GpuInfo::vram_bufs`.
/// - `SYS_GPU_MAP`    → **0**. Subir de más es EINVAL, no un recorte. El único
///   límite de `len` es el tamaño del búfer: estas dos son transferencias
///   masivas y NO comparten el techo de 16 MiB de un búfer de syscall normal
///   (lo compartían, y un tensor de 44 MiB daba EFAULT).
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
    /// 1 = `GPU_ALLOC_VRAM` entrega memoria que **este dispositivo** puede leer;
    /// 0 = el dispositivo existe pero no hay pool (en NVIDIA: GSP arrancado sin
    /// haber llegado a canal ni CE, que es todo lo que hay hoy en Ampere).
    ///
    /// Con 0, el `alloc` de VRAM falla en vez de repartir búferes del heap del
    /// kernel disfrazados: hacerlo pasar por VRAM daba `gpu_map` a OK y un
    /// `MATVF` calculado por la CPU del kernel, o sea «offload» sin GPU y un
    /// heap que se agota tensor a tensor. El de software lleva 1 —su heap ES su
    /// VRAM y su bucle de CPU es lo que promete—; la Intel, 0.
    ///
    /// Sale del hueco de `_pad`, así que el layout no cambia.
    pub vram_bufs: u8,
    pub _pad: [u8; 4],
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
    /// Subidas a VRAM que fueron por DMA del CE (sin copia de CPU) y por rebote.
    ///
    /// Existen porque el camino sin copias llevaba desde el 2026-08-17 **sin
    /// dispararse nunca con los pesos del modelo** —el payload de un shard empieza
    /// en el byte 64 y `subir_por_dma` exigía alineación de página— y desde fuera
    /// era indistinguible del camino rápido: el rebote funciona, sólo es lento.
    /// Un `uploads_bounce` distinto de 0 es una regresión, no un detalle.
    pub uploads_dma: u32,
    pub uploads_bounce: u32,
    /// Bytes que pasaron por el rebote (heap del kernel + rebote de 1 MiB del CE).
    pub bounce_bytes: u64,
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
/// El disco desde el que arrancó el sistema (USB live, virtio o NVMe
/// dual-boot). `soso-install` lo usa como origen de clonación; también marca
/// qué disco protege `sys_disk_write` (nunca deja escribir el propio disco
/// de arranque).
pub const DISK_FLAG_BOOT: u32 = 2;
/// GPT con magic SOSOFS10 en la partición 2: soso previo, reinstalar es seguro.
pub const DISK_FLAG_SOSO: u32 = 4;
/// Sin tabla de particiones y primer sector a cero.
pub const DISK_FLAG_EMPTY: u32 = 8;

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
pub const ESRCH: i64 = 3;
pub const EINTR: i64 = 4;
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
pub const EADDRINUSE: i64 = 98;
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
/// Con `O_WRONLY`: trunca el fichero existente al abrir.
pub const O_TRUNC: u64 = 8;
/// Con `O_CREAT`: falla si el fichero ya existe.
pub const O_EXCL: u64 = 16;

/// Reloj de pared (epoch Unix).
pub const CLOCK_REALTIME: u64 = 0;
/// Reloj monótono desde el arranque.
pub const CLOCK_MONOTONIC: u64 = 1;

/// Valor de stdio en `SpawnIo` para usar la tty del proceso (fd 0/1/2).
pub const FD_INHERIT_TTY: u64 = u64::MAX;
/// Como `FD_INHERIT_TTY`, pero ata esos fds a la consola serie, no a la del padre.
/// Sirve para demonios (askd) lanzados desde una sesión SSH: su stdout acaba
/// en el puerto serie y en `SOSOLOG.TXT`, no mezclado con la sesión remota.
pub const FD_SERIAL_TTY: u64 = u64::MAX - 1;

/// ¿`spec` de stdio es un centinela (no un fd del padre)?
pub fn stdio_es_tty(spec: u64) -> bool {
    spec == FD_INHERIT_TTY || spec == FD_SERIAL_TTY
}

// ---- seek ----

pub const SEEK_SET: u64 = 0;
pub const SEEK_CUR: u64 = 1;
pub const SEEK_END: u64 = 2;

// ---- tipos de fichero (coinciden con sosofs) ----

pub const FT_FILE: u8 = 1;
pub const FT_DIR: u8 = 2;

pub const NAME_MAX: usize = 255;

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

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
}

impl Dirent {
    pub fn name_bytes(&self) -> &[u8] {
        &self.name[..(self.name_len as usize).min(NAME_MAX)]
    }
}

impl Default for Dirent {
    fn default() -> Self {
        Self { ino: 0, file_type: 0, name_len: 0, name: [0; NAME_MAX] }
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
    /// Puntero a tabla `[(*const u8, len); ...]`; 0 = usar `args_ptr`/`args_len`.
    pub argv_ptr: u64,
    pub argv_count: u64,
    /// Puntero a tabla de cadenas `KEY=VAL`; puede ser 0.
    pub envp_ptr: u64,
    pub envp_count: u64,
}

/// wait() devuelve (pid << 8) | (código de salida & 0xff).
pub fn wait_decode(v: i64) -> (u64, u8) {
    ((v as u64) >> 8, v as u8)
}
