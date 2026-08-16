//! Volcado del ring `logbuf` a `SOSOLOG.TXT` pre-creado en la ESP (FAT).
//!
//! Solo sobrescribe los sectores de datos del fichero; no toca FAT ni directorio.

use crate::drivers::espfat::{self, SECTOR, Slot};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use spin::Once;

const FILE_SIZE: usize = 256 * 1024;
const CHUNK: usize = 128 * 1024;
const POLL_MS: u64 = 2000;

static SLOT: Once<Option<Slot>> = Once::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static LAST_FLUSH_LEN: AtomicUsize = AtomicUsize::new(0);
static FLUSH_COUNT: AtomicU32 = AtomicU32::new(0);
static LAST_POLL_MS: AtomicUsize = AtomicUsize::new(0);

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

/// Rate-limit ~2 s; solo escribe si el logbuf creció desde el último flush.
pub fn poll() {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let now = crate::arch::pit::uptime_ms() as usize;
    let prev = LAST_POLL_MS.load(Ordering::Relaxed);
    if now.saturating_sub(prev) < POLL_MS as usize {
        return;
    }
    LAST_POLL_MS.store(now, Ordering::Relaxed);
    let cur = crate::drivers::logbuf::len();
    if cur > LAST_FLUSH_LEN.load(Ordering::Relaxed) {
        let _ = flush();
    }
}

pub fn flush() -> Result<(), ()> {
    if !ACTIVE.load(Ordering::Relaxed) {
        return Err(());
    }
    let slot = SLOT.get().and_then(|s| *s).ok_or(())?;
    let log_len = crate::drivers::logbuf::len();
    let n = FLUSH_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    let uptime = crate::arch::pit::uptime_ms();

    static mut BUF: [u8; FILE_SIZE] = [b'\n'; FILE_SIZE];
    // SAFETY: solo la BSP llama a flush (scheduler/kshell/panic); no reentrante.
    let buf = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(BUF).cast(), FILE_SIZE)
    };

    let mut hdr = [0u8; 128];
    let hlen = format_header(&mut hdr, n, uptime, log_len);
    let copy = hlen.min(buf.len());
    buf[..copy].copy_from_slice(&hdr[..copy]);
    let mut pos = copy;

    let mut off = 0usize;
    let mut tmp = [0u8; 4096];
    while off < log_len && pos < FILE_SIZE {
        let n = crate::drivers::logbuf::copy_from(off, &mut tmp);
        if n == 0 {
            break;
        }
        let take = n.min(FILE_SIZE - pos);
        buf[pos..pos + take].copy_from_slice(&tmp[..take]);
        pos += take;
        off += n;
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
    let _ = write!(
        w,
        "=== soso log flush #{flush_n} uptime={uptime_ms}ms bytes={log_len} ===\n"
    );
    w.pos
}
