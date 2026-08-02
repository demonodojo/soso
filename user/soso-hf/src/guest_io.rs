//! I/O de ficheros vía syscalls para gguf2som en guest.

use alloc::format;
use alloc::string::String;
use gguf2som::{Read, Seek, SomOut};
use libsoso::sys;
use soso_abi::O_RDONLY;

pub struct GuestFile {
    fd: u64,
}

impl GuestFile {
    pub fn open(path: &str) -> Result<Self, ()> {
        let fd = sys::open(path, O_RDONLY);
        if fd < 0 {
            Err(())
        } else {
            Ok(Self { fd: fd as u64 })
        }
    }
}

impl Drop for GuestFile {
    fn drop(&mut self) {
        let _ = sys::close(self.fd);
    }
}

impl Read for GuestFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        let n = sys::read(self.fd, buf);
        if n < 0 {
            Err(format!("read errno {n}"))
        } else {
            Ok(n as usize)
        }
    }
}

impl Seek for GuestFile {
    fn seek_start(&mut self, pos: u64) -> Result<(), String> {
        let r = sys::seek(self.fd, pos as i64, soso_abi::SEEK_SET);
        if r < 0 {
            Err(format!("seek errno {r}"))
        } else {
            Ok(())
        }
    }

    fn stream_position(&mut self) -> Result<u64, String> {
        let r = sys::seek(self.fd, 0, soso_abi::SEEK_CUR);
        if r < 0 {
            Err(format!("tell errno {r}"))
        } else {
            Ok(r as u64)
        }
    }
}

pub struct GuestSomOut {
    base: String,
}

impl GuestSomOut {
    pub fn new(base: &str) -> Self {
        Self {
            base: String::from(base),
        }
    }

    fn full_path(&self, rel: &str) -> String {
        format!("{}/{}", self.base, rel)
    }
}

impl SomOut for GuestSomOut {
    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        let p = self.full_path(path);
        let _ = sys::mkdir(&p);
        Ok(())
    }

    fn write(&mut self, rel: &str, data: &[u8]) -> Result<(), String> {
        let p = self.full_path(rel);
        if let Some(parent) = rel.rsplit_once('/') {
            let _ = sys::mkdir(&self.full_path(parent.0));
        }
        let fd = sys::open(&p, soso_abi::O_WRONLY | soso_abi::O_CREAT | soso_abi::O_APPEND);
        if fd < 0 {
            return Err(format!("open {p} errno {fd}"));
        }
        let mut off = 0usize;
        while off < data.len() {
            let n = sys::write(fd as u64, &data[off..]);
            if n <= 0 {
                let _ = sys::close(fd as u64);
                return Err(format!("write {p}"));
            }
            off += n as usize;
        }
        let _ = sys::close(fd as u64);
        Ok(())
    }
}
