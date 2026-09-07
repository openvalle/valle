//! Backfill conservative content bounds for filter groups using shared native/WASM geometry.
//! Overestimation costs memory, but underestimation clips pixels. Use Bezier control hulls,
//! generous glyph bounds, and full filter outsets; ignoring clip intersections remains
//! conservative.

use crate::draw::PathRef;
use crate::geom::{Point, Rect};
use crate::program::recording::{Affine, FilterOp, Homography, ProgramRecording, RecordCmd};

impl ProgramRecording {
    /// Recompute each filter group's content bounds and filter outsets. Repeated calls are
    /// idempotent.
    pub fn backfill_filter_bounds(&mut self) {
        let mut stack: Vec<Frame> = vec![Frame::root()];
        for at in 0..self.cmds.len() {
            match &self.cmds[at] {
                RecordCmd::End => {
                    let Some(frame) = stack.pop() else {
                        // Validation rejects unbalanced groups; avoid panicking here.
                        return;
                    };
                    let mut bbox = frame.bbox;
                    if let Some(filter_at) = frame.filter_at {
                        let grown = bbox.map(|r| grow(r, frame.growth));
                        if let RecordCmd::BeginFilter { bounds, .. } = &mut self.cmds[filter_at] {
                            *bounds = grown;
                        }
                        bbox = grown;
                    }
                    if let (Some(parent), Some(rect)) = (stack.last_mut(), bbox) {
                        parent.add(map_frame(&frame, rect));
                    }
                    if stack.is_empty() {
                        return;
                    }
                }
                cmd => {
                    let opened = frame_for(cmd, self);
                    if let Some(mut frame) = opened {
                        // Record the command index here because frame_for does not receive it.
                        if matches!(cmd, RecordCmd::BeginFilter { .. }) {
                            frame.filter_at = Some(at);
                        }
                        stack.push(frame);
                    } else if let Some(rect) = leaf_bounds(cmd, self) {
                        if let Some(top) = stack.last_mut() {
                            top.add(rect);
                        }
                    }
                }
            }
        }
    }
}

/// An open group with bounds in its own local coordinate space. Map bounds into the parent when
/// closing the group, avoiding inverse transforms.
struct Frame {
    /// Transform introduced by this group.
    transform: Option<Affine>,
    perspective: Option<Homography>,
    /// Filter command index to update when the group closes.
    filter_at: Option<usize>,
    /// Conservative filter-chain outset, equal on all sides.
    growth: f64,
    bbox: Option<Rect>,
}

impl Frame {
    fn root() -> Self {
        Frame {
            transform: None,
            perspective: None,
            filter_at: None,
            growth: 0.0,
            bbox: None,
        }
    }

    fn add(&mut self, rect: Rect) {
        self.bbox = Some(match self.bbox {
            Some(cur) => union(cur, rect),
            None => rect,
        });
    }
}

/// Create a frame for a group opener; leaf commands return None.
fn frame_for(cmd: &RecordCmd, list: &ProgramRecording) -> Option<Frame> {
    let frame = match cmd {
        RecordCmd::BeginTransform { transform } => Frame {
            transform: Some(*transform),
            ..Frame::root()
        },
        RecordCmd::BeginPerspective { matrix } => Frame {
            perspective: Some(*matrix),
            ..Frame::root()
        },
        RecordCmd::BeginFilter { filters, .. } => Frame {
            growth: filter_growth(list.filters.get(filters.range()).unwrap_or(&[])),
            ..Frame::root()
        },
        RecordCmd::BeginGroup
        | RecordCmd::BeginOpacity { .. }
        | RecordCmd::BeginClipRect { .. }
        | RecordCmd::BeginClipRoundRect { .. }
        | RecordCmd::BeginClipPath { .. }
        | RecordCmd::BeginSaveLayer { .. }
        | RecordCmd::BeginBlend { .. }
        | RecordCmd::BeginMask { .. }
        | RecordCmd::BeginBackdropFilter { .. } => Frame::root(),
        _ => return None,
    };
    Some(frame)
}

/// Conservative filter-chain outset. Blur and shadows expand bounds; color-only filters do not. Add
/// sequential filter outsets using a three-sigma Gaussian extent.
fn filter_growth(ops: &[FilterOp]) -> f64 {
    ops.iter()
        .map(|op| match op {
            FilterOp::Blur { sigma } => sigma.abs() * 3.0,
            FilterOp::DropShadow { dx, dy, sigma, .. } => {
                sigma.abs() * 3.0 + dx.abs().max(dy.abs())
            }
            FilterOp::NoiseDisplacement { scale, .. } => scale.abs(),
            FilterOp::VelocityBlur {
                velocity_x,
                velocity_y,
                shutter_angle,
            } => {
                let exposure = (shutter_angle / 360.0).clamp(0.0, 1.0);
                velocity_x.abs().max(velocity_y.abs()) * exposure
            }
            _ => 0.0,
        })
        .sum::<f64>()
        // Allow one extra pixel for raster coverage and outward rounding.
        + if ops.is_empty() { 0.0 } else { 1.0 }
}

/// Leaf bounds in group-local coordinates; return None when geometry is unavailable.
fn leaf_bounds(cmd: &RecordCmd, list: &ProgramRecording) -> Option<Rect> {
    match cmd {
        RecordCmd::Path {
            path, fill, stroke, ..
        } => {
            let mut rect = path_bounds(list, *path)?;
            if let Some(stroke) = stroke {
                // Expand by a full stroke width to conservatively cover centered strokes and joins.
                rect = grow(rect, stroke.width.abs());
            } else if fill.is_none() {
                return None;
            }
            Some(rect)
        }
        RecordCmd::GlyphRun { font, glyphs, .. } => {
            let size = list.fonts.get(font.0 as usize).map_or(0.0, |f| f.size);
            let run = list.glyphs.get(glyphs.range())?;
            let mut bbox: Option<Rect> = None;
            for glyph in run {
                // Use a generous glyph-size envelope around the baseline origin; exact outlines
                // belong to the executor.
                let cell = Rect::new(
                    glyph.x - size * 0.5,
                    glyph.y - size * 1.5,
                    size * 2.0,
                    size * 2.0,
                );
                bbox = Some(match bbox {
                    Some(cur) => union(cur, cell),
                    None => cell,
                });
            }
            bbox
        }
        RecordCmd::GeometryBatch {
            geometry,
            instances,
        } => list
            .batch_instances
            .get(instances.range())?
            .iter()
            .fold(None, |bbox, item| {
                let rect = match geometry {
                    crate::program::recording::BatchGeometry::Circle => Rect::new(
                        item.position.x - item.size.x * 0.5,
                        item.position.y - item.size.y * 0.5,
                        item.size.x,
                        item.size.y,
                    ),
                    crate::program::recording::BatchGeometry::Rect => {
                        Rect::new(item.position.x, item.position.y, item.size.x, item.size.y)
                    }
                };
                Some(bbox.map_or(rect, |current| union(current, rect)))
            }),
        RecordCmd::Image { dst, .. } | RecordCmd::Texture { dst, .. } => Some(*dst),
        RecordCmd::Shadow {
            rrect,
            dx,
            dy,
            blur_sigma,
            spread,
            inset,
            ..
        } => {
            if *inset {
                // Inset shadows stay within the box.
                return Some(rrect.rect);
            }
            let base = Rect::new(
                rrect.rect.x + dx,
                rrect.rect.y + dy,
                rrect.rect.width,
                rrect.rect.height,
            );
            Some(grow(base, blur_sigma.abs() * 3.0 + spread.abs() + 1.0))
        }
        _ => None,
    }
}

/// Bound paths by their control-point hulls, which contain the Bezier curves.
fn path_bounds(list: &ProgramRecording, path: PathRef) -> Option<Rect> {
    let points = list.points.get(path.points.range())?;
    let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
    let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    if !x0.is_finite() || !y0.is_finite() || x1 < x0 || y1 < y0 {
        return None;
    }
    Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
}

fn union(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.width).max(b.x + b.width);
    let y1 = (a.y + a.height).max(b.y + b.height);
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

fn grow(r: Rect, by: f64) -> Rect {
    Rect::new(r.x - by, r.y - by, r.width + by * 2.0, r.height + by * 2.0)
}

/// Map all four corners into the parent space and take their axis-aligned bounds.
fn map_frame(frame: &Frame, r: Rect) -> Rect {
    let corners = [
        Point::new(r.x, r.y),
        Point::new(r.x + r.width, r.y),
        Point::new(r.x, r.y + r.height),
        Point::new(r.x + r.width, r.y + r.height),
    ];
    let mapped = corners.map(|p| {
        let p = if let Some(h) = frame.perspective {
            h.apply(p)
        } else {
            p
        };
        match frame.transform {
            Some(t) => t.apply(p),
            None => p,
        }
    });
    let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
    let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for c in mapped {
        x0 = x0.min(c.x);
        y0 = y0.min(c.y);
        x1 = x1.max(c.x);
        y1 = y1.max(c.y);
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}
