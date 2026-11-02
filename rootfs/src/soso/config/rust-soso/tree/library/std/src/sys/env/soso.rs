//! Variables de entorno en soso.
//!
//! El kernel entrega `envp` al proceso, así que leerlas es real. **Escribirlas
//! no**: soso no tiene `setenv`, y guardar una tabla aparte en `std` daría un
//! `set_var` que parece funcionar y que los hijos no ven. Se declara el hueco
//! en vez de fingirlo.

pub use super::common::Env;
use crate::ffi::{CStr, OsStr, OsString};
use crate::io;
use crate::os::soso::ffi::OsStrExt;
use crate::os::soso::ffi::OsStringExt;
use crate::ptr;
use crate::sync::atomic::{AtomicPtr, Ordering};

static ENVP: AtomicPtr<*const u8> = AtomicPtr::new(ptr::null_mut());

/// La llama `runtime_entry` de la PAL antes de `main`.
///
/// # Safety
/// `envp` tiene que ser un vector terminado en puntero nulo, de cadenas
/// `CLAVE=VALOR` terminadas en NUL, vivas durante todo el proceso.
pub unsafe fn init(envp: *const *const u8) {
    ENVP.store(envp as *mut _, Ordering::Relaxed);
}

/// Rellena el entorno con el bloque que el kernel guarda para este proceso
/// (`SYS_GETENV` con clave vacía). La ruta de arranque real (`soso_entry_from_kernel`)
/// no recibe `envp`, y sin esto `std::env::var("PATH")` era siempre `None` y un
/// `build.rs` no veía `OUT_DIR` ni los `CARGO_*`.
pub fn cargar_del_kernel() {
    let mut cap = 16 * 1024usize;
    let bloque = loop {
        let mut buf = vec![0u8; cap];
        let n = unsafe {
            soso_rt::syscall4(soso_rt::SYS_GETENV, 0, 0, buf.as_mut_ptr().expose_provenance() as u64, cap as u64)
        };
        if n <= 0 {
            return;
        }
        let n = n as usize;
        if n > cap {
            cap = n;
            continue;
        }
        buf.truncate(n);
        break buf;
    };
    // Cada entrada con su NUL; vive hasta el final del proceso.
    let mut datos: Vec<u8> = Vec::with_capacity(bloque.len() + 1);
    let mut inicios: Vec<usize> = Vec::new();
    for linea in bloque.split(|&b| b == b'\n') {
        if linea.is_empty() || !linea.contains(&b'=') {
            continue;
        }
        inicios.push(datos.len());
        datos.extend_from_slice(linea);
        datos.push(0);
    }
    let datos: &'static mut [u8] = Box::leak(datos.into_boxed_slice());
    let base = datos.as_ptr();
    let mut tabla: Vec<*const u8> = inicios.iter().map(|&i| unsafe { base.add(i) }).collect();
    tabla.push(crate::ptr::null());
    let tabla: &'static mut [*const u8] = Box::leak(tabla.into_boxed_slice());
    ENVP.store(tabla.as_mut_ptr() as *mut _, Ordering::Relaxed);
}

fn cada_entrada(mut f: impl FnMut(&[u8])) {
    let envp = ENVP.load(Ordering::Relaxed);
    if envp.is_null() {
        return;
    }
    let mut i = 0isize;
    loop {
        // SAFETY: `init` recibió un vector terminado en nulo.
        let p = unsafe { *envp.offset(i) };
        if p.is_null() {
            return;
        }
        f(unsafe { CStr::from_ptr(p as *const _) }.to_bytes());
        i += 1;
    }
}

fn partir(entrada: &[u8]) -> Option<(&[u8], &[u8])> {
    // Una entrada sin `=` no es una variable. Se descarta en vez de inventarle
    // un valor vacío, que es lo que haría parecer que existe.
    let pos = entrada.iter().position(|&b| b == b'=')?;
    Some((&entrada[..pos], &entrada[pos + 1..]))
}

pub fn env() -> Env {
    let mut v = Vec::new();
    cada_entrada(|e| {
        if let Some((k, val)) = partir(e) {
            v.push((OsString::from_vec(k.to_vec()), OsString::from_vec(val.to_vec())));
        }
    });
    Env::new(v)
}

pub fn getenv(clave: &OsStr) -> Option<OsString> {
    let clave = clave.as_bytes();
    let mut encontrado = None;
    cada_entrada(|e| {
        if encontrado.is_none()
            && let Some((k, val)) = partir(e)
            && k == clave
        {
            encontrado = Some(OsString::from_vec(val.to_vec()));
        }
    });
    encontrado
}

/// soso no tiene `setenv`. Ver el comentario del módulo.
pub unsafe fn setenv(_: &OsStr, _: &OsStr) -> io::Result<()> {
    Err(io::const_error!(io::ErrorKind::Unsupported, "soso no tiene setenv"))
}

/// soso no tiene `unsetenv`. Ver el comentario del módulo.
pub unsafe fn unsetenv(_: &OsStr) -> io::Result<()> {
    Err(io::const_error!(io::ErrorKind::Unsupported, "soso no tiene unsetenv"))
}
