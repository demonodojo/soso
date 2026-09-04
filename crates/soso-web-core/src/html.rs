//! Tokenizador HTML tolerante y reflow a texto plano.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use crate::decode::decode_entities;

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
}
