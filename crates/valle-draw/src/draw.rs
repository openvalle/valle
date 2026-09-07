//! Backend-neutral geometry commands shared by native Skia and web CanvasKit. Store path verbs,
//! points, and text in flat buffers addressed by spans, retaining capacity between frames. Keep
//! chart-specific concepts outside this representation.

use crate::color::Rgba;
use crate::geom::{Point, Rect};
use crate::measure::ResolvedTextStyle;
use crate::text::{HAlign, PlacedText, VAlign};
use serde::{Deserialize, Serialize};

/// Copyable range into a flat buffer with a compact serialized representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// Empty span for absent children or paint data.
    pub const EMPTY: Span = Span { start: 0, end: 0 };

    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }

    pub fn is_empty(self) -> bool {
        self.end <= self.start
    }
}

/// Path verbs consume fixed point counts: Move/Line 1, Quad 2, Cubic 3, Close 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum PathVerb {
    Move,
    Line,
    Quad,
    Cubic,
    Close,
}

impl PathVerb {
    pub fn point_count(self) -> usize {
        match self {
            PathVerb::Move | PathVerb::Line => 1,
            PathVerb::Quad => 2,
            PathVerb::Cubic => 3,
            PathVerb::Close => 0,
        }
    }
}

/// Path reference into flat buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct PathRef {
    pub verbs: Span,
    pub points: Span,
}

/// Gradient stop at a normalized offset along the start-to-end axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GradientStop {
    pub offset: f64,
    pub color: Rgba,
}

/// Linear gradient in the geometry coordinate space. Stops refer to the shared side table; multiply
/// alpha without changing shared stop data.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinearGradient {
    pub start: Point,
    pub end: Point,
    pub stops: Span,
    pub alpha: f64,
}

/// Solid or linear-gradient paint; gradient stops reside in the DrawList side table.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "value",
    deny_unknown_fields
)]
pub enum Paint {
    Solid(Rgba),
    Linear(LinearGradient),
}

impl Paint {
    pub fn solid(c: Rgba) -> Self {
        Paint::Solid(c)
    }

    /// Multiply opacity, preserving the paint unchanged when a >= 1 for exact final-frame identity.
    pub fn faded(self, a: f64) -> Self {
        if a >= 1.0 {
            return self;
        }
        match self {
            Paint::Solid(c) => Paint::Solid(c.with_opacity(a)),
            Paint::Linear(g) => Paint::Linear(LinearGradient {
                alpha: g.alpha * a.max(0.0),
                ..g
            }),
        }
    }
}

/// Stroke line style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum LineKind {
    Solid,
    Dashed,
    Dotted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Cap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Join {
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Stroke {
    pub paint: Paint,
    pub width: f64,
    /// Alternating dash lengths in pixels; None is solid.
    pub dash: Option<Vec<f64>>,
    pub cap: Cap,
    pub join: Join,
}

impl Stroke {
    pub fn solid(color: Rgba, width: f64) -> Self {
        Stroke {
            paint: Paint::Solid(color),
            width,
            dash: None,
            cap: Cap::Butt,
            join: Join::Miter,
        }
    }

    /// Scale dash patterns with stroke width.
    pub fn dashed(mut self, kind: LineKind) -> Self {
        let w = self.width.max(0.5);
        self.dash = match kind {
            LineKind::Solid => None,
            LineKind::Dashed => Some(vec![w * 4.0, w * 3.0]),
            LineKind::Dotted => {
                self.cap = Cap::Round;
                Some(vec![0.0, w * 2.5])
            }
        };
        self
    }
}

/// One drawing command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "op", deny_unknown_fields)]
pub enum DrawCmd {
    Save,
    Restore,
    ClipRect {
        rect: Rect,
    },
    /// Fill and/or stroke a path, applying fill before stroke.
    Path {
        path: PathRef,
        fill: Option<Paint>,
        stroke: Option<Stroke>,
    },
    Text {
        /// Span into the DrawList text buffer.
        text: Span,
        anchor: Point,
        h_align: HAlign,
        v_align: VAlign,
        /// Clockwise degrees around the anchor.
        rotate: f64,
        style: ResolvedTextStyle,
        paint: Paint,
    },
}

/// All drawing commands for one frame.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawList {
    pub cmds: Vec<DrawCmd>,
    pub verbs: Vec<PathVerb>,
    pub points: Vec<Point>,
    pub text: String,
    /// Shared gradient stops referenced by linear paints.
    pub gradient_stops: Vec<GradientStop>,
}

impl DrawList {
    pub fn new() -> Self {
        DrawList::default()
    }

    /// Clear frame contents while retaining allocated capacity.
    pub fn clear(&mut self) {
        self.cmds.clear();
        self.verbs.clear();
        self.points.clear();
        self.text.clear();
        self.gradient_stops.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    /// Resolve a path's verbs and points for backend iteration.
    pub fn path_data(&self, p: PathRef) -> (&[PathVerb], &[Point]) {
        (&self.verbs[p.verbs.range()], &self.points[p.points.range()])
    }

    pub fn text_of(&self, s: Span) -> &str {
        &self.text[s.range()]
    }

    /// Store gradient stops and return a linear paint with alpha 1.
    pub fn linear_gradient(&mut self, start: Point, end: Point, stops: &[(f64, Rgba)]) -> Paint {
        let s0 = self.gradient_stops.len() as u32;
        self.gradient_stops.extend(
            stops
                .iter()
                .map(|&(offset, color)| GradientStop { offset, color }),
        );
        Paint::Linear(LinearGradient {
            start,
            end,
            stops: Span {
                start: s0,
                end: self.gradient_stops.len() as u32,
            },
            alpha: 1.0,
        })
    }

    /// Resolve gradient stops from their span.
    pub fn gradient_stops_of(&self, s: Span) -> &[GradientStop] {
        &self.gradient_stops[s.range()]
    }

    pub fn begin_path(&mut self) -> PathBuilder<'_> {
        let (v0, p0) = (self.verb_len(), self.point_len());
        PathBuilder { list: self, v0, p0 }
    }

    pub fn push(&mut self, cmd: DrawCmd) {
        self.cmds.push(cmd);
    }

    pub fn fill(&mut self, path: PathRef, paint: Paint) {
        self.push(DrawCmd::Path {
            path,
            fill: Some(paint),
            stroke: None,
        });
    }

    pub fn stroke(&mut self, path: PathRef, stroke: Stroke) {
        self.push(DrawCmd::Path {
            path,
            fill: None,
            stroke: Some(stroke),
        });
    }

    pub fn fill_and_stroke(&mut self, path: PathRef, fill: Paint, stroke: Stroke) {
        self.push(DrawCmd::Path {
            path,
            fill: Some(fill),
            stroke: Some(stroke),
        });
    }

    pub fn fill_rect(&mut self, r: Rect, paint: Paint) {
        let p = self.begin_path().rect(r).finish();
        self.fill(p, paint);
    }

    pub fn stroke_line(&mut self, a: Point, b: Point, s: Stroke) {
        let p = self.begin_path().move_to(a).line_to(b).finish();
        self.stroke(p, s);
    }

    /// Place text measured during layout.
    pub fn place_text(&mut self, t: &PlacedText, paint: Paint) {
        if t.text.is_empty() {
            return;
        }
        let start = self.text.len() as u32;
        self.text.push_str(&t.text);
        let span = Span {
            start,
            end: self.text.len() as u32,
        };
        self.push(DrawCmd::Text {
            text: span,
            anchor: t.anchor,
            h_align: t.h_align,
            v_align: t.v_align,
            rotate: t.rotate,
            style: t.style.clone(),
            paint,
        });
    }

    /// Place text using its resolved style color.
    pub fn place_text_default(&mut self, t: &PlacedText) {
        let c = t.style.color;
        self.place_text(t, Paint::Solid(c));
    }
}

/// Shared flat-path storage interface for DrawList and ProgramRecording, allowing both containers
/// to use the same geometry builders.
pub trait PathSink {
    fn push_verb(&mut self, v: PathVerb);
    fn push_point(&mut self, p: Point);
    fn verb_len(&self) -> u32;
    fn point_len(&self) -> u32;
}

impl PathSink for DrawList {
    fn push_verb(&mut self, v: PathVerb) {
        self.verbs.push(v);
    }
    fn push_point(&mut self, p: Point) {
        self.points.push(p);
    }
    fn verb_len(&self) -> u32 {
        self.verbs.len() as u32
    }
    fn point_len(&self) -> u32 {
        self.points.len() as u32
    }
}

/// Path builder that returns a PathRef on finish. Dropping without finishing leaves appended data
/// visible for validation; it does not silently roll back.
pub struct PathBuilder<'a, S: PathSink = DrawList> {
    list: &'a mut S,
    v0: u32,
    p0: u32,
}

impl<'a, S: PathSink> PathBuilder<'a, S> {
    /// Start a path at the sink's current buffer lengths.
    pub fn new(list: &'a mut S, v0: u32, p0: u32) -> Self {
        PathBuilder { list, v0, p0 }
    }
}

impl<S: PathSink> PathBuilder<'_, S> {
    fn verb(&mut self, v: PathVerb) -> &mut Self {
        self.list.push_verb(v);
        self
    }

    pub fn move_to(mut self, p: Point) -> Self {
        self.verb(PathVerb::Move);
        self.list.push_point(p);
        self
    }

    pub fn line_to(mut self, p: Point) -> Self {
        self.verb(PathVerb::Line);
        self.list.push_point(p);
        self
    }

    pub fn quad_to(mut self, c: Point, p: Point) -> Self {
        self.verb(PathVerb::Quad);
        self.list.push_point(c);
        self.list.push_point(p);
        self
    }

    pub fn cubic_to(mut self, c1: Point, c2: Point, p: Point) -> Self {
        self.verb(PathVerb::Cubic);
        self.list.push_point(c1);
        self.list.push_point(c2);
        self.list.push_point(p);
        self
    }

    pub fn close(mut self) -> Self {
        self.verb(PathVerb::Close);
        self
    }

    pub fn rect(self, r: Rect) -> Self {
        self.move_to(Point::new(r.left(), r.top()))
            .line_to(Point::new(r.right(), r.top()))
            .line_to(Point::new(r.right(), r.bottom()))
            .line_to(Point::new(r.left(), r.bottom()))
            .close()
    }

    /// Rounded rectangle with radii ordered top-left, top-right, bottom-right, bottom-left. Clamp
    /// each radius to half its adjacent edge lengths.
    pub fn rrect(self, r: Rect, radii: [f64; 4]) -> Self {
        self.elliptical_rrect(r, radii.map(|radius| Point::new(radius, radius)))
    }

    /// CSS/SVG elliptical rounded rectangle. Each corner stores `(rx, ry)` and all corners are
    /// scaled by the CSS overlapping-curves factor so adjacent radii never cross.
    pub fn elliptical_rrect(self, r: Rect, radii: [Point; 4]) -> Self {
        let mut radii = radii.map(|radius| Point::new(radius.x.max(0.0), radius.y.max(0.0)));
        let ratio = |extent: f64, sum: f64| {
            if sum > 0.0 {
                extent.max(0.0) / sum
            } else {
                1.0
            }
        };
        let scale = 1.0_f64
            .min(ratio(r.width, radii[0].x + radii[1].x))
            .min(ratio(r.width, radii[3].x + radii[2].x))
            .min(ratio(r.height, radii[0].y + radii[3].y))
            .min(ratio(r.height, radii[1].y + radii[2].y));
        for radius in &mut radii {
            radius.x *= scale;
            radius.y *= scale;
        }
        let [tl, tr, br, bl] = radii;
        if radii
            .iter()
            .all(|radius| radius.x <= 0.0 || radius.y <= 0.0)
        {
            return self.rect(r);
        }
        // Cubic Bezier approximation constant for a quarter circle.
        const K: f64 = 0.552_284_749_831;
        let (l, t, rt, b) = (r.left(), r.top(), r.right(), r.bottom());
        let rounded = |radius: Point| radius.x > 0.0 && radius.y > 0.0;
        let mut path = self
            .move_to(if rounded(tl) {
                Point::new(l + tl.x, t)
            } else {
                Point::new(l, t)
            })
            .line_to(if rounded(tr) {
                Point::new(rt - tr.x, t)
            } else {
                Point::new(rt, t)
            });
        path = if rounded(tr) {
            path.cubic_to(
                Point::new(rt - tr.x + tr.x * K, t),
                Point::new(rt, t + tr.y - tr.y * K),
                Point::new(rt, t + tr.y),
            )
        } else {
            path
        };
        path = path.line_to(if rounded(br) {
            Point::new(rt, b - br.y)
        } else {
            Point::new(rt, b)
        });
        path = if rounded(br) {
            path.cubic_to(
                Point::new(rt, b - br.y + br.y * K),
                Point::new(rt - br.x + br.x * K, b),
                Point::new(rt - br.x, b),
            )
        } else {
            path
        };
        path = path.line_to(if rounded(bl) {
            Point::new(l + bl.x, b)
        } else {
            Point::new(l, b)
        });
        path = if rounded(bl) {
            path.cubic_to(
                Point::new(l + bl.x - bl.x * K, b),
                Point::new(l, b - bl.y + bl.y * K),
                Point::new(l, b - bl.y),
            )
        } else {
            path
        };
        path = path.line_to(if rounded(tl) {
            Point::new(l, t + tl.y)
        } else {
            Point::new(l, t)
        });
        path = if rounded(tl) {
            path.cubic_to(
                Point::new(l, t + tl.y - tl.y * K),
                Point::new(l + tl.x - tl.x * K, t),
                Point::new(l + tl.x, t),
            )
        } else {
            path
        };
        path.close()
    }

    /// Build a full circle from four cubic Bezier segments.
    pub fn circle(self, c: Point, r: f64) -> Self {
        const K: f64 = 0.552_284_749_831;
        let k = r * K;
        self.move_to(Point::new(c.x, c.y - r))
            .cubic_to(
                Point::new(c.x + k, c.y - r),
                Point::new(c.x + r, c.y - k),
                Point::new(c.x + r, c.y),
            )
            .cubic_to(
                Point::new(c.x + r, c.y + k),
                Point::new(c.x + k, c.y + r),
                Point::new(c.x, c.y + r),
            )
            .cubic_to(
                Point::new(c.x - k, c.y + r),
                Point::new(c.x - r, c.y + k),
                Point::new(c.x - r, c.y),
            )
            .cubic_to(
                Point::new(c.x - r, c.y - k),
                Point::new(c.x - k, c.y - r),
                Point::new(c.x, c.y - r),
            )
            .close()
    }

    /// Append an arc from the current point, which the caller must place at its start. Angles use
    /// radians with clockwise-positive screen coordinates. Approximate segments of at most 90
    /// degrees with cubic Beziers.
    pub fn arc_to(mut self, c: Point, r: f64, a0: f64, a1: f64) -> Self {
        let sweep = a1 - a0;
        if r <= 0.0 || sweep == 0.0 {
            return self;
        }
        let steps = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0);
        let d = sweep / steps;
        let k = 4.0 / 3.0 * crate::math::tan(d / 4.0);
        let mut a = a0;
        for _ in 0..steps as usize {
            let b = a + d;
            let (s0, c0) = crate::math::sin_cos(a);
            let (s1, c1) = crate::math::sin_cos(b);
            let p0 = Point::new(c.x + r * c0, c.y + r * s0);
            let p1 = Point::new(c.x + r * c1, c.y + r * s1);
            self = self.cubic_to(
                Point::new(p0.x - k * r * s0, p0.y + k * r * c0),
                Point::new(p1.x + k * r * s1, p1.y - k * r * c1),
                p1,
            );
            a = b;
        }
        self
    }

    /// Append the shorter corner arc around cc; the current point must be at its start.
    fn corner_arc(self, cc: Point, cr: f64, from: f64, to: f64) -> Self {
        let mut d = to - from;
        while d > std::f64::consts::PI {
            d -= std::f64::consts::TAU;
        }
        while d < -std::f64::consts::PI {
            d += std::f64::consts::TAU;
        }
        self.arc_to(cc, cr, from, from + d)
    }

    /// Build a rounded annular sector using corner circles tangent to radial and circular edges.
    /// Clamp the radius to half the ring thickness and prevent adjacent corners from intersecting.
    pub fn rounded_sector(self, c: Point, r0: f64, r1: f64, a0: f64, a1: f64, cr: f64) -> Self {
        use std::f64::consts::{FRAC_PI_2, TAU};
        let (r0, r1) = (r0.max(0.0).min(r1.max(0.0)), r1.max(0.0));
        let sweep = a1 - a0;
        // Full circles and degenerate shapes use the unrounded path.
        if cr <= 0.0 || r1 <= 0.0 || sweep == 0.0 || sweep.abs() >= TAU - 1e-9 {
            return self.sector(c, r0, r1, a0, a1);
        }
        let s = sweep.signum();
        let half = sweep.abs() / 2.0;
        // Corner circles cannot collide when the half-angle exceeds 90 degrees.
        let sh = if half >= FRAC_PI_2 {
            1.0
        } else {
            crate::math::sin(half).max(0.0)
        };
        let mut cr = cr.min((r1 - r0) / 2.0).min(r1 * sh / (1.0 + sh));
        if r0 > 0.0 && sh < 1.0 {
            cr = cr.min(r0 * sh / (1.0 - sh));
        }
        if cr <= 1e-6 {
            return self.sector(c, r0, r1, a0, a1);
        }

        let at = |r: f64, a: f64| {
            let (sn, cs) = crate::math::sin_cos(a);
            Point::new(c.x + r * cs, c.y + r * sn)
        };
        let ro = r1 - cr;
        let d_out = crate::math::asin((cr / ro).clamp(-1.0, 1.0));
        // Project each corner center onto the radial edge to find its tangent point.
        let foot_out = crate::math::sqrt((ro * ro - cr * cr).max(0.0));

        if r0 <= 0.0 {
            // For solid sectors, round only the outer corners and preserve the center tip.
            let (co0, co1) = (at(ro, a0 + s * d_out), at(ro, a1 - s * d_out));
            return self
                .move_to(c)
                .line_to(at(foot_out, a0))
                .corner_arc(co0, cr, a0 - s * FRAC_PI_2, a0 + s * d_out)
                .arc_to(c, r1, a0 + s * d_out, a1 - s * d_out)
                .corner_arc(co1, cr, a1 - s * d_out, a1 + s * FRAC_PI_2)
                .line_to(c)
                .close();
        }

        let ri = r0 + cr;
        let d_in = crate::math::asin((cr / ri).clamp(-1.0, 1.0));
        let foot_in = crate::math::sqrt((ri * ri - cr * cr).max(0.0));
        let (ci0, ci1) = (at(ri, a0 + s * d_in), at(ri, a1 - s * d_in));
        let (co0, co1) = (at(ro, a0 + s * d_out), at(ro, a1 - s * d_out));
        let pi = std::f64::consts::PI;
        self.move_to(at(r0, a0 + s * d_in))
            // Connect the inner starting corner to the radial edge.
            .corner_arc(ci0, cr, a0 + s * d_in + pi, a0 - s * FRAC_PI_2)
            .line_to(at(foot_out, a0))
            .corner_arc(co0, cr, a0 - s * FRAC_PI_2, a0 + s * d_out)
            .arc_to(c, r1, a0 + s * d_out, a1 - s * d_out)
            .corner_arc(co1, cr, a1 - s * d_out, a1 + s * FRAC_PI_2)
            .line_to(at(foot_in, a1))
            .corner_arc(ci1, cr, a1 + s * FRAC_PI_2, a1 - s * d_in + pi)
            .arc_to(c, r0, a1 - s * d_in, a0 + s * d_in)
            .close()
    }

    /// Build a solid or annular sector. Full sweeps become circles or rings; reverse the inner
    /// winding to create a hole with nonzero fill rules.
    pub fn sector(self, c: Point, r0: f64, r1: f64, a0: f64, a1: f64) -> Self {
        const FULL: f64 = std::f64::consts::TAU;
        let (r0, r1) = (r0.max(0.0).min(r1.max(0.0)), r1.max(0.0));
        if r1 <= 0.0 {
            return self;
        }
        let at = |r: f64, a: f64| {
            let (s, cs) = crate::math::sin_cos(a);
            Point::new(c.x + r * cs, c.y + r * s)
        };
        // Handle full sweeps explicitly so coincident endpoints do not collapse the shape.
        if (a1 - a0).abs() >= FULL - 1e-9 {
            let outer = self.circle(c, r1);
            return if r0 > 0.0 {
                outer.move_to(at(r0, 0.0)).arc_to(c, r0, 0.0, -FULL).close()
            } else {
                outer
            };
        }
        let b = self.move_to(at(r1, a0)).arc_to(c, r1, a0, a1);
        if r0 > 0.0 {
            b.line_to(at(r0, a1)).arc_to(c, r0, a1, a0).close()
        } else {
            b.line_to(c).close()
        }
    }

    pub fn finish(self) -> PathRef {
        PathRef {
            verbs: Span {
                start: self.v0,
                end: self.list.verb_len(),
            },
            points: Span {
                start: self.p0,
                end: self.list.point_len(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_buffers_pack_paths_back_to_back() {
        let mut d = DrawList::new();
        let a = d
            .begin_path()
            .move_to(Point::new(0.0, 0.0))
            .line_to(Point::new(1.0, 1.0))
            .finish();
        let b = d.begin_path().rect(Rect::new(0.0, 0.0, 2.0, 2.0)).finish();
        assert_eq!(a.verbs, Span { start: 0, end: 2 });
        assert_eq!(b.verbs, Span { start: 2, end: 7 }); // move + 3 line + close
        let (va, pa) = d.path_data(a);
        assert_eq!(va, &[PathVerb::Move, PathVerb::Line]);
        assert_eq!(pa.len(), 2);
        let (vb, pb) = d.path_data(b);
        assert_eq!(vb.last(), Some(&PathVerb::Close));
        assert_eq!(pb.len(), 4, "close must not consume points");
    }

    #[test]
    fn verb_point_counts_add_up() {
        // Path point counts must match backend cursor advancement.
        let mut d = DrawList::new();
        let p = d
            .begin_path()
            .move_to(Point::default())
            .quad_to(Point::default(), Point::default())
            .cubic_to(Point::default(), Point::default(), Point::default())
            .close()
            .finish();
        let (v, pts) = d.path_data(p);
        assert_eq!(v.iter().map(|x| x.point_count()).sum::<usize>(), pts.len());
    }

    #[test]
    fn clear_keeps_capacity() {
        let mut d = DrawList::new();
        for i in 0..100 {
            d.fill_rect(
                Rect::new(i as f64, 0.0, 1.0, 1.0),
                Paint::solid(Rgba::rgb(1, 2, 3)),
            );
        }
        d.linear_gradient(
            Point::default(),
            Point::new(0.0, 1.0),
            &[(0.0, Rgba::rgb(0, 0, 0)), (1.0, Rgba::rgb(255, 255, 255))],
        );
        let (vc, pc, gc) = (
            d.verbs.capacity(),
            d.points.capacity(),
            d.gradient_stops.capacity(),
        );
        d.clear();
        assert!(d.is_empty());
        assert!(d.gradient_stops.is_empty());
        assert_eq!(
            (
                d.verbs.capacity(),
                d.points.capacity(),
                d.gradient_stops.capacity()
            ),
            (vc, pc, gc),
            "clear must retain capacity"
        );
    }

    #[test]
    fn gradient_stops_intern_back_to_back() {
        let mut d = DrawList::new();
        let a = d.linear_gradient(
            Point::default(),
            Point::new(0.0, 1.0),
            &[(0.0, Rgba::rgb(1, 0, 0)), (1.0, Rgba::rgb(2, 0, 0))],
        );
        let b = d.linear_gradient(
            Point::default(),
            Point::new(1.0, 0.0),
            &[
                (0.0, Rgba::rgb(3, 0, 0)),
                (0.5, Rgba::rgb(4, 0, 0)),
                (1.0, Rgba::rgb(5, 0, 0)),
            ],
        );
        let (Paint::Linear(ga), Paint::Linear(gb)) = (a, b) else {
            panic!()
        };
        assert_eq!(ga.stops, Span { start: 0, end: 2 });
        assert_eq!(gb.stops, Span { start: 2, end: 5 });
        assert_eq!(d.gradient_stops_of(ga.stops)[1].color, Rgba::rgb(2, 0, 0));
        assert_eq!(d.gradient_stops_of(gb.stops)[0].color, Rgba::rgb(3, 0, 0));
        assert_eq!(ga.alpha, 1.0);
    }

    #[test]
    fn faded_is_identity_at_full_alpha_and_scales_below() {
        let solid = Paint::solid(Rgba::new(10, 20, 30, 200));
        assert_eq!(
            solid.faded(1.0),
            solid,
            "alpha at least one must preserve exact identity"
        );
        assert_eq!(solid.faded(2.0), solid);
        let Paint::Solid(c) = solid.faded(0.5) else {
            panic!()
        };
        assert_eq!(c.a, 100);

        let mut d = DrawList::new();
        let g = d.linear_gradient(
            Point::default(),
            Point::new(0.0, 1.0),
            &[(0.0, Rgba::rgb(0, 0, 0)), (1.0, Rgba::rgb(9, 9, 9))],
        );
        assert_eq!(g.faded(1.0), g);
        let Paint::Linear(lg) = g.faded(0.25) else {
            panic!()
        };
        assert_eq!(lg.alpha, 0.25);
        // Fading changes alpha without modifying shared gradient stops.
        assert_eq!(d.gradient_stops_of(lg.stops).len(), 2);
        let Paint::Linear(lg2) = Paint::Linear(lg).faded(0.5) else {
            panic!()
        };
        assert_eq!(lg2.alpha, 0.125, "fade factors multiply");
    }

    #[test]
    fn rrect_clamps_oversized_radii() {
        let mut d = DrawList::new();
        // Clamp oversized corner radii to preserve a non-self-intersecting closed path.
        let p = d
            .begin_path()
            .rrect(Rect::new(0.0, 0.0, 10.0, 20.0), [999.0; 4])
            .finish();
        let (_, pts) = d.path_data(p);
        assert!(
            pts.iter()
                .all(|q| (0.0..=10.0).contains(&q.x) && (0.0..=20.0).contains(&q.y))
        );
    }

    #[test]
    fn elliptical_rrect_keeps_horizontal_and_vertical_radii_separate() {
        let mut d = DrawList::new();
        let p = d
            .begin_path()
            .elliptical_rrect(Rect::new(0.0, 0.0, 10.0, 20.0), [Point::new(4.0, 2.0); 4])
            .finish();
        let (_, points) = d.path_data(p);
        assert_eq!(points[0], Point::new(4.0, 0.0));
        assert!(points.contains(&Point::new(10.0, 2.0)));
        assert!(!points.contains(&Point::new(10.0, 4.0)));

        let degenerate = d
            .begin_path()
            .elliptical_rrect(Rect::new(0.0, 0.0, 10.0, 20.0), [Point::new(4.0, 0.0); 4])
            .finish();
        let (verbs, points) = d.path_data(degenerate);
        assert_eq!(
            verbs,
            &[
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Close
            ]
        );
        assert_eq!(
            points,
            &[
                Point::new(0.0, 0.0),
                Point::new(10.0, 0.0),
                Point::new(10.0, 20.0),
                Point::new(0.0, 20.0)
            ]
        );
    }

    #[test]
    fn zero_radius_rrect_degenerates_to_a_plain_rect() {
        let mut d = DrawList::new();
        let a = d
            .begin_path()
            .rrect(Rect::new(0.0, 0.0, 4.0, 4.0), [0.0; 4])
            .finish();
        let b = d.begin_path().rect(Rect::new(0.0, 0.0, 4.0, 4.0)).finish();
        assert_eq!(d.path_data(a).0, d.path_data(b).0);
        assert_eq!(d.path_data(a).1, d.path_data(b).1);
    }

    #[test]
    fn arc_endpoints_land_where_the_trig_says() {
        // Bezier approximations must preserve exact arc endpoints.
        let c = Point::new(100.0, 50.0);
        for (a0, a1) in [
            (0.0, std::f64::consts::FRAC_PI_2),
            (-std::f64::consts::FRAC_PI_2, 1.3),
            (0.4, -2.9), // Counterclockwise.
        ] {
            let mut d = DrawList::new();
            let start = Point::new(c.x + 20.0 * a0.cos(), c.y + 20.0 * a0.sin());
            let p = d
                .begin_path()
                .move_to(start)
                .arc_to(c, 20.0, a0, a1)
                .finish();
            let (_, pts) = d.path_data(p);
            let last = *pts.last().unwrap();
            let want = Point::new(c.x + 20.0 * a1.cos(), c.y + 20.0 * a1.sin());
            assert!(
                (last.x - want.x).abs() < 1e-9 && (last.y - want.y).abs() < 1e-9,
                "{a0}→{a1}: {last:?} != {want:?}"
            );
            // Check segment midpoints as well as endpoints to detect poor arc approximations.
            let mut prev = start;
            for w in pts[1..].chunks(3) {
                let [c1, c2, p3] = [w[0], w[1], w[2]];
                let mid = |a: f64, b: f64, cc: f64, dd: f64| (a + 3.0 * b + 3.0 * cc + dd) / 8.0;
                let m = Point::new(mid(prev.x, c1.x, c2.x, p3.x), mid(prev.y, c1.y, c2.y, p3.y));
                let err = (((m.x - c.x).powi(2) + (m.y - c.y).powi(2)).sqrt() - 20.0).abs();
                assert!(
                    err < 0.02,
                    "arc midpoint deviates from the circle by {err}px"
                );
                prev = p3;
            }
        }
    }

    #[test]
    fn sector_closes_and_donuts_wind_the_hole_backwards() {
        let c = Point::new(0.0, 0.0);
        // Solid sector: outer arc, center, then close.
        let mut d = DrawList::new();
        let p = d.begin_path().sector(c, 0.0, 10.0, 0.0, 1.0).finish();
        let (v, pts) = d.path_data(p);
        assert_eq!(v[0], PathVerb::Move);
        assert_eq!(v.last(), Some(&PathVerb::Close));
        assert!(
            pts.iter().any(|q| q.x == 0.0 && q.y == 0.0),
            "must pass through the circle center"
        );

        // Annular sector: reverse the inner winding and omit the center point.
        let mut d = DrawList::new();
        let p = d.begin_path().sector(c, 4.0, 10.0, 0.0, 1.0).finish();
        let (_, pts) = d.path_data(p);
        assert!(!pts.iter().any(|q| q.x == 0.0 && q.y == 0.0));
        let r = |q: &Point| (q.x * q.x + q.y * q.y).sqrt();
        assert!((r(&pts[0]) - 10.0).abs() < 1e-9);
        assert!((r(pts.last().unwrap()) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn a_full_circle_sector_is_not_a_hairline_crack() {
        // A full sweep must remain a circle rather than collapsing at coincident endpoints.
        let mut d = DrawList::new();
        let full = d
            .begin_path()
            .sector(Point::new(0.0, 0.0), 0.0, 10.0, 0.0, std::f64::consts::TAU)
            .finish();
        let plain = d.begin_path().circle(Point::new(0.0, 0.0), 10.0).finish();
        assert_eq!(d.path_data(full).0, d.path_data(plain).0);
        assert_eq!(d.path_data(full).1, d.path_data(plain).1);
    }

    #[test]
    fn rounded_sector_stays_inside_the_ring_and_closes() {
        let c = Point::new(0.0, 0.0);
        let (r0, r1) = (30.0, 60.0);
        let mut d = DrawList::new();
        let p = d
            .begin_path()
            .rounded_sector(c, r0, r1, 0.0, 1.2, 8.0)
            .finish();
        let (v, pts) = d.path_data(p);
        assert_eq!(v[0], PathVerb::Move);
        assert_eq!(v.last(), Some(&PathVerb::Close));
        // Allow 6% radial tolerance for cubic control points while rejecting escaped geometry.
        let tol = r1 * 0.06;
        for q in pts {
            let r = (q.x * q.x + q.y * q.y).sqrt();
            assert!(
                r >= r0 - tol && r <= r1 + tol,
                "vertex outside the annulus: r={r} (range {r0}..{r1})"
            );
        }
    }

    #[test]
    fn oversized_corner_radius_is_clamped_not_self_intersecting() {
        // Clamp large corner radii in narrow sectors to avoid intersections.
        let c = Point::new(0.0, 0.0);
        let mut d = DrawList::new();
        let p = d
            .begin_path()
            .rounded_sector(c, 20.0, 25.0, 0.0, 0.05, 999.0)
            .finish();
        let (_, pts) = d.path_data(p);
        for q in pts {
            let r = (q.x * q.x + q.y * q.y).sqrt();
            assert!((19.0..=26.0).contains(&r), "clamping failed: r={r}");
        }
        // Rounded corners must not reverse the angular span.
        let ang: Vec<f64> = pts.iter().map(|q| q.y.atan2(q.x)).collect();
        assert!(
            ang.iter().all(|a| (-0.02..=0.07).contains(a)),
            "angle outside bounds: {ang:?}"
        );
    }

    #[test]
    fn zero_corner_radius_is_the_plain_sector() {
        let mut d = DrawList::new();
        let a = d
            .begin_path()
            .rounded_sector(Point::new(1.0, 2.0), 4.0, 9.0, 0.3, 2.0, 0.0)
            .finish();
        let b = d
            .begin_path()
            .sector(Point::new(1.0, 2.0), 4.0, 9.0, 0.3, 2.0)
            .finish();
        assert_eq!(d.path_data(a).0, d.path_data(b).0);
        assert_eq!(d.path_data(a).1, d.path_data(b).1);
        // Full circles have no corners to round.
        let full = d
            .begin_path()
            .rounded_sector(
                Point::new(0.0, 0.0),
                0.0,
                5.0,
                0.0,
                std::f64::consts::TAU,
                3.0,
            )
            .finish();
        let plain = d.begin_path().circle(Point::new(0.0, 0.0), 5.0).finish();
        assert_eq!(d.path_data(full).1, d.path_data(plain).1);
    }

    #[test]
    fn a_solid_wedge_keeps_a_sharp_apex() {
        // Preserve the center tip of a solid sector to avoid a hole.
        let mut d = DrawList::new();
        let p = d
            .begin_path()
            .rounded_sector(Point::new(0.0, 0.0), 0.0, 40.0, 0.0, 1.0, 6.0)
            .finish();
        let (_, pts) = d.path_data(p);
        assert!(
            pts.iter().any(|q| q.x == 0.0 && q.y == 0.0),
            "the tip must pass through the circle center"
        );
    }

    #[test]
    fn text_is_interned_into_one_buffer() {
        let mut d = DrawList::new();
        let mk = |s: &str| PlacedText {
            text: s.into(),
            anchor: Point::default(),
            h_align: HAlign::Left,
            v_align: VAlign::Top,
            rotate: 0.0,
            metrics: Default::default(),
            style: ResolvedTextStyle::root(),
        };
        d.place_text_default(&mk("甲"));
        d.place_text_default(&mk("乙"));
        assert_eq!(d.text, "甲乙");
        let DrawCmd::Text { text, .. } = d.cmds[1] else {
            panic!()
        };
        assert_eq!(d.text_of(text), "乙");
        // Empty text emits no command.
        d.place_text_default(&mk(""));
        assert_eq!(d.cmds.len(), 2);
    }
}
