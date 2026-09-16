//! Ring de eventos de actualización (`/var/log/actualizaciones.log`).
//!
//! Separado de `applog` a propósito: aquí van los hechos del **kernel** sobre
//! una actualización —quién escribió el hueco del kernel, con qué tamaño, qué
//! decía el buzón al arrancar—, que son justo los que hacen falta al recuperar
//! y los que no se pueden reconstruir después. Lo que cuenta `soso-update` de
//! cara al usuario sigue yendo por fd 3 al ring de aplicaciones.

use core::fmt::Write;
use spin::Mutex;

use super::logbuf::{Lectura, Ring};
use soso_log_core::texto::EscritorSlice;

/// Pocos eventos y muy espaciados: con 8 KiB sobra para varios arranques.
const CAP: usize = 8 * 1024;

static BUF: Mutex<Ring<CAP>> = Mutex::new(Ring::new());

/// Registra un evento con sello monótono. Pensado para llamarse con
/// `format_args!`, sin reservar memoria.
pub fn evento(args: core::fmt::Arguments<'_>) {
    let ns = crate::time::monotonic_ns();
    let mut linea = [0u8; 256];
    let mut w = EscritorSlice::new(&mut linea);
    let _ = write!(w, "[{:>8}.{:03}] ", ns / 1_000_000_000, (ns % 1_000_000_000) / 1_000_000);
    let _ = w.write_fmt(args);
    let n = w.len();
    let mut buf = BUF.lock();
    buf.append(&linea[..n]);
    buf.append(b"\n");
}

#[macro_export]
macro_rules! otalog {
    ($($arg:tt)*) => {
        $crate::drivers::otalog::evento(format_args!($($arg)*))
    };
}

pub fn escritos() -> u64 {
    BUF.lock().escritos()
}

pub fn cursor_minimo() -> u64 {
    BUF.lock().cursor_minimo()
}

pub fn leer_desde(cursor: u64, out: &mut [u8]) -> Lectura {
    BUF.lock().leer_desde(cursor, out)
}
