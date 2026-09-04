//! Layout bloque/inline mínimo con CSS en atributo `style`.

use alloc::string::String;
use alloc::vec::Vec;
use soso_web_core::html::{reflow_html, Link};

#[derive(Clone, Debug, Default)]
pub struct Style {
    pub font_size: f32,
    pub color: u32,
    pub margin_top: f32,
    pub margin_bottom: f32,
}

impl Style {
    pub fn parse_inline(s: &str) -> Self {
        let mut st = Self::default();
        for part in s.split(';') {
            let Some((k, v)) = part.split_once(':') else { continue };
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            match k.as_str() {
                "font-size" => {
                    if let Some(n) = v.strip_suffix("px").and_then(|x| x.parse().ok()) {
                        st.font_size = n;
                    }
                }
                "color" => {
                    if let Some(c) = parse_color(v) {
                        st.color = c;
                    }
                }
                "margin-top" => {
                    if let Some(n) = v.strip_suffix("px").and_then(|x| x.parse().ok()) {
                        st.margin_top = n;
                    }
                }
                "margin-bottom" => {
                    if let Some(n) = v.strip_suffix("px").and_then(|x| x.parse().ok()) {
                        st.margin_bottom = n;
                    }
                }
                _ => {}
            }
        }
        if st.font_size == 0.0 {
            st.font_size = 16.0;
        }
        if st.color == 0 {
            st.color = 0xC8D0B0;
        }
        st
    }
}

fn parse_color(v: &str) -> Option<u32> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix('#') {
        let n = u32::from_str_radix(hex, 16).ok()?;
        if hex.len() == 6 {
            return Some(0xFF00_0000 | n);
        }
    }
    None
}

#[derive(Clone, Debug)]
pub enum Node {
    Text { text: String, style: Style },
    Block { children: Vec<Node>, style: Style },
    Link { idx: usize, text: String, href: String, style: Style },
}

#[derive(Clone, Debug)]
pub struct Document {
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
}

/// Parseo gráfico simplificado: reutiliza reflow para enlaces y construye bloques por `<p>`.
pub fn layout_html(html: &str, base_url: &str) -> Document {
    let reflow = reflow_html(html, base_url, 80);
    let mut nodes = Vec::new();
    for line in reflow.text.lines() {
        if line.is_empty() {
            continue;
        }
        nodes.push(Node::Text {
            text: String::from(line),
            style: Style::default(),
        });
    }
    Document {
        nodes,
        links: reflow.links,
    }
}

/// Métricas de layout vertical simple.
pub struct LayoutBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub node_idx: usize,
}

pub fn compute_layout(doc: &Document, width: f32) -> (Vec<LayoutBox>, f32) {
    let mut boxes = Vec::new();
    let mut y = 8.0;
    let pad = 8.0;
    for (i, node) in doc.nodes.iter().enumerate() {
        let style = match node {
            Node::Text { style, .. } | Node::Block { style, .. } | Node::Link { style, .. } => style,
        };
        y += style.margin_top;
        let h = style.font_size * 1.4;
        boxes.push(LayoutBox {
            x: pad,
            y,
            w: width - pad * 2.0,
            h,
            node_idx: i,
        });
        y += h + style.margin_bottom;
    }
    (boxes, y + 8.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_style_basico() {
        let st = Style::parse_inline("font-size: 20px; color: #ff0000");
        assert_eq!(st.font_size, 20.0);
        assert_eq!(st.color, 0xFF00_0000 | 0xff0000);
    }
}
