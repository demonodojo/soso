//! Entrada/salida estándar en soso: fd 0/1/2 vía `soso_rt`.

use crate::io::{self, Read, Write};
use crate::sys::io::DEFAULT_BUF_SIZE;

pub const STDIN_BUF_SIZE: usize = DEFAULT_BUF_SIZE;

pub struct Stdin;

impl Stdin {
    pub const fn new() -> Self {
        Self
    }
}

pub struct Stdout;

impl Stdout {
    pub const fn new() -> Self {
        Self
    }
}

pub struct Stderr;

impl Stderr {
    pub const fn new() -> Self {
        Self
    }
}

fn map_syscall(n: i64) -> io::Result<usize> {
    if n < 0 {
        Err(io::Error::from_raw_os_error((-n) as i32))
    } else {
        Ok(n as usize)
    }
}

impl Read for Stdin {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: `buf` es un slice exclusivo; el kernel sólo escribe dentro.
        map_syscall(unsafe {
            soso_rt::syscall3(
                soso_rt::SYS_READ,
                0,
                buf.as_mut_ptr().expose_provenance() as u64,
                buf.len() as u64,
            )
        })
    }
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // SAFETY: `buf` es un slice compartido de solo lectura para el kernel.
        map_syscall(unsafe {
            soso_rt::syscall3(
                soso_rt::SYS_WRITE,
                1,
                buf.as_ptr().expose_provenance() as u64,
                buf.len() as u64,
            )
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Write for Stderr {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        map_syscall(unsafe {
            soso_rt::syscall3(
                soso_rt::SYS_WRITE,
                2,
                buf.as_ptr().expose_provenance() as u64,
                buf.len() as u64,
            )
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn is_ebadf(err: &io::Error) -> bool {
    err.raw_os_error() == Some(soso_rt::EBADF as i32)
}

pub fn panic_output() -> Option<impl io::Write> {
    Some(Stderr::new())
}
