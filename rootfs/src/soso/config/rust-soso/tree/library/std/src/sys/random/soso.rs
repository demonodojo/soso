//! Aleatoriedad en soso: `SYS_GETRANDOM`, con `rdrand` detrás en el kernel.
//!
//! El kernel llena el buffer **entero** o falla: no hay lecturas parciales
//! (`kernel/src/task/syscall.rs`, `sys_getrandom`). Aun así se comprueba el
//! número devuelto, porque confiar en esa propiedad sin mirarla es cómo se
//! cuelan los cambios de contrato.
//!
//! Si falla, esto entra en pánico. Es deliberado: `fill_bytes` alimenta, entre
//! otras cosas, las semillas de `HashMap`, y devolver ceros en silencio sería
//! una mentira con consecuencias de seguridad. Un fallo ruidoso es peor de
//! sufrir y mejor de diagnosticar.

pub fn fill_bytes(bytes: &mut [u8]) {
    if bytes.is_empty() {
        return;
    }
    // SAFETY: `bytes` es un slice válido y exclusivo; el kernel sólo escribe
    // dentro de [ptr, ptr+len) y valida el rango contra el espacio del proceso.
    let n = unsafe {
        soso_rt::syscall3(
            soso_rt::SYS_GETRANDOM,
            bytes.as_mut_ptr().expose_provenance() as u64,
            bytes.len() as u64,
            0,
        )
    };
    if n < 0 {
        panic!("soso: getrandom falló con errno {}", -n);
    }
    if n as usize != bytes.len() {
        panic!("soso: getrandom llenó {} de {} bytes", n, bytes.len());
    }
}
