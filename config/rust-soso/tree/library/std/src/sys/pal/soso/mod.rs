//! PAL soso — basada en `unsupported` + `soso_rt`.

#![deny(unsafe_op_in_unsafe_fn)]

use crate::io;
use crate::os::raw::c_char;
use crate::sys::env;

pub fn unsupported<T>() -> io::Result<T> {
    Err(unsupported_err())
}

pub fn unsupported_err() -> io::Error {
    io::Error::UNSUPPORTED_PLATFORM
}

pub fn abort_internal() -> ! {
    soso_rt::abort();
}

pub unsafe fn init(argc: isize, argv: *const *const u8, _sigpipe: u8) {
    // Antes de cualquier `Vec` de la PAL: un solo montón, el de `soso-alloc`.
    soso_rt::heap_init();
    unsafe {
        crate::sys::args::init(argc, argv);
    }
}

pub unsafe fn cleanup() {}

#[cfg(not(test))]
mod bootstrap {
    use core::ptr;

    use crate::os::raw::c_char;

    const MAGIC: &[u8; 4] = b"SOSA";
    const MAX_ARGC: usize = 256;
    const STR_CAP: usize = 4096;

    /// Bloque TLS mínimo del hilo principal (`%fs:0x28` para stack protector).
    #[repr(C, align(16))]
    struct Tcb {
        propio: *mut Tcb,
        _reservado: [u64; 4],
        canario: u64,
        _cola: [u64; 8],
    }

    const _: () = assert!(core::mem::offset_of!(Tcb, canario) == 0x28);

    static mut TCB: Tcb = Tcb {
        propio: ptr::null_mut(),
        _reservado: [0; 4],
        canario: 0,
        _cola: [0; 8],
    };

    fn tls_init() {
        unsafe {
            let p = &raw mut TCB;
            (*p).propio = p;
            let mut semilla = [0u8; 8];
            let n = soso_rt::syscall3(
                soso_rt::SYS_GETRANDOM,
                semilla.as_mut_ptr().expose_provenance() as u64,
                8,
                0,
            );
            let canario = if n == 8 {
                u64::from_ne_bytes(semilla)
            } else {
                0x00c0_ffee_5050_1234
            };
            (*p).canario = canario & !0xff;
            let _ = soso_rt::syscall1(soso_rt::SYS_SET_TLS, p.expose_provenance() as u64);
        }
    }

    /// Decodifica el blob SOSA del kernel (`kernel/src/task/argv.rs`) en punteros
    /// C sobre un buffer de stack; devuelve `(argc, argv)` o `(0, null)`.
    unsafe fn argv_desde_blob(
        blob: *const u8,
        len: usize,
        str_buf: *mut u8,
        str_cap: usize,
        ptrs: *mut *const u8,
        ptr_cap: usize,
    ) -> (i32, *const *const u8) {
        unsafe {
            if blob.is_null() || len < 8 || len > STR_CAP || ptr_cap == 0 {
                return (0, ptr::null());
            }
            let slice = core::slice::from_raw_parts(blob, len);
            if &slice[..4] != MAGIC {
                return (0, ptr::null());
            }
            let count = u32::from_le_bytes(slice[4..8].try_into().unwrap()) as usize;
            if count == 0 || count > ptr_cap - 1 {
                return (0, ptr::null());
            }
            let mut off = 8usize;
            let mut str_off = 0usize;
            let mut argc = 0usize;
            for _ in 0..count {
                if off + 4 > len {
                    break;
                }
                let slen = u32::from_le_bytes(slice[off..off + 4].try_into().unwrap()) as usize;
                off += 4;
                if off + slen > len || str_off + slen + 1 > str_cap {
                    break;
                }
                let dst = str_buf.add(str_off);
                core::ptr::copy_nonoverlapping(slice[off..off + slen].as_ptr(), dst, slen);
                *dst.add(slen) = 0;
                *ptrs.add(argc) = dst;
                argc += 1;
                str_off += slen + 1;
                off += slen;
            }
            *ptrs.add(argc) = ptr::null();
            (argc as i32, ptrs)
        }
    }

    /// Entrada real del proceso: el kernel pasa el blob SOSA en `rdi`/`rsi`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn soso_entry_from_kernel(blob_ptr: *const u8, blob_len: u64) -> ! {
        tls_init();
        let mut str_buf = [0u8; STR_CAP];
        let mut ptrs = [ptr::null(); MAX_ARGC + 1];
        let len = blob_len as usize;
        let (argc, argv) = unsafe {
            argv_desde_blob(
                blob_ptr,
                len,
                str_buf.as_mut_ptr(),
                STR_CAP,
                ptrs.as_mut_ptr(),
                MAX_ARGC + 1,
            )
        };
        unsafe {
            super::init(argc as isize, argv, 0);
        }
        unsafe extern "C" {
            fn main(argc: isize, argv: *const *const c_char) -> i32;
        }
        let code = unsafe { main(argc as isize, argv as *const *const c_char) };
        soso_rt::exit(code);
    }
}

#[cfg(not(test))]
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text._start")]
pub extern "C" fn _start() -> ! {
    // rdi/rsi ya traen el blob; alinear la pila antes del `call` (SysV).
    core::arch::naked_asm!(
        "xor rbp, rbp",
        "call {entry}",
        entry = sym bootstrap::soso_entry_from_kernel,
    );
}

#[cfg(not(test))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn runtime_entry(
    argc: i32,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> ! {
    unsafe extern "C" {
        fn main(argc: isize, argv: *const *const c_char) -> i32;
    }
    soso_rt::heap_init();
    unsafe { env::init(envp.cast()) };
    let code = unsafe { main(argc as isize, argv) };
    unsafe {
        crate::sys::thread_local::destructors::run();
    }
    crate::rt::thread_cleanup();
    soso_rt::exit(code);
}

#[unsafe(no_mangle)]
pub extern "C" fn __stack_chk_fail() -> ! {
    soso_rt::abort();
}
