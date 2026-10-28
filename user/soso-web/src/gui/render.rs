//! Rasterizado software con fontdue (TTF cargada en runtime).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use fontdue::Font;
use fontdue::FontSettings;
use fontdue::Metrics;
use libsoso::sys;

use super::chrome::{
    tab_label, Chrome, BACK_W, BM_LIST_W, BM_STAR_W, CLOSE_TAB_W, GO_W, NEW_TAB_W,
};
use super::fb::Framebuffer;
use super::layout::{
    Document, LayoutBox, Node, Style, BAR_ROW_HEIGHT, CHROME_HEIGHT, SCROLLBAR_W, TAB_ROW_HEIGHT,
};
use soso_web_core::html::TEXT_ALIGN_CENTER;

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

    pub fn draw_frame(
        &mut self,
        fb: &mut Framebuffer,
        doc: &Document,
        boxes: &[LayoutBox],
        scroll_y: f32,
        content_h: f32,
        hover_link: Option<usize>,
        chrome: &Chrome,
        field_values: &[String],
        bookmarked: bool,
    ) {
        let (pr, pg, pb) = color_rgb(doc.page_background);
        fill_bg(fb, pr, pg, pb);
        let view_h = fb.info.height as f32 - CHROME_HEIGHT;
        for lb in boxes {
            let screen_y = lb.y - scroll_y;
            if screen_y + lb.h < CHROME_HEIGHT || screen_y > fb.info.height as f32 {
                continue;
            }
            let node = &doc.nodes[lb.node_idx];
            if let Some(bg) = node_bg(node) {
                let (br, bgc, bb) = color_rgb(bg);
                fill_rect(fb, lb.x, screen_y, lb.w, lb.h, br, bgc, bb);
            }
            if let Some(style) = node_style(node) {
                if style.border_width > 0.0 && style.border_color != 0 {
                    let (r, g, b) = color_rgb(style.border_color);
                    let t = style.border_width;
                    fill_rect(fb, lb.x, screen_y, lb.w, t, r, g, b);
                    fill_rect(fb, lb.x, screen_y + lb.h - t, lb.w, t, r, g, b);
                    fill_rect(fb, lb.x, screen_y, t, lb.h, r, g, b);
                    fill_rect(fb, lb.x + lb.w - t, screen_y, t, lb.h, r, g, b);
                }
            }
            match node {
                Node::Image {
                    idx,
                    alt,
                    display_w,
                    display_h,
                    style,
                } => {
                    if let Some(Some(img)) = doc.decoded_images.get(*idx) {
                        draw_rgba(
                            fb,
                            lb.x,
                            screen_y,
                            *display_w,
                            *display_h,
                            &img.rgba,
                            img.w,
                            img.h,
                        );
                    } else {
                        let (r, g, b) = color_rgb(style.color);
                        draw_text_ttf(self, fb, alt, lb.x, screen_y + 4.0, style.font_size, r, g, b);
                    }
                }
                Node::FormInput {
                    field_idx,
                    password,
                    style,
                    ..
                } => {
                    let focused = chrome.field_focus == Some(*field_idx);
                    fill_rect(
                        fb,
                        lb.x,
                        screen_y,
                        lb.w,
                        lb.h,
                        if focused { 0x30 } else { 0x22 },
                        if focused { 0x34 } else { 0x24 },
                        if focused { 0x40 } else { 0x30 },
                    );
                    fill_rect(fb, lb.x, screen_y, lb.w, 1.0, 0x60, 0x68, 0x78);
                    fill_rect(fb, lb.x, screen_y + lb.h - 1.0, lb.w, 1.0, 0x60, 0x68, 0x78);
                    let val = field_values
                        .get(*field_idx)
                        .cloned()
                        .unwrap_or_default();
                    let show = if *password {
                        "•".repeat(val.chars().count().min(32))
                    } else {
                        val
                    };
                    let (r, g, b) = color_rgb(style.color);
                    draw_text_ttf(self, fb, &show, lb.x + 6.0, screen_y + 6.0, 14.0, r, g, b);
                }
                Node::FormSubmit { label, style, .. } => {
                    fill_rect(fb, lb.x, screen_y, lb.w, lb.h, 0x50, 0x70, 0xA0);
                    let (r, g, b) = color_rgb(style.color);
                    draw_text_ttf(
                        self,
                        fb,
                        label,
                        lb.x + 8.0,
                        screen_y + 6.0,
                        14.0,
                        r.max(0xE0),
                        g.max(0xE0),
                        b.max(0xE0),
                    );
                }
                Node::Text { text, style } | Node::Link { text, style, .. } => {
                    let size = style.font_size.max(10.0);
                    let (r, g, b) = match node {
                        Node::Link { idx, .. } if hover_link == Some(*idx) => (0x80, 0xC0, 0xFF),
                        Node::Link { .. } => (0x60, 0x90, 0xE0),
                        _ => color_rgb(style.color),
                    };
                    let mut tx = lb.x;
                    if style.text_align == TEXT_ALIGN_CENTER {
                        let tw = text_width_approx(text, size);
                        tx = lb.x + ((lb.w - tw) / 2.0).max(0.0);
                    }
                    draw_text_ttf(self, fb, text, tx, screen_y + 2.0, size, r, g, b);
                    if style.bold {
                        draw_text_ttf(self, fb, text, tx + 1.0, screen_y + 2.0, size, r, g, b);
                    }
                }
            }
        }
        self.draw_chrome(fb, chrome, bookmarked);
        self.draw_scrollbar(fb, scroll_y, content_h, view_h);
    }

    fn draw_chrome(&mut self, fb: &mut Framebuffer, chrome: &Chrome, bookmarked: bool) {
        let w = fb.info.width as f32;
        fill_rect(fb, 0.0, 0.0, w, CHROME_HEIGHT, 0x28, 0x2c, 0x38);
        fill_rect(fb, 0.0, 0.0, w, TAB_ROW_HEIGHT, 0x20, 0x22, 0x28);
        fill_rect(fb, 2.0, 4.0, NEW_TAB_W - 4.0, TAB_ROW_HEIGHT - 8.0, 0x40, 0x48, 0x58);
        draw_text_ttf(self, fb, "+", 10.0, 5.0, 14.0, 0xE0, 0xE4, 0xEC);
        fill_rect(
            fb,
            NEW_TAB_W + 2.0,
            4.0,
            CLOSE_TAB_W - 4.0,
            TAB_ROW_HEIGHT - 8.0,
            0x40,
            0x48,
            0x58,
        );
        draw_text_ttf(self, fb, "x", NEW_TAB_W + 10.0, 5.0, 14.0, 0xE0, 0xE4, 0xEC);
        let tab_w = 72.0;
        let mut tx = NEW_TAB_W + CLOSE_TAB_W;
        for (i, tab) in chrome.tabs.iter().enumerate() {
            if tx + tab_w > w - BM_LIST_W - BM_STAR_W {
                break;
            }
            let active = i == chrome.active;
            fill_rect(
                fb,
                tx,
                3.0,
                tab_w - 2.0,
                TAB_ROW_HEIGHT - 6.0,
                if active { 0x38 } else { 0x30 },
                if active { 0x3c } else { 0x32 },
                if active { 0x48 } else { 0x3a },
            );
            draw_text_ttf(
                self,
                fb,
                tab_label(tab),
                tx + 4.0,
                5.0,
                12.0,
                0xD0,
                0xD8,
                0xE0,
            );
            tx += tab_w;
        }
        fill_rect(
            fb,
            w - BM_LIST_W - 2.0,
            4.0,
            BM_LIST_W - 4.0,
            TAB_ROW_HEIGHT - 8.0,
            0x40,
            0x48,
            0x58,
        );
        draw_text_ttf(self, fb, "Bm", w - BM_LIST_W + 4.0, 5.0, 12.0, 0xE0, 0xE4, 0xEC);
        fill_rect(
            fb,
            w - BM_LIST_W - BM_STAR_W - 2.0,
            4.0,
            BM_STAR_W - 4.0,
            TAB_ROW_HEIGHT - 8.0,
            if bookmarked { 0x70 } else { 0x40 },
            if bookmarked { 0x60 } else { 0x48 },
            if bookmarked { 0x20 } else { 0x58 },
        );
        draw_text_ttf(
            self,
            fb,
            if bookmarked { "*" } else { "☆" },
            w - BM_LIST_W - BM_STAR_W + 8.0,
            5.0,
            12.0,
            0xE0,
            0xE4,
            0xEC,
        );

        let bar_y = TAB_ROW_HEIGHT;
        fill_rect(
            fb,
            4.0,
            bar_y + 6.0,
            BACK_W - 8.0,
            BAR_ROW_HEIGHT - 12.0,
            0x40,
            0x48,
            0x58,
        );
        draw_text_ttf(self, fb, "<<", 12.0, bar_y + 10.0, 14.0, 0xE0, 0xE4, 0xEC);
        let url_x = BACK_W + 4.0;
        let url_w = w - BACK_W - GO_W - 8.0;
        let tab = chrome.active_tab();
        fill_rect(
            fb,
            url_x,
            bar_y + 6.0,
            url_w,
            BAR_ROW_HEIGHT - 12.0,
            if chrome.url_focus { 0x38 } else { 0x1c },
            if chrome.url_focus { 0x40 } else { 0x20 },
            if chrome.url_focus { 0x50 } else { 0x28 },
        );
        let url_show = if let Some(ref e) = tab.error {
            e.as_str()
        } else {
            tab.url_text.as_str()
        };
        draw_text_ttf(
            self,
            fb,
            url_show,
            url_x + 6.0,
            bar_y + 10.0,
            14.0,
            0xD0,
            0xD8,
            0xE0,
        );
        fill_rect(
            fb,
            w - GO_W - 4.0,
            bar_y + 6.0,
            GO_W - 4.0,
            BAR_ROW_HEIGHT - 12.0,
            0x50,
            0x70,
            0xA0,
        );
        draw_text_ttf(self, fb, "Ir", w - GO_W + 6.0, bar_y + 10.0, 14.0, 0xFF, 0xFF, 0xFF);
    }

    fn draw_scrollbar(
        &mut self,
        fb: &mut Framebuffer,
        scroll_y: f32,
        content_h: f32,
        view_h: f32,
    ) {
        let w = fb.info.width as f32;
        let x = w - SCROLLBAR_W;
        let track_top = CHROME_HEIGHT;
        let track_h = fb.info.height as f32 - track_top;
        fill_rect(fb, x, track_top, SCROLLBAR_W, track_h, 0x20, 0x22, 0x28);
        let max_scroll = (content_h - view_h).max(0.0);
        if max_scroll <= 0.0 {
            return;
        }
        let thumb_h = (view_h / content_h * track_h).max(24.0);
        let t = scroll_y / max_scroll;
        let thumb_y = track_top + t * (track_h - thumb_h);
        fill_rect(fb, x + 2.0, thumb_y, SCROLLBAR_W - 4.0, thumb_h, 0x50, 0x58, 0x68);
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

fn fill_rect(fb: &mut Framebuffer, x: f32, y: f32, w: f32, h: f32, r: u8, g: u8, b: u8) {
    let x0 = x.max(0.0) as i32;
    let y0 = y.max(0.0) as i32;
    let x1 = (x + w).min(fb.info.width as f32) as i32;
    let y1 = (y + h).min(fb.info.height as f32) as i32;
    for py in y0..y1 {
        for px in x0..x1 {
            let bpp = fb.info.bytes_per_pixel as usize;
            let stride = fb.info.stride as usize;
            let off = py as usize * stride * bpp + px as usize * bpp;
            put_px(fb, off, bpp, r, g, b);
        }
    }
}

fn fill_bg(fb: &mut Framebuffer, r: u8, g: u8, b: u8) {
    let bpp = fb.info.bytes_per_pixel as usize;
    let stride = fb.info.stride as usize;
    let w = fb.info.width as usize;
    let h = fb.info.height as usize;
    for y in 0..h {
        for x in 0..w {
            let off = y * stride * bpp + x * bpp;
            if off + bpp > fb.back_mut().len() {
                continue;
            }
            put_px(fb, off, bpp, r, g, b);
        }
    }
}

fn put_px(fb: &mut Framebuffer, off: usize, bpp: usize, r: u8, g: u8, b: u8) {
    if off + bpp > fb.back_mut().len() {
        return;
    }
    match fb.info.pixel_format {
        soso_abi::FB_FMT_BGR => {
            let back = fb.back_mut();
            back[off] = b;
            back[off + 1] = g;
            back[off + 2] = r;
        }
        _ => {
            let back = fb.back_mut();
            back[off] = r;
            if bpp > 1 {
                back[off + 1] = g;
            }
            if bpp > 2 {
                back[off + 2] = b;
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
    let back = fb.back_mut();
    let br = back[off] as u16;
    let bg = if bpp > 1 { back[off + 1] as u16 } else { br };
    let bb = if bpp > 2 { back[off + 2] as u16 } else { br };
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

fn node_style(node: &Node) -> Option<&Style> {
    match node {
        Node::Text { style, .. }
        | Node::Link { style, .. }
        | Node::Image { style, .. }
        | Node::FormInput { style, .. }
        | Node::FormSubmit { style, .. } => Some(style),
    }
}

fn node_bg(node: &Node) -> Option<u32> {
    node_style(node).and_then(|style| {
        if style.background_color != 0 {
            Some(style.background_color)
        } else {
            None
        }
    })
}

fn text_width_approx(text: &str, size: f32) -> f32 {
    text.chars().count() as f32 * size * 0.55
}

fn draw_rgba(
    fb: &mut Framebuffer,
    x: f32,
    y: f32,
    dw: f32,
    dh: f32,
    rgba: &[u8],
    sw: u32,
    sh: u32,
) {
    if sw == 0 || sh == 0 {
        return;
    }
    let scale_x = dw / sw as f32;
    let scale_y = dh / sh as f32;
    for py in 0..dh as i32 {
        for px in 0..dw as i32 {
            let sx = ((px as f32 / scale_x) as u32).min(sw - 1);
            let sy = ((py as f32 / scale_y) as u32).min(sh - 1);
            let si = (sy as usize * sw as usize + sx as usize) * 4;
            if si + 3 >= rgba.len() {
                continue;
            }
            put_px_alpha(
                fb,
                x as i32 + px,
                y as i32 + py,
                rgba[si],
                rgba[si + 1],
                rgba[si + 2],
                rgba[si + 3],
            );
        }
    }
}

fn color_rgb(argb: u32) -> (u8, u8, u8) {
    let r = ((argb >> 16) & 0xFF) as u8;
    let g = ((argb >> 8) & 0xFF) as u8;
    let b = (argb & 0xFF) as u8;
    (r, g, b)
}

