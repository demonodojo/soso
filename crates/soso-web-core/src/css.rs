//! Parser CSS y cascada para layout gráfico.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::computed::ComputedStyle;
use crate::html::GfxStyle;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Specificity {
    pub ids: u32,
    pub classes: u32,
    pub tags: u32,
}

impl Specificity {
    fn cmp_key(&self) -> (u32, u32, u32) {
        (self.ids, self.classes, self.tags)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    Adjacent,
    Sibling,
}

#[derive(Clone, Debug)]
pub enum SelectorPart {
    Tag(String),
    Class(String),
    Id(String),
    PseudoLink,
}

#[derive(Clone, Debug)]
pub struct SelectorChain {
    pub parts: Vec<(Combinator, SelectorPart)>,
    pub specificity: Specificity,
}

#[derive(Clone, Debug)]
pub struct MediaQuery {
    pub min_width: Option<f32>,
    pub max_width: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct StyleRule {
    pub selectors: Vec<SelectorChain>,
    pub declarations: Vec<(String, String, bool)>,
    pub media: Option<MediaQuery>,
}

#[derive(Clone, Debug, Default)]
pub struct Stylesheet {
    pub rules: Vec<StyleRule>,
}

/// Compat: regla simple para tests antiguos.
#[derive(Clone, Debug, PartialEq)]
pub enum CssSelector {
    Tag(String),
    Class(String),
    Id(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CssRule {
    pub selector: CssSelector,
    pub style: GfxStyle,
}

pub fn parse_stylesheet(css: &str) -> Vec<CssRule> {
    let sheet = parse_stylesheet_full(css);
    let mut legacy = Vec::new();
    for rule in sheet.rules {
        if rule.media.is_some() {
            continue;
        }
        for sel in rule.selectors {
            if sel.parts.len() != 1 {
                continue;
            }
            if sel.parts[0].0 != Combinator::Descendant {
                continue;
            }
            let mut tmp = ComputedStyle::default();
            for (k, v, imp) in &rule.declarations {
                tmp.apply_declaration(k, v, *imp);
            }
            let st = gfx_from_computed(&tmp);
            match &sel.parts[0].1 {
                SelectorPart::Tag(t) => {
                    legacy.push(CssRule {
                        selector: CssSelector::Tag(t.clone()),
                        style: st,
                    });
                }
                SelectorPart::Class(c) => {
                    legacy.push(CssRule {
                        selector: CssSelector::Class(c.clone()),
                        style: st,
                    });
                }
                SelectorPart::Id(i) => {
                    legacy.push(CssRule {
                        selector: CssSelector::Id(i.clone()),
                        style: st,
                    });
                }
                _ => {}
            }
        }
    }
    legacy
}

fn gfx_from_computed(c: &ComputedStyle) -> GfxStyle {
    GfxStyle {
        font_size: c.font_size,
        color: c.color,
        margin_top: c.margin.top,
        margin_bottom: c.margin.bottom,
        background_color: c.background_color,
        padding: c.padding.top,
        text_align: c.text_align,
        bold: c.bold,
    }
}

pub fn parse_stylesheet_full(css: &str) -> Stylesheet {
    let mut sheet = Stylesheet::default();
    let mut i = 0;
    let bytes = css.as_bytes();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'@' {
            if let Some((rules, next)) = parse_at_rule(css, i) {
                sheet.rules.extend(rules);
                i = next;
            } else {
                i = skip_to_next_block(css, i);
            }
            continue;
        }
        if let Some((rule, next)) = parse_rule(css, i, None) {
            sheet.rules.push(rule);
            i = next;
        } else {
            break;
        }
    }
    sheet
}

fn parse_at_rule(css: &str, start: usize) -> Option<(Vec<StyleRule>, usize)> {
    let rest = &css[start..];
    if rest.starts_with("@media") {
        let open = rest.find('{')?;
        let cond = rest[6..open].trim();
        let media = parse_media(cond);
        let mut depth = 1;
        let mut i = start + open + 1;
        let bytes = css.as_bytes();
        let inner_start = i;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        let inner = &css[inner_start..i - 1];
        let mut rules = Vec::new();
        let mut j = 0;
        while j < inner.len() {
            while j < inner.len() && inner.as_bytes()[j].is_ascii_whitespace() {
                j += 1;
            }
            if let Some((rule, next)) = parse_rule(inner, j, media.clone()) {
                rules.push(rule);
                j = next;
            } else {
                break;
            }
        }
        return Some((rules, i));
    }
    None
}

fn parse_media(cond: &str) -> Option<MediaQuery> {
    let mut mq = MediaQuery {
        min_width: None,
        max_width: None,
    };
    for part in cond.split("and") {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("(min-width:") {
            if let Some(px) = v.trim().trim_end_matches(')').strip_suffix("px") {
                mq.min_width = px.trim().parse().ok();
            }
        } else if let Some(v) = part.strip_prefix("(max-width:") {
            if let Some(px) = v.trim().trim_end_matches(')').strip_suffix("px") {
                mq.max_width = px.trim().parse().ok();
            }
        }
    }
    if mq.min_width.is_some() || mq.max_width.is_some() {
        Some(mq)
    } else {
        None
    }
}

fn parse_rule(css: &str, start: usize, media: Option<MediaQuery>) -> Option<(StyleRule, usize)> {
    let bytes = css.as_bytes();
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'{' {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let selector_text = css[start..i].trim();
    i += 1;
    let decl_start = i;
    while i < bytes.len() && bytes[i] != b'}' {
        i += 1;
    }
    let decls = css[decl_start..i].trim();
    if i < bytes.len() {
        i += 1;
    }
    let selectors = parse_selector_list(selector_text)?;
    let declarations = parse_declarations(decls);
    Some((
        StyleRule {
            selectors,
            declarations,
            media,
        },
        i,
    ))
}

fn parse_selector_list(s: &str) -> Option<Vec<SelectorChain>> {
    let mut out = Vec::new();
    for part in split_commas(s) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(chain) = parse_selector_chain(part) {
            out.push(chain);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn split_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if c == ',' {
            out.push(&s[start..i]);
            start = i + 1;
        }
    }
    out.push(&s[start..]);
    out
}

fn parse_selector_chain(s: &str) -> Option<SelectorChain> {
    let tokens = tokenize_selector(s);
    if tokens.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    let mut spec = Specificity::default();
    let mut comb = Combinator::Descendant;
    for tok in tokens {
        match tok.as_str() {
            ">" => comb = Combinator::Child,
            "+" => comb = Combinator::Adjacent,
            "~" => comb = Combinator::Sibling,
            _ if tok.starts_with('#') => {
                let id = tok[1..].to_string();
                spec.ids += 1;
                parts.push((comb, SelectorPart::Id(id)));
                comb = Combinator::Descendant;
            }
            _ if tok.starts_with('.') => {
                let c = tok[1..].to_string();
                spec.classes += 1;
                parts.push((comb, SelectorPart::Class(c)));
                comb = Combinator::Descendant;
            }
            _ if tok.starts_with(':') => {
                if tok == ":link" {
                    parts.push((comb, SelectorPart::PseudoLink));
                }
                comb = Combinator::Descendant;
            }
            _ if tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => {
                spec.tags += 1;
                parts.push((comb, SelectorPart::Tag(tok.to_ascii_lowercase())));
                comb = Combinator::Descendant;
            }
            _ => {}
        }
    }
    Some(SelectorChain {
        parts,
        specificity: spec,
    })
}

fn tokenize_selector(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c == '>' || c == '+' || c == '~' {
            if !cur.is_empty() {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            out.push(c.to_string());
        } else if c.is_ascii_whitespace() {
            if !cur.is_empty() {
                out.push(cur.trim().to_string());
                cur.clear();
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn parse_declarations(s: &str) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    for part in s.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (k, v) = if let Some(i) = part.find(':') {
            (&part[..i], part[i + 1..].trim())
        } else {
            continue;
        };
        let mut important = false;
        let mut val = v.to_string();
        if val.to_ascii_lowercase().ends_with("!important") {
            important = true;
            val = val[..val.len().saturating_sub(10)].trim().to_string();
        }
        out.push((k.trim().to_ascii_lowercase(), val, important));
    }
    out
}

fn skip_to_next_block(css: &str, start: usize) -> usize {
    let mut depth = 0;
    let bytes = css.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    css.len()
}

pub fn apply_stylesheet(
    mut base: GfxStyle,
    tag: &str,
    class: &str,
    id: &str,
    rules: &[CssRule],
) -> GfxStyle {
    for rule in rules {
        let matches = match &rule.selector {
            CssSelector::Tag(t) => t == tag,
            CssSelector::Class(c) => class.split_whitespace().any(|x| x == c.as_str()),
            CssSelector::Id(i) => id == i.as_str(),
        };
        if matches {
            base = merge_sheet(base, &rule.style);
        }
    }
    base
}

fn merge_sheet(base: GfxStyle, over: &GfxStyle) -> GfxStyle {
    let mut s = base;
    if over.font_size > 0.0 {
        s.font_size = over.font_size;
    }
    if over.color != 0 && over.color != GfxStyle::default().color {
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

pub fn cascade_node(
    tag: &str,
    class: &str,
    id: &str,
    inline_attr: Option<&str>,
    sheet: &Stylesheet,
    viewport_w: f32,
    ancestors: &[(String, String, String)],
) -> ComputedStyle {
    let mut style = ComputedStyle::for_tag(tag);
    let mut winning: Vec<(Specificity, bool, String, String)> = Vec::new();

    for rule in &sheet.rules {
        if let Some(ref mq) = rule.media {
            if let Some(min) = mq.min_width {
                if viewport_w < min {
                    continue;
                }
            }
            if let Some(max) = mq.max_width {
                if viewport_w > max {
                    continue;
                }
            }
        }
        for sel in &rule.selectors {
            if selector_matches(sel, tag, class, id, ancestors) {
                for (k, v, imp) in &rule.declarations {
                    let spec = sel.specificity.cmp_key();
                    winning.push((
                        Specificity {
                            ids: spec.0,
                            classes: spec.1,
                            tags: spec.2,
                        },
                        *imp,
                        k.clone(),
                        v.clone(),
                    ));
                }
            }
        }
    }

    winning.sort_by(|a, b| {
        let sa = a.0.cmp_key();
        let sb = b.0.cmp_key();
        sa.cmp(&sb).then(a.1.cmp(&b.1))
    });

    let mut seen = alloc::vec::Vec::new();
    for (_, imp, k, v) in winning {
        if seen.contains(&(k.clone(), imp)) {
            continue;
        }
        seen.push((k.clone(), imp));
        style.apply_declaration(&k, &v, imp);
    }

    if let Some(inline) = inline_attr {
        for (k, v, _imp) in parse_declarations(inline) {
            style.apply_declaration(&k, &v, true);
        }
    }

    style
}

fn selector_matches(
    chain: &SelectorChain,
    tag: &str,
    class: &str,
    id: &str,
    ancestors: &[(String, String, String)],
) -> bool {
    if chain.parts.is_empty() {
        return false;
    }
    let mut ctx: Vec<(String, String, String)> = Vec::new();
    ctx.push((
        tag.to_string(),
        class.to_string(),
        id.to_string(),
    ));
    for a in ancestors.iter().rev() {
        ctx.push(a.clone());
    }

    let mut part_idx = chain.parts.len();
    let mut node_idx = 0usize;
    part_idx -= 1;
    loop {
        let part = &chain.parts[part_idx].1;
        if node_idx >= ctx.len() {
            return false;
        }
        let (t, c, i) = &ctx[node_idx];
        if !part_matches(part, t, c, i) {
            return false;
        }
        if part_idx == 0 {
            return true;
        }
        part_idx -= 1;
        node_idx += 1;
        if node_idx >= ctx.len() {
            return false;
        }
    }
}

fn part_matches(part: &SelectorPart, tag: &str, class: &str, id: &str) -> bool {
    match part {
        SelectorPart::Tag(t) => t == tag,
        SelectorPart::Class(c) => class.split_whitespace().any(|x| x == c.as_str()),
        SelectorPart::Id(i) => id == i.as_str(),
        SelectorPart::PseudoLink => tag == "a",
    }
}

pub fn cascade_document(doc: &mut crate::box_tree::BoxDocument, sheet: &Stylesheet, viewport_w: f32) {
    fn walk(
        nodes: &mut [crate::box_tree::BoxNode],
        idx: usize,
        sheet: &Stylesheet,
        viewport_w: f32,
        ancestors: &mut Vec<(String, String, String)>,
    ) {
        if idx >= nodes.len() {
            return;
        }
        let (tag, class, id, inline_css) = {
            let n = &nodes[idx];
            (
                n.tag.clone(),
                n.class.clone(),
                n.id.clone(),
                n.inline_css.clone(),
            )
        };
        let inline = if inline_css.is_empty() {
            None
        } else {
            Some(inline_css.as_str())
        };
        let style = cascade_node(&tag, &class, &id, inline, sheet, viewport_w, ancestors);
        nodes[idx].style = style;
        ancestors.push((tag, class, id));
        let children: Vec<usize> = nodes[idx]
            .children
            .iter()
            .filter_map(|c| match c {
                crate::box_tree::BoxChild::Element(i) => Some(*i),
                _ => None,
            })
            .collect();
        for child in children {
            walk(nodes, child, sheet, viewport_w, ancestors);
        }
        ancestors.pop();
    }
    let root = doc.root;
    let mut ancestors = Vec::new();
    walk(&mut doc.nodes, root, sheet, viewport_w, &mut ancestors);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn h1_color_from_sheet() {
        let rules = parse_stylesheet("h1 { color: #ffffff; font-size: 22px; }");
        let st = apply_stylesheet(GfxStyle::for_tag("h1"), "h1", "", "", &rules);
        assert_eq!(st.font_size, 22.0);
        assert_eq!(st.color, 0xFF00_0000 | 0xffffff);
    }

    #[test]
    fn inline_wins_over_sheet() {
        let rules = parse_stylesheet("p { font-size: 30px; }");
        let mut st = apply_stylesheet(GfxStyle::for_tag("p"), "p", "", "", &rules);
        st = crate::html::merge_gfx_style(st, &GfxStyle::parse_inline("font-size: 12px"));
        assert_eq!(st.font_size, 12.0);
    }

    #[test]
    fn comma_selectors() {
        let sheet = parse_stylesheet_full("h1, .nota { color: #ff0000; }");
        assert!(!sheet.rules.is_empty());
    }

    #[test]
    fn child_selector() {
        let sheet = parse_stylesheet_full("div > p { color: #0000ff; }");
        let ancestors = vec![(String::from("div"), String::new(), String::new())];
        let st = cascade_node("p", "", "", None, &sheet, 800.0, &ancestors);
        assert_eq!(st.color, 0xFF00_0000 | 0x0000ff);
    }

    #[test]
    fn media_min_width() {
        let sheet = parse_stylesheet_full("@media (min-width: 400px) { p { color: #00ff00; } }");
        let st_n = cascade_node("p", "", "", None, &sheet, 300.0, &[]);
        let st_y = cascade_node("p", "", "", None, &sheet, 500.0, &[]);
        assert_ne!(st_y.color, st_n.color);
    }
}
