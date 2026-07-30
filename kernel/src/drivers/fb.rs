//! Consola framebuffer mínima (UEFI GOP / bootloader).
//!
//! El bootloader ya mapea el buffer; `buffer_start` es una VA usable.
//! Espejo de diagnóstico para `println!`; SSH sigue siendo la consola de uso.

use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use core::fmt::{self, Write};
use spin::Mutex;

struct FbState {
    ptr: *mut u8,
    info: FrameBufferInfo,
    row: usize,
    col: usize,
}

unsafe impl Send for FbState {}

static FB: Mutex<Option<FbState>> = Mutex::new(None);

const COLS: usize = 80;
const ROWS: usize = 40;
const GLYPH_W: usize = 8;
const GLYPH_H: usize = 8;

fn glyph_row(ch: u8, row: usize) -> u8 {
    if ch == b' ' {
        return 0;
    }
    // Patrón legible a duras penas (sin fuente completa).
    let seed = ch.wrapping_mul(31).wrapping_add(row as u8 * 17);
    if ch.is_ascii_alphanumeric() {
        0x3c | (seed & 0xc3)
    } else if ch == b'\n' || ch == b'\r' {
        0
    } else {
        0x18
    }
}

/// `buffer_start` es VA del bootloader; `info` describe el layout.
pub fn init(buffer_start: u64, info: FrameBufferInfo) {
    let ptr = buffer_start as *mut u8;
    unsafe {
        core::ptr::write_bytes(ptr, 0, info.byte_len);
    }
    *FB.lock() = Some(FbState {
        ptr,
        info,
        row: 0,
        col: 0,
    });
    // No usar println! aquí: puede reentrar al espejo FB. El caller loguea.
}

#[allow(dead_code)]
pub fn available() -> bool {
    FB.lock().is_some()
}

pub fn info_log() -> Option<(usize, usize, usize, usize)> {
    FB.lock().as_ref().map(|s| {
        (
            s.info.width,
            s.info.height,
            s.info.stride,
            s.info.bytes_per_pixel,
        )
    })
}

fn put_pixel(st: &FbState, x: usize, y: usize, r: u8, g: u8, b: u8) {
    if x >= st.info.width || y >= st.info.height {
        return;
    }
    let bpp = st.info.bytes_per_pixel;
    let off = (y * st.info.stride + x) * bpp;
    unsafe {
        let p = st.ptr.add(off);
        match st.info.pixel_format {
            PixelFormat::Rgb => {
                *p = r;
                if bpp > 1 {
                    *p.add(1) = g;
                }
                if bpp > 2 {
                    *p.add(2) = b;
                }
            }
            PixelFormat::Bgr => {
                *p = b;
                if bpp > 1 {
                    *p.add(1) = g;
                }
                if bpp > 2 {
                    *p.add(2) = r;
                }
            }
            PixelFormat::U8 => {
                *p = r.max(g).max(b);
            }
            _ => {
                *p = r;
                if bpp > 1 {
                    *p.add(1) = g;
                }
                if bpp > 2 {
                    *p.add(2) = b;
                }
            }
        }
    }
}

fn draw_char(st: &mut FbState, ch: u8) {
    if ch == b'\n' {
        st.col = 0;
        st.row += 1;
        if st.row >= ROWS.min(st.info.height / GLYPH_H) {
            st.row = 0;
        }
        return;
    }
    if ch == b'\r' {
        st.col = 0;
        return;
    }
    let max_cols = COLS.min(st.info.width / GLYPH_W);
    let max_rows = ROWS.min(st.info.height / GLYPH_H);
    let x0 = st.col * GLYPH_W;
    let y0 = st.row * GLYPH_H;
    for row in 0..GLYPH_H {
        let bits = glyph_row(ch, row);
        for col in 0..GLYPH_W {
            let on = bits & (0x80 >> col) != 0;
            let (r, g, b) = if on { (0xc8, 0xd0, 0xb0) } else { (0, 0, 0) };
            put_pixel(st, x0 + col, y0 + row, r, g, b);
        }
    }
    st.col += 1;
    if st.col >= max_cols {
        st.col = 0;
        st.row += 1;
        if st.row >= max_rows {
            st.row = 0;
        }
    }
}

pub fn write_bytes(s: &[u8]) {
    let mut guard = FB.lock();
    let Some(st) = guard.as_mut() else { return };
    for &c in s {
        draw_char(st, c);
    }
}

#[allow(dead_code)]
pub struct FbWriter;

impl Write for FbWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_bytes(s.as_bytes());
        Ok(())
    }
}
