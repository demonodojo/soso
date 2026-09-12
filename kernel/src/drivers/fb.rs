//! Consola framebuffer mínima (UEFI GOP / bootloader).
//!
//! Buffer de texto de hasta 240×135 celdas (recortado a la resolución real).
//! Decodifica UTF-8 con estado (eco byte a byte desde userspace) y pinta un
//! glifo 8×8 o 12×12 por carácter Unicode (ASCII + Latin-1 + €), según
//! resolución.
//!
//! # Rotación
//!
//! Hay dos sistemas de coordenadas. El *lógico* es donde vive la consola: es
//! el que usan las celdas de texto y donde el shadow buffer guarda los píxeles.
//! El *físico* es el buffer del GOP. En un panel normal coinciden; en uno
//! montado girado (Steam Deck: 800×1280 vertical nativo) el volcado aplica la
//! rotación de `soso_hw::fbrot`.
//!
//! El volcado recorre **líneas físicas**, no lógicas: leer del shadow con
//! salto es barato porque está en RAM cacheada, mientras que escribir al GOP
//! salteado sería carísimo sobre memoria WC/UC. Con esa orientación, una
//! pantalla rotada cuesta lo mismo que el `memcpy` que ya hacía el scroll.

use alloc::vec::Vec;
use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use font8x8::legacy::{BASIC_LEGACY, LATIN_LEGACY};
use soso_abi::{FbInfo, FB_FMT_BGR, FB_FMT_RGB, FB_FMT_U8};
use soso_hw::fbrot::{self, Rot};
use spin::Mutex;

#[path = "font12x12.rs"]
mod font12x12;

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
            ((b0 & 0x0F) as u32) << 12 | ((b1 & 0x3F) as u32) << 6 | (b2 & 0x3F) as u32
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
    /// Copia en RAM de la consola, en **coordenadas lógicas**: aquí se pinta
    /// todo y de aquí sale el volcado al GOP (nunca se lee la VRAM del
    /// firmware AMD/UEFI). El scroll es un memmove sobre este buffer.
    shadow: Vec<u8>,
    info: FrameBufferInfo,
    mapped_height: usize,
    /// Rotación aplicada al volcar el shadow al GOP.
    rot: Rot,
    /// Dimensiones lógicas: `info.width`×`mapped_height` traspuestas si rota.
    log_w: usize,
    log_h: usize,
    row: usize,
    col: usize,
    cell_w: usize,
    cell_h: usize,
    glyph_w: usize,
    glyph_h: usize,
    text: Vec<[u16; COLS]>,
    rowbuf: Vec<u8>,
    /// Una línea física, para volcar rotado con escrituras secuenciales.
    linebuf: Vec<u8>,
    utf8: Utf8Acc,
    /// Instante (`tsc::now_ns`) del último scroll: decide si el siguiente es
    /// de una fila o un salto de ráfaga (`filas_salto`).
    last_scroll_ns: u64,
}

unsafe impl Send for FbState {}

static FB: Mutex<Option<FbState>> = Mutex::new(None);
static GRAPHICS_MODE: AtomicBool = AtomicBool::new(false);
/// Páginas del GOP remapeadas a write-combining (`usize::MAX` = no se pudo).
static WC_PAGES: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Dirección física del GOP, para el log de arranque.
static FB_PHYS: AtomicU64 = AtomicU64::new(0);

const COLS: usize = 240;
const ROWS: usize = 135;
const GLYPH_W: usize = 8;
const GLYPH_H: usize = 8;
const FG: (u8, u8, u8) = (0xc8, 0xd0, 0xb0);

/// Glifo 8×8 para U+20AC (€), ausente en LATIN_LEGACY.
const GLYPH_EURO: [u8; 8] = [
    0b00111100, 0b01100110, 0b01100000, 0b00111100, 0b01100000, 0b01100110, 0b00111100, 0b00000000,
];

fn bytes_per_scanline(info: &FrameBufferInfo) -> usize {
    info.stride * info.bytes_per_pixel
}

/// Copia al GOP con stores **non-temporal de 16 bytes** (`movntdq`).
///
/// El GOP es memoria UC o, tras `arch::pat`, WC: nunca pasa por la caché, así
/// que lo que cuesta es el número de transacciones. Un `memcpy` normal usa
/// `movaps` y en silicio (AMD GOP en concreto) eso es #GP por alineación: el
/// arranque se clavaba al primer scroll, antes de `fatlog`. La versión que
/// lo sustituyó (stores `u32` volátiles) no fallaba, pero en la ROG eran dos
/// millones de escrituras UC serializadas por pantalla (~100 ms por scroll).
///
/// Aquí el destino se alinea a 16 a mano con stores de 4/1 bytes (por eso
/// `movntdq`, que exige alineación, no puede fallar), el grueso va en
/// ráfagas de 16 bytes que el WC agrupa en líneas de 64, y la cola vuelve a
/// 4/1. El origen (shadow en RAM) se lee sin alinear. Quien llame debe cerrar
/// con `sfence` (`fin_copia_gop`): los stores NT son débilmente ordenados.
unsafe fn copy_to_gop(dst: *mut u8, src: *const u8, mut n: usize) {
    use core::arch::x86_64::{__m128i, _mm_loadu_si128, _mm_stream_si128};
    unsafe {
        let mut d = dst;
        let mut s = src;
        // Cabecera: hasta que el destino quede alineado a 16.
        while n > 0 && (d as usize) & 15 != 0 {
            if n >= 4 && (d as usize) & 3 == 0 {
                core::ptr::write_volatile(
                    d.cast::<u32>(),
                    core::ptr::read_unaligned(s.cast::<u32>()),
                );
                d = d.add(4);
                s = s.add(4);
                n -= 4;
            } else {
                core::ptr::write_volatile(d, core::ptr::read(s));
                d = d.add(1);
                s = s.add(1);
                n -= 1;
            }
        }
        while n >= 16 {
            let v = _mm_loadu_si128(s.cast::<__m128i>());
            _mm_stream_si128(d.cast::<__m128i>(), v);
            d = d.add(16);
            s = s.add(16);
            n -= 16;
        }
        while n >= 4 {
            core::ptr::write_volatile(d.cast::<u32>(), core::ptr::read_unaligned(s.cast::<u32>()));
            d = d.add(4);
            s = s.add(4);
            n -= 4;
        }
        while n > 0 {
            core::ptr::write_volatile(d, core::ptr::read(s));
            d = d.add(1);
            s = s.add(1);
            n -= 1;
        }
    }
}

/// Cierra una ronda de `copy_to_gop`: drena los búferes WC / stores NT para
/// que el panel vea los píxeles ahora y no cuando la CPU los desaloje.
#[inline]
fn fin_copia_gop() {
    unsafe { core::arch::x86_64::_mm_sfence() };
}

/// Bytes por línea **lógica** del shadow (sin el stride del panel: el shadow
/// es compacto).
fn bytes_per_logical_line(st: &FbState) -> usize {
    st.log_w * st.info.bytes_per_pixel
}

/// Vuelca al GOP el rectángulo lógico `(x0, y0, w, h)` aplicando la rotación.
///
/// Recorre las líneas del **destino físico** para que las escrituras al GOP
/// sean secuenciales; el salto se paga leyendo el shadow, que está en RAM.
fn flush_rect(st: &mut FbState, x0: usize, y0: usize, w: usize, h: usize) {
    let bpp = st.info.bytes_per_pixel;
    let bpl_f = bytes_per_scanline(&st.info);
    let bpl_l = bytes_per_logical_line(st);
    let x1 = (x0 + w).min(st.log_w);
    let y1 = (y0 + h).min(st.log_h);
    if x0 >= x1 || y0 >= y1 || bpp == 0 {
        return;
    }

    // Sin rotación el shadow y el panel tienen la misma orientación: cada
    // línea lógica es una línea física y basta copiarla entera.
    if st.rot.lineal() {
        let row_bytes = (x1 - x0) * bpp;
        for y in y0..y1 {
            let src = y * bpl_l + x0 * bpp;
            let dst = y * bpl_f + x0 * bpp;
            if dst + row_bytes > st.info.byte_len || src + row_bytes > st.shadow.len() {
                break;
            }
            unsafe {
                copy_to_gop(st.ptr.add(dst), st.shadow.as_ptr().add(src), row_bytes);
            }
        }
        fin_copia_gop();
        return;
    }

    let phys_w = st.info.width;
    let phys_h = st.mapped_height;
    let mut line = core::mem::take(&mut st.linebuf);

    // Cada línea física del destino es una fila o una columna del shadow, en
    // un sentido u otro. `paso` es el desplazamiento en el shadow entre dos
    // píxeles consecutivos de esa línea física.
    let (n_lineas, n_px, paso): (usize, usize, isize) = match st.rot {
        Rot::R90 | Rot::R270 => (x1 - x0, y1 - y0, bpl_l as isize),
        _ => (y1 - y0, x1 - x0, bpp as isize),
    };
    let paso = match st.rot {
        // El píxel físico avanza mientras el lógico retrocede.
        Rot::R270 | Rot::R180 => -paso,
        _ => paso,
    };

    for i in 0..n_lineas {
        // Origen en el shadow del primer píxel de esta línea física, y su
        // esquina en el panel.
        let (src0, px, py) = match st.rot {
            Rot::R90 => {
                let x = x0 + i;
                (y0 * bpl_l + x * bpp, y0, phys_h - 1 - x)
            }
            Rot::R270 => {
                let x = x0 + i;
                ((y1 - 1) * bpl_l + x * bpp, phys_w - y1, x)
            }
            _ => {
                let y = y0 + i;
                (y * bpl_l + (x1 - 1) * bpp, phys_w - x1, phys_h - 1 - y)
            }
        };
        let bytes = n_px * bpp;
        if line.len() < bytes {
            continue;
        }
        let mut src = src0 as isize;
        let mut off = 0usize;
        for _ in 0..n_px {
            if src < 0 || src as usize + bpp > st.shadow.len() {
                break;
            }
            let s = src as usize;
            line[off..off + bpp].copy_from_slice(&st.shadow[s..s + bpp]);
            off += bpp;
            src += paso;
        }
        let dst = py * bpl_f + px * bpp;
        if dst + off > st.info.byte_len {
            continue;
        }
        unsafe {
            copy_to_gop(st.ptr.add(dst), line.as_ptr(), off);
        }
    }
    st.linebuf = line;
    fin_copia_gop();
}

/// Vuelca la consola entera.
fn flush_all(st: &mut FbState) {
    flush_rect(st, 0, 0, st.log_w, st.log_h);
}

fn mapped_height(info: &FrameBufferInfo) -> usize {
    let bpl = bytes_per_scanline(info);
    if bpl == 0 {
        return info.height;
    }
    (info.byte_len / bpl).min(info.height)
}

/// Tamaño de celda en píxeles (glifo base 8×8 escalado proporcionalmente).
fn choose_cell_size(width: usize, height: usize) -> (usize, usize, usize) {
    // Paneles HD+: fuente Terminus 12×12 nativa (generada en build.rs).
    if width >= 1280 && height >= 720 {
        (12, font12x12::GLYPH_W, font12x12::GLYPH_H)
    } else {
        (GLYPH_W, GLYPH_W, GLYPH_H)
    }
}

/// Rango dentro de la celda para la fila/columna `g` del glifo 8×8.
fn glyph_cell_span(g: usize, cell: usize) -> (usize, usize) {
    let a = g * cell / GLYPH_W;
    let b = (g + 1) * cell / GLYPH_W;
    (a, b.max(a + 1))
}

fn glyph_row_cp(st: &FbState, cp: u16, row: usize) -> u16 {
    if row >= st.glyph_h {
        return 0;
    }
    if st.glyph_w == font12x12::GLYPH_W {
        return font12x12::row(cp, row);
    }
    if cp == 0x20AC {
        return GLYPH_EURO[row] as u16;
    }
    if cp < 0x80 {
        let idx = cp as usize;
        if idx < BASIC_LEGACY.len() {
            return BASIC_LEGACY[idx][row] as u16;
        }
    } else if (0xA0..=0xFF).contains(&cp) {
        let idx = (cp - 0xA0) as usize;
        if idx < LATIN_LEGACY.len() {
            return LATIN_LEGACY[idx][row] as u16;
        }
    }
    BASIC_LEGACY[b'?' as usize][row] as u16
}

fn glyph_native(st: &FbState) -> bool {
    st.cell_w == st.glyph_w && st.cell_h == st.glyph_h
}

/// Rotación pedida en el build (`SOSO_FB_ROT=0|90|180|270|auto`), o
/// automática por la forma del panel.
///
/// Un panel vertical se asume montado girado, que es el caso de la Steam Deck;
/// la anulación existe porque el sentido correcto (270 frente a 90) sólo se
/// confirma mirando la pantalla.
fn rot_inicial(phys_w: usize, phys_h: usize) -> Rot {
    match option_env!("SOSO_FB_ROT") {
        Some(spec) => Rot::parse(spec, phys_w, phys_h).unwrap_or(Rot::R0),
        None => Rot::automatica(phys_w, phys_h),
    }
}

pub fn init(buffer_start: u64, info: FrameBufferInfo) {
    let ptr = buffer_start as *mut u8;
    let mapped_height = mapped_height(&info);
    let rot = rot_inicial(info.width, mapped_height);
    let (log_w, log_h) = fbrot::dim_logica(rot, info.width, mapped_height);
    let (cell_w, glyph_w, glyph_h) = choose_cell_size(log_w, log_h);
    let cell_h = cell_w;
    let bpl_log = log_w * info.bytes_per_pixel;
    let rowbuf = alloc::vec![0u8; bpl_log * cell_h];
    // Una línea física completa: el volcado rotado la compone en RAM y la
    // escribe al GOP de una vez.
    let linebuf = alloc::vec![0u8; bytes_per_scanline(&info).max(bpl_log)];
    let shadow = alloc::vec![0u8; bpl_log * log_h];
    // Tipo de memoria del GOP: el bootloader lo deja WB, que sobre la MTRR UC
    // de la apertura es UC puro. Pasarlo a WC antes de escribir nada en él.
    crate::arch::pat::init_cpu();
    let wc = crate::mm::set_write_combining(buffer_start, info.byte_len as u64).ok();
    WC_PAGES.store(wc.unwrap_or(usize::MAX), Ordering::Relaxed);
    FB_PHYS.store(
        crate::mm::virt_to_phys(buffer_start).unwrap_or(0),
        Ordering::Relaxed,
    );
    unsafe {
        core::ptr::write_bytes(ptr, 0, bytes_per_scanline(&info) * mapped_height);
    }
    *FB.lock() = Some(FbState {
        ptr,
        shadow,
        info,
        mapped_height,
        rot,
        log_w,
        log_h,
        row: 0,
        col: 0,
        cell_w,
        cell_h,
        glyph_w,
        glyph_h,
        text: alloc::vec![[CELL_SPACE; COLS]; ROWS],
        rowbuf,
        linebuf,
        utf8: Utf8Acc::new(),
        last_scroll_ns: 0,
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
            s.cell_w,
        )
    })
}

/// Dirección física del GOP y páginas pasadas a WC (`None` si el PAT no lo
/// permitió), para el log de arranque.
pub fn mem_log() -> Option<(u64, Option<usize>)> {
    if !available() {
        return None;
    }
    let wc = match WC_PAGES.load(Ordering::Relaxed) {
        usize::MAX => None,
        n => Some(n),
    };
    Some((FB_PHYS.load(Ordering::Relaxed), wc))
}

/// Rotación activa y dimensiones lógicas, para el log de arranque.
pub fn rot_log() -> Option<(u16, usize, usize)> {
    FB.lock()
        .as_ref()
        .map(|s| (s.rot.grados(), s.log_w, s.log_h))
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
    COLS.min(st.log_w / st.cell_w)
}

fn max_rows(st: &FbState) -> usize {
    ROWS.min(st.log_h / st.cell_h)
}

fn fill_rect(st: &mut FbState, x0: usize, y0: usize, w: usize, h: usize, r: u8, g: u8, b: u8) {
    if w == 0 || h == 0 {
        return;
    }
    let x0 = x0.min(st.log_w);
    let y0 = y0.min(st.log_h);
    let w = w.min(st.log_w.saturating_sub(x0));
    let h = h.min(st.log_h.saturating_sub(y0));
    if w == 0 || h == 0 {
        return;
    }
    let bpp = st.info.bytes_per_pixel;
    let bpl = bytes_per_logical_line(st);
    let row_bytes = w * bpp;
    let start = y0 * bpl + x0 * bpp;
    if start + (h - 1) * bpl + row_bytes > st.shadow.len() {
        return;
    }
    if r == 0 && g == 0 && b == 0 {
        unsafe {
            if x0 == 0 && w == st.log_w && bpl * h <= st.shadow.len().saturating_sub(start) {
                core::ptr::write_bytes(st.shadow.as_mut_ptr().add(start), 0, bpl * h);
            } else {
                for dy in 0..h {
                    core::ptr::write_bytes(
                        st.shadow.as_mut_ptr().add(start + dy * bpl),
                        0,
                        row_bytes,
                    );
                }
            }
        }
        flush_rect(st, x0, y0, w, h);
        return;
    }
    let px = pack_pixel(st, r, g, b);
    unsafe {
        for dy in 0..h {
            let mut s = st.shadow.as_mut_ptr().add(start + dy * bpl);
            for _ in 0..w {
                core::ptr::copy_nonoverlapping(px.as_ptr(), s, bpp);
                s = s.add(bpp);
            }
        }
    }
    flush_rect(st, x0, y0, w, h);
}

fn paint_glyph(st: &mut FbState, row: usize, col: usize, cp: u16) {
    if cp == CELL_SPACE || cp == 0 {
        return;
    }
    let x0 = col * st.cell_w;
    let y0 = row * st.cell_h;
    let bpp = st.info.bytes_per_pixel;
    let bpl = bytes_per_logical_line(st);
    let px = pack_pixel(st, FG.0, FG.1, FG.2);
    let native = glyph_native(st);
    unsafe {
        for gy in 0..st.glyph_h {
            let bits = glyph_row_cp(st, cp, gy);
            for gx in 0..st.glyph_w {
                if bits & (1 << gx) == 0 {
                    continue;
                }
                let (px0, py0, pw, ph) = if native {
                    (x0 + gx, y0 + gy, 1, 1)
                } else {
                    let (sy0, sy1) = glyph_cell_span(gy, st.cell_h);
                    let (sx0, sx1) = glyph_cell_span(gx, st.cell_w);
                    (
                        x0 + sx0,
                        y0 + sy0,
                        sx1 - sx0,
                        sy1 - sy0,
                    )
                };
                let pw = pw.min(st.log_w.saturating_sub(px0));
                let ph = ph.min(st.log_h.saturating_sub(py0));
                if pw == 0 || ph == 0 {
                    continue;
                }
                for dy in 0..ph {
                    let y = py0 + dy;
                    let mut s = st.shadow.as_mut_ptr().add(y * bpl + px0 * bpp);
                    for _ in 0..pw {
                        core::ptr::copy_nonoverlapping(px.as_ptr(), s, bpp);
                        s = s.add(bpp);
                    }
                }
            }
        }
    }
    flush_rect(st, x0, y0, st.cell_w, st.cell_h);
}

fn clear_cell(st: &mut FbState, row: usize, col: usize) {
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

fn refresh_cell(st: &mut FbState, row: usize, col: usize) {
    clear_cell(st, row, col);
    let cp = st.text[row][col];
    if cp != CELL_SPACE {
        paint_glyph(st, row, col, cp);
    }
}

/// Rasteriza la fila `row` en el shadow. **No vuelca** al GOP: quien la llame
/// agrupa el volcado (tras un scroll se repintan varias filas y basta un
/// `flush_all`).
fn paint_row(st: &mut FbState, row: usize) {
    let bpl = bytes_per_logical_line(st);
    let y0 = row * st.cell_h;
    let h = st.cell_h.min(st.log_h.saturating_sub(y0));
    let bytes = (h * bpl).min(st.shadow.len().saturating_sub(y0 * bpl));
    if bytes == 0 || st.rowbuf.len() < bytes {
        return;
    }
    let cols = max_cols(st);
    let bpp = st.info.bytes_per_pixel;
    let px = pack_pixel(st, FG.0, FG.1, FG.2);

    let mut buf = core::mem::take(&mut st.rowbuf);
    buf[..bytes].fill(0);
    for c in 0..cols {
        let cp = st.text[row][c];
        if cp == CELL_SPACE || cp == 0 {
            continue;
        }
        let x0 = c * st.cell_w;
        let native = glyph_native(st);
        for gy in 0..st.glyph_h {
            let bits = glyph_row_cp(st, cp, gy);
            if bits == 0 {
                continue;
            }
            for gx in 0..st.glyph_w {
                if bits & (1 << gx) == 0 {
                    continue;
                }
                let (cx0, cy0, pw, ph) = if native {
                    (gx, gy, 1, 1)
                } else {
                    let (sy0, sy1) = glyph_cell_span(gy, st.cell_h);
                    let (sx0, sx1) = glyph_cell_span(gx, st.cell_w);
                    (sx0, sy0, sx1 - sx0, sy1 - sy0)
                };
                for dy in cy0..(cy0 + ph).min(h) {
                    let y = dy;
                    let pw = pw.min(st.cell_w.saturating_sub(cx0));
                    let mut off = y * bpl + (x0 + cx0) * bpp;
                    for _ in 0..pw {
                        if off + bpp > bytes {
                            break;
                        }
                        buf[off..off + bpp].copy_from_slice(&px[..bpp]);
                        off += bpp;
                    }
                }
            }
        }
    }
    unsafe {
        let off = y0 * bpl;
        core::ptr::copy_nonoverlapping(buf.as_ptr(), st.shadow.as_mut_ptr().add(off), bytes);
    }
    st.rowbuf = buf;
}

fn fila_vacia(st: &FbState, row: usize) -> bool {
    st.text[row].iter().all(|&c| c == CELL_SPACE || c == 0)
}

/// Aplica al shadow un scroll de `scroll_lines` filas de texto y lo vuelca
/// **una sola vez**.
///
/// El desplazamiento es un `memmove` en RAM; la banda que queda libre se pone
/// a negro y sólo se rasterizan las filas de esa banda que tienen texto (tras
/// un salto de varias filas casi todas están vacías). Antes cada fila
/// repintada y la banda se volcaban por separado: con 22 filas de salto eran
/// 23 volcados donde ahora hay uno.
fn sync_rows_after_scroll(st: &mut FbState, scroll_lines: usize) {
    let rows = max_rows(st);
    if rows == 0 || scroll_lines == 0 {
        return;
    }
    let bpl = bytes_per_logical_line(st);
    let band = st.cell_h.saturating_mul(scroll_lines);
    let start = if band < st.log_h {
        let move_bytes = (st.log_h - band) * bpl;
        unsafe {
            let sh = st.shadow.as_mut_ptr();
            core::ptr::copy(sh.add(band * bpl), sh, move_bytes);
            core::ptr::write_bytes(sh.add(move_bytes), 0, st.shadow.len() - move_bytes);
        }
        rows.saturating_sub(scroll_lines)
    } else {
        // Más scrolls que filas visibles en un solo write: el memmove no
        // cubre nada, se parte de pantalla negra y se repinta todo.
        st.shadow.fill(0);
        0
    };
    for r in start..rows {
        if !fila_vacia(st, r) {
            paint_row(st, r);
        }
    }
    flush_all(st);
}

/// Desplaza el texto `n` filas hacia arriba y deja las `n` últimas en blanco.
fn scroll_text(st: &mut FbState, n: usize) {
    let rows = max_rows(st);
    if rows == 0 || n == 0 {
        return;
    }
    let n = n.min(rows);
    for r in 0..rows - n {
        st.text[r] = st.text[r + n];
    }
    for r in rows - n..rows {
        st.text[r] = [CELL_SPACE; COLS];
    }
}

/// Ventana de «ráfaga»: dos scrolls separados por menos que esto son salida
/// continua (trazas de arranque, `ls`, `dmesg`), no alguien tecleando.
const RAFAGA_NS: u64 = 200_000_000;

/// Filas que salta un scroll. Uno a uno en uso interactivo; en ráfaga, un
/// cuarto de pantalla (*jump scroll*, como xterm `-j`).
///
/// Cada scroll cuesta volcar la pantalla entera al GOP (8 MiB en 1920×1080),
/// gane lo que gane el WC, y las trazas de arranque son cientos de líneas
/// seguidas. Saltando `rows/4` filas, las siguientes `rows/4 - 1` líneas sólo
/// pintan su fila (1/90 de pantalla) y el volcado completo se paga una vez
/// de cada ~22. Cada línea sigue volcándose en su `write`: si el arranque se
/// cuelga, el último marcador `boot:` está en pantalla, igual que antes.
fn filas_salto(st: &FbState, rows: usize) -> usize {
    let ahora = crate::arch::tsc::now_ns();
    if ahora.wrapping_sub(st.last_scroll_ns) < RAFAGA_NS {
        (rows / 4).max(1)
    } else {
        1
    }
}

/// Baja el cursor una fila. Si se sale por abajo desplaza el texto y devuelve
/// cuántas filas se movieron (0 si no hubo scroll).
fn avanzar_fila(st: &mut FbState, rows: usize) -> usize {
    st.row += 1;
    if st.row < rows {
        return 0;
    }
    let n = filas_salto(st, rows);
    scroll_text(st, n);
    st.row = rows - n;
    n
}

fn erase_cursor(st: &mut FbState) {
    refresh_cell(st, st.row, st.col);
}

fn draw_cursor(st: &mut FbState) {
    let y0 = st.row * st.cell_h;
    let bar_h = (st.cell_h / 6).max(1);
    let bar_y = y0 + st.cell_h.saturating_sub(bar_h);
    let x = st.col * st.cell_w;
    if bar_y < st.log_h {
        fill_rect(st, x, bar_y, st.cell_w, bar_h, FG.0, FG.1, FG.2);
    }
}

/// Procesa un carácter. Devuelve las filas de scroll que provocó (0 = ninguna).
fn draw_codepoint(st: &mut FbState, cp: u16, defer_paint: bool) -> usize {
    let cols = max_cols(st);
    let rows = max_rows(st);
    if cols == 0 || rows == 0 {
        return 0;
    }
    let mut scrolled = 0usize;

    if cp == 0x08 || cp == 0x7f {
        if st.col > 0 {
            st.col -= 1;
            st.text[st.row][st.col] = CELL_SPACE;
            if !defer_paint {
                clear_cell(st, st.row, st.col);
            }
        }
        return 0;
    }
    if cp == b'\n' as u16 {
        st.col = 0;
        return avanzar_fila(st, rows);
    }
    if cp == b'\r' as u16 {
        st.col = 0;
        st.text[st.row] = [CELL_SPACE; COLS];
        if !defer_paint {
            fill_rect(
                st,
                0,
                st.row * st.cell_h,
                st.log_w,
                st.cell_h.min(st.log_h.saturating_sub(st.row * st.cell_h)),
                0,
                0,
                0,
            );
        }
        return 0;
    }

    if st.col >= cols {
        st.col = 0;
        scrolled += avanzar_fila(st, rows);
    }
    let row = st.row;
    let col = st.col;
    st.text[row][col] = cp;
    if !defer_paint && scrolled == 0 {
        clear_cell(st, row, col);
        paint_glyph(st, row, col, cp);
    }
    st.col += 1;
    if st.col >= cols {
        st.col = 0;
        scrolled += avanzar_fila(st, rows);
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
            scroll_lines += draw_codepoint(st, cp, scroll_lines > 0);
        }
    }

    if scroll_lines > 0 {
        sync_rows_after_scroll(st, scroll_lines);
        st.last_scroll_ns = crate::arch::tsc::now_ns();
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
    // Geometría lógica: userspace pinta en la misma orientación que la
    // consola y es `present_from_user` quien aplica la rotación.
    out.present = 1;
    out.pixel_format = pixel_format_tag(st.info.pixel_format);
    out.bytes_per_pixel = st.info.bytes_per_pixel as u8;
    out.width = st.log_w as u32;
    out.height = st.log_h as u32;
    out.stride = st.log_w as u32;
    out.byte_len = st.shadow.len() as u64;
    true
}

/// Copia un búfer de píxeles (geometría lógica, `byte_len` de `user_info`) a
/// la pantalla, aplicando la rotación del panel.
pub fn present_from_user(buf: &[u8]) -> Result<(), ()> {
    let mut guard = FB.lock();
    let Some(st) = guard.as_mut() else {
        return Err(());
    };
    let want = st.shadow.len().min(buf.len());
    st.shadow[..want].copy_from_slice(&buf[..want]);
    flush_all(st);
    Ok(())
}
