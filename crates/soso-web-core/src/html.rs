//! Tokenizador HTML tolerante y reflow a texto plano.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use crate::box_tree::{BoxChild, BoxDocument, BoxNode};
use crate::css::{apply_stylesheet, parse_stylesheet, parse_stylesheet_full, CssRule, Stylesheet};
use crate::decode::decode_entities;

pub const TEXT_ALIGN_LEFT: u8 = 0;
pub const TEXT_ALIGN_CENTER: u8 = 1;

pub const DEFAULT_PAGE_BG: u32 = 0xFF_10_12_18;

/// Enlace numerado para navegación por teclado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub idx: usize,
    pub text: String,
    pub href: String,
}

/// Resultado del reflow de una página.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReflowOutput {
    pub text: String,
    pub links: Vec<Link>,
}

/// Estilo mínimo para layout gráfico (px / ARGB).
#[derive(Clone, Debug, PartialEq)]
pub struct GfxStyle {
    pub font_size: f32,
    pub color: u32,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub background_color: u32,
    pub padding: f32,
    pub text_align: u8,
    pub bold: bool,
}

impl Default for GfxStyle {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            color: 0xFF_C8_D0_B0,
            margin_top: 0.0,
            margin_bottom: 4.0,
            background_color: 0,
            padding: 0.0,
            text_align: TEXT_ALIGN_LEFT,
            bold: false,
        }
    }
}

impl GfxStyle {
    pub fn for_tag(tag: &str) -> Self {
        tag_default_style(tag)
    }

    pub fn parse_inline(s: &str) -> Self {
        let mut st = Self::default();
        for part in s.split(';') {
            let Some((k, v)) = part.split_once(':') else {
                continue;
            };
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            match k.as_str() {
                "font-size" => {
                    if let Some(n) = v.strip_suffix("px").and_then(|x| x.parse().ok()) {
                        st.font_size = n;
                    }
                }
                "color" => {
                    if let Some(c) = parse_color_hex(v) {
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
                "background-color" => {
                    if let Some(c) = parse_color_hex(v) {
                        st.background_color = c;
                    }
                }
                "padding" => {
                    if let Some(n) = v.strip_suffix("px").and_then(|x| x.parse().ok()) {
                        st.padding = n;
                    }
                }
                "text-align" => {
                    if v.eq_ignore_ascii_case("center") {
                        st.text_align = TEXT_ALIGN_CENTER;
                    }
                }
                "font-weight" => {
                    if v.eq_ignore_ascii_case("bold") || v == "700" {
                        st.bold = true;
                    }
                }
                _ => {}
            }
        }
        st
    }
}

fn tag_default_style(tag: &str) -> GfxStyle {
        let mut st = GfxStyle::default();
        match tag {
            "h1" => {
                st.font_size = 28.0;
                st.margin_top = 12.0;
                st.margin_bottom = 8.0;
            }
            "h2" => {
                st.font_size = 24.0;
                st.margin_top = 10.0;
                st.margin_bottom = 6.0;
            }
            "h3" => {
                st.font_size = 20.0;
                st.margin_top = 8.0;
                st.margin_bottom = 4.0;
            }
            "pre" => {
                st.font_size = 14.0;
                st.color = 0xFF_E0_E8_D0;
            }
            "li" => {
                st.margin_bottom = 2.0;
            }
            _ => {}
        }
        st
}

fn parse_color_hex(v: &str) -> Option<u32> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix('#') {
        let n = u32::from_str_radix(hex, 16).ok()?;
        if hex.len() == 6 {
            return Some(0xFF00_0000 | n);
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GfxInline {
    Text(String),
    Link { idx: usize },
    Image { idx: usize },
    FormInput {
        form_idx: usize,
        field_idx: usize,
    },
    FormSubmit {
        form_idx: usize,
        label: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GfxImage {
    pub idx: usize,
    pub src: String,
    pub alt: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GfxFormField {
    pub form_idx: usize,
    pub field_idx: usize,
    pub name: String,
    pub password: bool,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GfxForm {
    pub idx: usize,
    pub action: String,
    pub method: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GfxBlock {
    pub tag: String,
    pub class: String,
    pub id: String,
    pub style: GfxStyle,
    pub inlines: Vec<GfxInline>,
}

/// Árbol plano de bloques para el modo gráfico (sin reflow de consola).
#[derive(Clone, Debug)]
pub struct GfxDocument {
    pub page_title: Option<String>,
    pub page_background: u32,
    pub blocks: Vec<GfxBlock>,
    pub boxes: BoxDocument,
    pub stylesheet: Stylesheet,
    pub links: Vec<Link>,
    pub images: Vec<GfxImage>,
    pub forms: Vec<GfxForm>,
    pub form_fields: Vec<GfxFormField>,
}

const GFX_SKIP_TAGS: &[&str] = &["script", "head", "noscript", "template"];

/// Parsea HTML en bloques con enlaces resueltos para layout en píxeles.
pub fn parse_gfx_document(html: &str, base_url: &str) -> GfxDocument {
    let tokens = tokenize(html);
    let mut ctx = GfxCtx::new(base_url);
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Text(t) => ctx.push_text(t),
            Token::TagOpen { name, attrs, self_closing } => {
                let tag = name.to_ascii_lowercase();
                if GFX_SKIP_TAGS.contains(&tag.as_str()) {
                    ctx.skip_depth += 1;
                } else if tag == "style" {
                    ctx.in_style = true;
                    ctx.style_buf.clear();
                } else if tag == "title" {
                    ctx.in_title = true;
                    ctx.title_buf.clear();
                } else if tag == "br" || *self_closing && tag == "br" {
                    ctx.push_inline_text("\n");
                } else if tag == "a" {
                    ctx.in_anchor = true;
                    ctx.anchor_href = attr_value(attrs, "href");
                    ctx.anchor_text.clear();
                } else if tag == "form" {
                    ctx.open_form(attrs);
                } else if tag == "input" || *self_closing && tag == "input" {
                    ctx.handle_input(attrs);
                } else if tag == "textarea" {
                    ctx.in_textarea = true;
                    ctx.textarea_name = attr_value(attrs, "name").unwrap_or_default();
                    ctx.textarea_value.clear();
                } else if tag == "button" {
                    ctx.handle_button(attrs, "");
                } else if tag == "img" || *self_closing && tag == "img" {
                    ctx.handle_img(attrs);
                } else if BLOCK_TAGS.contains(&tag.as_str()) || heading_tag(&tag) || tag == "body" {
                    ctx.open_block(&tag, attrs);
                    if tag == "li" {
                        ctx.push_inline_text("• ");
                    }
                }
                if *self_closing && tag == "hr" {
                    ctx.open_block("hr", attrs);
                    ctx.push_inline_text("────────────────");
                    ctx.close_block("hr");
                }
            }
            Token::TagClose { name } => {
                let tag = name.to_ascii_lowercase();
                if GFX_SKIP_TAGS.contains(&tag.as_str()) && ctx.skip_depth > 0 {
                    ctx.skip_depth -= 1;
                } else if tag == "style" {
                    ctx.in_style = false;
                    ctx.full_sheet = parse_stylesheet_full(&ctx.style_buf);
                    ctx.css_rules = parse_stylesheet(&ctx.style_buf);
                    ctx.style_buf.clear();
                } else if tag == "title" {
                    ctx.in_title = false;
                    let t = ctx.title_buf.trim().to_string();
                    if !t.is_empty() {
                        ctx.page_title = Some(t);
                    }
                    ctx.title_buf.clear();
                } else if tag == "form" {
                    ctx.current_form = None;
                } else if tag == "textarea" {
                    ctx.finish_textarea();
                } else if tag == "a" {
                    ctx.close_anchor();
                } else if BLOCK_TAGS.contains(&tag.as_str()) || heading_tag(&tag) || tag == "body" {
                    ctx.close_block(&tag);
                }
            }
            Token::Comment => {}
        }
        i += 1;
    }
    ctx.finish()
}

fn heading_tag(tag: &str) -> bool {
    matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

struct GfxCtx<'a> {
    base_url: &'a str,
    blocks: Vec<GfxBlock>,
    links: Vec<Link>,
    images: Vec<GfxImage>,
    forms: Vec<GfxForm>,
    form_fields: Vec<GfxFormField>,
    css_rules: Vec<CssRule>,
    full_sheet: Stylesheet,
    box_nodes: Vec<BoxNode>,
    box_stack: Vec<usize>,
    skip_depth: usize,
    in_style: bool,
    style_buf: String,
    in_title: bool,
    title_buf: String,
    page_title: Option<String>,
    page_background: u32,
    current: Option<GfxBlock>,
    in_anchor: bool,
    anchor_href: Option<String>,
    anchor_text: String,
    current_form: Option<usize>,
    in_textarea: bool,
    textarea_name: String,
    textarea_value: String,
}

impl<'a> GfxCtx<'a> {
    fn new(base_url: &'a str) -> Self {
        let body = BoxNode {
            tag: String::from("body"),
            class: String::new(),
            id: String::new(),
            inline_css: String::new(),
            style: crate::computed::ComputedStyle::default(),
            children: Vec::new(),
        };
        let mut box_nodes = Vec::new();
        box_nodes.push(body);
        Self {
            base_url,
            blocks: Vec::new(),
            links: Vec::new(),
            images: Vec::new(),
            forms: Vec::new(),
            form_fields: Vec::new(),
            css_rules: Vec::new(),
            full_sheet: Stylesheet::default(),
            box_nodes,
            box_stack: vec![0],
            skip_depth: 0,
            in_style: false,
            style_buf: String::new(),
            in_title: false,
            title_buf: String::new(),
            page_title: None,
            page_background: DEFAULT_PAGE_BG,
            current: None,
            in_anchor: false,
            anchor_href: None,
            anchor_text: String::new(),
            current_form: None,
            in_textarea: false,
            textarea_name: String::new(),
            textarea_value: String::new(),
        }
    }

    fn push_text(&mut self, raw: &str) {
        if self.in_style {
            self.style_buf.push_str(raw);
            return;
        }
        if self.in_textarea {
            self.textarea_value.push_str(raw);
            return;
        }
        if self.skip_depth > 0 {
            return;
        }
        let text = decode_entities(raw);
        let collapsed = collapse_ws(&text);
        if collapsed.is_empty() {
            return;
        }
        if self.in_title {
            self.title_buf.push_str(&collapsed);
            self.title_buf.push(' ');
            return;
        }
        if self.in_anchor {
            self.anchor_text.push_str(&collapsed);
            self.anchor_text.push(' ');
            return;
        }
        self.push_inline_text(&collapsed);
    }

    fn push_inline_text(&mut self, text: &str) {
        self.ensure_flow_block();
        let Some(block) = self.current.as_mut() else {
            return;
        };
        if let Some(GfxInline::Text(last)) = block.inlines.last_mut() {
            if text == "\n" {
                last.push('\n');
                return;
            }
            if !last.ends_with('\n') && !last.is_empty() {
                last.push(' ');
            }
            last.push_str(text);
            return;
        }
        block.inlines.push(GfxInline::Text(text.to_string()));
        self.box_push_text(text);
    }

    fn box_push_text(&mut self, text: &str) {
        let Some(&idx) = self.box_stack.last() else {
            return;
        };
        if text == "\n" {
            return;
        }
        let node = &mut self.box_nodes[idx];
        if let Some(BoxChild::Text(last)) = node.children.last_mut() {
            if !last.ends_with('\n') && !last.is_empty() {
                last.push(' ');
            }
            last.push_str(text);
            return;
        }
        node.children.push(BoxChild::Text(text.to_string()));
    }

    fn ensure_flow_block(&mut self) {
        if self.current.is_none() {
            self.current = Some(GfxBlock {
                tag: String::from("p"),
                class: String::new(),
                id: String::new(),
                style: styled_tag(self, "p", "", ""),
                inlines: Vec::new(),
            });
        }
    }

    fn open_block(&mut self, tag: &str, attrs: &[(String, String)]) {
        if self.skip_depth > 0 {
            return;
        }
        self.flush_current();
        let class = attr_value(attrs, "class").unwrap_or_default();
        let id = attr_value(attrs, "id").unwrap_or_default();
        let mut style = styled_tag(self, tag, &class, &id);
        if let Some(s) = attr_value(attrs, "style") {
            style = merge_gfx_style(style, &GfxStyle::parse_inline(&s));
        }
        self.current = Some(GfxBlock {
            tag: tag.to_string(),
            class: class.clone(),
            id: id.clone(),
            style,
            inlines: Vec::new(),
        });
        let inline_css = attr_value(attrs, "style").unwrap_or_default();
        let idx = self.box_nodes.len();
        self.box_nodes.push(BoxNode {
            tag: tag.to_string(),
            class,
            id,
            inline_css,
            style: crate::computed::ComputedStyle::default(),
            children: Vec::new(),
        });
        if let Some(&parent) = self.box_stack.last() {
            self.box_nodes[parent]
                .children
                .push(BoxChild::Element(idx));
        }
        self.box_stack.push(idx);
    }

    fn open_form(&mut self, attrs: &[(String, String)]) {
        if self.skip_depth > 0 {
            return;
        }
        let action = attr_value(attrs, "action").unwrap_or_default();
        let method = attr_value(attrs, "method")
            .unwrap_or_else(|| String::from("get"))
            .to_ascii_lowercase();
        let idx = self.forms.len() + 1;
        let action = if action.is_empty() {
            self.base_url.to_string()
        } else {
            resolve_url(self.base_url, &action)
        };
        self.forms.push(GfxForm {
            idx,
            action,
            method,
        });
        self.current_form = Some(idx);
    }

    fn handle_input(&mut self, attrs: &[(String, String)]) {
        if self.skip_depth > 0 {
            return;
        }
        let form_idx = self.current_form.unwrap_or(1);
        let typ = attr_value(attrs, "type")
            .unwrap_or_else(|| String::from("text"))
            .to_ascii_lowercase();
        if typ == "submit" {
            let label = attr_value(attrs, "value").unwrap_or_else(|| String::from("Enviar"));
            self.ensure_flow_block();
            if let Some(block) = self.current.as_mut() {
                block.inlines.push(GfxInline::FormSubmit {
                    form_idx,
                    label: label.clone(),
                });
            }
            if let Some(&bi) = self.box_stack.last() {
                self.box_nodes[bi].children.push(BoxChild::FormSubmit {
                    form_idx,
                    label,
                });
            }
            return;
        }
        if typ != "text" && typ != "password" {
            return;
        }
        let name = attr_value(attrs, "name").unwrap_or_default();
        let value = attr_value(attrs, "value").unwrap_or_default();
        let field_idx = self.form_fields.len() + 1;
        self.form_fields.push(GfxFormField {
            form_idx,
            field_idx,
            name,
            password: typ == "password",
            value,
        });
        self.ensure_flow_block();
        if let Some(block) = self.current.as_mut() {
            block.inlines.push(GfxInline::FormInput {
                form_idx,
                field_idx,
            });
        }
        if let Some(&bi) = self.box_stack.last() {
            self.box_nodes[bi].children.push(BoxChild::FormInput {
                form_idx,
                field_idx,
            });
        }
    }

    fn handle_button(&mut self, attrs: &[(String, String)], inner: &str) {
        if self.skip_depth > 0 {
            return;
        }
        let form_idx = self.current_form.unwrap_or(1);
        let label = if inner.trim().is_empty() {
            attr_value(attrs, "value").unwrap_or_else(|| String::from("OK"))
        } else {
            inner.trim().to_string()
        };
        self.ensure_flow_block();
        if let Some(block) = self.current.as_mut() {
            block.inlines.push(GfxInline::FormSubmit {
                form_idx,
                label: label.clone(),
            });
        }
        if let Some(&bi) = self.box_stack.last() {
            self.box_nodes[bi].children.push(BoxChild::FormSubmit {
                form_idx,
                label,
            });
        }
    }

    fn finish_textarea(&mut self) {
        if self.skip_depth > 0 {
            self.in_textarea = false;
            return;
        }
        let form_idx = self.current_form.unwrap_or(1);
        let field_idx = self.form_fields.len() + 1;
        self.form_fields.push(GfxFormField {
            form_idx,
            field_idx,
            name: self.textarea_name.clone(),
            password: false,
            value: self.textarea_value.trim().to_string(),
        });
        self.ensure_flow_block();
        if let Some(block) = self.current.as_mut() {
            block.inlines.push(GfxInline::FormInput {
                form_idx,
                field_idx,
            });
        }
        if let Some(&bi) = self.box_stack.last() {
            self.box_nodes[bi].children.push(BoxChild::FormInput {
                form_idx,
                field_idx,
            });
        }
        self.in_textarea = false;
        self.textarea_value.clear();
    }

    fn handle_img(&mut self, attrs: &[(String, String)]) {
        if self.skip_depth > 0 {
            return;
        }
        let src = attr_value(attrs, "src").unwrap_or_default();
        if src.is_empty() {
            return;
        }
        let alt = attr_value(attrs, "alt").unwrap_or_default();
        let width = attr_value(attrs, "width")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let height = attr_value(attrs, "height")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let idx = self.images.len() + 1;
        self.images.push(GfxImage {
            idx,
            src: resolve_url(self.base_url, &src),
            alt,
            width,
            height,
        });
        self.ensure_flow_block();
        if let Some(block) = self.current.as_mut() {
            block.inlines.push(GfxInline::Image { idx });
        }
        if let Some(&bi) = self.box_stack.last() {
            self.box_nodes[bi].children.push(BoxChild::Image { idx });
        }
    }

    fn close_block(&mut self, tag: &str) {
        if self.skip_depth > 0 {
            return;
        }
        if tag == "body" {
            if let Some(b) = &self.current {
                if b.style.background_color != 0 {
                    self.page_background = b.style.background_color;
                }
            }
        }
        self.flush_current();
        if self.box_stack.len() > 1 {
            self.box_stack.pop();
        }
    }

    fn close_anchor(&mut self) {
        if self.skip_depth > 0 {
            self.in_anchor = false;
            self.anchor_href = None;
            self.anchor_text.clear();
            return;
        }
        if let Some(href) = self.anchor_href.take() {
            let text = if self.anchor_text.trim().is_empty() {
                href.clone()
            } else {
                self.anchor_text.trim().to_string()
            };
            let resolved = resolve_url(self.base_url, &href);
            let idx = self.links.len() + 1;
            self.links.push(Link {
                idx,
                text: text.clone(),
                href: resolved,
            });
            self.ensure_flow_block();
            if let Some(block) = self.current.as_mut() {
                block.inlines.push(GfxInline::Link { idx });
            }
            if let Some(&bi) = self.box_stack.last() {
                self.box_nodes[bi].children.push(BoxChild::Link { idx });
            }
        }
        self.in_anchor = false;
        self.anchor_text.clear();
    }

    fn flush_current(&mut self) {
        let Some(block) = self.current.take() else {
            return;
        };
        if block.inlines.is_empty() {
            return;
        }
        self.blocks.push(block);
    }

    fn finish(mut self) -> GfxDocument {
        self.flush_current();
        let boxes = BoxDocument {
            page_title: self.page_title.clone(),
            page_background: self.page_background,
            root: 0,
            nodes: self.box_nodes,
            links: self.links.clone(),
            images: self.images.clone(),
            forms: self.forms.clone(),
            form_fields: self.form_fields.clone(),
        };
        GfxDocument {
            page_title: self.page_title,
            page_background: self.page_background,
            blocks: self.blocks,
            boxes,
            stylesheet: self.full_sheet,
            links: self.links,
            images: self.images,
            forms: self.forms,
            form_fields: self.form_fields,
        }
    }
}

/// Cascada + layout del árbol de cajas.
pub fn layout_gfx_document(
    doc: &mut GfxDocument,
    viewport_w: f32,
    pad_x: f32,
) -> crate::gfx_layout::LayoutResult {
    crate::css::cascade_document(&mut doc.boxes, &doc.stylesheet, viewport_w);
    if doc.boxes.page_background == 0 {
        if let Some(body) = doc.boxes.node(doc.boxes.root) {
            if body.style.background_color != 0 {
                doc.page_background = body.style.background_color;
            }
        }
    }
    crate::gfx_layout::layout_tree(&doc.boxes, viewport_w, pad_x)
}

fn styled_tag(ctx: &GfxCtx<'_>, tag: &str, class: &str, id: &str) -> GfxStyle {
    let base = GfxStyle::for_tag(tag);
    apply_stylesheet(base, tag, class, id, &ctx.css_rules)
}

pub fn merge_gfx_style(base: GfxStyle, over: &GfxStyle) -> GfxStyle {
    let mut s = base;
    if over.font_size > 0.0 {
        s.font_size = over.font_size;
    }
    if over.color != 0 {
        s.color = over.color;
    }
    if over.margin_top > 0.0 {
        s.margin_top = over.margin_top;
    }
    if over.margin_bottom > 0.0 {
        s.margin_bottom = over.margin_bottom;
    }
    if over.background_color != 0 {
        s.background_color = over.background_color;
    }
    if over.padding > 0.0 {
        s.padding = over.padding;
    }
    if over.text_align != 0 {
        s.text_align = over.text_align;
    }
    if over.bold {
        s.bold = true;
    }
    s
}

const SKIP_TAGS: &[&str] = &["script", "style", "head", "noscript", "template"];

const BLOCK_TAGS: &[&str] = &[
    "p", "div", "section", "article", "header", "footer", "main", "nav",
    "h1", "h2", "h3", "h4", "h5", "h6", "li", "ul", "ol", "dl", "dt", "dd",
    "blockquote", "figure", "figcaption", "hr", "pre", "table", "tr", "td", "th",
];

/// Convierte HTML en texto con word-wrap y lista de enlaces.
pub fn reflow_html(html: &str, base_url: &str, width: usize) -> ReflowOutput {
    let width = width.max(20);
    let mut ctx = ReflowCtx::new(width);
    let tokens = tokenize(html);
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Text(t) => ctx.push_text(t),
            Token::TagOpen { name, attrs, self_closing } => {
                let tag = name.to_ascii_lowercase();
                if SKIP_TAGS.contains(&tag.as_str()) {
                    ctx.skip_depth += 1;
                } else if tag == "br" || *self_closing && tag == "br" {
                    ctx.newline();
                } else if tag == "a" {
                    ctx.in_anchor = true;
                    ctx.anchor_href = attr_value(attrs, "href");
                    ctx.anchor_text.clear();
                } else if tag == "li" {
                    ctx.push_text("• ");
                } else if BLOCK_TAGS.contains(&tag.as_str()) {
                    ctx.ensure_blank_line();
                }
                if *self_closing && tag == "hr" {
                    ctx.newline();
                    ctx.push_line_of_dashes(width);
                }
            }
            Token::TagClose { name } => {
                let tag = name.to_ascii_lowercase();
                if SKIP_TAGS.contains(&tag.as_str()) && ctx.skip_depth > 0 {
                    ctx.skip_depth -= 1;
                } else if tag == "a" {
                    if let Some(href) = ctx.anchor_href.take() {
                        let text = if ctx.anchor_text.is_empty() {
                            href.clone()
                        } else {
                            ctx.anchor_text.clone()
                        };
                        let resolved = resolve_url(base_url, &href);
                        ctx.links.push(Link {
                            idx: ctx.links.len() + 1,
                            text,
                            href: resolved,
                        });
                        ctx.push_text(&format!("[{}]", ctx.links.last().unwrap().idx));
                    }
                    ctx.in_anchor = false;
                    ctx.anchor_text.clear();
                } else if BLOCK_TAGS.contains(&tag.as_str()) {
                    ctx.newline();
                }
            }
            Token::Comment => {}
        }
        i += 1;
    }
    ctx.finish()
}

struct ReflowCtx {
    width: usize,
    out: String,
    line: String,
    links: Vec<Link>,
    skip_depth: usize,
    in_anchor: bool,
    anchor_href: Option<String>,
    anchor_text: String,
}

impl ReflowCtx {
    fn new(width: usize) -> Self {
        Self {
            width,
            out: String::new(),
            line: String::new(),
            links: Vec::new(),
            skip_depth: 0,
            in_anchor: false,
            anchor_href: None,
            anchor_text: String::new(),
        }
    }

    fn push_text(&mut self, raw: &str) {
        if self.skip_depth > 0 {
            return;
        }
        let text = decode_entities(raw);
        let collapsed = collapse_ws(&text);
        if collapsed.is_empty() {
            return;
        }
        if self.in_anchor {
            self.anchor_text.push_str(&collapsed);
            self.anchor_text.push(' ');
        }
        self.wrap_words(&collapsed);
    }

    fn wrap_words(&mut self, text: &str) {
        for word in text.split_whitespace() {
            let extra = if self.line.is_empty() {
                word.len()
            } else {
                1 + word.len()
            };
            if !self.line.is_empty() && self.line.len() + extra > self.width {
                self.flush_line();
            }
            if !self.line.is_empty() {
                self.line.push(' ');
            }
            self.line.push_str(word);
        }
    }

    fn flush_line(&mut self) {
        if self.line.is_empty() {
            return;
        }
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        self.out.push_str(&self.line);
        self.out.push('\n');
        self.line.clear();
    }

    fn newline(&mut self) {
        self.flush_line();
    }

    fn ensure_blank_line(&mut self) {
        self.flush_line();
        if !self.out.ends_with("\n\n") && !self.out.is_empty() {
            self.out.push('\n');
        }
    }

    fn push_line_of_dashes(&mut self, width: usize) {
        for _ in 0..width.min(40) {
            self.line.push('-');
        }
        self.flush_line();
    }

    fn finish(mut self) -> ReflowOutput {
        self.flush_line();
        while self.out.ends_with('\n') {
            self.out.pop();
        }
        ReflowOutput {
            text: self.out,
            links: self.links,
        }
    }
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

fn attr_value(attrs: &[(String, String)], key: &str) -> Option<String> {
    attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.clone())
}

/// Resuelve URL relativa contra la base.
pub fn resolve_url(base: &str, href: &str) -> String {
    let href = href.trim();
    if href.starts_with("https://") || href.starts_with("http://") {
        return href.to_string();
    }
    if href.starts_with('#') || href.is_empty() {
        return base.to_string();
    }
    if href.starts_with('/') {
        if let Some(host) = base.strip_prefix("https://").and_then(|r| r.split('/').next()) {
            return format!("https://{host}{href}");
        }
        return href.to_string();
    }
    let base_dir = base.rsplit_once('/').map(|(d, _)| d).unwrap_or(base);
    format!("{base_dir}/{href}")
}

enum Token {
    Text(String),
    TagOpen {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    TagClose {
        name: String,
    },
    Comment,
}

fn tokenize(html: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if bytes[i..].starts_with(b"<!--") {
                if let Some(end) = html[i..].find("-->") {
                    out.push(Token::Comment);
                    i += end + 3;
                    continue;
                }
            }
            if let Some((tok, consumed)) = parse_tag(&html[i..]) {
                out.push(tok);
                i += consumed;
                continue;
            }
        }
        let start = i;
        while i < bytes.len() && bytes[i] != b'<' {
            i += 1;
        }
        if i > start {
            out.push(Token::Text(html[start..i].to_string()));
        } else {
            i += 1;
        }
    }
    out
}

fn parse_tag(rest: &str) -> Option<(Token, usize)> {
    if !rest.starts_with('<') {
        return None;
    }
    let close = rest.starts_with("</");
    let mut i = if close { 2 } else { 1 };
    let bytes = rest.as_bytes();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let name_start = i;
    while i < bytes.len() && is_name_char(bytes[i]) {
        i += 1;
    }
    if i == name_start {
        return None;
    }
    let name = rest[name_start..i].to_string();
    if close {
        while i < bytes.len() && bytes[i] != b'>' {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        i += 1;
        return Some((Token::TagClose { name }, i));
    }
    let mut attrs = Vec::new();
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        if bytes[i] == b'>' || (bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'>') {
            break;
        }
        let key_start = i;
        while i < bytes.len() && is_attr_name_char(bytes[i]) {
            i += 1;
        }
        if i == key_start {
            return None;
        }
        let key = rest[key_start..i].to_string();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let q = bytes[i];
                i += 1;
                let vstart = i;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
                val = rest[vstart..i].to_string();
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                let vstart = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                val = rest[vstart..i].to_string();
            }
        }
        attrs.push((key, val));
    }
    let self_closing = bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'>';
    if self_closing {
        i += 2;
    } else {
        if bytes[i] != b'>' {
            return None;
        }
        i += 1;
    }
    Some((Token::TagOpen { name, attrs, self_closing }, i))
}

fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-'
}

fn is_attr_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b':'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflow_enlaces_y_bloques() {
        let html = r#"<html><body><h1>Titulo</h1><p>Hola <a href="/x">mundo</a>.</p></body></html>"#;
        let out = reflow_html(html, "https://ejemplo.org/", 40);
        assert!(out.text.contains("Titulo"));
        assert!(out.text.contains("[1]"));
        assert_eq!(out.links.len(), 1);
        assert_eq!(out.links[0].href, "https://ejemplo.org/x");
    }

    #[test]
    fn omite_script() {
        let html = "<p>ok</p><script>alert(1)</script><p>fin</p>";
        let out = reflow_html(html, "https://a/", 80);
        assert!(!out.text.contains("alert"));
        assert!(out.text.contains("ok"));
        assert!(out.text.contains("fin"));
    }

    #[test]
    fn resolve_url_relativa() {
        assert_eq!(
            resolve_url("https://host/a/b", "../c"),
            "https://host/a/../c"
        );
    }

    #[test]
    fn gfx_enlace_absoluto() {
        let html = r#"<p><a href="/x">ir</a></p>"#;
        let doc = parse_gfx_document(html, "https://ejemplo.org/p");
        assert_eq!(doc.links.len(), 1);
        assert_eq!(doc.links[0].href, "https://ejemplo.org/x");
        assert_eq!(doc.links[0].text, "ir");
        assert!(doc.blocks.iter().any(|b| {
            b.inlines
                .iter()
                .any(|i| matches!(i, GfxInline::Link { idx: 1 }))
        }));
    }

    #[test]
    fn gfx_h1_no_es_reflow() {
        let html = "<h1>Titulo</h1><p>cuerpo</p>";
        let doc = parse_gfx_document(html, "https://a/");
        assert_eq!(doc.blocks.len(), 2);
        assert_eq!(doc.blocks[0].tag, "h1");
        assert_eq!(doc.blocks[0].style.font_size, 28.0);
        assert!(doc.blocks[0]
            .inlines
            .iter()
            .any(|i| matches!(i, GfxInline::Text(t) if t.contains("Titulo"))));
    }

    #[test]
    fn gfx_img_src_absoluta() {
        let html = r#"<p><img src="/i.png" alt="x" width="10" height="10"></p>"#;
        let doc = parse_gfx_document(html, "https://host/a/");
        assert_eq!(doc.images.len(), 1);
        assert_eq!(doc.images[0].src, "https://host/i.png");
        let iidx = doc.images[0].idx;
        assert!(doc.blocks[0]
            .inlines
            .iter()
            .any(|i| matches!(i, GfxInline::Image { idx } if *idx == iidx)));
    }

    #[test]
    fn gfx_form_action() {
        let html = r#"<form action="/go" method="get"><input name="q" type="text"></form>"#;
        let doc = parse_gfx_document(html, "https://host/");
        assert_eq!(doc.forms.len(), 1);
        assert_eq!(doc.forms[0].action, "https://host/go");
        assert_eq!(doc.forms[0].method, "get");
    }

    #[test]
    fn gfx_omite_script() {
        let html = "<p>ok</p><script>alert(1)</script><p>fin</p>";
        let doc = parse_gfx_document(html, "https://a/");
        let flat: String = doc
            .blocks
            .iter()
            .flat_map(|b| &b.inlines)
            .filter_map(|i| match i {
                GfxInline::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert!(!flat.contains("alert"));
        assert!(flat.contains("ok"));
        assert!(flat.contains("fin"));
    }
}
