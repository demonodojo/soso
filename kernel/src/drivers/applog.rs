//! Ring de registros de aplicaciones (fd 3).
//!
//! Cada `write(3, …)` llega aquí con sello de timestamp monótono, pid y nombre
//! del proceso. Independiente del ring de consola (`logbuf` / dmesg).

use core::fmt::Write;
use spin::Mutex;

use super::logbuf::Ring;

const CAP: usize = 64 * 1024;

static BUF: Mutex<Ring<CAP>> = Mutex::new(Ring::new());

/// Registra un mensaje con sello del kernel.
pub fn append_record(pid: u64, name: &str, msg: &[u8]) {
    if msg.is_empty() {
        return;
    }
    let mut msg = msg;
    if msg.last() == Some(&b'\n') {
        msg = &msg[..msg.len() - 1];
    }
    let max = soso_abi::LOG_MSG_MAX.min(msg.len());
    let msg = &msg[..max];

    let ns = crate::time::monotonic_ns();
    let sec = ns / 1_000_000_000;
    let frac = ns % 1_000_000_000;
    let micro = frac / 1000;

    let mut line = [0u8; 512];
    let mut w = SliceWriter { buf: &mut line, pos: 0 };
    let _ = write!(
        w,
        "[{:>12}.{:06}] pid={} {}: ",
        sec,
        micro,
        pid,
        name
    );
    let prefix_len = w.pos;
    let room = line.len().saturating_sub(prefix_len);
    let n = msg.len().min(room);
    line[prefix_len..prefix_len + n].copy_from_slice(&msg[..n]);
    let total = prefix_len + n;
    let mut buf = BUF.lock();
    buf.append(&line[..total]);
    buf.append(b"\n");
}

struct SliceWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl Write for SliceWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let n = bytes.len().min(self.buf.len().saturating_sub(self.pos));
        self.buf[self.pos..self.pos + n].copy_from_slice(&bytes[..n]);
        self.pos += n;
        Ok(())
    }
}

/// Copia una ventana del ring (offset 0 = byte más antiguo).
pub fn copy_from(offset: usize, out: &mut [u8]) -> usize {
    BUF.lock().copy_from(offset, out)
}
