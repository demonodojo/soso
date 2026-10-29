//! Volcado del ring `logbuf` a `SOSOLOG.TXT` pre-creado en la ESP (FAT).
//!
//! Solo sobrescribe los sectores de datos del fichero; no toca FAT ni directorio.

use crate::drivers::espfat::{self, Slot, SECTOR};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use soso_log_core::por_cada_tramo_cambiado;
use spin::Once;

const FILE_SIZE: usize = 256 * 1024;
const POLL_MS: u64 = 2000;
const FULL_WHEN_RING_FULL_MS: u64 = 30_000;
const BOMBEO_CHUNK: usize = 16 * 1024;
/// Reserva fija para la cabecera de flush (ver `format_header`).
const HDR_SLOT: usize = 160;
const LOGBUF_CAP: usize = 256 * 1024;

static SLOT: Once<Option<Slot>> = Once::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static LAST_FLUSH_LEN: AtomicUsize = AtomicUsize::new(0);
static LAST_FLUSH_ESCRITOS: AtomicU64 = AtomicU64::new(0);
static FLUSH_COUNT: AtomicU32 = AtomicU32::new(0);
static LAST_POLL_MS: AtomicUsize = AtomicUsize::new(0);
static LAST_FULL_RING_MS: AtomicU64 = AtomicU64::new(0);
static BOOT_FULL_DONE: AtomicBool = AtomicBool::new(false);
/// Duración del último volcado y máximo observado (para diagnosticar parones de red).
static ULTIMO_VOLCADO_MS: AtomicU64 = AtomicU64::new(0);
static MAX_VOLCADO_MS: AtomicU64 = AtomicU64::new(0);
/// Último latido de teclado volcado, para no reescribir sin necesidad.
static LAST_LATIDO: AtomicU32 = AtomicU32::new(u32::MAX);

/// Contenido del fichero. Estático de módulo (y no local de `flush`) porque
/// `flush_cabecera` reescribe su primer sector sin regenerar todo lo demás.
static mut BUF: [u8; FILE_SIZE] = [b'\n'; FILE_SIZE];
/// Instantánea de lo último escrito con éxito en la ESP (volcado incremental).
static mut SHADOW: [u8; FILE_SIZE] = [b'\n'; FILE_SIZE];

pub fn init() {
    if crate::drivers::modo::instalado_declarado() {
        crate::println!("fatlog: instalación declarada; el log va a /var/log");
        return;
    }
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
            flush_or_warn(false);
        }
        None => crate::println!("fatlog: SOSOLOG.TXT no encontrado o no contiguo; desactivado"),
    }
}

fn flush_or_warn(desde_poll: bool) {
    if flush_inner(desde_poll).is_err() {
        crate::println!("fatlog: aviso: flush a SOSOLOG.TXT falló");
    }
}

/// Volcado inmediato si fatlog está activo (checkpoints de arranque).
pub fn flush_checkpoint() {
    if ACTIVE.load(Ordering::Relaxed) {
        flush_or_warn(false);
    }
}

/// Duración del último volcado completo a la ESP (ms).
#[cfg_attr(not(feature = "lxdde"), allow(dead_code))]
pub fn ultimo_volcado_ms() -> u64 {
    ULTIMO_VOLCADO_MS.load(Ordering::Relaxed)
}

/// Máximo observado desde el arranque.
#[allow(dead_code)]
pub fn max_volcado_ms() -> u64 {
    MAX_VOLCADO_MS.load(Ordering::Relaxed)
}

/// Rate-limit ~2 s; escribe si el logbuf creció o el ring lleno sigue rotando.
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
    let escritos = crate::drivers::logbuf::escritos();
    let prev_len = LAST_FLUSH_LEN.load(Ordering::Relaxed);
    let prev_esc = LAST_FLUSH_ESCRITOS.load(Ordering::Relaxed);
    let ring_lleno = cur >= LOGBUF_CAP;
    if cur > prev_len || (ring_lleno && escritos > prev_esc) {
        let _ = flush_inner(true);
        return;
    }
    let (sc, _, _, ent) = crate::drivers::kbd::latido();
    let marca = sc ^ (ent << 16);
    if marca != LAST_LATIDO.swap(marca, Ordering::Relaxed) {
        let _ = flush_cabecera(false);
    }
}

fn registrar_duracion(ms: u64) {
    ULTIMO_VOLCADO_MS.store(ms, Ordering::Relaxed);
    let mut max = MAX_VOLCADO_MS.load(Ordering::Relaxed);
    while ms > max {
        match MAX_VOLCADO_MS.compare_exchange_weak(max, ms, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(v) => max = v,
        }
    }
}

fn bombeo_red_bsp() {
    crate::net::poll();
    #[cfg(feature = "lxdde")]
    crate::lxdde::poll();
}

fn write_bytes(
    slot: Slot,
    offset: usize,
    data: &[u8],
    bombeo_entre_bloques: bool,
) -> Result<(), ()> {
    if data.is_empty() {
        return Ok(());
    }
    let base_lba = slot.data_lba + (offset / SECTOR) as u64;
    if !bombeo_entre_bloques {
        return espfat::write(base_lba, data).map_err(|_| ());
    }
    let mut off = 0usize;
    while off < data.len() {
        let chunk = (data.len() - off).min(BOMBEO_CHUNK);
        espfat::write(base_lba + (off / SECTOR) as u64, &data[off..off + chunk]).map_err(|_| ())?;
        off += chunk;
        if off < data.len() {
            bombeo_red_bsp();
        }
    }
    Ok(())
}

fn write_tramo(
    slot: Slot,
    buf: &[u8],
    offset: usize,
    len: usize,
    bombeo: bool,
) -> Result<(), ()> {
    write_bytes(slot, offset, &buf[offset..offset + len], bombeo)
}

fn write_todo_el_fichero(slot: Slot, buf: &[u8], bombeo: bool) -> Result<(), ()> {
    let mut off = 0usize;
    while off < FILE_SIZE {
        let chunk = (FILE_SIZE - off).min(BOMBEO_CHUNK);
        write_bytes(slot, off, &buf[off..off + chunk], bombeo)?;
        off += chunk;
    }
    Ok(())
}

fn preparar_buf(buf: &mut [u8], flush_n: u32, uptime: u64) -> (usize, usize) {
    let (log_len, body_copied) = crate::drivers::logbuf::copy_log_into(&mut buf[HDR_SLOT..]);
    let mut hdr = [0u8; HDR_SLOT];
    let hlen = format_header(&mut hdr, flush_n, uptime, log_len).min(HDR_SLOT);
    buf[..hlen].copy_from_slice(&hdr[..hlen]);
    if hlen < HDR_SLOT {
        buf[hlen..HDR_SLOT].fill(b'\n');
    }
    let pos = HDR_SLOT + body_copied;
    if pos < FILE_SIZE {
        buf[pos..FILE_SIZE].fill(b'\n');
    }
    (log_len, body_copied)
}

fn flush_inner(desde_poll: bool) -> Result<(), ()> {
    if !ACTIVE.load(Ordering::Relaxed) {
        return Err(());
    }
    let slot = SLOT.get().and_then(|s| *s).ok_or(())?;
    let t0 = crate::arch::tsc::uptime_ms();
    let n = FLUSH_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    let uptime = crate::arch::tsc::uptime_ms();
    let bombeo = desde_poll;

    let buf = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(BUF).cast::<u8>(), FILE_SIZE)
    };
    let shadow = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(SHADOW).cast::<u8>(), FILE_SIZE)
    };

    let (log_len, _) = crate::drivers::logbuf::run_without_capture(|| preparar_buf(buf, n, uptime));

    let ring_lleno = log_len >= LOGBUF_CAP;
    let now = crate::arch::tsc::uptime_ms();
    let necesita_full = !BOOT_FULL_DONE.load(Ordering::Relaxed)
        || (ring_lleno
            && now.saturating_sub(LAST_FULL_RING_MS.load(Ordering::Relaxed)) >= FULL_WHEN_RING_FULL_MS);

    let ok = crate::drivers::logbuf::run_without_capture(|| {
        if necesita_full {
            write_todo_el_fichero(slot, buf, bombeo)?;
            if ring_lleno {
                LAST_FULL_RING_MS.store(now, Ordering::Relaxed);
            }
            BOOT_FULL_DONE.store(true, Ordering::Relaxed);
        } else {
            let mut fallo = false;
            por_cada_tramo_cambiado(shadow, buf, SECTOR, |t| {
                if write_tramo(slot, buf, t.offset, t.len, bombeo).is_err() {
                    fallo = true;
                }
            });
            if fallo {
                return Err(());
            }
        }
        Ok::<(), ()>(())
    });

    if ok.is_err() {
        return Err(());
    }

    shadow.copy_from_slice(buf);
    LAST_FLUSH_LEN.store(log_len, Ordering::Relaxed);
    LAST_FLUSH_ESCRITOS.store(crate::drivers::logbuf::escritos(), Ordering::Relaxed);
    registrar_duracion(crate::arch::tsc::uptime_ms().saturating_sub(t0));
    Ok(())
}

pub fn flush() -> Result<(), ()> {
    flush_inner(false)
}

/// Reescribe solo la cabecera (primer sector) con el latido al día.
fn flush_cabecera(desde_poll: bool) -> Result<(), ()> {
    let slot = SLOT.get().and_then(|s| *s).ok_or(())?;
    let n = FLUSH_COUNT.load(Ordering::Relaxed);
    let uptime = crate::arch::tsc::uptime_ms();
    let log_len = LAST_FLUSH_LEN.load(Ordering::Relaxed);

    let buf = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(BUF).cast::<u8>(), FILE_SIZE)
    };
    let mut hdr = [0u8; 160];
    let hlen = format_header(&mut hdr, n, uptime, log_len).min(SECTOR);
    buf[..hlen].copy_from_slice(&hdr[..hlen]);

    let ok = crate::drivers::logbuf::run_without_capture(|| {
        write_bytes(slot, 0, &buf[..SECTOR], desde_poll).is_ok()
    });
    if ok {
        unsafe {
            SHADOW[..SECTOR].copy_from_slice(&buf[..SECTOR]);
        }
        Ok(())
    } else {
        Err(())
    }
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
