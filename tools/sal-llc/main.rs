//! Arranque de LLVM con std de soso y los adaptadores POSIX existentes.
mod shims;
use std::ffi::{CStr, CString, c_char, c_int};

unsafe extern "C" {
    fn herramienta_main(argc: c_int, argv: *const *const c_char) -> c_int;
    static mut environ: *mut *mut c_char;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sal_llc_message(message: *const c_char, error: c_int) {
    let message = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    if error != 0 { eprintln!("sal-llc: {message}"); } else { println!("{message}"); }
}

fn main() {
    let env: Vec<CString> = std::env::vars().map(|(k, v)| CString::new(format!("{k}={v}")).unwrap()).collect();
    let mut envp: Vec<_> = env.iter().map(|s| s.as_ptr() as *mut c_char).collect();
    envp.push(std::ptr::null_mut());
    unsafe { environ = envp.as_mut_ptr(); }
    let args: Vec<CString> = std::env::args().map(|s| CString::new(s).unwrap()).collect();
    if let Some(exe) = args.first() { *shims::EXE.lock().unwrap() = exe.as_bytes().to_vec(); }
    let mut argv: Vec<_> = args.iter().map(|s| s.as_ptr()).collect();
    argv.push(std::ptr::null());
    let code = unsafe { herramienta_main(args.len() as c_int, argv.as_ptr()) };
    // No glibc exit/stdio: su ruta interna contiene syscalls de Linux.
    std::process::exit(code);
}
