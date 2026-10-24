//! Adaptador privado del runtime CPU de Sal. Sin libc ni syscalls de Linux.
#![no_std]
#![allow(unsafe_op_in_unsafe_fn)]
extern crate alloc;

use alloc::{alloc::{alloc, dealloc, realloc}, ffi::CString, string::String, vec::Vec};
use core::{alloc::Layout, ffi::{CStr, c_char, c_int}, ptr, slice};
use libsoso::{abi, sys};

unsafe extern "C" {
    #[link_name = "main"]
    fn sal_main(argc: i64, argv: *const *const c_char) -> i64;
}
libsoso::entry!(entrada);
fn entrada(_: &[String]) -> u8 {
    let args: Vec<CString> = libsoso::argv().iter().map(|a| CString::new(a.as_bytes()).unwrap()).collect();
    let mut ptrs: Vec<_> = args.iter().map(|a| a.as_ptr()).collect();
    ptrs.push(ptr::null());
    unsafe { sal_main(args.len() as i64, ptrs.as_ptr()) as u8 }
}

unsafe fn texto<'a>(p: *const c_char) -> &'a str {
    if p.is_null() { return ""; }
    CStr::from_ptr(p).to_str().unwrap_or_else(|_| fallo("ruta o argumento no UTF-8"))
}
fn fallo(s: &str) -> ! {
    let _ = sys::write_all(2, b"sal: ");
    let _ = sys::write_all(2, s.as_bytes());
    let _ = sys::write_all(2, b"\n");
    sys::exit(1)
}
unsafe fn copiar(bytes: &[u8]) -> *mut c_char {
    let p = sal_os_malloc(bytes.len() + 1);
    if p.is_null() { fallo("memoria insuficiente"); }
    ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    *p.add(bytes.len()) = 0;
    p.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_malloc(n: usize) -> *mut u8 {
    let Some(size) = n.max(1).checked_add(16) else { return ptr::null_mut(); };
    let Ok(layout) = Layout::from_size_align(size, 16) else { return ptr::null_mut(); };
    let p = alloc(layout);
    if p.is_null() { return p; }
    p.cast::<usize>().write(size);
    p.add(16)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_free(p: *mut u8) {
    if !p.is_null() {
        let base = p.sub(16);
        dealloc(base, Layout::from_size_align_unchecked(base.cast::<usize>().read(), 16));
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_realloc(p: *mut u8, n: usize) -> *mut u8 {
    if p.is_null() { return sal_os_malloc(n); }
    let Some(size) = n.max(1).checked_add(16) else { return ptr::null_mut(); };
    if Layout::from_size_align(size, 16).is_err() { return ptr::null_mut(); }
    let base = p.sub(16);
    let q = realloc(base, Layout::from_size_align_unchecked(base.cast::<usize>().read(), 16), size);
    if q.is_null() { return q; }
    q.cast::<usize>().write(size);
    q.add(16)
}

#[repr(C)]
pub struct Archivo { fd: i64, error: c_int }
static mut SAL_STDOUT: Archivo = Archivo { fd: 1, error: 0 };
static mut SAL_STDERR: Archivo = Archivo { fd: 2, error: 0 };
#[unsafe(no_mangle)]
pub static mut sal_os_stdout: *mut Archivo = &raw mut SAL_STDOUT;
#[unsafe(no_mangle)]
pub static mut sal_os_stderr: *mut Archivo = &raw mut SAL_STDERR;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_fopen(p: *const c_char, mode: *const c_char) -> *mut Archivo {
    let flags = if texto(mode).starts_with('w') { abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC } else { 0 };
    if flags == 0 {
        let mut st = abi::Stat::default();
        if sys::stat(texto(p), &mut st) < 0 || st.file_type != abi::FT_FILE { return ptr::null_mut(); }
    }
    let fd = sys::open(texto(p), flags);
    if fd < 0 { return ptr::null_mut(); }
    let f = sal_os_malloc(core::mem::size_of::<Archivo>()).cast::<Archivo>();
    if f.is_null() { sys::close(fd as u64); return f; }
    f.write(Archivo { fd, error: 0 }); f
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_fclose(f: *mut Archivo) -> c_int {
    let result = sys::close((*f).fd as u64);
    let failed = result < 0 || (*f).error != 0;
    sal_os_free(f.cast());
    if failed { -1 } else { 0 }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_fread(p: *mut u8, size: usize, count: usize, f: *mut Archivo) -> usize {
    let Some(n) = size.checked_mul(count) else { (*f).error = 1; return 0; };
    if n == 0 { return 0; }
    let buf = slice::from_raw_parts_mut(p, n);
    let mut got = 0;
    while got < n {
        let k = sys::read((*f).fd as u64, &mut buf[got..]);
        if k < 0 { (*f).error = 1; fallo("error de lectura"); }
        if k == 0 { break; }
        got += k as usize;
    }
    got / size
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_fwrite(p: *const u8, size: usize, count: usize, f: *mut Archivo) -> usize {
    let Some(n) = size.checked_mul(count) else { (*f).error = 1; return 0; };
    if n == 0 { return 0; }
    if sys::write_all((*f).fd as u64, slice::from_raw_parts(p, n)).is_err() { (*f).error = 1; 0 } else { count }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_fseek(f: *mut Archivo, offset: i64, whence: c_int) -> c_int {
    if sys::seek((*f).fd as u64, offset, whence as u64) < 0 { (*f).error = 1; -1 } else { 0 }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_ftell(f: *mut Archivo) -> i64 { sys::seek((*f).fd as u64, 0, 1) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_fflush(f: *mut Archivo) -> c_int { if (*f).error == 0 { 0 } else { -1 } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_access(p: *const c_char, _: c_int) -> c_int {
    let mut st = abi::Stat::default();
    if sys::stat(texto(p), &mut st) < 0 || st.file_type != abi::FT_FILE { return -1; }
    let fd = sys::open(texto(p), 0);
    if fd < 0 { -1 } else { sys::close(fd as u64); 0 }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_stat(p: *const c_char, mode: *mut u64) -> c_int {
    let mut st = abi::Stat::default();
    if sys::stat(texto(p), &mut st) < 0 { -1 } else { mode.write(st.file_type as u64); 0 }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_mkdir(p: *const c_char, _: c_int) -> c_int { sys::mkdir(texto(p)) as c_int }
#[unsafe(no_mangle)]
pub extern "C" fn sal_os_getpid() -> c_int { sys::getpid() as c_int }
#[unsafe(no_mangle)]
pub extern "C" fn sal_os_exit(code: c_int) -> ! { sys::exit(code as u8) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_getenv(p: *const c_char) -> *mut c_char {
    let mut buf = [0u8; 8192];
    let n = sys::getenv(texto(p), &mut buf);
    if n < 0 || n as usize > buf.len() { return ptr::null_mut(); }
    // C getenv storage lives until exit; Sal copies it into its own string.
    copiar(&buf[..n as usize])
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_realpath(p: *const c_char) -> *mut c_char {
    let p = texto(p);
    let mut cwd = [0u8; 4096];
    let mut path = String::new();
    if !p.starts_with('/') {
        let n = sys::getcwd(&mut cwd);
        if n < 0 { fallo("no se puede leer el directorio actual"); }
        path.push_str(core::str::from_utf8(&cwd[..n as usize]).unwrap());
        path.push('/');
    }
    path.push_str(p);
    let mut parts = Vec::new();
    for c in path.split('/') {
        match c { "" | "." => {}, ".." => { parts.pop(); }, _ => parts.push(c) }
    }
    copiar(alloc::format!("/{}", parts.join("/")).as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_strtod(p: *const c_char, end: *mut *const c_char) -> f64 {
    let s = texto(p);
    // Runtime callers pass a complete numeric token and no end pointer.
    let v = s.parse::<f64>().unwrap_or_else(|_| fallo("número real inválido"));
    if !end.is_null() { end.write(p.add(s.len())); }
    v
}
#[unsafe(no_mangle)]
pub extern "C" fn expf(x: f32) -> f32 { libm::expf(x) }

fn lanzar(bin: &str, argv: &[&str], stdio: [u64; 3]) -> i64 {
    // spawn transfiere fds: reservar ranuras libres y duplicar antes de lanzarlo.
    // El runtime v1 es monohilo; ningún open puede competir con esta reserva.
    let mut copias = Vec::new();
    for fd in stdio {
        let slot = sys::open("/", 0);
        if slot < 0 || sys::dup2(fd, slot as u64) < 0 {
            if slot >= 0 { sys::close(slot as u64); }
            for slot in copias { sys::close(slot); }
            return -1;
        }
        copias.push(slot as u64);
    }
    let pid = sys::spawn_io_ex(bin, argv, &[], copias[0], copias[1], copias[2]);
    for slot in copias { sys::close(slot); }
    pid
}

fn esperar(pid: i64) -> i64 {
    if pid < 0 { return 1; }
    loop { match sys::wait() {
        Ok((p, code)) if p == pid as u64 => return code as i64,
        Ok(_) => {}, Err(_) => return 1,
    }}
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_spawn(bin: *const c_char, args: *const *const c_char, n: usize) -> i64 {
    let argv: Vec<&str> = slice::from_raw_parts(args, n).iter().map(|p| texto(*p)).collect();
    esperar(lanzar(texto(bin), &argv, [0, 1, 2]))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_capture(bin: *const c_char, arg: *const c_char) -> *mut c_char {
    let bin = texto(bin);
    // The legacy API takes a whitespace-separated argument line, not shell syntax.
    let mut argv = alloc::vec![bin];
    argv.extend(texto(arg).split_whitespace());
    let (r, w) = sys::pipe().unwrap_or_else(|_| fallo("no se puede abrir pipe"));
    let pid = lanzar(bin, &argv, [0, w, 2]);
    sys::close(w);
    if pid < 0 { sys::close(r); fallo("no se puede ejecutar el proceso"); }
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = sys::read(r, &mut buf);
        if n < 0 { fallo("error de lectura del proceso"); }
        if n == 0 { break; }
        out.extend_from_slice(&buf[..n as usize]);
    }
    sys::close(r);
    if esperar(pid) != 0 { fallo("el proceso terminó con error"); }
    copiar(&out)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_clock_gettime(id: c_int, out: *mut abi::Timespec) -> c_int {
    sys::clock_gettime(id as u64, &mut *out) as c_int
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_os_format_f64(value: f64, out: *mut u8, cap: usize) -> usize {
    let mut s = alloc::format!("{value}");
    if s.len() > cap { s = alloc::format!("{value:e}"); }
    if s.len() > cap { fallo("desbordamiento al formatear un real"); }
    ptr::copy_nonoverlapping(s.as_ptr(), out, s.len()); s.len()
}
#[unsafe(no_mangle)]
pub extern "C" fn sqrt(x: f64) -> f64 { libm::sqrt(x) }
#[unsafe(no_mangle)]
pub extern "C" fn pow(x: f64, y: f64) -> f64 { libm::pow(x, y) }
#[unsafe(no_mangle)]
pub extern "C" fn sin(x: f64) -> f64 { libm::sin(x) }
#[unsafe(no_mangle)]
pub extern "C" fn cos(x: f64) -> f64 { libm::cos(x) }
