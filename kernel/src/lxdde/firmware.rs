//! G2: request_firmware — carga blobs desde sosofs (/lib/firmware/…).

use crate::vfs;
use alloc::vec::Vec;
use spin::Mutex;

static FIRMWARE_CACHE: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

pub fn init() {}

#[unsafe(no_mangle)]
pub extern "C" fn lx_request_firmware(
    name: *const u8,
    out_data: *mut *const u8,
    out_len: *mut usize,
) -> i32 {
    if name.is_null() || out_data.is_null() || out_len.is_null() {
        return -1;
    }
    let path = unsafe {
        let mut len = 0usize;
        while *name.add(len) != 0 {
            len += 1;
            if len > 256 {
                return -1;
            }
        }
        let slice = core::slice::from_raw_parts(name, len);
        alloc::string::String::from_utf8_lossy(slice).into_owned()
    };
    let vfs_path = if path.starts_with('/') {
        path
    } else {
        alloc::format!("/lib/firmware/{path}")
    };
    match vfs::resolve(&vfs_path).and_then(|ino| vfs::read_file(ino)) {
        Ok(buf) => {
            let len = buf.len();
            let mut cache = FIRMWARE_CACHE.lock();
            cache.push(buf);
            let data = cache.last().unwrap();
            unsafe {
                *out_data = data.as_ptr();
                *out_len = len;
            }
            crate::println!("lxdde-fw: cargado {} ({} bytes)", vfs_path, len);
            0
        }
        Err(_) => {
            crate::println!("lxdde-fw: no encontrado {}", vfs_path);
            -2
        }
    }
}

/// Libera de verdad el blob: el llamante (`load_one` en `gsp_fw.c`) ya se ha
/// quedado con su propia copia. Sin esto la caché retenía cada firmware para
/// siempre — 121 MiB solo entre los dos ucode GSP de 60,6 MiB.
#[unsafe(no_mangle)]
pub extern "C" fn lx_release_firmware(data: *const u8) {
    if data.is_null() {
        return;
    }
    let mut cache = FIRMWARE_CACHE.lock();
    if let Some(i) = cache.iter().position(|b| b.as_ptr() == data) {
        cache.swap_remove(i);
    }
}
