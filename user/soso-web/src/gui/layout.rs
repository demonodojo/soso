//! Layout gráfico vía motor CSS en `soso-web-core`.

use alloc::string::String;
use alloc::vec::Vec;
use soso_web_core::gfx_layout::LaidKind;
use soso_web_core::html::{
    layout_gfx_document, parse_gfx_document, GfxForm, GfxFormField, Link, TEXT_ALIGN_CENTER,
};

use crate::png::DecodedImage;

pub const TAB_ROW_HEIGHT: f32 = 22.0;
pub const BAR_ROW_HEIGHT: f32 = 36.0;
pub const CHROME_HEIGHT: f32 = TAB_ROW_HEIGHT + BAR_ROW_HEIGHT;
pub const SCROLLBAR_W: f32 = 12.0;

#[derive(Clone, Debug, Default)]
pub struct Style {
    pub font_size: f32,
    pub color: u32,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub background_color: u32,
    pub padding: f32,
    pub text_align: u8,
    pub bold: bool,
    pub border_width: f32,
    pub border_color: u32,
}

impl Style {
    pub fn parse_inline(s: &str) -> Self {
        Self::from_gfx(&soso_web_core::html::GfxStyle::parse_inline(s))
    }

    pub fn from_computed(c: &soso_web_core::computed::ComputedStyle) -> Self {
        Self {
            font_size: c.font_size,
            color: c.color,
            margin_top: c.margin.top,
            margin_bottom: c.margin.bottom,
            background_color: c.background_color,
            padding: c.padding.top,
            text_align: c.text_align,
            bold: c.bold,
            border_width: c.border_width.top,
            border_color: c.border_color,
        }
    }

    fn from_gfx(g: &soso_web_core::html::GfxStyle) -> Self {
        Self {
            font_size: g.font_size,
            color: g.color,
            margin_top: g.margin_top,
            margin_bottom: g.margin_bottom,
            background_color: g.background_color,
            padding: g.padding,
            text_align: g.text_align,
            bold: g.bold,
            border_width: 0.0,
            border_color: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Node {
    Text { text: String, style: Style },
    Link {
        idx: usize,
        text: String,
        href: String,
        style: Style,
    },
    Image {
        idx: usize,
        alt: String,
        display_w: f32,
        display_h: f32,
        style: Style,
    },
    FormInput {
        form_idx: usize,
        field_idx: usize,
        password: bool,
        style: Style,
    },
    FormSubmit {
        form_idx: usize,
        label: String,
        style: Style,
    },
}

#[derive(Clone, Debug)]
pub struct Document {
    pub page_title: Option<String>,
    pub page_background: u32,
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    pub forms: Vec<GfxForm>,
    pub form_fields: Vec<GfxFormField>,
    pub decoded_images: Vec<Option<DecodedImage>>,
}

pub fn layout_html(
    html: &str,
    base_url: &str,
    content_width: f32,
    decoded_images: &[Option<DecodedImage>],
) -> Document {
    let mut gfx = parse_gfx_document(html, base_url);
    let viewport = content_width + 16.0 + SCROLLBAR_W;
    let laid = layout_gfx_document(&mut gfx, viewport, 8.0);
    let mut nodes = Vec::new();
    for lb in &laid.boxes {
        let style = Style::from_computed(&lb.style);
        match &lb.kind {
            LaidKind::Text { text, link_idx } => {
                if let Some(idx) = link_idx {
                    let href = gfx
                        .links
                        .iter()
                        .find(|l| l.idx == *idx)
                        .map(|l| l.href.clone())
                        .unwrap_or_default();
                    nodes.push(Node::Link {
                        idx: *idx,
                        text: text.clone(),
                        href,
                        style,
                    });
                } else {
                    nodes.push(Node::Text {
                        text: text.clone(),
                        style,
                    });
                }
            }
            LaidKind::Image { idx, alt } => {
                nodes.push(Node::Image {
                    idx: *idx,
                    alt: alt.clone(),
                    display_w: lb.w,
                    display_h: lb.h,
                    style,
                });
            }
            LaidKind::FormInput {
                form_idx,
                field_idx,
                password,
            } => {
                nodes.push(Node::FormInput {
                    form_idx: *form_idx,
                    field_idx: *field_idx,
                    password: *password,
                    style,
                });
            }
            LaidKind::FormSubmit { form_idx, label } => {
                nodes.push(Node::FormSubmit {
                    form_idx: *form_idx,
                    label: label.clone(),
                    style,
                });
            }
            LaidKind::Block => {}
        }
    }
    Document {
        page_title: gfx.page_title,
        page_background: gfx.page_background,
        nodes,
        links: gfx.links,
        forms: gfx.forms,
        form_fields: gfx.form_fields,
        decoded_images: decoded_images.to_vec(),
    }
}

#[derive(Clone, Debug)]
pub struct LayoutBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub node_idx: usize,
}

pub fn compute_layout(doc: &Document, width: f32) -> (Vec<LayoutBox>, f32) {
    let mut gfx = parse_gfx_document("", "about:blank");
    gfx.page_background = doc.page_background;
    let viewport = width;
    let laid = layout_gfx_document(&mut gfx, viewport, 8.0);
    let _ = laid;
    let mut boxes = Vec::new();
    let mut y = CHROME_HEIGHT + 8.0;
    let pad = 8.0;
    let content_w = width - pad * 2.0 - SCROLLBAR_W;
    for (i, node) in doc.nodes.iter().enumerate() {
        let (style, box_w, box_h) = match node {
            Node::Text { style, .. } | Node::Link { style, .. } => {
                (style, content_w.max(40.0), style.font_size * 1.4 + style.padding * 2.0)
            }
            Node::Image {
                display_w,
                display_h,
                style,
                ..
            } => (
                style,
                display_w.min(content_w),
                *display_h + style.padding * 2.0,
            ),
            Node::FormInput { style, .. } => (style, content_w.min(420.0), 28.0 + style.padding * 2.0),
            Node::FormSubmit { style, .. } => (style, 120.0, 28.0 + style.padding * 2.0),
        };
        y += style.margin_top;
        let mut x = pad + style.padding;
        let w = box_w;
        let h = box_h;
        if style.text_align == TEXT_ALIGN_CENTER {
            x = pad + ((content_w - w) / 2.0).max(0.0);
        }
        boxes.push(LayoutBox {
            x,
            y,
            w,
            h,
            node_idx: i,
        });
        y += h + style.margin_bottom;
    }
    (boxes, y + 8.0)
}

/// Layout unificado: documento + cajas en un solo paso.
pub fn layout_page(
    html: &str,
    base_url: &str,
    fb_width: f32,
    decoded_images: &[Option<DecodedImage>],
) -> (Document, Vec<LayoutBox>, f32) {
    let mut gfx = parse_gfx_document(html, base_url);
    let laid = layout_gfx_document(&mut gfx, fb_width, 8.0);
    let content_top = CHROME_HEIGHT + 8.0;
    let mut nodes = Vec::new();
    let mut boxes = Vec::new();
    for lb in &laid.boxes {
        if matches!(lb.kind, LaidKind::Block) {
            continue;
        }
        let style = Style::from_computed(&lb.style);
        let node_idx = nodes.len();
        match &lb.kind {
            LaidKind::Text { text, link_idx } => {
                if let Some(idx) = link_idx {
                    let href = gfx
                        .links
                        .iter()
                        .find(|l| l.idx == *idx)
                        .map(|l| l.href.clone())
                        .unwrap_or_default();
                    nodes.push(Node::Link {
                        idx: *idx,
                        text: text.clone(),
                        href,
                        style: style.clone(),
                    });
                } else {
                    nodes.push(Node::Text {
                        text: text.clone(),
                        style: style.clone(),
                    });
                }
            }
            LaidKind::Image { idx, alt } => {
                nodes.push(Node::Image {
                    idx: *idx,
                    alt: alt.clone(),
                    display_w: lb.w,
                    display_h: lb.h,
                    style: style.clone(),
                });
            }
            LaidKind::FormInput {
                form_idx,
                field_idx,
                password,
            } => {
                nodes.push(Node::FormInput {
                    form_idx: *form_idx,
                    field_idx: *field_idx,
                    password: *password,
                    style: style.clone(),
                });
            }
            LaidKind::FormSubmit { form_idx, label } => {
                nodes.push(Node::FormSubmit {
                    form_idx: *form_idx,
                    label: label.clone(),
                    style: style.clone(),
                });
            }
            LaidKind::Block => {}
        }
        boxes.push(LayoutBox {
            x: lb.x,
            y: lb.y + content_top,
            w: lb.w,
            h: lb.h,
            node_idx,
        });
    }

    let doc = Document {
        page_title: gfx.page_title,
        page_background: gfx.page_background,
        nodes,
        links: gfx.links,
        forms: gfx.forms,
        form_fields: gfx.form_fields,
        decoded_images: decoded_images.to_vec(),
    };
    (doc, boxes, laid.content_height + content_top)
}

pub fn content_width_for_fb(fb_width: f32) -> f32 {
    fb_width - 16.0 - SCROLLBAR_W
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_style_basico() {
        let st = Style::parse_inline("font-size: 20px; color: #ff0000; padding: 4px");
        assert_eq!(st.font_size, 20.0);
        assert_eq!(st.color, 0xFF00_0000 | 0xff0000);
        assert_eq!(st.padding, 4.0);
    }

    #[test]
    fn layout_flex_en_pagina() {
        let html = r#"<style>.row { display: flex; } .c { width: 40px; }</style><div class="row"><div class="c">A</div><div class="c">B</div></div>"#;
        let imgs: [Option<DecodedImage>; 0] = [];
        let (_, boxes, _) = layout_page(html, "https://a/", 400.0, &imgs);
        assert!(boxes.len() >= 2);
    }
}
