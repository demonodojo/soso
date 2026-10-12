//! Tuberías sobre `SYS_PIPE`.

use libsoso::sys;

pub struct PipeReader(u64);
pub struct PipeWriter(u64);

pub fn pipe() -> Result<(PipeReader, PipeWriter), i64> {
    let (r, w) = sys::pipe()?;
    Ok((PipeReader(r), PipeWriter(w)))
}

impl PipeReader {
    pub fn fd(&self) -> u64 {
        self.0
    }

    pub fn read(&self, buf: &mut [u8]) -> i64 {
        sys::read(self.0, buf)
    }
}

impl PipeWriter {
    pub fn fd(&self) -> u64 {
        self.0
    }

    pub fn write_all(&self, data: &[u8]) -> Result<(), i64> {
        sys::write_all(self.0, data)
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        sys::close(self.0);
    }
}

impl Drop for PipeWriter {
    fn drop(&mut self) {
        sys::close(self.0);
    }
}
