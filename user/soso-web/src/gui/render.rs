//! Rasterizado software con fontdue (TTF cargada en runtime).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use fontdue::Font;
use fontdue::FontSettings;
use fontdue::Metrics;
use libsoso::sys;

use super::fb::Framebuffer;
use super::layout::{Document, LayoutBox, Node, Style};

pub const DEFAULT_FONT: &str = "/lib/fonts/DejaVuSans.ttf";

pub struct Renderer {
    font: Font,
    cache: BTreeMap<(u32, u32), (Metrics, Vec<u8>)>,
}

impl Renderer {
    pub fn open(path: &str) -> Option<Self> {
        let bytes = read_file(path)?;
        let font = Font::from_bytes(bytes, FontSettings::default()).ok()?;
        Some(Self {
            font,
            cache: BTreeMap::new(),
        })
    }

    pub fn draw_document(
        &mut self,
        fb: &mut Framebuffer,
        doc: &Document,
        boxes: &[LayoutBox],
        scroll_y: f32,
        hover_link: Option<usize>,
    ) {
        let bg = (0x10u8, 0x12u8, 0x18u8);
        fill_bg(fb, bg.0, bg.1, bg.2);
        for lb in boxes {
            if lb.y + lb.h < scroll_y || lb.y > scroll_y + fb.info.height as f32 {
                continue;
            }
            let node = &doc.nodes[lb.node_idx];
            let (text, style) = node_text_style(node);
            let size = style.font_size.max(10.0);
            let (r, g, b) = if matches!(node, Node::Link { idx, .. } if hover_link == Some(*idx)) {
                (0x80, 0xC0, 0xFF)
            } else {
                color_rgb(style.color)
            };
            draw_text_ttf(
                self,
                fb,
                &text,
                lb.x,
                lb.y - scroll_y,
                size,
                r,
                g,
                b,
            );
        }
    }

    fn lookup_glyph(&mut self, ch: char, size: f32) -> (Metrics, Vec<u8>) {
        let key = (ch as u32, size.to_bits());
        self.cache
            .entry(key)
            .or_insert_with(|| self.font.rasterize(ch, size));
        let (m, b) = self.cache.get(&key).unwrap();
        (*m, b.clone())
    }
}

fn read_file(path: &str) -> Option<Vec<u8>> {
    let fd = sys::open(path, soso_abi::O_RDONLY);
    if fd < 0 {
        return None;
    }
    let fd = fd as u64;
    let mut st = soso_abi::Stat::default();
    if sys::stat(path, &mut st) < 0 || st.size <= 0 || st.size > 4 * 1024 * 1024 {
        let _ = sys::close(fd);
        return None;
    }
    let mut buf = vec![0u8; st.size as usize];
    let n = sys::read(fd, &mut buf);
    let _ = sys::close(fd);
    if n != st.size as i64 {
        return None;
    }
    Some(buf)
}

fn fill_bg(fb: &mut Framebuffer, r: u8, g: u8, b: u8) {
    let bpp = fb.info.bytes_per_pixel as usize;
    let stride = fb.info.stride as usize;
    let w = fb.info.width as usize;
    let h = fb.info.height as usize;
    for y in 0..h {
        for x in 0..w {
            let off = y * stride * bpp + x * bpp;
            if off + bpp > fb.back.len() {
                continue;
            }
            put_px(fb, off, bpp, r, g, b);
        }
    }
}

fn put_px(fb: &mut Framebuffer, off: usize, bpp: usize, r: u8, g: u8, b: u8) {
    if off + bpp > fb.back.len() {
        return;
    }
    match fb.info.pixel_format {
        soso_abi::FB_FMT_BGR => {
            fb.back[off] = b;
            fb.back[off + 1] = g;
            fb.back[off + 2] = r;
        }
        _ => {
            fb.back[off] = r;
            if bpp > 1 {
                fb.back[off + 1] = g;
            }
            if bpp > 2 {
                fb.back[off + 2] = b;
            }
        }
    }
}

fn put_px_alpha(fb: &mut Framebuffer, x: i32, y: i32, r: u8, g: u8, b: u8, alpha: u8) {
    if alpha == 0 {
        return;
    }
    let bpp = fb.info.bytes_per_pixel as usize;
    let stride = fb.info.stride as usize;
    let w = fb.info.width as i32;
    let h = fb.info.height as i32;
    if x < 0 || y < 0 || x >= w || y >= h {
        return;
    }
    let off = y as usize * stride * bpp + x as usize * bpp;
    if alpha >= 250 {
        put_px(fb, off, bpp, r, g, b);
        return;
    }
    let a = alpha as u16;
    let inv = 255 - a;
    let br = fb.back[off] as u16;
    let bg = if bpp > 1 { fb.back[off + 1] as u16 } else { br };
    let bb = if bpp > 2 { fb.back[off + 2] as u16 } else { br };
    let nr = ((r as u16 * a + br * inv) / 255) as u8;
    let ng = ((g as u16 * a + bg * inv) / 255) as u8;
    let nb = ((b as u16 * a + bb * inv) / 255) as u8;
    put_px(fb, off, bpp, nr, ng, nb);
}

fn draw_text_ttf(
    renderer: &mut Renderer,
    fb: &mut Framebuffer,
    text: &str,
    mut x: f32,
    y: f32,
    size: f32,
    r: u8,
    g: u8,
    b: u8,
) {
    for ch in text.chars() {
        let (metrics, bitmap) = renderer.lookup_glyph(ch, size);
        let base_x = x + metrics.xmin as f32;
        let base_y = y + metrics.ymin as f32;
        let w = metrics.width;
        for row in 0..metrics.height {
            for col in 0..w {
                let alpha = bitmap[row * w + col];
                if alpha == 0 {
                    continue;
                }
                put_px_alpha(
                    fb,
                    base_x as i32 + col as i32,
                    base_y as i32 + row as i32,
                    r,
                    g,
                    b,
                    alpha,
                );
            }
        }
        x += metrics.advance_width;
    }
}

fn node_text_style(node: &Node) -> (String, Style) {
    match node {
        Node::Text { text, style } => (text.clone(), style.clone()),
        Node::Block { children, style } => {
            let mut s = String::new();
            for c in children {
                if let Node::Text { text, .. } = c {
                    s.push_str(text);
                    s.push(' ');
                }
            }
            (s, style.clone())
        }
        Node::Link { text, idx, style, .. } => (format!("[{idx}] {text}"), style.clone()),
    }
}

fn color_rgb(argb: u32) -> (u8, u8, u8) {
    let r = ((argb >> 16) & 0xFF) as u8;
    let g = ((argb >> 8) & 0xFF) as u8;
    let b = (argb & 0xFF) as u8;
    (r, g, b)
}

/// Decodifica PNG en RAM (stub: devuelve error si no es PNG válido mínimo).
pub fn decode_png_stub(_data: &[u8]) -> Result<(u32, u32, alloc::vec::Vec<u8>), ()> {
    Err(())
}
