//! Tuberías de soso: `SYS_PIPE` devuelve `(escritura << 32) | lectura`.
//! Lo usan `Command::spawn` / `Command::output` para capturar stdout y stderr.

use crate::fmt;
use crate::io::{self, BorrowedCursor, IoSlice, IoSliceMut};

pub struct Pipe {
    fd: u64,
}

/// `(lectura, escritura)`.
pub fn pipe() -> io::Result<(Pipe, Pipe)> {
    let rc = unsafe { soso_rt::syscall0(soso_rt::SYS_PIPE) };
    if rc < 0 {
        return Err(io::Error::from_raw_os_error((-rc) as i32));
    }
    let rc = rc as u64;
    Ok((Pipe { fd: rc & 0xffff_ffff }, Pipe { fd: rc >> 32 }))
}

impl Pipe {
    pub fn fd(&self) -> u64 {
        self.fd
    }

    /// Entrega el fd sin cerrarlo.
    pub fn into_fd(self) -> u64 {
        let fd = self.fd;
        crate::mem::forget(self);
        fd
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Err(io::Error::UNSUPPORTED_PLATFORM)
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let n = unsafe { soso_rt::read(self.fd, buf.as_mut_ptr(), buf.len()) };
        if n < 0 { Err(io::Error::from_raw_os_error(soso_rt::EIO as i32)) } else { Ok(n as usize) }
    }

    pub fn read_buf(&self, mut cursor: BorrowedCursor<'_, u8>) -> io::Result<()> {
        let n = unsafe {
            let dst = cursor.as_mut();
            soso_rt::read(self.fd, dst.as_mut_ptr().cast(), dst.len())
        };
        if n < 0 {
            return Err(io::Error::from_raw_os_error(soso_rt::EIO as i32));
        }
        unsafe { cursor.advance(n as usize) };
        Ok(())
    }

    pub fn read_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        match bufs.iter_mut().find(|b| !b.is_empty()) {
            Some(b) => self.read(b),
            None => Ok(0),
        }
    }

    pub fn is_read_vectored(&self) -> bool {
        false
    }

    pub fn read_to_end(&self, buf: &mut Vec<u8>) -> io::Result<usize> {
        let mut total = 0;
        let mut trozo = [0u8; 4096];
        loop {
            let n = self.read(&mut trozo)?;
            if n == 0 {
                return Ok(total);
            }
            buf.extend_from_slice(&trozo[..n]);
            total += n;
        }
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        let n = unsafe { soso_rt::write(self.fd, buf.as_ptr(), buf.len()) };
        if n < 0 { Err(io::Error::from_raw_os_error(soso_rt::EIO as i32)) } else { Ok(n as usize) }
    }

    pub fn write_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        match bufs.iter().find(|b| !b.is_empty()) {
            Some(b) => self.write(b),
            None => Ok(0),
        }
    }

    pub fn is_write_vectored(&self) -> bool {
        false
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe {
            let _ = soso_rt::syscall1(soso_rt::SYS_CLOSE, self.fd);
        }
    }
}

impl fmt::Debug for Pipe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pipe").field("fd", &self.fd).finish()
    }
}
