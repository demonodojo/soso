//! Mini-libstd de soso: crt0, wrappers de syscall, print!, panic handler
//! y un allocator global basado en sbrk.

#![no_std]

extern crate alloc;

pub mod sys;
pub mod thread;

pub use soso_abi as abi;

use core::fmt;

// ---- crt0 ----

#[macro_export]
macro_rules! entry {
    ($main:ident) => {
        #[unsafe(no_mangle)]
        extern "C" fn __soso_main(ptr: *const u8, len: usize) -> u8 {
            let args = unsafe {
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len))
            };
            $main(args)
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

pub fn errno_str(e: i64) -> &'static str {
    match -e {
        x if x == abi::ENOENT => "no existe",
        x if x == abi::EIO => "error de E/S",
        x if x == abi::EBADF => "descriptor inválido",
        x if x == abi::ECHILD => "sin hijos",
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
        _ => "error desconocido",
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic de usuario: {info}");
    sys::exit(101);
}

// ---- allocator dinámico: arena por sbrk para pequeño, mmap para grande ----

/// Umbral a partir del cual una reserva va a mmap anónimo (liberable con
/// munmap). Por debajo, bump en el arena — el KV cache y los buffers de
/// inferencia de GBs no caben en la región brk.
const MMAP_ALLOC_MIN: usize = 1024 * 1024;

/// Lo que se le pide al kernel de una vez. Antes se pedía **por asignación**: dos
/// o tres syscalls (`sbrk(0)`, el relleno de alineación y el tamaño) cada vez que
/// alguien hacía un `Vec::push` que crecía. Un arranque de `soso-llm` con el
/// modelo tiny gastaba 7050 syscalls sbrk en eso, y bajo SMP cada una se pelea por
/// el candado de PROCS con los cores ociosos.
const ARENA_CHUNK: usize = 256 * 1024;

struct ArenaState {
    /// Siguiente byte libre del chunk actual.
    cur: usize,
    /// Fin del chunk actual (exclusivo).
    end: usize,
    /// Último bloque servido, para poder deshacerlo o agrandarlo en el sitio. Es
    /// lo que convierte el crecimiento de un `Vec` (asignar, copiar, liberar) en
    /// un avance del cursor sin copia: el bloque que crece casi siempre es el
    /// último que se pidió.
    last_start: usize,
    last_end: usize,
}

struct Arena {
    lock: core::sync::atomic::AtomicBool,
    st: core::cell::UnsafeCell<ArenaState>,
}

/// El arena lo comparten los hilos del proceso (`thread_spawn` comparte el
/// AddrSpace), así que hace falta exclusión propia. Antes la daba el kernel de
/// rebote —cada `sbrk` es atómica—; al dejar de llamarle en el camino rápido, hay
/// que ponerla aquí o dos hilos se reparten el mismo trozo de memoria.
unsafe impl Sync for Arena {}

static ARENA: Arena = Arena {
    lock: core::sync::atomic::AtomicBool::new(false),
    st: core::cell::UnsafeCell::new(ArenaState {
        cur: 0,
        end: 0,
        last_start: 0,
        last_end: 0,
    }),
};

impl Arena {
    fn lock(&self) {
        use core::sync::atomic::Ordering;
        while self
            .lock
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    fn unlock(&self) {
        self.lock.store(false, core::sync::atomic::Ordering::Release);
    }
}

struct SbrkAllocator;

impl SbrkAllocator {
    /// Bump dentro del arena, pidiendo un chunk nuevo si no cabe. El candado ya
    /// está tomado.
    unsafe fn bump(st: &mut ArenaState, size: usize, align: usize) -> *mut u8 {
        let mut start = (st.cur + align - 1) & !(align - 1);
        if start + size > st.end {
            // Un chunk que quepa: si la reserva es mayor que el chunk normal se
            // pide a medida (más el margen de alineación), porque si no el bucle
            // pediría chunks que nunca la admiten.
            let want = if size + align > ARENA_CHUNK {
                size + align
            } else {
                ARENA_CHUNK
            };
            // `sbrk` devuelve el break ANTERIOR, o sea la base de lo que acaba de
            // dar: con eso sobra una sola syscall por chunk. La versión de antes
            // preguntaba con `sbrk(0)` y luego crecía, y eso era la mitad del coste.
            let base = sys::sbrk(want as i64);
            if base < 0 {
                return core::ptr::null_mut();
            }
            st.cur = base as usize;
            st.end = base as usize + want;
            start = (st.cur + align - 1) & !(align - 1);
            if start + size > st.end {
                return core::ptr::null_mut();
            }
        }
        st.cur = start + size;
        st.last_start = start;
        st.last_end = st.cur;
        start as *mut u8
    }
}

unsafe impl core::alloc::GlobalAlloc for SbrkAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let size = layout.size();
        if size >= MMAP_ALLOC_MIN && layout.align() <= 4096 {
            let p = sys::mmap(0, size as u64, u64::MAX, 0);
            if p > 0 {
                return p as *mut u8;
            }
            // si el mmap falla, se intenta por el arena
        }
        let align = layout.align().max(16);
        ARENA.lock();
        let ptr = Self::bump(&mut *ARENA.st.get(), size, align);
        ARENA.unlock();
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        if layout.size() >= MMAP_ALLOC_MIN && layout.align() <= 4096 {
            // Si el alloc grande cayó al arena por fallo de mmap, munmap devuelve
            // EINVAL y se ignora (fuga, como siempre).
            let _ = sys::munmap(ptr as u64, (layout.size() as u64).next_multiple_of(4096));
            return;
        }
        // El arena sigue sin liberar en general (es un bump), pero deshacer el
        // ÚLTIMO bloque es gratis y es el caso que más aparece: liberar lo que se
        // acaba de pedir. Sin esto, un `Vec` que crece deja atrás cada etapa.
        let p = ptr as usize;
        ARENA.lock();
        let st = &mut *ARENA.st.get();
        if p == st.last_start && p + layout.size() == st.last_end && st.cur == st.last_end {
            st.cur = st.last_start;
            st.last_end = st.last_start;
        }
        ARENA.unlock();
    }

    /// Crecer en el sitio cuando el bloque es el último del arena.
    ///
    /// Es el caso de un `Vec` que dobla: sin esto, cada etapa asigna, copia y
    /// abandona la anterior. Con esto, mientras el `Vec` sea lo último que se pidió,
    /// crecer es mover el cursor — ni copia ni memoria abandonada.
    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        let en_arena = layout.size() < MMAP_ALLOC_MIN || layout.align() > 4096;
        let sigue_en_arena = new_size < MMAP_ALLOC_MIN || layout.align() > 4096;
        if en_arena && sigue_en_arena {
            let p = ptr as usize;
            ARENA.lock();
            let st = &mut *ARENA.st.get();
            let es_ultimo =
                p == st.last_start && p + layout.size() == st.last_end && st.cur == st.last_end;
            if es_ultimo && p + new_size <= st.end {
                st.cur = p + new_size;
                st.last_end = st.cur;
                ARENA.unlock();
                return ptr;
            }
            ARENA.unlock();
        }
        // Camino general: reservar, copiar lo que quepa y soltar el viejo.
        let nuevo = core::alloc::Layout::from_size_align_unchecked(new_size, layout.align());
        let dst = self.alloc(nuevo);
        if !dst.is_null() {
            core::ptr::copy_nonoverlapping(ptr, dst, layout.size().min(new_size));
            self.dealloc(ptr, layout);
        }
        dst
    }
}

#[global_allocator]
static ALLOCATOR: SbrkAllocator = SbrkAllocator;
