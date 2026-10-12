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

    pub(crate) fn byte_len(&self) -> usize {
        self.len
    }

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
fn with_read<R>(f: impl FnOnce(&LogBuf) -> R) -> R {
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
