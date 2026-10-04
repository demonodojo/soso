//! PAL soso — basada en `unsupported` + `soso_rt`.

#![deny(unsafe_op_in_unsafe_fn)]

use crate::io;
use crate::os::raw::c_char;
use crate::sys::env;

#[cfg(not(test))]
pub mod dl;
#[cfg(not(test))]
mod libc_alloc;
#[cfg(not(test))]
mod libc_tls;
#[cfg(not(test))]
mod libc_time;
#[cfg(not(test))]
mod libc_sys;

pub fn unsupported<T>() -> io::Result<T> {
    Err(unsupported_err())
}

pub fn unsupported_err() -> io::Error {
    io::Error::UNSUPPORTED_PLATFORM
}

pub fn abort_internal() -> ! {
    soso_rt::abort();
}

/// Imagen TLS nueva para un hilo que no es el principal. Hay que llamarla
/// antes de tocar thread-locals: si no, `%fs` sigue siendo el del padre.
#[cfg(not(test))]
pub fn tls_hilo_nuevo() {
    bootstrap::tls_para_hilo();
    // glibc inicializa las tablas `ctype` por hilo (`start_thread`): un hilo
    // nuevo las tiene a cero y `isalpha` de LLVM moría en `0x5c`.
    iniciar_ctype();
}

pub unsafe fn init(argc: isize, argv: *const *const u8, _sigpipe: u8) {
    // Antes de cualquier `Vec` de la PAL: un solo montón, el de `soso-alloc`.
    soso_rt::heap_init();
    #[cfg(not(test))]
    iniciar_ctype();
    #[cfg(not(test))]
    env::cargar_del_kernel();
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
    // Igual que el kernel (`read_spawn_args`): cargo pasa a rustc líneas largas.
    const MAX_ARGC: usize = 4096;
    const STR_CAP: usize = 128 * 1024;

    /// Bloque TLS mínimo del hilo principal (`%fs:0x28` para stack protector).
    /// glibc lee el tid del hilo en `%fs:0x2d0`. Si ese entero es 0, un
    /// `pthread_rwlock` sin dueño (también 0) devuelve EDEADLK al instante.
    ///
    /// El código C de glibc enlazado estáticamente (bin `cargo`) espera además
    /// que `%fs:0x10` apunte a su `struct pthread`, que empieza en este mismo
    /// bloque (stdio, `pthread_self`, cancelación): sin él, page fault en
    /// `0x308` dentro de `__libc_cleanup_push_defer`. El resto va a ceros.
    #[repr(C, align(16))]
    struct Tcb {
        propio: *mut Tcb,
        _dtv: u64,
        hilo: *mut Tcb,
        _reservado: [u64; 2],
        canario: u64,
        _hasta_tid: [u8; 0x2d0 - 0x30],
        tid: u32,
        _cola: [u8; 0xa00 - 0x2d4],
    }

    const _: () = assert!(core::mem::offset_of!(Tcb, canario) == 0x28);
    const _: () = assert!(core::mem::offset_of!(Tcb, tid) == 0x2d0);
    const _: () = assert!(core::mem::offset_of!(Tcb, hilo) == 0x10);

    static mut TCB: Tcb = Tcb {
        propio: ptr::null_mut(),
        _dtv: 0,
        hilo: ptr::null_mut(),
        _reservado: [0; 2],
        canario: 0,
        _hasta_tid: [0; 0x2d0 - 0x30],
        tid: 0,
        _cola: [0; 0xa00 - 0x2d4],
    };

    fn tid_hilo() -> u32 {
        let pid = unsafe { soso_rt::syscall0(soso_rt::SYS_GETPID) };
        if pid <= 0 { 1 } else { pid as u32 }
    }

    fn canario() -> u64 {
        let mut semilla = [0u8; 8];
        let n = unsafe {
            soso_rt::syscall3(
                soso_rt::SYS_GETRANDOM,
                semilla.as_mut_ptr().expose_provenance() as u64,
                8,
                0,
            )
        };
        let canario = if n == 8 {
            u64::from_ne_bytes(semilla)
        } else {
            0x00c0_ffee_5050_1234
        };
        canario & !0xff
    }

    fn tls_minimo() {
        unsafe {
            let p = &raw mut TCB;
            (*p).propio = p;
            (*p).hilo = p;
            (*p).canario = canario();
            (*p).tid = tid_hilo();
            let _ = soso_rt::syscall1(soso_rt::SYS_SET_TLS, p.expose_provenance() as u64);
        }
    }

    /// Plantilla `PT_TLS` del ejecutable ya cargado. glibc lee el locale en un
    /// desplazamiento negativo desde `%fs`, así que la imagen tiene que quedar
    /// justo delante del bloque y `%fs:0` tiene que ser ese bloque.
    fn tls_plantilla() -> Option<(*const u8, u64, u64, u64)> {
        unsafe extern "C" {
            static __ehdr_start: u8;
        }
        unsafe {
            let ehdr = &raw const __ehdr_start;
            let phoff = ptr::read_unaligned(ehdr.byte_add(32).cast::<u64>());
            let phentsize = ptr::read_unaligned(ehdr.byte_add(54).cast::<u16>());
            let phnum = ptr::read_unaligned(ehdr.byte_add(56).cast::<u16>());
            if phentsize < 56 || phnum == 0 {
                return None;
            }
            let phdrs = ehdr.byte_add(phoff as usize);
            for i in 0..phnum as usize {
                let ph = phdrs.byte_add(i * phentsize as usize);
                let ty = ptr::read_unaligned(ph.cast::<u32>());
                if ty != 7 {
                    continue;
                }
                let vaddr = ptr::read_unaligned(ph.byte_add(16).cast::<u64>());
                let filesz = ptr::read_unaligned(ph.byte_add(32).cast::<u64>());
                let memsz = ptr::read_unaligned(ph.byte_add(40).cast::<u64>());
                let align = ptr::read_unaligned(ph.byte_add(48).cast::<u64>());
                if memsz == 0 || filesz > memsz {
                    return None;
                }
                return Some((
                    ptr::with_exposed_provenance(vaddr as usize),
                    filesz,
                    memsz,
                    align.max(8),
                ));
            }
            None
        }
    }

    fn tls_init() {
        instalar_tls(true);
    }

    /// TLS de un hilo que no es el principal. No usa el `TCB` estático:
    /// ese es el del padre.
    pub(super) fn tls_para_hilo() {
        instalar_tls(false);
    }

    fn tls_minimo_mapa() {
        let len = core::mem::size_of::<Tcb>() as u64;
        let raw = unsafe { soso_rt::syscall4(soso_rt::SYS_MMAP, 0, len, u64::MAX, 0) };
        if raw <= 0 {
            return;
        }
        unsafe {
            let tcb = ptr::with_exposed_provenance_mut::<Tcb>(raw as usize);
            ptr::write_bytes(tcb.cast::<u8>(), 0, core::mem::size_of::<Tcb>());
            (*tcb).propio = tcb;
            (*tcb).hilo = tcb;
            (*tcb).canario = canario();
            (*tcb).tid = tid_hilo();
            let _ = soso_rt::syscall1(soso_rt::SYS_SET_TLS, tcb.expose_provenance() as u64);
        }
    }

    fn instalar_tls(estatico: bool) {
        let Some((src, filesz, memsz, align)) = tls_plantilla() else {
            if estatico {
                tls_minimo();
            } else {
                tls_minimo_mapa();
            }
            return;
        };
        let tcb_off = memsz.next_multiple_of(align);
        super::dl::TLS_PROPIO.store(tcb_off as usize, core::sync::atomic::Ordering::Relaxed);
        // T80: bajo el TCB se reserva sitio para los objetos compartidos
        // (`dl.rs`); el bloque propio sigue terminando justo en el TP.
        let bajo = tcb_off.max(super::dl::TLS_RESERVA as u64).next_multiple_of(align);
        let total = bajo + core::mem::size_of::<Tcb>() as u64;
        let map_len = total + align;
        let raw = unsafe { soso_rt::syscall4(soso_rt::SYS_MMAP, 0, map_len, u64::MAX, 0) };
        if raw <= 0 {
            if estatico {
                tls_minimo();
            } else {
                tls_minimo_mapa();
            }
            return;
        }
        unsafe {
            let base = (raw as u64).next_multiple_of(align);
            let dst = ptr::with_exposed_provenance_mut::<u8>((base + bajo - tcb_off) as usize);
            ptr::copy_nonoverlapping(src, dst, filesz as usize);
            if memsz > filesz {
                ptr::write_bytes(dst.add(filesz as usize), 0, (memsz - filesz) as usize);
            }
            let tcb = ptr::with_exposed_provenance_mut::<Tcb>((base + bajo) as usize);
            ptr::write_bytes(tcb.cast::<u8>(), 0, core::mem::size_of::<Tcb>());
            (*tcb).propio = tcb;
            (*tcb).hilo = tcb;
            (*tcb).canario = canario();
            (*tcb).tid = tid_hilo();
            let _ = soso_rt::syscall1(soso_rt::SYS_SET_TLS, tcb.expose_provenance() as u64);
            // Objetos compartidos ya cargados: su plantilla TLS en este hilo.
            super::dl::tls_instalar(tcb.expose_provenance());
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

    /// `R_X86_64_IRELATIVE`: el hueco de `memchr` y el resto queda a 0 hasta que
    /// se llama al resolvedor. Sin el `_start` de glibc nadie lo hace, y la
    /// primera llamada salta a la dirección 0.
    fn aplicar_ifunc() {
        const SHT_RELA: u32 = 4;
        const R_X86_64_IRELATIVE: u64 = 37;
        unsafe extern "C" {
            static __ehdr_start: u8;
        }
        unsafe {
            let ehdr = &raw const __ehdr_start;
            let shoff = ptr::read_unaligned(ehdr.byte_add(40).cast::<u64>());
            let shentsize = ptr::read_unaligned(ehdr.byte_add(58).cast::<u16>());
            let shnum = ptr::read_unaligned(ehdr.byte_add(60).cast::<u16>());
            if shoff == 0 || shentsize < 64 || shnum == 0 {
                return;
            }
            let shdrs = ehdr.byte_add(shoff as usize);
            for i in 0..shnum as usize {
                let sh = shdrs.byte_add(i * shentsize as usize);
                let ty = ptr::read_unaligned(sh.byte_add(4).cast::<u32>());
                if ty != SHT_RELA {
                    continue;
                }
                let addr = ptr::read_unaligned(sh.byte_add(16).cast::<u64>());
                let size = ptr::read_unaligned(sh.byte_add(32).cast::<u64>());
                let entsize = ptr::read_unaligned(sh.byte_add(56).cast::<u64>());
                if addr == 0 || entsize < 24 || size < entsize {
                    continue;
                }
                let n = size / entsize;
                let base = ptr::with_exposed_provenance::<u8>(addr as usize);
                for j in 0..n {
                    let ent = base.add((j * entsize) as usize);
                    let r_offset = ptr::read_unaligned(ent.cast::<u64>());
                    let r_info = ptr::read_unaligned(ent.byte_add(8).cast::<u64>());
                    let r_addend = ptr::read_unaligned(ent.byte_add(16).cast::<u64>());
                    if r_info & 0xffff_ffff != R_X86_64_IRELATIVE || r_addend == 0 {
                        continue;
                    }
                    let resolver = core::mem::transmute::<usize, extern "C" fn() -> usize>(
                        r_addend as usize,
                    );
                    let fp = resolver();
                    let slot = ptr::with_exposed_provenance_mut::<usize>(r_offset as usize);
                    *slot = fp;
                }
            }
        }
    }

    /// `PT_GNU_EH_FRAME` apunta a `.eh_frame_hdr`. `crtbegin` ya no llama a
    /// `__register_frame`: libgcc busca el FDE en `registered_frames` y, si
    /// está vacío, en `_dl_find_object`. Esta entrada no arranca el
    /// enlazador dinámico, así que sin el registro el desenrollado aborta.
    fn registrar_eh_frame() {
        const PT_GNU_EH_FRAME: u32 = 0x6474_e550;
        const DW_EH_PE_PCREL_SDATA4: u8 = 0x1b;
        unsafe extern "C" {
            #[linkage = "extern_weak"]
            fn __register_frame(begin: *const u8);
            static __ehdr_start: u8;
        }
        if (__register_frame as *const ()).addr() == 0 {
            return;
        }
        unsafe {
            let ehdr = &raw const __ehdr_start;
            let phoff = ptr::read_unaligned(ehdr.byte_add(32).cast::<u64>());
            let phentsize = ptr::read_unaligned(ehdr.byte_add(54).cast::<u16>());
            let phnum = ptr::read_unaligned(ehdr.byte_add(56).cast::<u16>());
            if phoff == 0 || phentsize < 56 || phnum == 0 {
                return;
            }
            let phdrs = ehdr.byte_add(phoff as usize);
            for i in 0..phnum as usize {
                let ph = phdrs.byte_add(i * phentsize as usize);
                let ty = ptr::read_unaligned(ph.cast::<u32>());
                if ty != PT_GNU_EH_FRAME {
                    continue;
                }
                let vaddr = ptr::read_unaligned(ph.byte_add(16).cast::<u64>());
                if vaddr == 0 {
                    return;
                }
                let hdr = ptr::with_exposed_provenance::<u8>(vaddr as usize);
                if ptr::read_unaligned(hdr) != 1
                    || ptr::read_unaligned(hdr.add(1)) != DW_EH_PE_PCREL_SDATA4
                {
                    return;
                }
                let rel = ptr::read_unaligned(hdr.add(4).cast::<i32>());
                let eh = vaddr.wrapping_add(4).wrapping_add(rel as u64);
                let primera = ptr::read_unaligned(ptr::with_exposed_provenance::<u32>(eh as usize));
                if eh == 0 || primera == 0 {
                    return;
                }
                __register_frame(ptr::with_exposed_provenance(eh as usize));
                return;
            }
        }
    }

    /// `.init_array`: constructores de C++ y de `crtbegin`.
    fn aplicar_init() {
        const SHT_INIT_ARRAY: u32 = 14;
        unsafe extern "C" {
            static __ehdr_start: u8;
        }
        unsafe {
            let ehdr = &raw const __ehdr_start;
            let shoff = ptr::read_unaligned(ehdr.byte_add(40).cast::<u64>());
            let shentsize = ptr::read_unaligned(ehdr.byte_add(58).cast::<u16>());
            let shnum = ptr::read_unaligned(ehdr.byte_add(60).cast::<u16>());
            if shoff == 0 || shentsize < 64 || shnum == 0 {
                return;
            }
            let shdrs = ehdr.byte_add(shoff as usize);
            for i in 0..shnum as usize {
                let sh = shdrs.byte_add(i * shentsize as usize);
                let ty = ptr::read_unaligned(sh.byte_add(4).cast::<u32>());
                if ty != SHT_INIT_ARRAY {
                    continue;
                }
                let addr = ptr::read_unaligned(sh.byte_add(16).cast::<u64>());
                let size = ptr::read_unaligned(sh.byte_add(32).cast::<u64>());
                if addr == 0 || size < 8 {
                    continue;
                }
                let n = size / 8;
                let base = ptr::with_exposed_provenance::<u64>(addr as usize);
                for j in 0..n {
                    let fp = ptr::read_unaligned(base.add(j as usize));
                    if fp == 0 {
                        continue;
                    }
                    let ctor = core::mem::transmute::<usize, extern "C" fn()>(fp as usize);
                    ctor();
                }
            }
        }
    }

    /// Entrada real del proceso: el kernel pasa el blob SOSA en `rdi`/`rsi`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn soso_entry_from_kernel(blob_ptr: *const u8, blob_len: u64) -> ! {
        tls_init();
        aplicar_ifunc();
        registrar_eh_frame();
        aplicar_init();
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
        soso_rt::exit_group(code);
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
    soso_rt::exit_group(code);
}

/// `__ctype_b_loc` & co. leen unos TLS que rellena `__ctype_init` de glibc en su
/// arranque, que aquí no corre: tabla nula y page fault en `0xc6` dentro de
/// `isspace` de libgit2. Sólo existe en los binarios que enlazan `libc.a`.
#[cfg(not(test))]
fn iniciar_ctype() {
    // Un `fn` weak se da por no nulo y LLVM borra la comprobación: `Option`.
    unsafe extern "C" {
        #[linkage = "extern_weak"]
        static __ctype_init: Option<unsafe extern "C" fn()>;
    }
    if let Some(init) = unsafe { __ctype_init } {
        unsafe { init() };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __stack_chk_fail() -> ! {
    soso_rt::abort();
}
