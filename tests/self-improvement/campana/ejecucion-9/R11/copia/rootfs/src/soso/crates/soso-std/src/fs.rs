//! Sistema de ficheros.

use alloc::vec::Vec;
use libsoso::{abi, sys};

pub struct File {
    fd: u64,
}

impl File {
    pub fn open(path: &str) -> Result<Self, i64> {
        let fd = sys::open(path, abi::O_RDONLY);
        if fd < 0 {
            Err(fd)
        } else {
            Ok(Self { fd: fd as u64 })
        }
    }

    pub fn read_to_end(&self) -> Result<Vec<u8>, i64> {
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = sys::read(self.fd, &mut buf);
            if n < 0 {
                return Err(n);
            }
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
        Ok(out)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        sys::close(self.fd);
    }
}
