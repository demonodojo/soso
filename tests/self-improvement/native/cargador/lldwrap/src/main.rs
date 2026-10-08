//! Envoltorio del `lld` de soso: `lldwrap <lld> <opción-de-hilos> <args…>` ejecuta `<lld> <opción-de-hilos>
//! <args…>` y devuelve su código. En soso no hay hilos (`pthread_create` falla) y `lld` crea su
//! pool de hilos al enlazar; hay que pasarle `--threads=1` (`ld.lld`) o `/threads:1` (`lld-link`)
//! siempre. rustc llama al «enlazador» sin esa opción, y `rustflags` por target no siempre llega.
//!
//! Se instala como `ld.lld-soso` y `lld-link-soso` y toma el `lld` y la opción de su propio nombre.

use std::process::{Command, exit};

fn main() {
    let mut args = std::env::args();
    let yo = args.next().unwrap_or_default();
    let nombre = yo.rsplit('/').next().unwrap_or("");
    // Un solo `lld` (73 MB) con `-flavor`: copiarlo con otros nombres tres veces dejó copias con
    // bloques corruptos en la imagen de datos (ver la ficha C-142).
    let lld = "/var/t42/llvm/bin/lld";
    let (flavor, hilos) = if nombre.contains("lld-link") {
        ("link", "/threads:1")
    } else {
        ("gnu", "--threads=1")
    };
    // rustc ya pasa `-flavor link` (primero) cuando el target pide `lld-link`; `lld` exige que
    // `-flavor` vaya el primero, así que se respeta ese orden y la opción de hilos va detrás.
    let mut args: Vec<String> = args.collect();
    let flavor = match args.iter().position(|a| a == "-flavor") {
        Some(p) if p + 1 < args.len() => {
            let v = args.remove(p + 1);
            args.remove(p);
            v
        }
        _ => flavor.to_string(),
    };
    let st = Command::new(lld).arg("-flavor").arg(flavor).arg(hilos).args(&args).status();
    exit(match st {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("lldwrap: no pude lanzar {lld}: {e}");
            127
        }
    });
}
