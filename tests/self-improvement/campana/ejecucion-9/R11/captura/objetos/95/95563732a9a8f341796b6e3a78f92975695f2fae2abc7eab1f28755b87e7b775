//! Traits de E/S mínimos (sin depender de `core::io` en el target soso-user).

extern crate alloc;
use alloc::string::String;

pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, String>;
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), String> {
        let mut off = 0;
        while off < buf.len() {
            let n = self.read(&mut buf[off..])?;
            if n == 0 {
                return Err("eof inesperado".into());
            }
            off += n;
        }
        Ok(())
    }
}

pub trait Seek {
    fn seek_start(&mut self, pos: u64) -> Result<(), String>;
    fn stream_position(&mut self) -> Result<u64, String>;
}

pub trait ReadSeek: Read + Seek {}

impl<T: Read + Seek> ReadSeek for T {}

#[cfg(feature = "std")]
pub mod std_file {
    use super::{Read, Seek};
    use std::io::{Read as StdRead, Seek as StdSeek, SeekFrom};

    pub struct File(pub std::fs::File);

    impl Read for File {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, String> {
            StdRead::read(&mut self.0, buf).map_err(|e| e.to_string())
        }
    }

    impl Seek for File {
        fn seek_start(&mut self, pos: u64) -> Result<(), String> {
            StdSeek::seek(&mut self.0, SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
            Ok(())
        }
        fn stream_position(&mut self) -> Result<u64, String> {
            StdSeek::stream_position(&mut self.0).map_err(|e| e.to_string())
        }
    }
}
