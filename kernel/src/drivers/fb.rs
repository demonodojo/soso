//! Consola framebuffer mínima (UEFI GOP / bootloader).
//!
//! Buffer de texto de hasta 240×135 celdas (recortado a la resolución real).
//! Scroll en modo "redraw" (como fbcon de Linux sobre efifb/simpledrm):
//! cada fila se compone en RAM y se vuelca a VRAM de una pasada; jamás se
//! lee del framebuffer, que en hardware real es write-combining y leerlo
//! cuesta un orden de magnitud más que escribirlo.

use alloc::vec::Vec;
use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use core::fmt::{self, Write};
use font8x8::legacy::BASIC_LEGACY;
use spin::Mutex;

struct FbState {
    ptr: *mut u8,
    info: FrameBufferInfo,
    mapped_height: usize,
    row: usize,
    col: usize,
    scale: usize,
    cell_w: usize,
    cell_h: usize,
    /// ROWS filas de COLS celdas, en heap: inline (32 KiB) reventaba el stack
    /// de arranque de 80 KiB al construir `FbState` (double fault en debug,
    /// que duplica el agregado en temporales).
    text: Vec<[u8; COLS]>,
    /// Fila de celdas compuesta en RAM antes de volcarla a VRAM de una
    /// pasada (bpl × cell_h bytes). El framebuffer real es write-combining:
    /// escribir es rápido pero leerlo es un orden de magnitud más lento,
    /// así que nunca se usa como origen ni se pinta dos veces (parpadeo).
    rowbuf: Vec<u8>,
}

unsafe impl Send for FbState {}

static FB: Mutex<Option<FbState>> = Mutex::new(None);

/// Dimensión máxima del buffer de texto: 4K entera a 2× (3840/16 × 2160/16).
/// El área activa la recortan `max_cols`/`max_rows` según la resolución real.
const COLS: usize = 240;
const ROWS: usize = 135;
const GLYPH_W: usize = 8;
const GLYPH_H: usize = 8;
const FG: (u8, u8, u8) = (0xc8, 0xd0, 0xb0);

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

/// 2× = celdas de 16 px, el mismo raster que las trazas del bootloader
/// (validado en placa real: 3×/4× resulta enorme y 1× ilegible). 1× solo
/// para framebuffers pequeños (QEMU con ventana chica), donde 8 px se lee bien.
fn choose_scale(width: usize, height: usize) -> usize {
    if width >= 1280 && height >= 720 { 2 } else { 1 }
}

fn glyph_row(ch: u8, row: usize) -> u8 {
    if row >= GLYPH_H {
        return 0;
    }
    let idx = if (ch as usize) < BASIC_LEGACY.len() {
        ch as usize
    } else {
        b'?' as usize
    };
    BASIC_LEGACY[idx][row]
}

pub fn init(buffer_start: u64, info: FrameBufferInfo) {
    let ptr = buffer_start as *mut u8;
    let mapped_height = mapped_height(&info);
    let scale = choose_scale(info.width, mapped_height);
    let cell_w = GLYPH_W * scale;
    let cell_h = GLYPH_H * scale;
    let rowbuf = alloc::vec![0u8; bytes_per_scanline(&info) * cell_h];
    // Limpiar solo lo mapeado: byte_len puede exceder el mapeo del bootloader
    // (mismo motivo por el que existe `mapped_height`).
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
        text: alloc::vec![[b' '; COLS]; ROWS],
        rowbuf,
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

fn paint_glyph(st: &FbState, row: usize, col: usize, ch: u8) {
    if ch == b' ' || ch == 0 {
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
            let bits = glyph_row(ch, gy);
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
    let ch = st.text[row][col];
    if ch != b' ' {
        paint_glyph(st, row, col, ch);
    }
}

/// Compone la fila `row` entera en `rowbuf` (RAM cacheada) y la vuelca a
/// VRAM con un solo memcpy. Es el modo "redraw" de fbcon en Linux
/// (efifb/simpledrm): el scroll nunca lee del framebuffer ni escribe dos
/// veces el mismo píxel, solo escrituras secuenciales write-combining.
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
        let ch = st.text[row][c];
        if ch == b' ' || ch == 0 {
            continue;
        }
        let x0 = c * st.cell_w;
        for gy in 0..GLYPH_H {
            let bits = glyph_row(ch, gy);
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

/// Tras un scroll el texto de todas las filas cambia: se redibujan todas
/// desde el buffer de texto (nunca se desplazan píxeles en VRAM).
fn sync_rows_after_scroll(st: &mut FbState, scroll_lines: usize) {
    if scroll_lines == 0 {
        return;
    }
    for r in 0..max_rows(st) {
        paint_row(st, r);
    }
}

/// Solo mueve el buffer de texto; el FB se sincroniza al final del write.
fn scroll_text(st: &mut FbState) {
    let rows = max_rows(st);
    if rows == 0 {
        return;
    }
    for r in 0..rows.saturating_sub(1) {
        st.text[r] = st.text[r + 1];
    }
    st.text[rows - 1] = [b' '; COLS];
}

fn cursor_bar(st: &FbState) -> (usize, usize, usize, usize) {
    let y0 = st.row * st.cell_h;
    let bar_y = y0 + st.cell_h.saturating_sub(st.scale);
    (st.col * st.cell_w, bar_y, st.cell_w, st.scale)
}

fn erase_cursor(st: &FbState) {
    refresh_cell(st, st.row, st.col);
}

fn draw_cursor(st: &FbState) {
    let (x, y, w, h) = cursor_bar(st);
    if y < st.mapped_height {
        fill_rect(st, x, y, w, h, FG.0, FG.1, FG.2);
    }
}

/// Devuelve `true` si hubo scroll (hace falta redibujar el plano).
fn draw_char(st: &mut FbState, ch: u8, defer_paint: bool) -> bool {
    let cols = max_cols(st);
    let rows = max_rows(st);
    if cols == 0 || rows == 0 {
        return false;
    }
    let mut scrolled = false;

    if ch == 0x08 || ch == 0x7f {
        if st.col > 0 {
            st.col -= 1;
            st.text[st.row][st.col] = b' ';
            if !defer_paint {
                clear_cell(st, st.row, st.col);
            }
        }
        return false;
    }
    if ch == b'\n' {
        st.col = 0;
        st.row += 1;
        if st.row >= rows {
            scroll_text(st);
            st.row = rows - 1;
            scrolled = true;
        }
        return scrolled;
    }
    if ch == b'\r' {
        st.col = 0;
        st.text[st.row] = [b' '; COLS];
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
    st.text[row][col] = ch;
    if !defer_paint && !scrolled {
        clear_cell(st, row, col);
        paint_glyph(st, row, col, ch);
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

/// Solo panic handler: suelta el lock si el contexto interrumpido lo tenía,
/// para que el mensaje de panic llegue a la pantalla.
///
/// # Safety
/// El poseedor anterior del lock ya no va a continuar (estamos en panic).
pub unsafe fn force_unlock() {
    unsafe { FB.force_unlock() };
}

pub fn write_bytes(s: &[u8]) {
    let mut guard = FB.lock();
    let Some(st) = guard.as_mut() else { return };

    erase_cursor(st);

    let mut scroll_lines = 0usize;
    for &c in s {
        if draw_char(st, c, scroll_lines > 0) {
            scroll_lines += 1;
        }
    }

    if scroll_lines > 0 {
        sync_rows_after_scroll(st, scroll_lines);
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
