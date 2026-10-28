//! Salida lx_emul → log del kernel (serie / dmesg), sin framebuffer.

use core::ffi::c_char;

#[unsafe(no_mangle)]
pub extern "C" fn lx_puts(s: *const c_char) {
    if s.is_null() {
        return;
    }
    unsafe {
        let mut len = 0usize;
        while *s.add(len) != 0 {
            len += 1;
        }
        let slice = core::slice::from_raw_parts(s as *const u8, len);
        if let Ok(txt) = core::str::from_utf8(slice) {
            crate::klog!("{txt}");
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_putchar(c: u8) {
    crate::klog!("{}", c as char);
}
