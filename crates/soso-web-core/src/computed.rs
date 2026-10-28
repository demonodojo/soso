//! Estilos calculados para layout gráfico.

use crate::html::{TEXT_ALIGN_CENTER, TEXT_ALIGN_LEFT};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Display {
    #[default]
    Block,
    Inline,
    InlineBlock,
    Flex,
    Grid,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Float {
    #[default]
    None,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Clear {
    #[default]
    None,
    Left,
    Right,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Position {
    #[default]
    Static,
    Relative,
    Absolute,
    Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FlexDirection {
    #[default]
    Row,
    Column,
    RowReverse,
    ColumnReverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FlexWrap {
    #[default]
    Nowrap,
    Wrap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum JustifyContent {
    #[default]
    FlexStart,
    Center,
    FlexEnd,
    SpaceBetween,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AlignItems {
    #[default]
    Stretch,
    FlexStart,
    Center,
    FlexEnd,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    Auto,
    Px(f32),
    Percent(f32),
}

impl Default for Length {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Clone, Debug, Default)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

#[derive(Clone, Debug, Default)]
pub struct GridTrack {
    pub tracks: alloc::vec::Vec<GridTrackSize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GridTrackSize {
    Px(f32),
    Fr(f32),
    Percent(f32),
}

#[derive(Clone, Debug)]
pub struct ComputedStyle {
    pub display: Display,
    pub float: Float,
    pub clear: Clear,
    pub position: Position,
    pub top: Length,
    pub right: Length,
    pub bottom: Length,
    pub left: Length,
    pub width: Length,
    pub height: Length,
    pub margin: Edges,
    pub padding: Edges,
    pub border_width: Edges,
    pub border_color: u32,
    pub font_size: f32,
    pub color: u32,
    pub background_color: u32,
    pub text_align: u8,
    pub bold: bool,
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub gap: f32,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Length,
    pub grid_columns: GridTrack,
    pub grid_rows: GridTrack,
    pub column_gap: f32,
    pub row_gap: f32,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            display: Display::Block,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            top: Length::Auto,
            right: Length::Auto,
            bottom: Length::Auto,
            left: Length::Auto,
            width: Length::Auto,
            height: Length::Auto,
            margin: Edges::default(),
            padding: Edges::default(),
            border_width: Edges::default(),
            border_color: 0,
            font_size: 16.0,
            color: 0xFF_C8_D0_B0,
            background_color: 0,
            text_align: TEXT_ALIGN_LEFT,
            bold: false,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Nowrap,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            gap: 0.0,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Length::Auto,
            grid_columns: GridTrack::default(),
            grid_rows: GridTrack::default(),
            column_gap: 0.0,
            row_gap: 0.0,
        }
    }
}

impl ComputedStyle {

    pub fn for_tag(tag: &str) -> Self {
        let mut s = Self::default();
        match tag {
            "span" | "a" | "strong" | "em" | "label" => s.display = Display::Inline,
            "img" | "input" | "button" => s.display = Display::InlineBlock,
            "h1" => {
                s.font_size = 28.0;
                s.margin.top = 12.0;
                s.margin.bottom = 8.0;
            }
            "h2" => {
                s.font_size = 24.0;
                s.margin.top = 10.0;
                s.margin.bottom = 6.0;
            }
            "h3" => {
                s.font_size = 20.0;
                s.margin.top = 8.0;
                s.margin.bottom = 4.0;
            }
            "pre" => {
                s.font_size = 14.0;
                s.color = 0xFF_E0_E8_D0;
            }
            "li" => s.margin.bottom = 2.0,
            _ => {}
        }
        s
    }

    pub fn apply_declaration(&mut self, name: &str, value: &str, important: bool) {
        let _ = important;
        let k = name.trim().to_ascii_lowercase();
        let v = value.trim();
        match k.as_str() {
            "display" => {
                self.display = match v.to_ascii_lowercase().as_str() {
                    "none" => Display::None,
                    "inline" => Display::Inline,
                    "inline-block" => Display::InlineBlock,
                    "flex" => Display::Flex,
                    "grid" => Display::Grid,
                    _ => Display::Block,
                };
            }
            "float" => {
                self.float = match v.to_ascii_lowercase().as_str() {
                    "left" => Float::Left,
                    "right" => Float::Right,
                    _ => Float::None,
                };
            }
            "clear" => {
                self.clear = match v.to_ascii_lowercase().as_str() {
                    "left" => Clear::Left,
                    "right" => Clear::Right,
                    "both" => Clear::Both,
                    _ => Clear::None,
                };
            }
            "position" => {
                self.position = match v.to_ascii_lowercase().as_str() {
                    "relative" => Position::Relative,
                    "absolute" => Position::Absolute,
                    "fixed" => Position::Fixed,
                    _ => Position::Static,
                };
            }
            "top" | "right" | "bottom" | "left" => {
                let len = parse_length(v, self.font_size);
                match k.as_str() {
                    "top" => self.top = len,
                    "right" => self.right = len,
                    "bottom" => self.bottom = len,
                    _ => self.left = len,
                }
            }
            "width" => self.width = parse_length(v, self.font_size),
            "height" => self.height = parse_length(v, self.font_size),
            "margin" => apply_shorthand_edges(&mut self.margin, v, self.font_size),
            "margin-top" => self.margin.top = parse_length_px(v, self.font_size),
            "margin-right" => self.margin.right = parse_length_px(v, self.font_size),
            "margin-bottom" => self.margin.bottom = parse_length_px(v, self.font_size),
            "margin-left" => self.margin.left = parse_length_px(v, self.font_size),
            "padding" => apply_shorthand_edges(&mut self.padding, v, self.font_size),
            "padding-top" => self.padding.top = parse_length_px(v, self.font_size),
            "padding-right" => self.padding.right = parse_length_px(v, self.font_size),
            "padding-bottom" => self.padding.bottom = parse_length_px(v, self.font_size),
            "padding-left" => self.padding.left = parse_length_px(v, self.font_size),
            "border-width" => apply_shorthand_edges(&mut self.border_width, v, self.font_size),
            "border-color" => {
                if let Some(c) = parse_color(v) {
                    self.border_color = c;
                }
            }
            "font-size" => {
                if let Some(n) = v.strip_suffix("px").and_then(|x| x.parse().ok()) {
                    self.font_size = n;
                } else if let Some(n) = v.strip_suffix("em").and_then(|x| x.parse::<f32>().ok()) {
                    self.font_size = self.font_size * n;
                }
            }
            "color" => {
                if let Some(c) = parse_color(v) {
                    self.color = c;
                }
            }
            "background-color" => {
                if let Some(c) = parse_color(v) {
                    self.background_color = c;
                }
            }
            "text-align" => {
                if v.eq_ignore_ascii_case("center") {
                    self.text_align = TEXT_ALIGN_CENTER;
                }
            }
            "font-weight" => {
                if v.eq_ignore_ascii_case("bold") || v == "700" {
                    self.bold = true;
                }
            }
            "flex-direction" => {
                self.flex_direction = match v.to_ascii_lowercase().as_str() {
                    "column" => FlexDirection::Column,
                    "column-reverse" => FlexDirection::ColumnReverse,
                    "row-reverse" => FlexDirection::RowReverse,
                    _ => FlexDirection::Row,
                };
            }
            "flex-wrap" => {
                if v.eq_ignore_ascii_case("wrap") {
                    self.flex_wrap = FlexWrap::Wrap;
                }
            }
            "justify-content" => {
                self.justify_content = match v.to_ascii_lowercase().as_str() {
                    "center" => JustifyContent::Center,
                    "flex-end" | "end" => JustifyContent::FlexEnd,
                    "space-between" => JustifyContent::SpaceBetween,
                    _ => JustifyContent::FlexStart,
                };
            }
            "align-items" => {
                self.align_items = match v.to_ascii_lowercase().as_str() {
                    "center" => AlignItems::Center,
                    "flex-start" | "start" => AlignItems::FlexStart,
                    "flex-end" | "end" => AlignItems::FlexEnd,
                    _ => AlignItems::Stretch,
                };
            }
            "gap" => {
                self.gap = parse_length_px(v, self.font_size);
                self.column_gap = self.gap;
                self.row_gap = self.gap;
            }
            "column-gap" => self.column_gap = parse_length_px(v, self.font_size),
            "row-gap" => self.row_gap = parse_length_px(v, self.font_size),
            "flex-grow" => {
                if let Ok(n) = v.parse::<f32>() {
                    self.flex_grow = n;
                }
            }
            "flex-shrink" => {
                if let Ok(n) = v.parse::<f32>() {
                    self.flex_shrink = n;
                }
            }
            "flex-basis" => self.flex_basis = parse_length(v, self.font_size),
            "grid-template-columns" => self.grid_columns = parse_grid_tracks(v),
            "grid-template-rows" => self.grid_rows = parse_grid_tracks(v),
            _ => {}
        }
    }

    pub fn merge_over(&mut self, over: &ComputedStyle) {
        *self = over.clone();
    }
}

pub fn parse_color(v: &str) -> Option<u32> {
    let v = v.trim();
    if v.eq_ignore_ascii_case("transparent") {
        return Some(0);
    }
    if v.eq_ignore_ascii_case("white") {
        return Some(0xFF_FF_FF_FF);
    }
    if v.eq_ignore_ascii_case("black") {
        return Some(0xFF_00_00_00);
    }
    if let Some(c) = parse_color_hex_local(v) {
        return Some(c);
    }
    if let Some(inner) = v.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        return parse_rgb_fn(inner);
    }
    if let Some(inner) = v.strip_prefix("rgba(").and_then(|s| s.strip_suffix(')')) {
        return parse_rgba_fn(inner);
    }
    None
}

fn parse_rgb_fn(inner: &str) -> Option<u32> {
    let parts: alloc::vec::Vec<_> = inner.split(',').map(|s| s.trim()).collect();
    if parts.len() != 3 {
        return None;
    }
    let r: u32 = parts[0].parse().ok()?;
    let g: u32 = parts[1].parse().ok()?;
    let b: u32 = parts[2].parse().ok()?;
    Some(0xFF00_0000 | (r << 16) | (g << 8) | b)
}

fn parse_rgba_fn(inner: &str) -> Option<u32> {
    let parts: alloc::vec::Vec<_> = inner.split(',').map(|s| s.trim()).collect();
    if parts.len() != 4 {
        return parse_rgb_fn(inner);
    }
    let r: u32 = parts[0].parse().ok()?;
    let g: u32 = parts[1].parse().ok()?;
    let b: u32 = parts[2].parse().ok()?;
    let _a: f32 = parts[3].parse().ok()?;
    Some(0xFF00_0000 | (r << 16) | (g << 8) | b)
}

pub fn parse_length(v: &str, font_size: f32) -> Length {
    let v = v.trim();
    if v.eq_ignore_ascii_case("auto") {
        return Length::Auto;
    }
    if let Some(n) = v.strip_suffix("px").and_then(|x| x.parse().ok()) {
        return Length::Px(n);
    }
    if let Some(n) = v.strip_suffix('%').and_then(|x| x.parse().ok()) {
        return Length::Percent(n);
    }
    if let Some(n) = v.strip_suffix("em").and_then(|x| x.parse::<f32>().ok()) {
        return Length::Px(font_size * n);
    }
    Length::Auto
}

pub fn parse_length_px(v: &str, font_size: f32) -> f32 {
    match parse_length(v, font_size) {
        Length::Px(n) => n,
        Length::Percent(p) => p,
        Length::Auto => 0.0,
    }
}

pub fn resolve_length(len: Length, containing: f32, _font_size: f32) -> f32 {
    match len {
        Length::Auto => 0.0,
        Length::Px(n) => n,
        Length::Percent(p) => containing * p / 100.0,
    }
    .max(0.0)
}

pub fn resolve_length_opt(len: Length, containing: f32, font_size: f32) -> Option<f32> {
    match len {
        Length::Auto => None,
        Length::Px(n) => Some(n),
        Length::Percent(p) => Some(containing * p / 100.0),
    }
}

fn apply_shorthand_edges(edges: &mut Edges, v: &str, font_size: f32) {
    let parts: alloc::vec::Vec<_> = v.split_whitespace().collect();
    let px = |s: &str| parse_length_px(s, font_size);
    match parts.len() {
        1 => {
            let n = px(parts[0]);
            edges.top = n;
            edges.right = n;
            edges.bottom = n;
            edges.left = n;
        }
        2 => {
            edges.top = px(parts[0]);
            edges.bottom = px(parts[0]);
            edges.left = px(parts[1]);
            edges.right = px(parts[1]);
        }
        4 => {
            edges.top = px(parts[0]);
            edges.right = px(parts[1]);
            edges.bottom = px(parts[2]);
            edges.left = px(parts[3]);
        }
        _ => {}
    }
}

fn parse_grid_tracks(v: &str) -> GridTrack {
    let mut tracks = alloc::vec::Vec::new();
    let v = v.trim();
    if let Some(inner) = v.strip_prefix("repeat(") {
        if let Some((n_s, rest)) = inner.split_once(',') {
            if let Some(count_s) = n_s.trim().strip_suffix(')') {
                if let Ok(count) = count_s.trim().parse::<usize>() {
                    let part = rest.trim().trim_end_matches(')');
                    let sz = parse_one_track(part);
                    for _ in 0..count.min(32) {
                        tracks.push(sz);
                    }
                    return GridTrack { tracks };
                }
            }
        }
    }
    for part in v.split_whitespace() {
        tracks.push(parse_one_track(part));
    }
    GridTrack { tracks }
}

fn parse_color_hex_local(v: &str) -> Option<u32> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix('#') {
        if hex.len() == 6 {
            let n = u32::from_str_radix(hex, 16).ok()?;
            return Some(0xFF00_0000 | n);
        }
        if hex.len() == 3 {
            let r = u32::from_str_radix(&hex[0..1], 16).ok()?;
            let g = u32::from_str_radix(&hex[1..2], 16).ok()?;
            let b = u32::from_str_radix(&hex[2..3], 16).ok()?;
            return Some(0xFF00_0000 | (r * 17 << 16) | (g * 17 << 8) | (b * 17));
        }
    }
    None
}

fn parse_one_track(part: &str) -> GridTrackSize {
    if let Some(n) = part.strip_suffix("fr").and_then(|x| x.parse().ok()) {
        GridTrackSize::Fr(n)
    } else if let Some(n) = part.strip_suffix("px").and_then(|x| x.parse().ok()) {
        GridTrackSize::Px(n)
    } else if let Some(n) = part.strip_suffix('%').and_then(|x| x.parse().ok()) {
        GridTrackSize::Percent(n)
    } else {
        GridTrackSize::Fr(1.0)
    }
}
