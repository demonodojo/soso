//! I/O de guest: scratch en sosomfs e import vía syscalls.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use gguf2som::{Read, Seek, SomOut};
use libsoso::sys;

const BLOCK: usize = 4096;

/// GGUF temporal en la cola de la partición de modelos (scratch sosomfs).
pub struct ScratchFile {
    start_lba: u64,
    size: u64,
    pos: u64,
    cache_lba: u64,
    cache: [u8; BLOCK],
}

impl ScratchFile {
    pub fn new(start_lba: u64, size: u64) -> Self {
        Self {
            start_lba,
            size,
            pos: 0,
            cache_lba: u64::MAX,
            cache: [0u8; BLOCK],
        }
    }

    fn load_block(&mut self, lba: u64) -> Result<(), String> {
        if self.cache_lba == lba {
            return Ok(());
        }
        let off = lba.saturating_sub(self.start_lba) * BLOCK as u64;
        let r = sys::som_scratch_read(self.start_lba, off, &mut self.cache);
        if r < 0 {
            return Err(format!("scratch read errno {r}"));
        }
        self.cache_lba = lba;
        Ok(())
    }
}

impl Read for ScratchFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        if self.pos >= self.size {
            return Ok(0);
        }
        let mut n = 0usize;
        while n < buf.len() && self.pos < self.size {
            let lba = self.start_lba + self.pos / BLOCK as u64;
            self.load_block(lba)?;
            let in_block = (self.pos % BLOCK as u64) as usize;
            let take = (BLOCK - in_block)
                .min(buf.len() - n)
                .min((self.size - self.pos) as usize);
            buf[n..n + take].copy_from_slice(&self.cache[in_block..in_block + take]);
            self.pos += take as u64;
            n += take;
        }
        Ok(n)
    }
}

impl Seek for ScratchFile {
    fn seek_start(&mut self, pos: u64) -> Result<(), String> {
        if pos > self.size {
            return Err(format!("seek fuera de scratch ({pos} > {})", self.size));
        }
        self.pos = pos;
        Ok(())
    }

    fn stream_position(&mut self) -> Result<u64, String> {
        Ok(self.pos)
    }
}

/// Destino `.som` vía importador del kernel → `/models/<nombre>/`.
pub struct SomImportOut;

impl SomOut for SomImportOut {
    fn mkdir(&mut self, _path: &str) -> Result<(), String> {
        Ok(())
    }

    fn write(&mut self, rel: &str, data: &[u8]) -> Result<(), String> {
        let r = sys::som_put(rel, data);
        if r < 0 {
            Err(format!("som_put {rel} errno {r}"))
        } else {
            Ok(())
        }
    }
}

/// Sink HTTP que escribe en scratch sosomfs (bloques de 4 KiB).
pub struct ScratchSink {
    start_lba: u64,
    offset: u64,
    pending: Vec<u8>,
}

impl ScratchSink {
    pub fn new(start_lba: u64) -> Self {
        Self {
            start_lba,
            offset: 0,
            pending: Vec::new(),
        }
    }

    pub fn finish(self) -> Result<(), String> {
        if !self.pending.is_empty() {
            let mut pad = [0u8; BLOCK];
            let n = self.pending.len();
            pad[..n].copy_from_slice(&self.pending);
            let r = sys::som_scratch_write(self.start_lba, self.offset, &pad);
            if r < 0 {
                return Err(format!("scratch write final errno {r}"));
            }
        }
        Ok(())
    }
}

impl soso_http::BodySink for ScratchSink {
    fn write_body(&mut self, chunk: &[u8]) -> Result<(), soso_http::HttpError> {
        self.pending.extend_from_slice(chunk);
        while self.pending.len() >= BLOCK {
            let r = sys::som_scratch_write(self.start_lba, self.offset, &self.pending[..BLOCK]);
            if r < 0 {
                return Err(soso_http::HttpError::Io);
            }
            self.offset += BLOCK as u64;
            self.pending.drain(..BLOCK);
        }
        Ok(())
    }
}
