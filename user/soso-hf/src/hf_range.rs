//! GGUF remoto vía HTTP Range (sin materializar el fichero en scratch).

use alloc::format;
use alloc::string::{String, ToString};
use gguf2som::{convert, Read, Seek};
use crate::guest_io::SomImportOut;

const MAX_RANGE: u64 = 8 * 1024 * 1024;

/// Fichero GGUF remoto: cada lectura es un GET con `Range`.
pub struct HfRangeFile {
    url: String,
    token: Option<String>,
    size: u64,
    pos: u64,
}

impl HfRangeFile {
    fn new(url: &str, token: Option<&str>, size: u64) -> Self {
        Self {
            url: url.to_string(),
            token: token.map(String::from),
            size,
            pos: 0,
        }
    }

    fn fetch(&self, start: u64, end: u64) -> Result<alloc::vec::Vec<u8>, String> {
        let (status, body) =
            crate::net::https_get_range_bytes(&self.url, self.token.as_deref(), start, end)
                .map_err(|_| format!("range GET {start}-{end}"))?;
        if status != 206 {
            return Err(format!("range status {status} (esperado 206)"));
        }
        Ok(body)
    }
}

impl Read for HfRangeFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        if self.pos >= self.size {
            return Ok(0);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        let want = buf
            .len()
            .min((self.size - self.pos) as usize)
            .min(MAX_RANGE as usize);
        let end = self.pos + want as u64 - 1;
        let body = self.fetch(self.pos, end)?;
        let n = body.len().min(want);
        buf[..n].copy_from_slice(&body[..n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HfRangeFile {
    fn seek_start(&mut self, pos: u64) -> Result<(), String> {
        if pos > self.size {
            return Err(format!("seek fuera de GGUF ({pos} > {})", self.size));
        }
        self.pos = pos;
        Ok(())
    }

    fn stream_position(&mut self) -> Result<u64, String> {
        Ok(self.pos)
    }
}

fn supports_range(url: &str, token: Option<&str>) -> bool {
    match crate::net::https_get_range_bytes(url, token, 0, 0) {
        Ok((206, _)) => true,
        _ => false,
    }
}

/// Convierte GGUF remoto a `.som` vía importador (sesión `som_begin` ya abierta).
pub fn convert_from_url(
    url: &str,
    token: Option<&str>,
    model_name: &str,
    file_size: u64,
) -> Result<(), ()> {
    if !supports_range(url, token) {
        return Err(());
    }
    let mut src = HfRangeFile::new(url, token, file_size);
    let mut out = SomImportOut;
    convert(&mut src, &mut out, Some(model_name)).map_err(|_| ())
}
