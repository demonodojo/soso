//! Consola framebuffer mínima (UEFI GOP / bootloader).
//!
//! Buffer de texto de hasta 240×135 celdas (recortado a la resolución real).
//! Decodifica UTF-8 con estado (eco byte a byte desde userspace) y pinta un
//! glifo 8×8 por carácter Unicode (ASCII + Latin-1 + €).

use alloc::vec::Vec;
use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};
use font8x8::legacy::{BASIC_LEGACY, LATIN_LEGACY};
use spin::Mutex;
use soso_abi::{FbInfo, FB_FMT_BGR, FB_FMT_RGB, FB_FMT_U8};

const CELL_SPACE: u16 = b' ' as u16;

struct Utf8Acc {
    buf: [u8; 4],
    len: u8,
    need: u8,
}

impl Utf8Acc {
    const fn new() -> Self {
        Self {
            buf: [0; 4],
            len: 0,
            need: 0,
        }
    }

    fn reset(&mut self) {
        self.len = 0;
        self.need = 0;
    }

    /// Devuelve `Some(cp)` cuando hay un carácter completo; `None` si falta
    /// continuación. Secuencias inválidas emiten `?`.
    fn feed(&mut self, b: u8) -> Option<u16> {
        if self.len == 0 {
            if b < 0x80 {
                return Some(b as u16);
            }
            self.buf[0] = b;
            self.len = 1;
            self.need = if b & 0xE0 == 0xC0 {
                2
            } else if b & 0xF0 == 0xE0 {
                3
            } else if b & 0xF8 == 0xF0 {
                4
            } else {
                self.reset();
                return Some(b'?' as u16);
            };
            return None;
        }

        if b & 0xC0 != 0x80 {
            self.reset();
            return Some(b'?' as u16);
        }

        self.buf[self.len as usize] = b;
        self.len += 1;
        if self.len < self.need {
            return None;
        }

        let cp = decode_utf8(&self.buf[..self.len as usize]);
        self.reset();
        Some(cp)
    }
}

fn decode_utf8(bytes: &[u8]) -> u16 {
    let c = match bytes {
        [b0, b1] if b0 & 0xE0 == 0xC0 => {
            let cp = ((b0 & 0x1F) as u32) << 6 | (b1 & 0x3F) as u32;
            if cp < 0x80 {
                return b'?' as u16;
            }
            cp
        }
        [b0, b1, b2] if b0 & 0xF0 == 0xE0 => {
            ((b0 & 0x0F) as u32) << 12
                | ((b1 & 0x3F) as u32) << 6
                | (b2 & 0x3F) as u32
        }
        [b0, b1, b2, b3] if b0 & 0xF8 == 0xF0 => {
            let cp = ((b0 & 0x07) as u32) << 18
                | ((b1 & 0x3F) as u32) << 12
                | ((b2 & 0x3F) as u32) << 6
                | (b3 & 0x3F) as u32;
            if cp > 0xFFFF {
                return b'?' as u16;
            }
            cp
        }
        _ => return b'?' as u16,
    };
    if c > 0xFFFF {
        b'?' as u16
    } else {
        c as u16
    }
}

struct FbState {
    ptr: *mut u8,
    info: FrameBufferInfo,
    mapped_height: usize,
    row: usize,
    col: usize,
    scale: usize,
    cell_w: usize,
    cell_h: usize,
    text: Vec<[u16; COLS]>,
    rowbuf: Vec<u8>,
    utf8: Utf8Acc,
}

unsafe impl Send for FbState {}

static FB: Mutex<Option<FbState>> = Mutex::new(None);
static GRAPHICS_MODE: AtomicBool = AtomicBool::new(false);

const COLS: usize = 240;
const ROWS: usize = 135;
const GLYPH_W: usize = 8;
const GLYPH_H: usize = 8;
const FG: (u8, u8, u8) = (0xc8, 0xd0, 0xb0);

/// Glifo 8×8 para U+20AC (€), ausente en LATIN_LEGACY.
const GLYPH_EURO: [u8; 8] = [
    0b00111100,
    0b01100110,
    0b01100000,
    0b00111100,
    0b01100000,
    0b01100110,
    0b00111100,
    0b00000000,
];

fn bytes_per_scanline(info: &FrameBufferInfo) -> usize {
    info.stride * info.bytes_per_pixel
}

fn mapped_height(info: &FrameBufferInfo) -> usize {
    let bpl = bytes_per_scanline(info);
    if bpl == 0 {
        return info.height;
    }
    (info.byte_len / bpl).min(info.height)
}

fn choose_scale(width: usize, height: usize) -> usize {
    if width >= 1280 && height >= 720 {
        2
    } else {
        1
    }
}

fn glyph_row_cp(cp: u16, row: usize) -> u8 {
    if row >= GLYPH_H {
        return 0;
    }
    if cp == 0x20AC {
        return GLYPH_EURO[row];
    }
    if cp < 0x80 {
        let idx = cp as usize;
        if idx < BASIC_LEGACY.len() {
            return BASIC_LEGACY[idx][row];
        }
    } else if (0xA0..=0xFF).contains(&cp) {
        let idx = (cp - 0xA0) as usize;
        if idx < LATIN_LEGACY.len() {
            return LATIN_LEGACY[idx][row];
        }
    }
    BASIC_LEGACY[b'?' as usize][row]
}

pub fn init(buffer_start: u64, info: FrameBufferInfo) {
    let ptr = buffer_start as *mut u8;
    let mapped_height = mapped_height(&info);
    let scale = choose_scale(info.width, mapped_height);
    let cell_w = GLYPH_W * scale;
    let cell_h = GLYPH_H * scale;
    let rowbuf = alloc::vec![0u8; bytes_per_scanline(&info) * cell_h];
    unsafe {
        core::ptr::write_bytes(ptr, 0, bytes_per_scanline(&info) * mapped_height);
    }
    *FB.lock() = Some(FbState {
        ptr,
        info,
        mapped_height,
        row: 0,
        col: 0,
        scale,
        cell_w,
        cell_h,
        text: alloc::vec![[CELL_SPACE; COLS]; ROWS],
        rowbuf,
        utf8: Utf8Acc::new(),
    });
}

#[allow(dead_code)]
pub fn available() -> bool {
    FB.lock().is_some()
}

pub fn info_log() -> Option<(usize, usize, usize, usize, usize, usize)> {
    FB.lock().as_ref().map(|s| {
        (
            s.info.width,
            s.info.height,
            s.mapped_height,
            s.info.stride,
            s.info.bytes_per_pixel,
            s.scale,
        )
    })
}

fn pack_pixel(st: &FbState, r: u8, g: u8, b: u8) -> [u8; 4] {
    let mut px = [0u8; 4];
    match st.info.pixel_format {
        PixelFormat::Rgb => {
            px[0] = r;
            px[1] = g;
            px[2] = b;
            px[3] = 0xff;
        }
        PixelFormat::Bgr => {
            px[0] = b;
            px[1] = g;
            px[2] = r;
            px[3] = 0xff;
        }
        PixelFormat::U8 => {
            px[0] = r.max(g).max(b);
        }
        _ => {
            px[0] = r;
            px[1] = g;
            px[2] = b;
            px[3] = 0xff;
        }
    }
    px
}

fn max_cols(st: &FbState) -> usize {
    COLS.min(st.info.width / st.cell_w)
}

fn max_rows(st: &FbState) -> usize {
    ROWS.min(st.mapped_height / st.cell_h)
}

fn fill_rect(st: &FbState, x0: usize, y0: usize, w: usize, h: usize, r: u8, g: u8, b: u8) {
    if w == 0 || h == 0 {
        return;
    }
    let x0 = x0.min(st.info.width);
    let y0 = y0.min(st.mapped_height);
    let w = w.min(st.info.width.saturating_sub(x0));
    let h = h.min(st.mapped_height.saturating_sub(y0));
    if w == 0 || h == 0 {
        return;
    }
    let bpp = st.info.bytes_per_pixel;
    let bpl = bytes_per_scanline(&st.info);
    let row_bytes = w * bpp;
    let start = y0 * bpl + x0 * bpp;
    if start + (h - 1) * bpl + row_bytes > st.info.byte_len {
        return;
    }
    if r == 0 && g == 0 && b == 0 {
        unsafe {
            if x0 == 0 && w == st.info.width && bpl * h <= st.info.byte_len.saturating_sub(start)
            {
                core::ptr::write_bytes(st.ptr.add(start), 0, bpl * h);
            } else {
                for dy in 0..h {
                    core::ptr::write_bytes(st.ptr.add(start + dy * bpl), 0, row_bytes);
                }
            }
        }
        return;
    }
    let px = pack_pixel(st, r, g, b);
    unsafe {
        for dy in 0..h {
            let mut p = st.ptr.add(start + dy * bpl);
            for _ in 0..w {
                core::ptr::copy_nonoverlapping(px.as_ptr(), p, bpp);
                p = p.add(bpp);
            }
        }
    }
}

fn paint_glyph(st: &FbState, row: usize, col: usize, cp: u16) {
    if cp == CELL_SPACE || cp == 0 {
        return;
    }
    let x0 = col * st.cell_w;
    let y0 = row * st.cell_h;
    let bpp = st.info.bytes_per_pixel;
    let bpl = bytes_per_scanline(&st.info);
    let scale = st.scale;
    let px = pack_pixel(st, FG.0, FG.1, FG.2);
    unsafe {
        for gy in 0..GLYPH_H {
            let bits = glyph_row_cp(cp, gy);
            for gx in 0..GLYPH_W {
                if bits & (1 << gx) == 0 {
                    continue;
                }
                let px0 = x0 + gx * scale;
                let py0 = y0 + gy * scale;
                for dy in 0..scale {
                    let y = py0 + dy;
                    if y >= st.mapped_height {
                        break;
                    }
                    let mut p = st.ptr.add(y * bpl + px0 * bpp);
                    for _ in 0..scale {
                        core::ptr::copy_nonoverlapping(px.as_ptr(), p, bpp);
                        p = p.add(bpp);
                    }
                }
            }
        }
    }
}

fn clear_cell(st: &FbState, row: usize, col: usize) {
    fill_rect(
        st,
        col * st.cell_w,
        row * st.cell_h,
        st.cell_w,
        st.cell_h,
        0,
        0,
        0,
    );
}

fn refresh_cell(st: &FbState, row: usize, col: usize) {
    clear_cell(st, row, col);
    let cp = st.text[row][col];
    if cp != CELL_SPACE {
        paint_glyph(st, row, col, cp);
    }
}

fn paint_row(st: &mut FbState, row: usize) {
    let bpl = bytes_per_scanline(&st.info);
    let y0 = row * st.cell_h;
    let h = st.cell_h.min(st.mapped_height.saturating_sub(y0));
    let bytes = (h * bpl).min(st.info.byte_len.saturating_sub(y0 * bpl));
    if bytes == 0 || st.rowbuf.len() < bytes {
        return;
    }
    let cols = max_cols(st);
    let bpp = st.info.bytes_per_pixel;
    let scale = st.scale;
    let px = pack_pixel(st, FG.0, FG.1, FG.2);

    let mut buf = core::mem::take(&mut st.rowbuf);
    buf[..bytes].fill(0);
    for c in 0..cols {
        let cp = st.text[row][c];
        if cp == CELL_SPACE || cp == 0 {
            continue;
        }
        let x0 = c * st.cell_w;
        for gy in 0..GLYPH_H {
            let bits = glyph_row_cp(cp, gy);
            if bits == 0 {
                continue;
            }
            for gx in 0..GLYPH_W {
                if bits & (1 << gx) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    let y = gy * scale + dy;
                    if y >= h {
                        break;
                    }
                    let mut off = y * bpl + (x0 + gx * scale) * bpp;
                    for _ in 0..scale {
                        buf[off..off + bpp].copy_from_slice(&px[..bpp]);
                        off += bpp;
                    }
                }
            }
        }
    }
    unsafe {
        core::ptr::copy_nonoverlapping(buf.as_ptr(), st.ptr.add(y0 * bpl), bytes);
    }
    st.rowbuf = buf;
}

fn sync_rows_after_scroll(st: &mut FbState) {
    for r in 0..max_rows(st) {
        paint_row(st, r);
    }
}

fn scroll_text(st: &mut FbState) {
    let rows = max_rows(st);
    if rows == 0 {
        return;
    }
    for r in 0..rows.saturating_sub(1) {
        st.text[r] = st.text[r + 1];
    }
    st.text[rows - 1] = [CELL_SPACE; COLS];
}

fn erase_cursor(st: &FbState) {
    refresh_cell(st, st.row, st.col);
}

fn draw_cursor(st: &FbState) {
    let y0 = st.row * st.cell_h;
    let bar_y = y0 + st.cell_h.saturating_sub(st.scale);
    let x = st.col * st.cell_w;
    if bar_y < st.mapped_height {
        fill_rect(st, x, bar_y, st.cell_w, st.scale, FG.0, FG.1, FG.2);
    }
}

fn draw_codepoint(st: &mut FbState, cp: u16, defer_paint: bool) -> bool {
    let cols = max_cols(st);
    let rows = max_rows(st);
    if cols == 0 || rows == 0 {
        return false;
    }
    let mut scrolled = false;

    if cp == 0x08 || cp == 0x7f {
        if st.col > 0 {
            st.col -= 1;
            st.text[st.row][st.col] = CELL_SPACE;
            if !defer_paint {
                clear_cell(st, st.row, st.col);
            }
        }
        return false;
    }
    if cp == b'\n' as u16 {
        st.col = 0;
        st.row += 1;
        if st.row >= rows {
            scroll_text(st);
            st.row = rows - 1;
            scrolled = true;
        }
        return scrolled;
    }
    if cp == b'\r' as u16 {
        st.col = 0;
        st.text[st.row] = [CELL_SPACE; COLS];
        if !defer_paint {
            fill_rect(
                st,
                0,
                st.row * st.cell_h,
                st.info.width,
                st.cell_h
                    .min(st.mapped_height.saturating_sub(st.row * st.cell_h)),
                0,
                0,
                0,
            );
        }
        return false;
    }

    if st.col >= cols {
        st.col = 0;
        st.row += 1;
        if st.row >= rows {
            scroll_text(st);
            st.row = rows - 1;
            scrolled = true;
        }
    }
    let row = st.row;
    let col = st.col;
    st.text[row][col] = cp;
    if !defer_paint && !scrolled {
        clear_cell(st, row, col);
        paint_glyph(st, row, col, cp);
    }
    st.col += 1;
    if st.col >= cols {
        st.col = 0;
        st.row += 1;
        if st.row >= rows {
            scroll_text(st);
            st.row = rows - 1;
            scrolled = true;
        }
    }
    scrolled
}

pub unsafe fn force_unlock() {
    unsafe { FB.force_unlock() };
}

pub fn write_bytes(s: &[u8]) {
    if graphics_mode() {
        return;
    }
    let mut guard = FB.lock();
    let Some(st) = guard.as_mut() else {
        return;
    };

    erase_cursor(st);

    let mut scroll_lines = 0usize;
    for &b in s {
        if let Some(cp) = st.utf8.feed(b) {
            if draw_codepoint(st, cp, scroll_lines > 0) {
                scroll_lines += 1;
            }
        }
    }

    if scroll_lines > 0 {
        sync_rows_after_scroll(st);
    }
    draw_cursor(st);
}

#[allow(dead_code)]
pub struct FbWriter;

impl Write for FbWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_bytes(s.as_bytes());
        Ok(())
    }
}

pub fn set_graphics_mode(on: bool) {
    GRAPHICS_MODE.store(on, Ordering::Relaxed);
    if !on {
        let mut guard = FB.lock();
        if let Some(st) = guard.as_mut() {
            erase_cursor(st);
            draw_cursor(st);
        }
    }
}

pub fn graphics_mode() -> bool {
    GRAPHICS_MODE.load(Ordering::Relaxed)
}

fn pixel_format_tag(fmt: PixelFormat) -> u8 {
    match fmt {
        PixelFormat::Rgb => FB_FMT_RGB,
        PixelFormat::Bgr => FB_FMT_BGR,
        PixelFormat::U8 => FB_FMT_U8,
        _ => FB_FMT_RGB,
    }
}

pub fn user_info(out: &mut FbInfo) -> bool {
    let guard = FB.lock();
    let Some(st) = guard.as_ref() else {
        *out = FbInfo::default();
        return false;
    };
    out.present = 1;
    out.pixel_format = pixel_format_tag(st.info.pixel_format);
    out.bytes_per_pixel = st.info.bytes_per_pixel as u8;
    out.width = st.info.width as u32;
    out.height = st.info.height as u32;
    out.stride = st.info.stride as u32;
    out.byte_len = st.info.byte_len as u64;
    true
}

/// Copia un búfer de píxeles (mismo tamaño que `byte_len`) al framebuffer físico.
pub fn present_from_user(buf: &[u8]) -> Result<(), ()> {
    let mut guard = FB.lock();
    let Some(st) = guard.as_mut() else {
        return Err(());
    };
    let want = st.info.byte_len.min(buf.len());
    unsafe {
        core::ptr::copy_nonoverlapping(buf.as_ptr(), st.ptr, want);
    }
    Ok(())
}
