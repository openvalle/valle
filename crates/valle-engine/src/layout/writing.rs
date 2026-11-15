//! Adapt physical CSS boxes to Taffy's inline-x/block-y layout coordinates. Painting always
//! receives physical boxes. CSS grid tracks, flex direction and gaps are already logical.
use taffy::{AxisStaticEdge, AxisStaticPosition, LayoutOutput, Point, Rect, Size};

use crate::style::WritingMode;

pub(super) fn vertical(mode: WritingMode) -> bool {
    mode != WritingMode::Horizontal
}

pub(super) fn size<T>(s: Size<T>, mode: WritingMode) -> Size<T> {
    if vertical(mode) { s.transpose() } else { s }
}

pub(super) fn edges<T>(r: Rect<T>, mode: WritingMode) -> Rect<T> {
    match mode {
        WritingMode::Horizontal => r,
        WritingMode::VerticalRl => Rect {
            left: r.top,
            right: r.bottom,
            top: r.right,
            bottom: r.left,
        },
        WritingMode::VerticalLr => Rect {
            left: r.top,
            right: r.bottom,
            top: r.left,
            bottom: r.right,
        },
    }
}

fn physical_edges<T>(r: Rect<T>, mode: WritingMode) -> Rect<T> {
    match mode {
        WritingMode::Horizontal => r,
        WritingMode::VerticalRl => Rect {
            left: r.bottom,
            right: r.top,
            top: r.left,
            bottom: r.right,
        },
        WritingMode::VerticalLr => Rect {
            left: r.top,
            right: r.bottom,
            top: r.left,
            bottom: r.right,
        },
    }
}

pub(super) fn style(s: &taffy::Style, mode: WritingMode) -> taffy::Style {
    let mut out = s.clone();
    if vertical(mode) {
        out.size = s.size.transpose();
        out.min_size = s.min_size.transpose();
        out.max_size = s.max_size.transpose();
        out.aspect_ratio = s.aspect_ratio.map(|r| r.recip());
        out.overflow = s.overflow.transpose();
        out.inset = edges(s.inset, mode);
        out.margin = edges(s.margin, mode);
        out.padding = edges(s.padding, mode);
        out.border = edges(s.border, mode);
    }
    out
}

/// Store dimensions and edges physically; the location remains logical until the containing
/// block's final size is known (vertical-rl needs that width to place its right edge).
pub(super) fn layout(l: &taffy::Layout, mode: WritingMode) -> taffy::Layout {
    taffy::Layout {
        size: size(l.size, mode),
        scrollbar_size: size(l.scrollbar_size, mode),
        border: physical_edges(l.border, mode),
        padding: physical_edges(l.padding, mode),
        margin: physical_edges(l.margin, mode),
        ..*l
    }
}

pub(super) fn location(l: &taffy::Layout, parent: Size<f32>, mode: WritingMode) -> Point<f32> {
    let p = l.location;
    match mode {
        WritingMode::Horizontal => p,
        WritingMode::VerticalLr => p.transpose(),
        WritingMode::VerticalRl => Point {
            x: parent.width - p.y - l.size.width,
            y: p.x,
        },
    }
}

fn flip(mut p: AxisStaticPosition, extent: f32) -> AxisStaticPosition {
    p.area = taffy::Line {
        start: extent - p.area.end,
        end: extent - p.area.start,
    };
    let reverse = |edge| match edge {
        AxisStaticEdge::Start => AxisStaticEdge::End,
        AxisStaticEdge::End => AxisStaticEdge::Start,
        AxisStaticEdge::Center => AxisStaticEdge::Center,
    };
    p.align.keyword = reverse(p.align.keyword);
    p.align.fallback = reverse(p.align.fallback);
    p
}

/// Hoisted absolute/fixed descendants carry static positions through every intervening writing
/// mode, including a reversed block axis. Explicit insets are resolved by their containing block.
pub(super) fn output(mut out: LayoutOutput, own: WritingMode, parent: WritingMode) -> LayoutOutput {
    if own == parent {
        return out;
    }
    let physical = size(out.size, own);
    for c in out.oof_candidates.as_mut_slice() {
        let p = c.static_position;
        let p = match own {
            WritingMode::Horizontal => p,
            WritingMode::VerticalLr => p.transpose(),
            WritingMode::VerticalRl => Point {
                x: flip(p.y, physical.width),
                y: p.x,
            },
        };
        c.static_position = match parent {
            WritingMode::Horizontal => p,
            WritingMode::VerticalLr => p.transpose(),
            WritingMode::VerticalRl => Point {
                x: p.y,
                y: flip(p.x, physical.width),
            },
        };
    }
    if vertical(own) != vertical(parent) {
        out = out.transpose();
    } else {
        out.baselines = taffy::Baselines::NONE;
        out.top_margin = taffy::CollapsibleMarginSet::ZERO;
        out.bottom_margin = taffy::CollapsibleMarginSet::ZERO;
        out.margins_can_collapse_through = false;
    }
    out
}
