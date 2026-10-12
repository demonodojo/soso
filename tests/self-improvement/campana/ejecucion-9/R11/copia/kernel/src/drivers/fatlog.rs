//! Volcado del ring `logbuf` a `SOSOLOG.TXT` pre-creado en la ESP (FAT).
//!
//! Solo sobrescribe los sectores de datos del fichero; no toca FAT ni directorio.

use crate::drivers::espfat::{self, Slot, SECTOR};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use spin::Once;

const FILE_SIZE: usize = 256 * 1024;
const CHUNK: usize = 128 * 1024;
const POLL_MS: u64 = 2000;
/// Reserva fija para la cabecera de flush (ver `format_header`).
const HDR_SLOT: usize = 160;

static SLOT: Once<Option<Slot>> = Once::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static LAST_FLUSH_LEN: AtomicUsize = AtomicUsize::new(0);
static FLUSH_COUNT: AtomicU32 = AtomicU32::new(0);
static LAST_POLL_MS: AtomicUsize = AtomicUsize::new(0);
/// Último latido de teclado volcado, para no reescribir sin necesidad.
static LAST_LATIDO: AtomicU32 = AtomicU32::new(u32::MAX);

/// Contenido del fichero. Estático de módulo (y no local de `flush`) porque
/// `flush_cabecera` reescribe su primer sector sin regenerar todo lo demás.
static mut BUF: [u8; FILE_SIZE] = [b'\n'; FILE_SIZE];

pub fn init() {
    // Basta con tener ESP: el log no necesita que el root live haya montado, y
    // es justo cuando NO monta cuando más falta hace poder leerlo.
    if !crate::drivers::live_disk::esp_available() {
        return;
    }
    match espfat::locate(b"SOSOLOG ", b"TXT", FILE_SIZE) {
        Some(slot) => {
            SLOT.call_once(|| Some(slot));
            ACTIVE.store(true, Ordering::Relaxed);
            crate::println!(
                "fatlog: SOSOLOG.TXT LBA {} ({} KiB)",
                slot.data_lba,
                FILE_SIZE / 1024
            );
            flush_or_warn();
        }
        None => crate::println!("fatlog: SOSOLOG.TXT no encontrado o no contiguo; desactivado"),
    }
}

fn flush_or_warn() {
    if flush().is_err() {
        crate::println!("fatlog: aviso: flush a SOSOLOG.TXT falló");
    }
}

/// Volcado inmediato si fatlog está activo (checkpoints de arranque).
pub fn flush_checkpoint() {
    if ACTIVE.load(Ordering::Relaxed) {
        flush_or_warn();
    }
}

/// Rate-limit ~2 s; solo escribe si el logbuf creció desde el último flush.
pub fn poll() {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let now = crate::arch::tsc::uptime_ms() as usize;
    let prev = LAST_POLL_MS.load(Ordering::Relaxed);
    if now.saturating_sub(prev) < POLL_MS as usize {
        return;
    }
    LAST_POLL_MS.store(now, Ordering::Relaxed);
    let cur = crate::drivers::logbuf::len();
    if cur > LAST_FLUSH_LEN.load(Ordering::Relaxed) {
        let _ = flush();
        return;
    }
    // Escribir teclas no imprime nada, así que sin esto el latido nunca
    // llegaría al pendrive y un cuelgue tecleando dejaría la cabecera de
    // arranque, que no dice nada. Solo se reescribe el **primer sector**: 512 B
    // cada 2 s, en vez de los 256 KiB del volcado entero, que además pelearían
    // por el candado del USB justo con lo que estamos intentando diagnosticar.
    let (sc, _, _, ent) = crate::drivers::kbd::latido();
    let marca = sc ^ (ent << 16);
    if marca != LAST_LATIDO.swap(marca, Ordering::Relaxed) {
        let _ = flush_cabecera();
    }
}

/// Reescribe solo la cabecera (primer sector) con el latido al día.
fn flush_cabecera() -> Result<(), ()> {
    let slot = SLOT.get().and_then(|s| *s).ok_or(())?;
    let n = FLUSH_COUNT.load(Ordering::Relaxed);
    let uptime = crate::arch::tsc::uptime_ms();
    let log_len = LAST_FLUSH_LEN.load(Ordering::Relaxed);

    // SAFETY: mismo BUF y mismo llamante único que `flush`.
    let buf = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(BUF).cast::<u8>(), FILE_SIZE)
    };
    let mut hdr = [0u8; 160];
    let hlen = format_header(&mut hdr, n, uptime, log_len).min(SECTOR);
    buf[..hlen].copy_from_slice(&hdr[..hlen]);

    let ok = crate::drivers::logbuf::run_without_capture(|| {
        espfat::write(slot.data_lba, &buf[..SECTOR]).is_ok()
    });
    if ok {
        Ok(())
    } else {
        Err(())
    }
}

pub fn flush() -> Result<(), ()> {
    if !ACTIVE.load(Ordering::Relaxed) {
        return Err(());
    }
    let slot = SLOT.get().and_then(|s| *s).ok_or(())?;
    let n = FLUSH_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    let uptime = crate::arch::tsc::uptime_ms();

    // SAFETY: solo la BSP llama a flush (scheduler/kshell/panic); no reentrante.
    let buf =
        unsafe { core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(BUF).cast(), FILE_SIZE) };

    // Una sola toma del lock: evita perder el final de la última línea si una IRQ
    // escribe en logbuf entre lecturas sueltas (checkpoints de arranque).
    let (log_len, body_copied) = crate::drivers::logbuf::copy_log_into(&mut buf[HDR_SLOT..]);

    let mut hdr = [0u8; HDR_SLOT];
    let hlen = format_header(&mut hdr, n, uptime, log_len).min(HDR_SLOT);
    buf[..hlen].copy_from_slice(&hdr[..hlen]);
    if hlen < HDR_SLOT {
        buf[hlen..HDR_SLOT].fill(b'\n');
    }

    let pos = HDR_SLOT + body_copied;
    // Sin esto quedan restos del volcado anterior y parece que la última línea
    // está cortada o hay texto fantasma tras un checkpoint.
    if pos < FILE_SIZE {
        buf[pos..FILE_SIZE].fill(b'\n');
    }

    let write_ok = crate::drivers::logbuf::run_without_capture(|| {
        let mut lba = slot.data_lba;
        let mut written = 0usize;
        while written < FILE_SIZE {
            let chunk = (FILE_SIZE - written).min(CHUNK);
            if espfat::write(lba, &buf[written..written + chunk]).is_err() {
                return false;
            }
            lba += (chunk / SECTOR) as u64;
            written += chunk;
        }
        true
    });
    if !write_ok {
        return Err(());
    }

    LAST_FLUSH_LEN.store(log_len, Ordering::Relaxed);
    Ok(())
}

fn format_header(out: &mut [u8], flush_n: u32, uptime_ms: u64, log_len: usize) -> usize {
    use core::fmt::Write;
    struct W<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl Write for W<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let n = s.len().min(self.buf.len().saturating_sub(self.pos));
            self.buf[self.pos..self.pos + n].copy_from_slice(&s.as_bytes()[..n]);
            self.pos += n;
            Ok(())
        }
    }
    let mut w = W { buf: out, pos: 0 };
    // El latido del teclado va en la cabecera a propósito: si la máquina se
    // cuelga escribiendo, esto es lo ÚNICO que queda en el pendrive para saber
    // por dónde se atascó. `sc` sube = la IRQ y el sondeo siguen vivos;
    // `enc` sube y `ent` no = el atasco está del lado de la tty/consola.
    let (sc, ultimo, enc, ent) = crate::drivers::kbd::latido();
    let _ = write!(
        w,
        "=== soso log flush #{flush_n} uptime={uptime_ms}ms bytes={log_len} \
         kbd sc={sc} ultimo={ultimo:#04x} enc={enc} ent={ent} ===\n"
    );
    w.pos
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_fatlog_flush() {
    flush_checkpoint();
}
