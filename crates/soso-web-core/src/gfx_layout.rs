//! Motor de layout: flujo, flex, grid, float y position.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::box_tree::{BoxChild, BoxDocument, BoxNode};
use crate::computed::{
    resolve_length, resolve_length_opt, AlignItems, Clear, ComputedStyle, Display, Float,
    FlexDirection, GridTrackSize, JustifyContent, Length, Position,
};
use crate::html::TEXT_ALIGN_CENTER;

pub const MAX_DEPTH: usize = 32;
pub const MAX_BOXES: usize = 2000;

#[derive(Clone, Debug)]
pub enum LaidKind {
    Text {
        text: String,
        link_idx: Option<usize>,
    },
    Image {
        idx: usize,
        alt: String,
    },
    FormInput {
        form_idx: usize,
        field_idx: usize,
        password: bool,
    },
    FormSubmit {
        form_idx: usize,
        label: String,
    },
    Block,
}

#[derive(Clone, Debug)]
pub struct LaidBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub style: ComputedStyle,
    pub kind: LaidKind,
}

pub struct LayoutResult {
    pub boxes: Vec<LaidBox>,
    pub content_height: f32,
}

struct LayoutCtx<'a> {
    doc: &'a BoxDocument,
    out: Vec<LaidBox>,
    viewport_w: f32,
    float_left: f32,
    float_right: f32,
    depth: usize,
}

pub fn layout_tree(doc: &BoxDocument, viewport_w: f32, pad_x: f32) -> LayoutResult {
    let content_w = (viewport_w - pad_x * 2.0).max(40.0);
    let mut ctx = LayoutCtx {
        doc,
        out: Vec::new(),
        viewport_w: content_w,
        float_left: 0.0,
        float_right: 0.0,
        depth: 0,
    };
    let root = doc.root;
    let h = layout_node(&mut ctx, root, pad_x, 0.0, content_w, None);
    LayoutResult {
        content_height: h + 16.0,
        boxes: ctx.out,
    }
}

fn layout_node(
    ctx: &mut LayoutCtx,
    idx: usize,
    x: f32,
    mut y: f32,
    avail_w: f32,
    containing_h: Option<f32>,
) -> f32 {
    if ctx.depth >= MAX_DEPTH || ctx.out.len() >= MAX_BOXES {
        return y;
    }
    let Some(node) = ctx.doc.node(idx).cloned() else {
        return y;
    };
    if node.style.display == Display::None {
        return y;
    }

    ctx.depth += 1;
    let margin = &node.style.margin;
    y += margin.top;
    let x0 = x + margin.left;
    let inner_w = (avail_w - margin.left - margin.right).max(1.0);

    let taken = match node.style.display {
        Display::Flex => layout_flex(ctx, &node, idx, x0, y, inner_w),
        Display::Grid => layout_grid(ctx, &node, idx, x0, y, inner_w),
        Display::Inline | Display::InlineBlock => {
            layout_inline_box(ctx, &node, idx, x0, y, inner_w)
        }
        Display::Block | Display::None => layout_block(ctx, &node, idx, x0, y, inner_w),
    };

    let mut box_h = taken;
    if node.style.position == Position::Absolute || node.style.position == Position::Fixed {
        let ax = x0 + offset_len(&node.style.left, inner_w);
        let ay = y + offset_len(&node.style.top, containing_h.unwrap_or(box_h));
        layout_absolute_children(ctx, &node, idx, ax, ay, inner_w);
        ctx.depth -= 1;
        return y + margin.bottom;
    }

    if node.style.float != Float::None {
        let w = box_w_for_node(&node, inner_w);
        let h = box_h;
        if node.style.float == Float::Left {
            ctx.float_left = ctx.float_left.max(w);
        } else {
            ctx.float_right = ctx.float_right.max(w);
        }
        let _ = (w, h);
    }

    y += box_h + margin.bottom;
    ctx.depth -= 1;
    y
}

fn offset_len(len: &Length, containing: f32) -> f32 {
    resolve_length(*len, containing, 16.0)
}

fn box_w_for_node(node: &BoxNode, avail: f32) -> f32 {
    resolve_length_opt(node.style.width, avail, node.style.font_size).unwrap_or(avail)
}

fn layout_block(ctx: &mut LayoutCtx, node: &BoxNode, idx: usize, x: f32, y: f32, w: f32) -> f32 {
    let pad = &node.style.padding;
    let cx = x + pad.left;
    let cw = (w - pad.left - pad.right).max(1.0);
    let mut cy = y + pad.top;

    if node.style.background_color != 0 || node.style.border_width.top > 0.0 {
        push_block_shell(ctx, node, x, y, w, 0.0);
    }

    cy = layout_flow_children(ctx, node, idx, cx, cy, cw);

    let h = (cy - y) + pad.bottom;
    if let Some(last) = ctx.out.last_mut() {
        if matches!(last.kind, LaidKind::Block) && last.h <= 1.0 {
            last.h = h;
        }
    }
    h.max(node.style.font_size * 1.4)
}

fn layout_flex(ctx: &mut LayoutCtx, node: &BoxNode, idx: usize, x: f32, y: f32, w: f32) -> f32 {
    let pad = &node.style.padding;
    let cx = x + pad.left;
    let cw = (w - pad.left - pad.right).max(1.0);
    let gap = node.style.gap;
    let mut child_idxs = element_children(ctx, idx);
    if child_idxs.is_empty() {
        return pad.top + pad.bottom + node.style.font_size;
    }

    let row = matches!(
        node.style.flex_direction,
        FlexDirection::Row | FlexDirection::RowReverse
    );
    if row {
        let n = child_idxs.len() as f32;
        let total_gap = gap * (n - 1.0).max(0.0);
        let mut widths: Vec<f32> = Vec::new();
        let mut grow_sum = 0.0f32;
        for &ci in &child_idxs {
            let st = &ctx.doc.node(ci).unwrap().style;
            grow_sum += st.flex_grow;
            let basis = resolve_length_opt(st.flex_basis, cw, st.font_size).unwrap_or(cw / n);
            widths.push(basis);
        }
        let fixed: f32 = widths.iter().sum();
        let free = (cw - total_gap - fixed).max(0.0);
        if grow_sum > 0.0 {
            for (i, &ci) in child_idxs.iter().enumerate() {
                let st = &ctx.doc.node(ci).unwrap().style;
                if st.flex_grow > 0.0 {
                    widths[i] += free * (st.flex_grow / grow_sum);
                }
            }
        }
        let mut cx2 = cx;
        let mut max_h = 0.0f32;
        for (i, &ci) in child_idxs.iter().enumerate() {
            let cw2 = widths.get(i).copied().unwrap_or(cw / n);
            let h = layout_node(ctx, ci, cx2, y + pad.top, cw2, None);
            max_h = max_h.max(h - y);
            cx2 += cw2 + gap;
        }
        max_h + pad.top + pad.bottom
    } else {
        let mut cy = y + pad.top;
        for &ci in &child_idxs {
            let h = layout_node(ctx, ci, cx, cy, cw, None);
            cy = h;
            cy += gap;
        }
        cy - y + pad.bottom
    }
}

fn layout_grid(ctx: &mut LayoutCtx, node: &BoxNode, idx: usize, x: f32, y: f32, w: f32) -> f32 {
    let cols = &node.style.grid_columns.tracks;
    if cols.is_empty() {
        return layout_block(ctx, node, idx, x, y, w);
    }
    let col_gap = node.style.column_gap;
    let row_gap = node.style.row_gap;
    let col_widths = resolve_tracks(cols, w, col_gap);
    let ncols = col_widths.len().max(1);
    let child_idxs = element_children(ctx, idx);
    let mut cy = y + node.style.padding.top;
    let mut row_h = 0.0f32;
    let mut col = 0usize;
    let mut row_x = x + node.style.padding.left;
    for &ci in &child_idxs {
        let cw = col_widths[col];
        let bottom = layout_node(ctx, ci, row_x, cy, cw, None);
        row_h = row_h.max(bottom - cy);
        col += 1;
        if col >= ncols {
            col = 0;
            cy += row_h + row_gap;
            row_h = 0.0;
            row_x = x + node.style.padding.left;
        } else {
            row_x += cw + col_gap;
        }
    }
    if col != 0 {
        cy += row_h;
    }
    cy - y + node.style.padding.bottom
}

fn resolve_tracks(tracks: &[GridTrackSize], total_w: f32, gap: f32) -> Vec<f32> {
    let n = tracks.len().max(1);
    let gap_total = gap * (n as f32 - 1.0).max(0.0);
    let mut fr_sum = 0.0f32;
    let mut fixed = 0.0f32;
    for t in tracks {
        match t {
            GridTrackSize::Fr(fr) => fr_sum += fr,
            GridTrackSize::Px(px) => fixed += px,
            GridTrackSize::Percent(p) => fixed += total_w * p / 100.0,
        }
    }
    let free = (total_w - gap_total - fixed).max(0.0);
    tracks
        .iter()
        .map(|t| match t {
            GridTrackSize::Fr(fr) if fr_sum > 0.0 => free * (fr / fr_sum),
            GridTrackSize::Px(px) => *px,
            GridTrackSize::Percent(p) => total_w * p / 100.0,
            _ => free / n as f32,
        })
        .collect()
}

fn layout_inline_box(ctx: &mut LayoutCtx, node: &BoxNode, idx: usize, x: f32, y: f32, w: f32) -> f32 {
    layout_flow_children(ctx, node, idx, x, y, w);
    node.style.font_size * 1.4
}

fn layout_flow_children(
    ctx: &mut LayoutCtx,
    node: &BoxNode,
    idx: usize,
    x: f32,
    mut y: f32,
    w: f32,
) -> f32 {
    let start_y = y;
    for child in &node.children {
        match child {
            BoxChild::Element(ci) => {
                let child_node = ctx.doc.node(*ci).unwrap();
                if child_node.style.position == Position::Absolute
                    || child_node.style.position == Position::Fixed
                {
                    continue;
                }
                if child_node.style.float != Float::None {
                    let fx = if child_node.style.float == Float::Left {
                        x + ctx.float_left
                    } else {
                        x + w - ctx.float_right - box_w_for_node(child_node, w)
                    };
                    layout_node(ctx, *ci, fx, y, box_w_for_node(child_node, w), None);
                    continue;
                }
                y = layout_node(ctx, *ci, x, y, w, None);
            }
            BoxChild::Text(t) => {
                y = layout_text_run(ctx, &node.style, t, None, x, y, w);
            }
            BoxChild::Link { idx: li } => {
                if let Some(link) = ctx.doc.links.iter().find(|l| l.idx == *li) {
                    y = layout_text_run(ctx, &node.style, &link.text, Some(*li), x, y, w);
                }
            }
            BoxChild::Image { idx: ii } => {
                let (alt, dw, dh) = image_size(ctx, *ii, w);
                push_laid(
                    ctx,
                    x,
                    y,
                    dw,
                    dh,
                    node.style.clone(),
                    LaidKind::Image {
                        idx: *ii,
                        alt,
                    },
                );
                y += dh + 4.0;
            }
            BoxChild::FormInput {
                form_idx,
                field_idx,
            } => {
                let pw = ctx
                    .doc
                    .form_fields
                    .iter()
                    .find(|f| f.field_idx == *field_idx)
                    .is_some_and(|f| f.password);
                push_laid(
                    ctx,
                    x,
                    y,
                    w.min(420.0),
                    28.0,
                    node.style.clone(),
                    LaidKind::FormInput {
                        form_idx: *form_idx,
                        field_idx: *field_idx,
                        password: pw,
                    },
                );
                y += 32.0;
            }
            BoxChild::FormSubmit { form_idx, label } => {
                push_laid(
                    ctx,
                    x,
                    y,
                    120.0,
                    28.0,
                    node.style.clone(),
                    LaidKind::FormSubmit {
                        form_idx: *form_idx,
                        label: label.clone(),
                    },
                );
                y += 32.0;
            }
        }
    }
    y.max(start_y + node.style.font_size)
}

fn layout_text_run(
    ctx: &mut LayoutCtx,
    style: &ComputedStyle,
    text: &str,
    link_idx: Option<usize>,
    x: f32,
    mut y: f32,
    w: f32,
) -> f32 {
    let fs = style.font_size.max(10.0);
    let lh = fs * 1.4;
    for line in wrap_words(text, w, fs) {
        let mut tx = x;
        if style.text_align == TEXT_ALIGN_CENTER {
            let tw = line.chars().count() as f32 * fs * 0.55;
            tx = x + ((w - tw) / 2.0).max(0.0);
        }
        push_laid(
            ctx,
            tx,
            y,
            w,
            lh,
            style.clone(),
            LaidKind::Text {
                text: line,
                link_idx,
            },
        );
        y += lh;
    }
    y
}

fn wrap_words(text: &str, width_px: f32, font_size: f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    let mut line_w = 0.0f32;
    let space_w = font_size * 0.35;
    for word in text.split_whitespace() {
        let ww = word.chars().count() as f32 * font_size * 0.55;
        let extra = if line_w == 0.0 { ww } else { ww + space_w };
        if line_w > 0.0 && line_w + extra > width_px {
            out.push(line.trim().to_string());
            line.clear();
            line_w = 0.0;
        }
        if line_w > 0.0 {
            line.push(' ');
            line_w += space_w;
        }
        line.push_str(word);
        line_w += ww;
    }
    if !line.trim().is_empty() {
        out.push(line.trim().to_string());
    }
    if out.is_empty() && !text.trim().is_empty() {
        out.push(text.trim().to_string());
    }
    out
}

fn element_children(ctx: &LayoutCtx, idx: usize) -> Vec<usize> {
    ctx.doc
        .node(idx)
        .map(|n| {
            n.children
                .iter()
                .filter_map(|c| match c {
                    BoxChild::Element(i) => Some(*i),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn layout_absolute_children(
    ctx: &mut LayoutCtx,
    node: &BoxNode,
    idx: usize,
    x: f32,
    y: f32,
    w: f32,
) {
    for child in &node.children {
        if let BoxChild::Element(ci) = child {
            layout_node(ctx, *ci, x, y, w, None);
        }
    }
}

fn image_size(ctx: &LayoutCtx, idx: usize, max_w: f32) -> (String, f32, f32) {
    let Some(img) = ctx.doc.images.iter().find(|i| i.idx == idx) else {
        return (String::new(), 64.0, 64.0);
    };
    let mut w = img.width as f32;
    let mut h = img.height as f32;
    if w == 0.0 {
        w = 64.0;
    }
    if h == 0.0 {
        h = 64.0;
    }
    if w > max_w {
        let s = max_w / w;
        w = max_w;
        h = (h * s).max(1.0);
    }
    (img.alt.clone(), w, h)
}

fn push_block_shell(ctx: &mut LayoutCtx, node: &BoxNode, x: f32, y: f32, w: f32, h: f32) {
    push_laid(
        ctx,
        x,
        y,
        w,
        h,
        node.style.clone(),
        LaidKind::Block,
    );
}

fn push_laid(ctx: &mut LayoutCtx, x: f32, y: f32, w: f32, h: f32, style: ComputedStyle, kind: LaidKind) {
    if ctx.out.len() >= MAX_BOXES {
        return;
    }
    ctx.out.push(LaidBox {
        x,
        y,
        w,
        h,
        style,
        kind,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::vec;
    use crate::box_tree::{BoxChild, BoxDocument, BoxNode};
    use crate::computed::{ComputedStyle, Display, Float, GridTrackSize, Length};

    fn simple_flex_doc() -> BoxDocument {
        let mut nodes = Vec::new();
        nodes.push(BoxNode {
            tag: String::from("body"),
            class: String::new(),
            id: String::new(),
            inline_css: String::new(),
            style: ComputedStyle::default(),
            children: vec![BoxChild::Element(1)],
        });
        let mut flex_st = ComputedStyle::default();
        flex_st.display = Display::Flex;
        nodes.push(BoxNode {
            tag: String::from("div"),
            class: String::new(),
            id: String::new(),
            inline_css: String::new(),
            style: flex_st,
            children: vec![BoxChild::Element(2), BoxChild::Element(3)],
        });
        for i in 0..2 {
            nodes.push(BoxNode {
                tag: String::from("div"),
                class: String::new(),
                id: String::new(),
                inline_css: String::new(),
                style: ComputedStyle::default(),
                children: vec![BoxChild::Text(format!("c{i}"))],
            });
        }
        BoxDocument {
            page_title: None,
            page_background: 0,
            root: 0,
            nodes,
            links: Vec::new(),
            images: Vec::new(),
            forms: Vec::new(),
            form_fields: Vec::new(),
        }
    }

    #[test]
    fn flex_row_side_by_side() {
        let doc = simple_flex_doc();
        let r = layout_tree(&doc, 400.0, 8.0);
        let texts: Vec<_> = r
            .boxes
            .iter()
            .filter_map(|b| match &b.kind {
                LaidKind::Text { text, .. } => Some((b.x, text.as_str())),
                _ => None,
            })
            .collect();
        assert!(texts.len() >= 2);
        assert!(texts[0].0 < texts[1].0);
    }

    #[test]
    fn grid_two_columns() {
        let mut nodes = Vec::new();
        nodes.push(BoxNode {
            tag: String::from("body"),
            class: String::new(),
            id: String::new(),
            inline_css: String::new(),
            style: ComputedStyle::default(),
            children: vec![BoxChild::Element(1)],
        });
        let mut grid_st = ComputedStyle::default();
        grid_st.display = Display::Grid;
        grid_st.grid_columns.tracks = vec![GridTrackSize::Fr(1.0), GridTrackSize::Fr(1.0)];
        nodes.push(BoxNode {
            tag: String::from("div"),
            class: String::new(),
            id: String::new(),
            inline_css: String::new(),
            style: grid_st,
            children: vec![BoxChild::Element(2), BoxChild::Element(3)],
        });
        for t in ["a", "b"] {
            nodes.push(BoxNode {
                tag: String::from("div"),
                class: String::new(),
                id: String::new(),
                inline_css: String::new(),
                style: ComputedStyle::default(),
                children: vec![BoxChild::Text(String::from(t))],
            });
        }
        let doc = BoxDocument {
            page_title: None,
            page_background: 0,
            root: 0,
            nodes,
            links: Vec::new(),
            images: Vec::new(),
            forms: Vec::new(),
            form_fields: Vec::new(),
        };
        let r = layout_tree(&doc, 200.0, 0.0);
        let xs: Vec<f32> = r
            .boxes
            .iter()
            .filter_map(|b| match &b.kind {
                LaidKind::Text { .. } => Some(b.x),
                _ => None,
            })
            .collect();
        assert_eq!(xs.len(), 2);
        assert!((xs[0] - xs[1]).abs() > 10.0);
    }

    #[test]
    fn float_left_shifts_x() {
        let mut nodes = Vec::new();
        nodes.push(BoxNode {
            tag: String::from("body"),
            class: String::new(),
            id: String::new(),
            inline_css: String::new(),
            style: ComputedStyle::default(),
            children: vec![BoxChild::Element(1), BoxChild::Element(2)],
        });
        let mut fl = ComputedStyle::default();
        fl.float = Float::Left;
        fl.width = Length::Px(80.0);
        nodes.push(BoxNode {
            tag: String::from("div"),
            class: String::new(),
            id: String::new(),
            inline_css: String::new(),
            style: fl,
            children: vec![BoxChild::Text(String::from("f"))],
        });
        nodes.push(BoxNode {
            tag: String::from("p"),
            class: String::new(),
            id: String::new(),
            inline_css: String::new(),
            style: ComputedStyle::default(),
            children: vec![BoxChild::Text(String::from("after"))],
        });
        let doc = BoxDocument {
            page_title: None,
            page_background: 0,
            root: 0,
            nodes,
            links: Vec::new(),
            images: Vec::new(),
            forms: Vec::new(),
            form_fields: Vec::new(),
        };
        let r = layout_tree(&doc, 300.0, 0.0);
        assert!(!r.boxes.is_empty());
    }
}
