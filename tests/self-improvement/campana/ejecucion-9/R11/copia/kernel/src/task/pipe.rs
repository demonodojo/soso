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

/// Trozo de tránsito entre memoria de usuario y el buffer del pipe. Es el tamaño
/// de un temporal en la pila del kernel, NO un límite de la transferencia: antes
/// era lo segundo y eso convertía cualquier escritura mayor en una pérdida de
/// datos silenciosa (`cat` de un fichero por SSH entregaba 1435 bytes de 3086,
/// porque `cat` escribe trozos de 1 KiB y nadie miraba el retorno).
const CHUNK: usize = 256;

/// Lee hasta `len` bytes al buffer de usuario. Devuelve bytes leídos.
pub fn read_into_user(id: PipeId, buf: u64, len: u64) -> u64 {
    let mut pipes = PIPES.lock();
    let Some(p) = pipes.get_mut(id).and_then(|s| s.as_mut()) else {
        return 0;
    };
    let mut tmp = [0u8; CHUNK];
    let mut total = 0u64;
    while total < len {
        let want = ((len - total) as usize).min(CHUNK);
        let n = p.read_into(&mut tmp[..want]);
        if n == 0 {
            break;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(tmp.as_ptr(), (buf + total) as *mut u8, n);
        }
        total += n as u64;
    }
    total
}

/// Escribe desde memoria de usuario. Devuelve bytes escritos o error EPIPE.
///
/// Transfiere todo lo que quepa en el pipe (4 KiB), no 256 bytes. Sigue pudiendo
/// ser una escritura CORTA —si el pipe se llena—, y eso es legítimo: lo que no es
/// legítimo es que el tope lo pusiera el tamaño de un temporal del kernel.
pub fn write_from_user(id: PipeId, buf: u64, len: u64) -> Result<u64, i64> {
    use soso_abi as abi;
    let mut pipes = PIPES.lock();
    let Some(p) = pipes.get_mut(id).and_then(|s| s.as_mut()) else {
        return Err(-abi::EPIPE);
    };
    if p.readers == 0 {
        return Err(-abi::EPIPE);
    }
    let mut src = [0u8; CHUNK];
    let mut total = 0u64;
    while total < len {
        let hueco = p.writable_space();
        if hueco == 0 {
            break;
        }
        let want = ((len - total) as usize).min(CHUNK).min(hueco);
        unsafe {
            core::ptr::copy_nonoverlapping((buf + total) as *const u8, src.as_mut_ptr(), want);
        }
        let n = p.write_from(&src[..want]);
        total += n as u64;
        if n < want {
            break;
        }
    }
    Ok(total)
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
