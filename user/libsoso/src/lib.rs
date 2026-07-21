//! Mini-libstd de soso: crt0, wrappers de syscall, print!, panic handler
//! y un allocator global basado en sbrk.

#![no_std]

extern crate alloc;

pub mod sys;

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
        sys::write(1, s.as_bytes());
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

// ---- allocator dinámico: sbrk para pequeño, mmap anónimo para grande ----

/// Umbral a partir del cual una reserva va a mmap anónimo (liberable con
/// munmap). Por debajo, bump por sbrk sin liberación — el KV cache y los
/// buffers de inferencia de GBs no caben en la región brk.
const MMAP_ALLOC_MIN: usize = 1024 * 1024;

struct SbrkAllocator;

unsafe impl core::alloc::GlobalAlloc for SbrkAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let size = layout.size();
        if size >= MMAP_ALLOC_MIN && layout.align() <= 4096 {
            let p = sys::mmap(0, size as u64, u64::MAX, 0);
            if p > 0 {
                return p as *mut u8;
            }
            // si el mmap falla, se intenta por sbrk
        }
        let align = layout.align().max(16);
        let cur = sys::sbrk(0);
        if cur < 0 {
            return core::ptr::null_mut();
        }
        let mut start = cur as usize;
        let rem = start % align;
        if rem != 0 {
            let pad = align - rem;
            if sys::sbrk(pad as i64) < 0 {
                return core::ptr::null_mut();
            }
            start += pad;
        }
        let end = sys::sbrk(size as i64);
        if end < 0 {
            return core::ptr::null_mut();
        }
        start as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        // Solo se liberan los bloques mmap. Si el alloc grande cayó a sbrk
        // por fallo de mmap, munmap devuelve EINVAL y se ignora (fuga, como
        // siempre fue en sbrk).
        if layout.size() >= MMAP_ALLOC_MIN && layout.align() <= 4096 {
            let _ = sys::munmap(ptr as u64, (layout.size() as u64).next_multiple_of(4096));
        }
    }
}

#[global_allocator]
static ALLOCATOR: SbrkAllocator = SbrkAllocator;
