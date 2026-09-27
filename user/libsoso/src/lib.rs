//! Mini-libstd de soso: crt0, wrappers de syscall, print!, panic handler
//! y un allocator global basado en sbrk.

#![no_std]

extern crate alloc;

pub mod glob;
pub mod linea;
pub mod sys;
pub mod thread;

pub use soso_abi as abi;

use core::fmt;

use spin::Mutex;

// ---- crt0 ----

/// Inicializa el allocator global antes de `main`. Idempotente.
///
/// El montón vive en `soso-alloc` (el mismo que usa libstd). Aquí sólo se
/// registra como alocador global de los binarios que enlazan libsoso.
pub fn heap_init() {
    soso_alloc::heap_init();
}

/// Comprueba invariantes del arena (no hace nada sin `SOSO_HEAP_DEBUG=1`).
pub fn heap_audit() {
    soso_alloc::heap_audit();
}

/// argv completo (argv[0] = path del binario) tal como lo mandó el kernel.
static ARGV: Mutex<Option<alloc::vec::Vec<alloc::string::String>>> = Mutex::new(None);

/// La cadena que entregó un llamante **sin** argv estructurado (`spawn_io`).
///
/// Sólo la hay por el camino antiguo, y existe por un consumidor concreto:
/// `soso-llm ask <pregunta>` tiene que recibir el texto **tal como se
/// escribió**, con sus espacios seguidos y sus comillas. Partirlo y volver a
/// juntarlo lo cambiaría, y esa literalidad es justo lo que su prueba de la
/// suite comprueba.
static LINEA_CRUDA: Mutex<Option<alloc::string::String>> = Mutex::new(None);

/// Decodifica el blob `SOSA` del kernel (`kernel/src/task/argv.rs`):
/// `"SOSA" u32 count { u32 len, bytes }*`. `None` si no lleva el magic.
pub fn decode_argv(blob: &[u8]) -> Option<alloc::vec::Vec<alloc::string::String>> {
    use alloc::string::String;
    use alloc::vec::Vec;

    const MAGIC: &[u8; 4] = b"SOSA";
    if blob.len() < 8 || &blob[..4] != MAGIC {
        return None;
    }
    let count = u32::from_le_bytes(blob[4..8].try_into().unwrap()) as usize;
    let mut argv: Vec<String> = Vec::with_capacity(count.min(256));
    let mut off = 8usize;
    for _ in 0..count {
        if off + 4 > blob.len() {
            break;
        }
        let slen = u32::from_le_bytes(blob[off..off + 4].try_into().unwrap()) as usize;
        off += 4;
        if off + slen > blob.len() {
            break;
        }
        if let Ok(s) = core::str::from_utf8(&blob[off..off + slen]) {
            argv.push(String::from(s));
        }
        off += slen;
    }
    Some(argv)
}

/// argv exacto del proceso (sin unir por espacios): `argv[0]` es el path del
/// binario. Vacío si el crt0 aún no ha corrido o el kernel no mandó blob.
pub fn argv() -> alloc::vec::Vec<alloc::string::String> {
    ARGV.lock().clone().unwrap_or_default()
}

/// La línea tal como llegó, si el llamante no pasó argv estructurado.
///
/// `None` significa que **sí** hubo argv: entonces la línea no existe, y
/// reconstruirla juntando los argumentos sería inventarla. Quien la quiera de
/// todas formas que haga el `join` y se vea en el código.
pub fn linea_cruda() -> Option<alloc::string::String> {
    LINEA_CRUDA.lock().clone()
}

/// Decodifica el blob SOSA del kernel y devuelve los argumentos del programa
/// (`argv[1..]`; `argv[0]` es la ruta del binario).
///
/// Antes esto devolvía los argumentos **juntados por espacios** y cada programa
/// los volvía a partir. Era lossy por construcción: una ruta con un espacio
/// dentro salía del otro lado como dos rutas, y no había forma de distinguirla
/// de dos argumentos de verdad (T62). Quien quiera una cadena la arma con
/// `join(" ")`, que es lo mismo pero se ve en el código.
pub fn args_for_main(blob: &[u8]) -> alloc::vec::Vec<alloc::string::String> {
    use alloc::string::String;
    use alloc::vec::Vec;

    if let Some(argv) = decode_argv(blob) {
        // Blob válido: sin más elementos que argv[0] no hay argumentos. Nunca
        // devolver aquí los bytes crudos «SOSA…».
        let args: Vec<String> = if argv.len() > 1 {
            argv[1..].to_vec()
        } else {
            Vec::new()
        };
        *ARGV.lock() = Some(argv);
        return args;
    }
    // Sin magic: el blob es texto suelto de un llamante antiguo (`spawn_io`,
    // y el tokenizador de sosh). Se parte por espacios porque no hay nada
    // mejor que hacer con él —y porque es lo que hacían ya todos los
    // programas—, pero el original se guarda: quien necesite el texto literal
    // lo pide con [`linea_cruda`].
    let texto = core::str::from_utf8(blob).unwrap_or("");
    *LINEA_CRUDA.lock() = Some(String::from(texto));
    texto.split_whitespace().map(String::from).collect()
}

#[macro_export]
macro_rules! entry {
    ($main:ident) => {
        #[unsafe(no_mangle)]
        extern "C" fn __soso_main(ptr: *const u8, len: usize) -> u8 {
            $crate::tls_init();
            $crate::heap_init();
            let blob = unsafe { core::slice::from_raw_parts(ptr, len) };
            let args = $crate::args_for_main(blob);
            $main(&args)
        }
    };
}

#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text._start")]
extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "xor rbp, rbp",
        "call __soso_main",
        "mov rdi, rax",
        "mov rax, {nr_exit}",
        "syscall",
        "ud2",
        nr_exit = const soso_abi::SYS_EXIT,
    )
}

// ---- soporte de C freestanding ----

/// Protector de pila del compilador de C.
///
/// El `cc` de las dependencias en C (el `ring` que arrastra rustls) trae
/// `-fstack-protector` activado por defecto en la mayoría de distribuciones, y
/// emite una llamada a `__stack_chk_fail` cuando detecta que se ha pisado el
/// canario. En Linux lo pone la libc; aquí no hay libc, así que el enlazado
/// moría con `undefined symbol: __stack_chk_fail` y se llevaba por delante todo
/// el workspace de userspace — no sólo al binario que lo arrastraba.
///
/// La alternativa sería compilar el C con `-fno-stack-protector`, pero eso hay
/// que recordarlo en cada dependencia nueva; proveer el símbolo lo arregla de
/// una vez y además **conserva la comprobación**: si el canario salta, el
/// proceso muere aquí en vez de seguir con la pila corrupta.
/// Bloque TLS mínimo del hilo principal.
///
/// `-fstack-protector` no sólo llama a `__stack_chk_fail` cuando el canario
/// salta: **lee** el canario en `%fs:0x28` al entrar en cada función. Sin base
/// FS eso es la dirección lineal `0x28`, y el proceso muere con
/// «page fault de usuario en 0x28» dentro de `curve25519.c` —81 lecturas así
/// hay sólo en `soso-update`—. Proveer `__stack_chk_fail` no arregla eso: el
/// fallo está en leer el canario, no en comprobarlo.
///
/// El layout es el del ABI de TLS de x86-64: puntero a sí mismo en `fs:0x00` y
/// canario en `fs:0x28`. Nada más se usa; si algún día hace falta `#[thread_local]`
/// de verdad, esto se queda corto y hay que montar el bloque desde `PT_TLS`.
#[repr(C, align(16))]
struct Tcb {
    propio: *mut Tcb,
    _reservado: [u64; 4],
    canario: u64,
    _cola: [u64; 8],
}

const _: () = assert!(core::mem::offset_of!(Tcb, canario) == 0x28);

static mut TCB: Tcb = Tcb {
    propio: core::ptr::null_mut(),
    _reservado: [0; 4],
    canario: 0,
    _cola: [0; 8],
};

/// Fija la base FS del proceso. La llama el `entry!` antes que nada; sin heap.
///
/// Los hilos heredan `tls_base` del padre (`task::thread_spawn`), así que
/// comparten este bloque: el canario sólo se lee, nunca se escribe.
pub fn tls_init() {
    unsafe {
        let p = &raw mut TCB;
        (*p).propio = p;
        let mut semilla = [0u8; 8];
        let canario = if sys::getrandom(&mut semilla) == 8 {
            u64::from_ne_bytes(semilla)
        } else {
            // Sin entropía el canario deja de ser impredecible, pero sigue
            // detectando el desbordamiento accidental, que es lo que importa
            // aquí. Lo que no se puede es dejar la base FS sin fijar.
            0x00c0_ffee_5050_1234
        };
        // Byte bajo a cero: un desbordamiento por cadena no puede copiar el
        // canario entero con un `strcpy`.
        (*p).canario = canario & !0xff;
        sys::set_tls(p as u64);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __stack_chk_fail() -> ! {
    panic!("__stack_chk_fail: canario de pila pisado en código C");
}

// ---- print ----

struct Stdout;

impl fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // `write_all`, no `write`: una escritura puede ser CORTA y aquí nadie
        // miraba el retorno. Con stdout en un pipe —que es lo que hay en toda
        // sesión SSH y en `spawn_io`— eso perdía la cola de cualquier línea larga
        // sin un solo error. Si falla de verdad (pipe cerrado) no hay a quién
        // avisar desde un `fmt::Write`, así que se ignora el error pero NO el
        // progreso parcial.
        let _ = sys::write_all(1, s.as_bytes());
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    let _ = Stdout.write_fmt(args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

/// Escribe un registro en fd 3 (canal de log del kernel). Ignora errores
/// (p. ej. fd cerrado con `3>&-`).
#[macro_export]
macro_rules! logln {
    () => {
        let _ = $crate::sys::write_all($crate::abi::LOG_FD, b"\n");
    };
    ($($arg:tt)*) => {{
        use alloc::string::String;
        use core::fmt::Write;
        let mut s = String::new();
        let _ = write!(s, "{}\n", format_args!($($arg)*));
        let _ = $crate::sys::write_all($crate::abi::LOG_FD, s.as_bytes());
    }};
}

/// Un campo de texto de tamaño fijo del kernel (`GpuInfo::name`, `::phase`, un
/// nombre de dirent…) como `&str`: hasta el primer NUL, o el campo entero si no
/// lo hay, y `"?"` si no es UTF-8 válido.
///
/// Estaba copiado en cuatro sitios —dos los añadió el campo `phase` de `GpuInfo`—
/// y cada copia es una ocasión más de olvidar el `unwrap_or` y hacer panic en un
/// binario cuyo panic handler no tiene a dónde escribir.
pub fn str_hasta_nul(bytes: &[u8]) -> &str {
    let n = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..n]).unwrap_or("?")
}

/// Contador de ciclos de la CPU, para medir tramos cortos desde userspace.
///
/// Existe porque el único reloj que había —`SYS_UPTIME_MS`— es el PIT, y el PIT
/// **subcuenta durante el polling de disco**: medir con él un camino que mezcla
/// E/S y CPU mueve el resultado por razones que no son el cambio que se está
/// midiendo. Y funciona sin tocar el kernel: nadie pone `CR4.TSD`, así que `rdtsc`
/// no está restringido a ring 0.
///
/// No sirve para comparar entre cores (el TSC es invariante en el hardware
/// objetivo, pero nada aquí lo garantiza) ni para convertir a segundos sin conocer
/// la frecuencia: es para restar dos lecturas del mismo hilo.
#[inline]
pub fn ciclos() -> u64 {
    // SAFETY: rdtsc no toca memoria y está disponible en ring 3 (CR4.TSD=0).
    unsafe {
        let hi: u32;
        let lo: u32;
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
        ((hi as u64) << 32) | lo as u64
    }
}

/// Entropía para todo el userspace: la pide al kernel, no a la CPU.
///
/// `getrandom` (y con él `ring`/rustls) busca un backend; sin esto usa RDRAND
/// directamente desde ring 3 y se queda sin aleatoriedad donde la detección
/// falla. Se registra aquí, en la biblioteca que enlazan todos los binarios.
fn entropia_del_kernel(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    if sys::getrandom(buf) < 0 {
        return Err(getrandom::Error::UNSUPPORTED);
    }
    Ok(())
}

getrandom::register_custom_getrandom!(entropia_del_kernel);

pub fn errno_str(e: i64) -> &'static str {
    match -e {
        x if x == abi::ENOENT => "no existe",
        x if x == abi::EIO => "error de E/S",
        x if x == abi::EBADF => "descriptor inválido",
        x if x == abi::ECHILD => "sin hijos",
        x if x == abi::EINTR => "interrumpido",
        x if x == abi::ESRCH => "proceso inexistente",
        x if x == abi::ENOMEM => "sin memoria",
        x if x == abi::EFAULT => "puntero inválido",
        x if x == abi::EEXIST => "ya existe",
        x if x == abi::ENOTDIR => "no es un directorio",
        x if x == abi::EISDIR => "es un directorio",
        x if x == abi::EINVAL => "argumento inválido",
        x if x == abi::EMFILE => "demasiados ficheros abiertos",
        x if x == abi::ENOSPC => "sin espacio",
        x if x == abi::ENAMETOOLONG => "nombre demasiado largo",
        x if x == abi::ENOSYS => "syscall inexistente",
        x if x == abi::ENOTEMPTY => "directorio no vacío",
        x if x == abi::ENOTSUP => "no soportado",
        x if x == abi::ETIMEDOUT => "timeout",
        x if x == abi::ENOTCONN => "sin red",
        // Escribir en una tubería cuyo lector cerró. Faltaba en esta tabla, y
        // sin el nombre la sonda de T33 informaba «error desconocido» de algo
        // que soso hace **bien**: devolver EPIPE en vez de matar al escritor.
        x if x == abi::EPIPE => "la tubería no tiene lector",
        // No es «no tienes permiso»: es que ahora mismo eso no se toca,
        // porque hay una actualización armada sobre esas rutas.
        x if x == abi::EROFS => "hay una actualización en curso; esa ruta no se toca hasta reiniciar",
        x if x == abi::EBUSY => "ocupado",
        _ => "error desconocido",
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic de usuario: {info}");
    sys::exit(101);
}

#[global_allocator]
static ALLOCATOR: soso_alloc::Allocator = soso_alloc::Allocator::new();
