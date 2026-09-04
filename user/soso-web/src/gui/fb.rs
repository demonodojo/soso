//! Acceso al framebuffer vía syscalls.

use libsoso::sys;
use soso_abi::FbInfo;

pub struct Framebuffer {
    pub info: FbInfo,
    pub back: alloc::vec::Vec<u8>,
}

impl Framebuffer {
    pub fn open() -> Result<Self, i64> {
        let mut info = FbInfo::default();
        if sys::fb_info(&mut info) < 0 {
            return Err(-soso_abi::ENOTSUP);
        }
        if info.present == 0 {
            return Err(-soso_abi::ENOTSUP);
        }
        sys::fb_set_mode(soso_abi::FB_MODE_GRAPHICS)?;
        let bytes = info.byte_len as usize;
        Ok(Self {
            info,
            back: alloc::vec![0u8; bytes],
        })
    }

    pub fn present(&self) -> Result<(), i64> {
        sys::fb_present(self.back.as_ptr() as u64, self.back.len() as u64)
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        let _ = sys::fb_set_mode(soso_abi::FB_MODE_CONSOLE);
    }
}
