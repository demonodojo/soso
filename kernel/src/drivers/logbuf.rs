//! Ring buffer de consola (dmesg).
//!
//! El framebuffer solo muestra ~40 filas; al arrancar (USB, WiFi, PCI…) las
//! trazas tempranas se pierden por scroll. Este buffer guarda los últimos
//! bytes escritos por `print!`/`println!`/`serial::write_bytes` para
//! recuperarlos con el comando `dmesg` del kshell.

use core::sync::atomic::{AtomicBool, Ordering};
use spin::Mutex;

pub use soso_log_core::ring::{Lectura, Ring};

/// ~256 KiB: mismo tamaño que SOSOLOG.TXT en la ESP.
const CAP: usize = 256 * 1024;

static BUF: Mutex<Ring<CAP>> = Mutex::new(Ring::new());
/// Evita reentrar al buffer mientras `dmesg` vuelca (no duplicar el dump).
static QUIET: AtomicBool = AtomicBool::new(false);

/// Solo panic handler: suelta el lock si el contexto interrumpido lo tenía.
///
/// # Safety
/// El poseedor anterior del lock ya no va a continuar (estamos en panic).
pub unsafe fn force_unlock() {
    unsafe { BUF.force_unlock() };
}

pub fn append(bytes: &[u8]) {
    if bytes.is_empty() || QUIET.load(Ordering::Relaxed) {
        return;
    }
    BUF.lock().append(bytes);
}

pub fn len() -> usize {
    BUF.lock().byte_len()
}

/// Bytes anexados desde el arranque. A diferencia de `len()`, **no se satura**:
/// es lo que permite a `logfs` saber que el ring sigue creciendo cuando ya está
/// lleno (ver `docs/PLAN-ACTUALIZACIONES.md`, U1).
pub fn escritos() -> u64 {
    BUF.lock().escritos()
}

/// Cursor del byte más antiguo que todavía está en el ring.
pub fn cursor_minimo() -> u64 {
    BUF.lock().cursor_minimo()
}

/// Copia a partir de un cursor absoluto, informando de lo sobrescrito.
pub fn leer_desde(cursor: u64, out: &mut [u8]) -> Lectura {
    with_read(|log| log.leer_desde(cursor, out))
}

/// Copia el ring completo en `dest` bajo un solo lock.
/// Devuelve `(bytes_en_ring, bytes_escritos_en_dest)`.
pub fn copy_log_into(dest: &mut [u8]) -> (usize, usize) {
    with_read(|log| {
        let log_len = log.byte_len();
        let mut off = 0usize;
        let mut pos = 0usize;
        let mut chunk = [0u8; 4096];
        while off < log_len && pos < dest.len() {
            let want = chunk.len().min(log_len - off).min(dest.len() - pos);
            let nc = log.copy_from(off, &mut chunk[..want]);
            if nc == 0 {
                break;
            }
            dest[pos..pos + nc].copy_from_slice(&chunk[..nc]);
            pos += nc;
            off += nc;
        }
        (log_len, pos)
    })
}

/// Lee el ring bajo un solo lock (sin capturar `print!` durante la copia).
fn with_read<R>(f: impl FnOnce(&Ring<CAP>) -> R) -> R {
    // Igual que `_print`: evita que una IRQ intercale bytes mientras copiamos.
    x86_64::instructions::interrupts::without_interrupts(|| {
        QUIET.store(true, Ordering::Relaxed);
        let b = BUF.lock();
        let r = f(&b);
        QUIET.store(false, Ordering::Relaxed);
        r
    })
}

/// Copia una ventana del log a `out` (offset 0 = byte más antiguo).
pub fn copy_from(offset: usize, out: &mut [u8]) -> usize {
    with_read(|log| log.copy_from(offset, out))
}

/// Ejecuta `f` sin capturar sus `print!` en el ring (p. ej. volcado FAT).
/// Devuelve el valor de `f`.
pub fn run_without_capture<T>(f: impl FnOnce() -> T) -> T {
    QUIET.store(true, Ordering::Relaxed);
    let r = f();
    QUIET.store(false, Ordering::Relaxed);
    r
}
