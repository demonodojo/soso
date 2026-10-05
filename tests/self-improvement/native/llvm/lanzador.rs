//! `main` de Rust para una herramienta C++ de LLVM (`herramienta_main`): corre el runtime
//! de soso (TLS, argv, pila), publica `environ` y entrega el control. Ver `enlazar.sh`.

mod shims;

use std::ffi::{c_char, c_int, CString};

unsafe extern "C" {
    fn herramienta_main(argc: c_int, argv: *const *const c_char) -> c_int;
    /// `exit` de glibc: ejecuta los destructores estáticos y vuelca los `FILE*`.
    fn exit(code: c_int) -> !;
    static mut environ: *mut *mut c_char;
}

fn fs10() -> usize {
    let v: usize;
    unsafe { std::arch::asm!("mov {}, fs:0x10", out(reg) v, options(nostack, readonly)) };
    v
}

fn main() {
    // glibc estática sin `__libc_start_main`: `environ` está a NULL y `getenv` no ve nada.
    let envp: Vec<CString> = std::env::vars_os()
        .map(|(k, v)| {
            let mut s = k.into_encoded_bytes();
            s.push(b'=');
            s.extend(v.into_encoded_bytes());
            CString::new(s).unwrap()
        })
        .collect();
    let mut penv: Vec<*mut c_char> = envp.iter().map(|c| c.as_ptr() as *mut c_char).collect();
    penv.push(std::ptr::null_mut());
    unsafe { environ = penv.as_mut_ptr() };

    // `--soso-depura` (primer argumento) activa trazas del lanzador; no llega a la herramienta.
    let mut a: Vec<_> = std::env::args_os().collect();
    let depura = a.get(1).is_some_and(|s| s == "--soso-depura");
    if depura {
        a.remove(1);
    }
    let args: Vec<CString> = a.into_iter().map(|s| CString::new(s.into_encoded_bytes()).unwrap()).collect();
    let mut argv: Vec<*const c_char> = args.iter().map(|c| c.as_ptr()).collect();
    argv.push(std::ptr::null());
    if depura {
        eprintln!("lanzador: fs:0x10 antes = {:#x}", fs10());
    }
    let code = unsafe { herramienta_main(args.len() as c_int, argv.as_ptr()) };
    if depura {
        eprintln!("lanzador: fs:0x10 después = {:#x} (código {code})", fs10());
    }
    unsafe { exit(code) }
}
