//! Ring buffer de consola (dmesg).
//!
//! El framebuffer solo muestra ~40 filas; al arrancar (USB, WiFi, PCI…) las
//! trazas tempranas se pierden por scroll. Este buffer guarda los últimos
//! bytes escritos por `print!`/`println!`/`serial::write_bytes` para
//! recuperarlos con el comando `dmesg` del kshell.

use core::sync::atomic::{AtomicBool, Ordering};
use spin::Mutex;

/// ~256 KiB: mismo tamaño que SOSOLOG.TXT en la ESP.
const CAP: usize = 256 * 1024;

struct LogBuf {
    data: [u8; CAP],
    /// Siguiente índice de escritura (módulo CAP).
    pos: usize,
    /// Bytes válidos (como máximo CAP).
    len: usize,
}

impl LogBuf {
    const fn new() -> Self {
        Self {
            data: [0; CAP],
            pos: 0,
            len: 0,
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.data[self.pos] = b;
            self.pos = (self.pos + 1) % CAP;
            if self.len < CAP {
                self.len += 1;
            }
        }
    }

    fn start(&self) -> usize {
        (self.pos + CAP - self.len) % CAP
    }

    /// Copia hasta `max` bytes desde `offset` lógico (0 = más antiguo) a `out`.
    /// Devuelve cuántos bytes se copiaron.
    fn copy_from(&self, offset: usize, out: &mut [u8]) -> usize {
        if offset >= self.len || out.is_empty() {
            return 0;
        }
        let n = out.len().min(self.len - offset);
        let mut idx = (self.start() + offset) % CAP;
        for slot in out.iter_mut().take(n) {
            *slot = self.data[idx];
            idx = (idx + 1) % CAP;
        }
        n
    }
}

static BUF: Mutex<LogBuf> = Mutex::new(LogBuf::new());
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
    BUF.lock().len
}

/// Copia una ventana del log a `out` (offset 0 = byte más antiguo).
/// No mantiene el lock fuera de la copia; el llamante escribe a consola después.
pub fn copy_from(offset: usize, out: &mut [u8]) -> usize {
    QUIET.store(true, Ordering::Relaxed);
    let n = BUF.lock().copy_from(offset, out);
    QUIET.store(false, Ordering::Relaxed);
    n
}

/// Ejecuta `f` sin capturar sus `print!` en el ring (p. ej. volcado FAT).
/// Devuelve el valor de `f`.
pub fn run_without_capture<T>(f: impl FnOnce() -> T) -> T {
    QUIET.store(true, Ordering::Relaxed);
    let r = f();
    QUIET.store(false, Ordering::Relaxed);
    r
}
