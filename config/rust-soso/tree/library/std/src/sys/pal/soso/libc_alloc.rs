//! `malloc` y compañía sobre el alocador de soso.
//!
//! El bin `cargo` enlaza `libc.a` del host (OpenSSL, curl, libgcc_eh). El
//! `malloc` de glibc pide memoria con syscalls de Linux, que soso no habla, y
//! devuelve NULL: `__register_frame` escribía en `0x18`. Estos símbolos hacen
//! que `libc.a` no arrastre su `malloc.o`.
//!
//! Son símbolos débiles: `rustc` también enlaza la glibc (por LLVM) y allí
//! manda la suya; sólo `cargo`, que no la necesita para nada más, usa éstas.
//!
//! Cada bloque lleva delante una cabecera de `align` bytes (mínimo 16) cuyos
//! últimos 16 guardan el tamaño total y la alineación.

use crate::alloc::Layout;
use crate::ptr;

const ALIGN_MIN: usize = 16;

unsafe fn reservar(size: usize, align: usize, a_cero: bool) -> *mut u8 {
    let align = align.max(ALIGN_MIN);
    if !align.is_power_of_two() {
        return ptr::null_mut();
    }
    let Some(total) = size.checked_add(align) else { return ptr::null_mut() };
    let Ok(layout) = Layout::from_size_align(total, align) else { return ptr::null_mut() };
    unsafe {
        let base = soso_rt::alloc(layout);
        if base.is_null() {
            return ptr::null_mut();
        }
        let user = base.add(align);
        if a_cero {
            ptr::write_bytes(user, 0, size);
        }
        user.sub(16).cast::<usize>().write(total);
        user.sub(8).cast::<usize>().write(align);
        user
    }
}

unsafe fn cabecera(p: *mut u8) -> (usize, usize) {
    unsafe { (p.sub(16).cast::<usize>().read(), p.sub(8).cast::<usize>().read()) }
}

unsafe fn liberar(p: *mut u8) {
    if p.is_null() {
        return;
    }
    unsafe {
        let (total, align) = cabecera(p);
        soso_rt::dealloc(p.sub(align), Layout::from_size_align_unchecked(total, align));
    }
}

unsafe fn redimensionar(p: *mut u8, n: usize) -> *mut u8 {
    if p.is_null() {
        return unsafe { reservar(n, ALIGN_MIN, false) };
    }
    unsafe {
        let (total, align) = cabecera(p);
        let viejo = total - align;
        let nuevo = reservar(n, align, false);
        if nuevo.is_null() {
            return nuevo;
        }
        ptr::copy_nonoverlapping(p, nuevo, viejo.min(n));
        liberar(p);
        nuevo
    }
}

macro_rules! libc_alloc {
    ($($malloc:ident $free:ident $calloc:ident $realloc:ident $memalign:ident;)*) => {$(
        #[linkage = "weak"]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $malloc(n: usize) -> *mut u8 {
            unsafe { reservar(n, ALIGN_MIN, false) }
        }
        #[linkage = "weak"]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $free(p: *mut u8) {
            unsafe { liberar(p) }
        }
        #[linkage = "weak"]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $calloc(n: usize, m: usize) -> *mut u8 {
            match n.checked_mul(m) {
                Some(t) => unsafe { reservar(t, ALIGN_MIN, true) },
                None => ptr::null_mut(),
            }
        }
        #[linkage = "weak"]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $realloc(p: *mut u8, n: usize) -> *mut u8 {
            unsafe { redimensionar(p, n) }
        }
        #[linkage = "weak"]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $memalign(align: usize, n: usize) -> *mut u8 {
            unsafe { reservar(n, align, false) }
        }
    )*};
}

libc_alloc! {
    malloc free calloc realloc memalign;
    __libc_malloc __libc_free __libc_calloc __libc_realloc __libc_memalign;
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aligned_alloc(align: usize, n: usize) -> *mut u8 {
    unsafe { reservar(n, align, false) }
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_memalign(out: *mut *mut u8, align: usize, n: usize) -> i32 {
    if !align.is_power_of_two() || align < size_of::<usize>() {
        return 22; // EINVAL
    }
    let p = unsafe { reservar(n, align, false) };
    if p.is_null() {
        return 12; // ENOMEM
    }
    unsafe { out.write(p) };
    0
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn malloc_usable_size(p: *mut u8) -> usize {
    if p.is_null() {
        return 0;
    }
    let (total, align) = unsafe { cabecera(p) };
    total - align
}
