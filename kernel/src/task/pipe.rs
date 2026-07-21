//! Pipes anónimos en memoria del kernel (buffer circular de 4 KiB).

use alloc::vec::Vec;
use spin::Mutex;

pub type PipeId = usize;

const PIPE_CAP: usize = 4096;

pub struct Pipe {
    buf: [u8; PIPE_CAP],
    head: usize,
    len: usize,
    pub readers: u32,
    pub writers: u32,
    pub write_closed: bool,
}

static PIPES: Mutex<Vec<Option<Pipe>>> = Mutex::new(Vec::new());

impl Pipe {
    fn new() -> Self {
        Self {
            buf: [0; PIPE_CAP],
            head: 0,
            len: 0,
            readers: 0,
            writers: 0,
            write_closed: false,
        }
    }

    fn readable(&self) -> usize {
        self.len
    }

    fn writable_space(&self) -> usize {
        PIPE_CAP - self.len
    }

    fn read_into(&mut self, dst: &mut [u8]) -> usize {
        let n = dst.len().min(self.len);
        for i in 0..n {
            dst[i] = self.buf[(self.head + i) % PIPE_CAP];
        }
        self.head = (self.head + n) % PIPE_CAP;
        self.len -= n;
        n
    }

    fn write_from(&mut self, src: &[u8]) -> usize {
        let n = src.len().min(self.writable_space());
        for (i, &b) in src[..n].iter().enumerate() {
            let pos = (self.head + self.len + i) % PIPE_CAP;
            self.buf[pos] = b;
        }
        self.len += n;
        n
    }
}

pub fn alloc_pipe() -> PipeId {
    let mut pipes = PIPES.lock();
    if let Some(i) = pipes.iter().position(|p| p.is_none()) {
        pipes[i] = Some(Pipe::new());
        i
    } else {
        let id = pipes.len();
        pipes.push(Some(Pipe::new()));
        id
    }
}

pub fn add_reader(id: PipeId) {
    if let Some(p) = PIPES.lock().get_mut(id).and_then(|s| s.as_mut()) {
        p.readers += 1;
    }
}

pub fn add_writer(id: PipeId) {
    if let Some(p) = PIPES.lock().get_mut(id).and_then(|s| s.as_mut()) {
        p.writers += 1;
    }
}

pub fn close_reader(id: PipeId) {
    let mut pipes = PIPES.lock();
    if let Some(p) = pipes.get_mut(id).and_then(|s| s.as_mut()) {
        p.readers = p.readers.saturating_sub(1);
        if p.readers == 0 && p.writers == 0 && p.len == 0 {
            pipes[id] = None;
        }
    }
}

pub fn close_writer(id: PipeId) {
    let mut pipes = PIPES.lock();
    if let Some(p) = pipes.get_mut(id).and_then(|s| s.as_mut()) {
        p.writers = p.writers.saturating_sub(1);
        if p.writers == 0 {
            p.write_closed = true;
        }
        if p.readers == 0 && p.writers == 0 && p.len == 0 {
            pipes[id] = None;
        }
    }
}

pub fn has_data(id: PipeId) -> bool {
    PIPES
        .lock()
        .get(id)
        .and_then(|s| s.as_ref())
        .is_some_and(|p| p.readable() > 0)
}

pub fn has_space(id: PipeId) -> bool {
    PIPES
        .lock()
        .get(id)
        .and_then(|s| s.as_ref())
        .is_some_and(|p| p.writable_space() > 0)
}

pub fn write_closed(id: PipeId) -> bool {
    PIPES
        .lock()
        .get(id)
        .and_then(|s| s.as_ref())
        .is_some_and(|p| p.write_closed)
}

pub fn no_readers(id: PipeId) -> bool {
    PIPES
        .lock()
        .get(id)
        .and_then(|s| s.as_ref())
        .is_some_and(|p| p.readers == 0)
}

/// Lee hasta `len` bytes al buffer de usuario. Devuelve bytes leídos.
pub fn read_into_user(id: PipeId, buf: u64, len: u64) -> u64 {
    let want = len.min(256) as usize;
    let mut tmp = [0u8; 256];
    let n = PIPES
        .lock()
        .get_mut(id)
        .and_then(|s| s.as_mut())
        .map(|p| p.read_into(&mut tmp[..want]))
        .unwrap_or(0);
    if n > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(tmp.as_ptr(), buf as *mut u8, n);
        }
    }
    n as u64
}

/// Escribe desde memoria de usuario. Devuelve bytes escritos o error EPIPE.
pub fn write_from_user(id: PipeId, buf: u64, len: u64) -> Result<u64, i64> {
    use soso_abi as abi;
    let want = len.min(256) as usize;
    let mut src = [0u8; 256];
    unsafe {
        core::ptr::copy_nonoverlapping(buf as *const u8, src.as_mut_ptr(), want);
    }
    let mut pipes = PIPES.lock();
    let Some(p) = pipes.get_mut(id).and_then(|s| s.as_mut()) else {
        return Err(-abi::EPIPE);
    };
    if p.readers == 0 {
        return Err(-abi::EPIPE);
    }
    let written = p.write_from(&src[..want]);
    Ok(written as u64)
}

/// Intenta leer todo lo pedido en bucle corto (sin bloquear).
pub fn try_read(id: PipeId, buf: u64, len: u64) -> u64 {
    let mut total = 0u64;
    while total < len {
        let n = read_into_user(id, buf + total, len - total);
        if n == 0 {
            break;
        }
        total += n;
    }
    total
}

/// Intenta escribir todo lo pedido en bucle corto (sin bloquear).
pub fn try_write(id: PipeId, buf: u64, len: u64) -> Result<u64, i64> {
    let mut total = 0u64;
    while total < len {
        let n = write_from_user(id, buf + total, len - total)?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}
