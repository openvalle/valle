//! Construction-only recording for Motion DrawPrograms. Pair Begin/End groups around leaf drawing
//! commands and store owned data in flat side tables. Validate group balance and references before
//! compilation. Component-level paint groups are separate from timeline clip effects. Blur values
//! always store Gaussian sigma; CSS box/text-shadow blur radii are converted to sigma by dividing
//! by two.

use serde::{Deserialize, Serialize};

use crate::color::Rgba;
use crate::draw::{Cap, GradientStop, Join, PathBuilder, PathRef, PathSink, PathVerb, Span};
use crate::geom::{Point, Rect};
use crate::requirements::DigestBytes;

/// Aggregate instance side-table budget for one emitted frame, across every GeometryBatch node.
pub const MAX_BATCH_INSTANCES_PER_RECORDING: usize = 100_000;

// Core types.

/// Two-dimensional affine matrix in CSS matrix(a, b, c, d, e, f) order: x' = a*x + c*y + e, y' =
/// b*x + d*y + f.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Affine(pub [f64; 6]);

impl Affine {
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn translate(dx: f64, dy: f64) -> Affine {
        Affine([1.0, 0.0, 0.0, 1.0, dx, dy])
    }

    pub fn scale(sx: f64, sy: f64) -> Affine {
        Affine([sx, 0.0, 0.0, sy, 0.0, 0.0])
    }

    /// Clockwise rotation in degrees using downward-positive screen y coordinates.
    pub fn rotate(degrees: f64) -> Affine {
        let (s, c) = crate::math::sin_cos(degrees.to_radians());
        Affine([c, s, -s, c, 0.0, 0.0])
    }

    /// Apply self, then next; equivalent to CSS transform: next self.
    pub fn then(self, next: Affine) -> Affine {
        let (a, b) = (self.0, next.0);
        Affine([
            a[0] * b[0] + a[1] * b[2],
            a[0] * b[1] + a[1] * b[3],
            a[2] * b[0] + a[3] * b[2],
            a[2] * b[1] + a[3] * b[3],
            a[4] * b[0] + a[5] * b[2] + b[4],
            a[4] * b[1] + a[5] * b[3] + b[5],
        ])
    }

    pub fn apply(self, p: Point) -> Point {
        let m = self.0;
        Point::new(
            m[0] * p.x + m[2] * p.y + m[4],
            m[1] * p.x + m[3] * p.y + m[5],
        )
    }

    /// Invert one finite, non-degenerate affine. Locate/picking uses the same transform stack as
    /// paint and fails closed for zero-scale matrices instead of guessing a local coordinate.
    pub fn inverse(self) -> Option<Affine> {
        let [a, b, c, d, e, f] = self.0;
        let determinant = a * d - b * c;
        if !determinant.is_finite() || determinant == 0.0 {
            return None;
        }
        let inverse = 1.0 / determinant;
        let output = Affine([
            d * inverse,
            -b * inverse,
            -c * inverse,
            a * inverse,
            (c * f - d * e) * inverse,
            (b * e - a * f) * inverse,
        ]);
        output
            .0
            .iter()
            .all(|value| value.is_finite())
            .then_some(output)
    }
}

/// Row-major 3x3 homography in Skia/CanvasKit order. Divide transformed x and y by the perspective
/// denominator to support projected layers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Homography(pub [f64; 9]);

impl Homography {
    pub const IDENTITY: Homography = Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    pub fn apply(self, p: Point) -> Point {
        self.try_apply(p).unwrap_or(p)
    }

    /// Map a point through the homography. Returns `None` when the point sits on the
    /// vanishing line (`w == 0`) or the result is not finite — pick/locate fail closed
    /// instead of inventing a local coordinate.
    pub fn try_apply(self, p: Point) -> Option<Point> {
        let m = self.0;
        let w = m[6] * p.x + m[7] * p.y + m[8];
        if !w.is_finite() || w == 0.0 {
            return None;
        }
        let mapped = Point::new(
            (m[0] * p.x + m[1] * p.y + m[2]) / w,
            (m[3] * p.x + m[4] * p.y + m[5]) / w,
        );
        (mapped.x.is_finite() && mapped.y.is_finite()).then_some(mapped)
    }

    /// CSS `matrix(a, b, c, d, e, f)` as a 3×3 with a zero perspective row.
    pub fn from_affine(transform: Affine) -> Homography {
        let [a, b, c, d, e, f] = transform.0;
        Homography([a, c, e, b, d, f, 0.0, 0.0, 1.0])
    }

    /// Apply self before next, matching Affine::then.
    pub fn then(self, next: Homography) -> Homography {
        Homography(mul3(next.0, self.0))
    }

    /// Invert one finite, non-degenerate homography. Same fail-closed contract as
    /// [`Affine::inverse`]: pick must not guess a local coordinate.
    pub fn inverse(self) -> Option<Homography> {
        let [a, b, c, d, e, f, g, h, i] = self.0;
        let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
        if !det.is_finite() || det == 0.0 {
            return None;
        }
        let inv = 1.0 / det;
        let matrix = [
            (e * i - f * h) * inv,
            (c * h - b * i) * inv,
            (b * f - c * e) * inv,
            (f * g - d * i) * inv,
            (a * i - c * g) * inv,
            (c * d - a * f) * inv,
            (d * h - e * g) * inv,
            (b * g - a * h) * inv,
            (a * e - b * d) * inv,
        ];
        matrix
            .iter()
            .all(|value| value.is_finite())
            .then_some(Homography(matrix))
    }

    /// Compose CSS perspective(d) rotateY(y) rotateX(x) around a center. Rotate in 3D before
    /// projecting once; multiplying separately projected axes is not equivalent. Require positive
    /// perspective and omit negligible rotations.
    pub fn layer(
        cx: f64,
        cy: f64,
        rotate_y_deg: f64,
        rotate_x_deg: f64,
        perspective: f64,
    ) -> Option<Homography> {
        if !cx.is_finite()
            || !cy.is_finite()
            || !rotate_y_deg.is_finite()
            || !rotate_x_deg.is_finite()
            || !perspective.is_finite()
            || perspective <= 0.0
        {
            return None;
        }
        if rotate_y_deg.abs() < 1e-6 && rotate_x_deg.abs() < 1e-6 {
            return None;
        }
        let (sy, cy_y) = crate::math::sin_cos(rotate_y_deg.to_radians());
        let (sx, cx_x) = crate::math::sin_cos(rotate_x_deg.to_radians());
        let d = perspective;
        // Apply Ry * Rx to (x, y, 0), then project with w = 1 - z/d. Positive rotateY moves the
        // right edge away.
        // Positive rotateX moves the lower edge toward the viewer.
        let rotate = [
            cy_y,
            sy * sx,
            0.0,
            0.0,
            cx_x,
            0.0,
            sy / d,
            -cy_y * sx / d,
            1.0,
        ];
        let to_origin = [1.0, 0.0, -cx, 0.0, 1.0, -cy, 0.0, 0.0, 1.0];
        let from_origin = [1.0, 0.0, cx, 0.0, 1.0, cy, 0.0, 0.0, 1.0];
        let matrix = mul3(from_origin, mul3(rotate, to_origin));
        matrix
            .iter()
            .all(|value| value.is_finite())
            .then_some(Homography(matrix))
    }

    /// Projective map of a source rectangle onto a destination quad.
    ///
    /// `dst` is TL, TR, BR, BL in the same space the resulting matrix is applied.
    /// This is still one 3×3 plane warp — not `preserve-3d`, not a 4×4 scene graph.
    /// Degenerate or non-finite correspondences return `None`.
    pub fn map_rect(src: Rect, dst: [Point; 4]) -> Option<Homography> {
        if !src.width.is_finite()
            || !src.height.is_finite()
            || src.width.abs() < 1e-9
            || src.height.abs() < 1e-9
        {
            return None;
        }
        let src_pts = [
            Point::new(src.x, src.y),
            Point::new(src.x + src.width, src.y),
            Point::new(src.x + src.width, src.y + src.height),
            Point::new(src.x, src.y + src.height),
        ];
        Self::map_quad(src_pts, dst)
    }

    /// Direct linear transform of four source points onto four destination points.
    ///
    /// Order is TL, TR, BR, BL. The plane must stay finite (`w ≠ 0` at each source).
    pub fn map_quad(src: [Point; 4], dst: [Point; 4]) -> Option<Homography> {
        if !quad_is_usable(src) || !quad_is_usable(dst) {
            return None;
        }
        // Ah = b with h = [h00 h01 h02 h10 h11 h12 h20 h21], h22 = 1.
        let mut a = [[0.0; 8]; 8];
        let mut b = [0.0; 8];
        for i in 0..4 {
            let Point { x, y } = src[i];
            let Point { x: u, y: v } = dst[i];
            let r = i * 2;
            a[r] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y];
            b[r] = u;
            a[r + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y];
            b[r + 1] = v;
        }
        let h = solve8(a, b)?;
        let matrix = [h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7], 1.0];
        if !matrix.iter().all(|value| value.is_finite()) {
            return None;
        }
        let homo = Homography(matrix);
        for i in 0..4 {
            let mapped = homo.try_apply(src[i])?;
            if (mapped.x - dst[i].x).abs() > 1e-6 || (mapped.y - dst[i].y).abs() > 1e-6 {
                return None;
            }
        }
        Some(homo)
    }
}

fn quad_is_usable(pts: [Point; 4]) -> bool {
    if pts.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return false;
    }
    // Two triangles of the quad must keep a consistent, non-zero orientation.
    let area = |a: Point, b: Point, c: Point| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let a0 = area(pts[0], pts[1], pts[2]);
    let a1 = area(pts[0], pts[2], pts[3]);
    a0.is_finite()
        && a1.is_finite()
        && a0.abs() > 1e-8
        && a1.abs() > 1e-8
        && a0.signum() == a1.signum()
}

fn solve8(mut a: [[f64; 8]; 8], mut b: [f64; 8]) -> Option<[f64; 8]> {
    for col in 0..8 {
        let mut pivot = col;
        let mut best = a[col][col].abs();
        for row in (col + 1)..8 {
            let value = a[row][col].abs();
            if value > best {
                best = value;
                pivot = row;
            }
        }
        if best < 1e-12 {
            return None;
        }
        if pivot != col {
            a.swap(col, pivot);
            b.swap(col, pivot);
        }
        let diag = a[col][col];
        for row in (col + 1)..8 {
            let f = a[row][col] / diag;
            if !f.is_finite() {
                return None;
            }
            for k in col..8 {
                a[row][k] -= f * a[col][k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0.0; 8];
    for i in (0..8).rev() {
        let mut acc = b[i];
        for k in (i + 1)..8 {
            acc -= a[i][k] * x[k];
        }
        x[i] = acc / a[i][i];
        if !x[i].is_finite() {
            return None;
        }
    }
    Some(x)
}

fn mul3(a: [f64; 9], b: [f64; 9]) -> [f64; 9] {
    let at = |row: usize, col: usize| a[row * 3 + col];
    let bt = |row: usize, col: usize| b[row * 3 + col];
    let mut out = [0.0; 9];
    for row in 0..3 {
        for col in 0..3 {
            out[row * 3 + col] =
                at(row, 0) * bt(0, col) + at(row, 1) * bt(1, col) + at(row, 2) * bt(2, col);
        }
    }
    out
}

impl Default for Affine {
    fn default() -> Self {
        Affine::IDENTITY
    }
}

/// Path fill rule matching CSS and SVG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// Rounded rectangle with horizontal/vertical radii ordered top-left, top-right, bottom-right,
/// bottom-left.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoundRect {
    pub rect: Rect,
    pub radii: [Point; 4],
}

impl RoundRect {
    /// Construct the common circular-corner form.
    pub fn new(rect: Rect, radii: [f64; 4]) -> Self {
        RoundRect {
            rect,
            radii: radii.map(|radius| Point::new(radius, radius)),
        }
    }

    pub fn elliptical(rect: Rect, radii: [Point; 4]) -> Self {
        RoundRect { rect, radii }
    }

    pub fn sharp(rect: Rect) -> Self {
        RoundRect {
            rect,
            radii: [Point::default(); 4],
        }
    }
}

/// CSS blend modes plus DestinationIn for internal mask sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    DestinationIn,
}

/// Mask channel selection matching CSS mask-mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum MaskMode {
    #[default]
    Alpha,
    Luminance,
}

/// Image or gradient mask source, represented as a value rather than a subtree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum MaskSource {
    Image { image: ImageId },
    Paint { paint: Paint },
}

/// One operation in an ordered CSS filter chain. Brightness, contrast, and saturation are neutral
/// at 1; bounded effects use 0..1; blur stores Gaussian sigma.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum FilterOp {
    Blur {
        sigma: f64,
    },
    Brightness {
        amount: f64,
    },
    Contrast {
        amount: f64,
    },
    Grayscale {
        amount: f64,
    },
    HueRotate {
        degrees: f64,
    },
    Invert {
        amount: f64,
    },
    Opacity {
        amount: f64,
    },
    Saturate {
        amount: f64,
    },
    Sepia {
        amount: f64,
    },
    /// Drop shadow of a painted subtree's alpha contour, distinct from a box shadow based on
    /// rectangular geometry.
    DropShadow {
        dx: f64,
        dy: f64,
        sigma: f64,
        color: Rgba,
    },
    /// Node-local displacement driven by deterministic Perlin noise.
    NoiseDisplacement {
        frequency_x: f64,
        frequency_y: f64,
        octaves: u8,
        seed: u32,
        scale: f64,
        turbulence: bool,
    },
    /// Node-local directional blur using velocity and shutter parameters.
    VelocityBlur {
        velocity_x: f64,
        velocity_y: f64,
        shutter_angle: f64,
    },
}

// Paint types.

/// Gradient repetition and behavior outside the stop range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum SpreadMode {
    #[default]
    Pad,
    Repeat,
    Reflect,
}

/// Linear gradient referencing the recording's stop table.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinearGradient {
    pub start: Point,
    pub end: Point,
    pub stops: Span,
    pub spread: SpreadMode,
    /// Overall opacity multiplier that leaves shared stop data unchanged.
    pub alpha: f64,
}

/// Radial gradient with independent horizontal and vertical radii.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RadialGradient {
    pub center: Point,
    pub radii: Point,
    pub stops: Span,
    pub spread: SpreadMode,
    pub alpha: f64,
}

/// Conic gradient with clockwise degrees from the positive x axis. Translate CSS's top-origin angle
/// by -90 degrees before recording.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConicGradient {
    pub center: Point,
    pub start_angle: f64,
    /// Angular extent of one gradient tile in degrees. Repeating CSS conics use a value below 360.
    pub sweep_angle: f64,
    pub stops: Span,
    pub spread: SpreadMode,
    pub alpha: f64,
}

/// Recording paint supporting solid, linear, radial, and conic forms. Convert leaf paints through
/// from_leaf.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "value",
    deny_unknown_fields
)]
pub enum Paint {
    Solid(Rgba),
    Linear(LinearGradient),
    Radial(RadialGradient),
    Conic(ConicGradient),
}

impl Paint {
    pub fn solid(c: Rgba) -> Self {
        Paint::Solid(c)
    }

    /// Convert a leaf paint, adjusting its stop span when the caller merges gradient tables.
    pub fn from_leaf(p: crate::draw::Paint, stop_offset: u32) -> Self {
        match p {
            crate::draw::Paint::Solid(c) => Paint::Solid(c),
            crate::draw::Paint::Linear(g) => Paint::Linear(LinearGradient {
                start: g.start,
                end: g.end,
                stops: Span {
                    start: g.stops.start + stop_offset,
                    end: g.stops.end + stop_offset,
                },
                spread: SpreadMode::Pad,
                alpha: g.alpha,
            }),
        }
    }

    /// Gradient stop span for validation; solid paints return None.
    pub fn stops(&self) -> Option<Span> {
        match self {
            Paint::Solid(_) => None,
            Paint::Linear(g) => Some(g.stops),
            Paint::Radial(g) => Some(g.stops),
            Paint::Conic(g) => Some(g.stops),
        }
    }
}

/// Stroke using the recording's complete paint vocabulary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Stroke {
    pub paint: Paint,
    pub width: f64,
    /// Alternating dash lengths in pixels; None is solid.
    pub dash: Option<Vec<f64>>,
    /// Phase applied to the dash interval pattern, in path units.
    pub dash_offset: f64,
    pub cap: Cap,
    pub join: Join,
    pub miter_limit: f64,
}

impl Stroke {
    pub fn solid(color: Rgba, width: f64) -> Self {
        Stroke {
            paint: Paint::Solid(color),
            width,
            dash: None,
            dash_offset: 0.0,
            cap: Cap::Butt,
            join: Join::Miter,
            // CSS/SVG default miter limit.
            miter_limit: 4.0,
        }
    }
}

// Side-table entries.

/// Positioned glyph with a face-local glyph index and final baseline origin. Layout is complete
/// before recording.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Glyph {
    pub id: u32,
    pub x: f64,
    pub y: f64,
}

/// Font face resolved through the backend's font registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontFace {
    pub family: String,
    /// CSS numeric font weight; 400 is normal and 700 is bold.
    pub weight: u16,
    pub italic: bool,
    /// Font size in pixels for raster density and hinting; glyph positions are already resolved.
    pub size: f64,
}

/// CSS object-fit modes; Fill is the initial value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ObjectFit {
    #[default]
    Fill,
    Contain,
    Cover,
    ScaleDown,
    None,
}

impl ObjectFit {
    pub fn is_fill(&self) -> bool {
        matches!(self, ObjectFit::Fill)
    }

    /// Fit intrinsic dimensions into the destination box, centered as with object-position: 50%
    /// 50%. Fill and degenerate inputs return the original destination.
    pub fn place(self, dst: Rect, iw: f64, ih: f64) -> Rect {
        if iw <= 0.0 || ih <= 0.0 || dst.width <= 0.0 || dst.height <= 0.0 {
            return dst;
        }
        let (w, h) = match self {
            ObjectFit::Fill => return dst,
            ObjectFit::None => (iw, ih),
            ObjectFit::Contain | ObjectFit::ScaleDown | ObjectFit::Cover => {
                let sx = dst.width / iw;
                let sy = dst.height / ih;
                let scale = match self {
                    ObjectFit::Cover => sx.max(sy),
                    // Scale-down selects the smaller of intrinsic size and contain scaling.
                    ObjectFit::ScaleDown => sx.min(sy).min(1.0),
                    _ => sx.min(sy),
                };
                (iw * scale, ih * scale)
            }
        };
        Rect {
            x: dst.x + (dst.width - w) / 2.0,
            y: dst.y + (dst.height - h) / 2.0,
            width: w,
            height: h,
        }
    }
}

/// Content-addressed image reference without pixel data. Resources are resolved before rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageSource {
    pub asset: String,
    pub width: u32,
    pub height: u32,
    /// Optional video sample time in seconds. None denotes a static image. Video frames share the
    /// image rendering path for clipping, sampling, and alpha.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_time_s: Option<f64>,
}

/// A host-generated raster surface. The wire carries only an opaque provider key and the exact
/// pixel dimensions requested by the producer; pixels and domain-specific inputs never enter the
/// ProgramRecording JSON. Providers must resolve the key before command execution starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeneratedTextureSource {
    pub provider_key: String,
    pub width: u32,
    pub height: u32,
}

/// One generated texture as placed by the ProgramRecording transform / perspective stack.
///
/// Locate and Scene3D pick invert [`Self::canvas_from_local`] — the same 3×3 the
/// rasterizer concatenates — so a `rotateX`/`rotateY` ancestor cannot desync hit
/// testing from the painted quad.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedTexturePlacement {
    pub provider_key: String,
    pub dst: Rect,
    pub canvas_from_local: Homography,
}

macro_rules! id_newtype {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name(pub u32);

        impl $name {
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

id_newtype!(
    /// Index into ProgramRecording::fonts.
    FontId
);
id_newtype!(
    /// Index into ProgramRecording::images.
    ImageId
);
id_newtype!(
    /// Index into ProgramRecording::generated_textures.
    GeneratedTextureId
);
id_newtype!(
    /// Index into ProgramRecording::text_sources.
    TextSourceId
);
id_newtype!(
    /// Index into ProgramRecording::shader_programs.
    ShaderProgramId
);

/// A locked shader package reference. Source code deliberately stays out of the display wire:
/// executors must resolve this pin through the admitted package registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShaderProgram {
    pub uri: String,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub content_hash: DigestBytes,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub abi_hash: DigestBytes,
}

/// One frame-evaluated, typed shader uniform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum ShaderUniformValue {
    Float { value: f32 },
    Float2 { value: [f32; 2] },
    Color { value: [f32; 4] },
    Bool { value: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShaderUniformBinding {
    pub name: String,
    pub value: ShaderUniformValue,
}

/// One declared texture input, in manifest ABI order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShaderTextureInput {
    pub name: String,
    pub image: ImageId,
}

/// Glyph-run source location anchored to a Scene node and byte offsets in that node's own text,
/// independent of sibling text concatenation during shaping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlyphSource {
    /// Scene text node referenced through the text-source table.
    pub node: TextSourceId,
    /// One source byte range per glyph, relative to the node's own text.
    pub ranges: Span,
}

/// Primitive used by one batched geometry command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum BatchGeometry {
    Circle,
    Rect,
}

/// One instance in [`RecordCmd::GeometryBatch`]. Positions are local to the owning Scene node.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchInstance {
    pub position: Point,
    pub size: Point,
    pub color: Rgba,
}

// Recording commands.

/// One recording command. Begin commands and End must balance; all other commands are leaves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "op",
    deny_unknown_fields
)]
pub enum RecordCmd {
    // Grouping and transform state.
    /// Logical group without an offscreen surface, used for hierarchy and location.
    BeginGroup,
    BeginTransform {
        transform: Affine,
    },
    /// Projected layer transform with concatenation semantics matching BeginTransform.
    BeginPerspective {
        matrix: Homography,
    },
    /// Apply opacity to the composited group, not independently to overlapping leaves.
    BeginOpacity {
        alpha: f64,
    },
    BeginClipRect {
        rect: Rect,
    },
    BeginClipRoundRect {
        rrect: RoundRect,
    },
    BeginClipPath {
        path: PathRef,
        fill_rule: FillRule,
    },

    // Offscreen groups.
    /// Explicit offscreen layer with optional clip bounds; None delegates bounds to the executor.
    BeginSaveLayer {
        bounds: Option<Rect>,
        alpha: f64,
    },
    /// Blend this layer with its backdrop using the selected CSS blend mode.
    BeginBlend {
        mode: BlendMode,
    },
    /// Mask this layer, placing the mask source in its resolved destination rectangle.
    BeginMask {
        source: MaskSource,
        mode: MaskMode,
        rect: Rect,
    },
    /// Apply an ordered filter chain to this layer's content.
    BeginFilter {
        filters: Span,
        /// Offscreen content bounds including filter outsets. These bounds also clip pixels.
        /// Backfill them through the shared calculation instead of allocating a full-canvas surface
        /// or inferring them separately in each backend.
        bounds: Option<Rect>,
    },
    /// Apply filters to the layer's backdrop.
    BeginBackdropFilter {
        filters: Span,
        /// Backdrop sampling bounds; None uses this layer's clip bounds.
        bounds: Option<Rect>,
    },
    /// Apply one admitted local shader to the complete nested subtree. `bounds` is the explicit
    /// node-local border box; executors must not substitute the canvas or current clip.
    BeginShaderLayer {
        program: ShaderProgramId,
        uniforms: Span,
        inputs: Span,
        bounds: Rect,
    },
    /// Paint one validated Motion Glass material from the Current destination, then paint the
    /// nested commands as its real foreground in author order. This construction-only recording
    /// carries the typed program; the outer DrawProgram is the only serialized execution packet.
    BeginMotionGlass {
        program: Box<super::MotionGlassProgram>,
    },
    /// Clip the nested ordinary subtree to one materialized member shape. `tone` and
    /// `protection` remain digest-pinned metadata for the owner kernel; the command never carries
    /// a synthetic foreground primitive.
    BeginMotionGlassForeground {
        program: Box<super::MotionGlassForegroundProgram>,
    },
    /// Close one group; validation rejects missing or extra ends.
    End,

    // Leaf commands.
    /// Fill and/or stroke a path, applying fill before stroke.
    Path {
        path: PathRef,
        fill_rule: FillRule,
        fill: Option<Paint>,
        stroke: Option<Stroke>,
    },
    /// Many homogeneous primitives in one command. The side table keeps command and Scene
    /// topology constant while particle/data instances change from frame to frame.
    GeometryBatch {
        geometry: BatchGeometry,
        instances: Span,
    },
    /// A run of positioned glyphs.
    GlyphRun {
        font: FontId,
        glyphs: Span,
        paint: Paint,
        /// Optional centered glyph stroke. Apply fill before stroke within the same command to
        /// preserve CSS paint order across backends.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<Stroke>,
        /// Optional source node identity and per-glyph byte ranges. None indicates unavailable
        /// source addressing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<GlyphSource>,
    },
    Image {
        image: ImageId,
        /// Destination rectangle in canvas coordinates.
        dst: Rect,
        /// Source rectangle in image pixels; None selects the full image.
        src: Option<Rect>,
        alpha: f64,
        /// The executor applies object-fit using decoded intrinsic dimensions, which the layout and
        /// emission stages do not know.
        #[serde(default, skip_serializing_if = "ObjectFit::is_fill")]
        fit: ObjectFit,
    },
    /// Draw one already-generated premultiplied RGBA texture. Unlike [`Self::Image`], generated
    /// textures have no intrinsic-size/object-fit semantics: source and destination dimensions
    /// are exact and the provider is responsible for producing those pixels before execution.
    Texture {
        texture: GeneratedTextureId,
        dst: Rect,
        alpha: f64,
    },
    /// Box shadow based on rectangular geometry, with optional inset placement.
    Shadow {
        rrect: RoundRect,
        dx: f64,
        dy: f64,
        /// Gaussian sigma equals half the CSS shadow blur radius.
        blur_sigma: f64,
        spread: f64,
        color: Rgba,
        inset: bool,
    },
}

impl RecordCmd {
    /// Group-depth change: Begin +1, End -1, leaves 0.
    pub fn depth_delta(&self) -> i32 {
        match self {
            RecordCmd::BeginGroup
            | RecordCmd::BeginTransform { .. }
            | RecordCmd::BeginPerspective { .. }
            | RecordCmd::BeginOpacity { .. }
            | RecordCmd::BeginClipRect { .. }
            | RecordCmd::BeginClipRoundRect { .. }
            | RecordCmd::BeginClipPath { .. }
            | RecordCmd::BeginSaveLayer { .. }
            | RecordCmd::BeginBlend { .. }
            | RecordCmd::BeginMask { .. }
            | RecordCmd::BeginFilter { .. }
            | RecordCmd::BeginBackdropFilter { .. }
            | RecordCmd::BeginShaderLayer { .. }
            | RecordCmd::BeginMotionGlass { .. }
            | RecordCmd::BeginMotionGlassForeground { .. } => 1,
            RecordCmd::End => -1,
            _ => 0,
        }
    }
}

// Recording storage.

/// Recording commands and side tables for one component subtree in one frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramRecording {
    pub cmds: Vec<RecordCmd>,
    pub verbs: Vec<PathVerb>,
    pub points: Vec<Point>,
    pub gradient_stops: Vec<GradientStop>,
    pub filters: Vec<FilterOp>,
    pub glyphs: Vec<Glyph>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub batch_instances: Vec<BatchInstance>,
    /// Per-glyph byte ranges grouped by GlyphSource::ranges.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glyph_source_ranges: Vec<Span>,
    /// Scene text-node keys, matching artifact node identities for source lookup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_sources: Vec<String>,
    pub fonts: Vec<FontFace>,
    pub images: Vec<ImageSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub generated_textures: Vec<GeneratedTextureSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shader_programs: Vec<ShaderProgram>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shader_uniforms: Vec<ShaderUniformBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shader_inputs: Vec<ShaderTextureInput>,
}

impl Default for ProgramRecording {
    fn default() -> Self {
        ProgramRecording {
            cmds: Vec::new(),
            verbs: Vec::new(),
            points: Vec::new(),
            gradient_stops: Vec::new(),
            filters: Vec::new(),
            glyphs: Vec::new(),
            batch_instances: Vec::new(),
            glyph_source_ranges: Vec::new(),
            text_sources: Vec::new(),
            fonts: Vec::new(),
            images: Vec::new(),
            generated_textures: Vec::new(),
            shader_programs: Vec::new(),
            shader_uniforms: Vec::new(),
            shader_inputs: Vec::new(),
        }
    }
}

impl PathSink for ProgramRecording {
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

/// Structural recording error detected before compilation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingError {
    /// End encountered when group depth is already zero.
    UnbalancedEnd {
        at: usize,
    },
    /// Groups remain open at the end of the recording.
    UnclosedGroups {
        depth: i32,
    },
    /// Side-table reference outside its valid range.
    OutOfRange {
        at: usize,
        table: &'static str,
        index: u32,
        len: usize,
    },
    /// Path point count does not match the verb arity.
    PathArity {
        at: usize,
        expected: usize,
        found: usize,
    },
    /// Glyph and source-range spans have different lengths, preventing reliable per-glyph source
    /// lookup.
    GlyphSourceArity {
        at: usize,
        glyphs: u32,
        source_ranges: u32,
    },
    /// Reject NaN and infinities before JSON serialization can silently convert them to null.
    NonFinite {
        at: usize,
        table: &'static str,
    },
    TooManyBatchInstances {
        found: usize,
        max: usize,
    },
    InvalidShaderLayer {
        at: usize,
        reason: &'static str,
    },
    InvalidGeneratedTexture {
        at: usize,
        reason: &'static str,
    },
    InvalidTextureCommand {
        at: usize,
        reason: &'static str,
    },
    InvalidMotionGlass {
        at: usize,
    },
    InvalidMotionGlassForeground {
        at: usize,
    },
}

impl core::fmt::Display for RecordingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RecordingError::UnbalancedEnd { at } => write!(f, "cmd {at}: End without Begin"),
            RecordingError::UnclosedGroups { depth } => write!(f, "{depth} group(s) left unclosed"),
            RecordingError::OutOfRange {
                at,
                table,
                index,
                len,
            } => write!(f, "cmd {at}: {table}[{index}] out of range (len {len})"),
            RecordingError::PathArity {
                at,
                expected,
                found,
            } => write!(
                f,
                "cmd {at}: path needs {expected} points, span has {found}"
            ),
            RecordingError::GlyphSourceArity {
                at,
                glyphs,
                source_ranges,
            } => write!(
                f,
                "cmd {at}: {glyphs} glyph(s) but {source_ranges} source range(s)"
            ),
            RecordingError::NonFinite { at, table } => {
                write!(f, "{table}[{at}]: NaN/infinity is not representable")
            }
            RecordingError::TooManyBatchInstances { found, max } => {
                write!(f, "batchInstances has {found} entries (max {max})")
            }
            RecordingError::InvalidShaderLayer { at, reason } => {
                write!(f, "cmd {at}: invalid ShaderLayer: {reason}")
            }
            RecordingError::InvalidGeneratedTexture { at, reason } => {
                write!(f, "generatedTextures[{at}]: {reason}")
            }
            RecordingError::InvalidTextureCommand { at, reason } => {
                write!(f, "cmd {at}: invalid generated texture draw: {reason}")
            }
            RecordingError::InvalidMotionGlass { at } => {
                write!(f, "cmd {at}: invalid MotionGlassProgram")
            }
            RecordingError::InvalidMotionGlassForeground { at } => {
                write!(f, "cmd {at}: invalid MotionGlassForegroundProgram")
            }
        }
    }
}

impl std::error::Error for RecordingError {}

/// Deterministic recording snapshot failure; structural validation precedes serialization.
#[derive(Debug)]
pub enum CanonicalRecordingError {
    Invalid(RecordingError),
    Serialize(serde_json::Error),
}

impl core::fmt::Display for CanonicalRecordingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CanonicalRecordingError::Invalid(error) => write!(f, "invalid display list: {error}"),
            CanonicalRecordingError::Serialize(error) => {
                write!(f, "display list serialization failed: {error}")
            }
        }
    }
}

impl std::error::Error for CanonicalRecordingError {}

/// Finite-value checks for all floating-point wire fields.
mod finite {
    use super::*;

    pub fn point(p: &Point) -> bool {
        p.x.is_finite() && p.y.is_finite()
    }

    pub fn rect(r: &Rect) -> bool {
        r.x.is_finite() && r.y.is_finite() && r.width.is_finite() && r.height.is_finite()
    }

    pub fn rrect(r: &RoundRect) -> bool {
        rect(&r.rect) && r.radii.iter().all(|v| point(v))
    }

    pub fn paint(p: &Paint) -> bool {
        match p {
            Paint::Solid(_) => true,
            Paint::Linear(g) => point(&g.start) && point(&g.end) && g.alpha.is_finite(),
            Paint::Radial(g) => {
                point(&g.center)
                    && point(&g.radii)
                    && g.radii.x > 0.0
                    && g.radii.y > 0.0
                    && g.alpha.is_finite()
            }
            Paint::Conic(g) => {
                point(&g.center)
                    && g.start_angle.is_finite()
                    && g.sweep_angle.is_finite()
                    && g.sweep_angle > 0.0
                    && g.sweep_angle <= 360.0
                    && g.alpha.is_finite()
            }
        }
    }

    pub fn stroke(s: &Stroke) -> bool {
        paint(&s.paint)
            && s.width.is_finite()
            && s.miter_limit.is_finite()
            && s.dash_offset.is_finite()
            && s.dash
                .as_ref()
                .is_none_or(|d| d.iter().all(|v| v.is_finite()))
    }

    pub fn filter(op: &FilterOp) -> bool {
        match op {
            FilterOp::Blur { sigma } => sigma.is_finite(),
            FilterOp::Brightness { amount }
            | FilterOp::Contrast { amount }
            | FilterOp::Grayscale { amount }
            | FilterOp::Invert { amount }
            | FilterOp::Opacity { amount }
            | FilterOp::Saturate { amount }
            | FilterOp::Sepia { amount } => amount.is_finite(),
            FilterOp::HueRotate { degrees } => degrees.is_finite(),
            FilterOp::DropShadow { dx, dy, sigma, .. } => {
                dx.is_finite() && dy.is_finite() && sigma.is_finite()
            }
            FilterOp::NoiseDisplacement {
                frequency_x,
                frequency_y,
                octaves,
                scale,
                ..
            } => {
                frequency_x.is_finite()
                    && frequency_y.is_finite()
                    && (0.000_001..=4.0).contains(frequency_x)
                    && (0.000_001..=4.0).contains(frequency_y)
                    && (1..=4).contains(octaves)
                    && scale.is_finite()
                    && scale.abs() <= 512.0
            }
            FilterOp::VelocityBlur {
                velocity_x,
                velocity_y,
                shutter_angle,
            } => {
                velocity_x.is_finite()
                    && velocity_y.is_finite()
                    && shutter_angle.is_finite()
                    && velocity_x.abs() <= 2048.0
                    && velocity_y.abs() <= 2048.0
                    && (0.0..=360.0).contains(shutter_angle)
            }
        }
    }

    pub fn cmd(c: &RecordCmd) -> bool {
        match c {
            RecordCmd::BeginTransform { transform } => transform.0.iter().all(|v| v.is_finite()),
            RecordCmd::BeginPerspective { matrix } => matrix.0.iter().all(|v| v.is_finite()),
            RecordCmd::BeginOpacity { alpha } => alpha.is_finite(),
            RecordCmd::BeginClipRect { rect: r } => rect(r),
            RecordCmd::BeginClipRoundRect { rrect: r } => rrect(r),
            RecordCmd::BeginSaveLayer { bounds, alpha } => {
                bounds.as_ref().is_none_or(rect) && alpha.is_finite()
            }
            RecordCmd::BeginMask {
                source, rect: r, ..
            } => {
                rect(r)
                    && match source {
                        MaskSource::Image { .. } => true,
                        MaskSource::Paint { paint: p } => paint(p),
                    }
            }
            RecordCmd::BeginBackdropFilter { bounds, .. } => bounds.as_ref().is_none_or(rect),
            RecordCmd::BeginShaderLayer { bounds, .. } => rect(bounds),
            RecordCmd::BeginMotionGlass { .. } | RecordCmd::BeginMotionGlassForeground { .. } => {
                true
            }
            RecordCmd::Path {
                fill, stroke: s, ..
            } => fill.as_ref().is_none_or(paint) && s.as_ref().is_none_or(stroke),
            RecordCmd::GeometryBatch { .. } => true,
            // Validate glyph strokes just like path strokes before serialization.
            RecordCmd::GlyphRun {
                paint: p,
                stroke: s,
                ..
            } => paint(p) && s.as_ref().is_none_or(stroke),
            RecordCmd::Image {
                dst, src, alpha, ..
            } => rect(dst) && src.as_ref().is_none_or(rect) && alpha.is_finite(),
            RecordCmd::Texture { dst, alpha, .. } => rect(dst) && alpha.is_finite(),
            RecordCmd::Shadow {
                rrect: r,
                dx,
                dy,
                blur_sigma,
                spread,
                ..
            } => {
                rrect(r)
                    && dx.is_finite()
                    && dy.is_finite()
                    && blur_sigma.is_finite()
                    && spread.is_finite()
            }
            RecordCmd::BeginGroup
            | RecordCmd::BeginClipPath { .. }
            | RecordCmd::BeginBlend { .. }
            | RecordCmd::BeginFilter { .. }
            | RecordCmd::End => true,
        }
    }
}

impl ProgramRecording {
    pub fn new() -> Self {
        ProgramRecording::default()
    }

    /// Clear all frame data while retaining capacity.
    pub fn clear(&mut self) {
        self.cmds.clear();
        self.verbs.clear();
        self.points.clear();
        self.gradient_stops.clear();
        self.filters.clear();
        self.glyphs.clear();
        self.batch_instances.clear();
        self.glyph_source_ranges.clear();
        self.text_sources.clear();
        self.fonts.clear();
        self.images.clear();
        self.generated_textures.clear();
        self.shader_programs.clear();
        self.shader_uniforms.clear();
        self.shader_inputs.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    pub fn push(&mut self, cmd: RecordCmd) {
        self.cmds.push(cmd);
    }

    pub fn begin_path(&mut self) -> PathBuilder<'_, ProgramRecording> {
        let (v0, p0) = (self.verb_len(), self.point_len());
        PathBuilder::new(self, v0, p0)
    }

    pub fn path_data(&self, p: PathRef) -> (&[PathVerb], &[Point]) {
        (&self.verbs[p.verbs.range()], &self.points[p.points.range()])
    }

    /// Store gradient stops in the shared side table and return their span.
    pub fn intern_stops(&mut self, stops: &[(f64, Rgba)]) -> Span {
        let start = self.gradient_stops.len() as u32;
        self.gradient_stops.extend(
            stops
                .iter()
                .map(|&(offset, color)| GradientStop { offset, color }),
        );
        Span {
            start,
            end: self.gradient_stops.len() as u32,
        }
    }

    pub fn gradient_stops_of(&self, s: Span) -> &[GradientStop] {
        &self.gradient_stops[s.range()]
    }

    /// Store an ordered filter chain.
    pub fn intern_filters(&mut self, ops: &[FilterOp]) -> Span {
        let start = self.filters.len() as u32;
        self.filters.extend_from_slice(ops);
        Span {
            start,
            end: self.filters.len() as u32,
        }
    }

    /// Store a run of positioned glyphs.
    pub fn intern_glyphs(&mut self, glyphs: &[Glyph]) -> Span {
        let start = self.glyphs.len() as u32;
        self.glyphs.extend_from_slice(glyphs);
        Span {
            start,
            end: self.glyphs.len() as u32,
        }
    }

    /// Register a text-source key, reusing an existing entry when present.
    pub fn intern_text_source(&mut self, key: &str) -> TextSourceId {
        if let Some(i) = self.text_sources.iter().position(|k| k == key) {
            return TextSourceId(i as u32);
        }
        self.text_sources.push(key.to_owned());
        TextSourceId(self.text_sources.len() as u32 - 1)
    }

    /// Store one node-relative source byte range per glyph.
    pub fn intern_glyph_source_ranges(&mut self, ranges: &[Span]) -> Span {
        let start = self.glyph_source_ranges.len() as u32;
        self.glyph_source_ranges.extend_from_slice(ranges);
        Span {
            start,
            end: self.glyph_source_ranges.len() as u32,
        }
    }

    /// Register a font face, reusing equal entries.
    pub fn intern_font(&mut self, face: FontFace) -> FontId {
        if let Some(i) = self.fonts.iter().position(|f| *f == face) {
            return FontId(i as u32);
        }
        self.fonts.push(face);
        FontId(self.fonts.len() as u32 - 1)
    }

    /// Register an image reference, reusing equal entries.
    pub fn intern_image(&mut self, src: ImageSource) -> ImageId {
        if let Some(i) = self.images.iter().position(|s| *s == src) {
            return ImageId(i as u32);
        }
        self.images.push(src);
        ImageId(self.images.len() as u32 - 1)
    }

    /// Register one generated texture request, deduplicated by provider identity and size.
    pub fn intern_generated_texture(
        &mut self,
        source: GeneratedTextureSource,
    ) -> GeneratedTextureId {
        if let Some(index) = self
            .generated_textures
            .iter()
            .position(|item| *item == source)
        {
            return GeneratedTextureId(index as u32);
        }
        self.generated_textures.push(source);
        GeneratedTextureId(self.generated_textures.len() as u32 - 1)
    }

    /// Replay the transform stack (affine + 3×3 perspective) and record every
    /// generated texture's canvas-from-local homography.
    pub fn generated_texture_placements(
        &self,
    ) -> Result<Vec<GeneratedTexturePlacement>, RecordingError> {
        let mut stack = vec![Homography::IDENTITY];
        let mut placements = Vec::new();
        for (at, command) in self.cmds.iter().enumerate() {
            match command {
                RecordCmd::BeginTransform { transform } => {
                    let parent = *stack.last().expect("root transform exists");
                    stack.push(Homography::from_affine(*transform).then(parent));
                }
                RecordCmd::BeginPerspective { matrix } => {
                    let parent = *stack.last().expect("root transform exists");
                    stack.push(matrix.then(parent));
                }
                RecordCmd::End => {
                    if stack.len() == 1 {
                        return Err(RecordingError::UnbalancedEnd { at });
                    }
                    stack.pop();
                }
                RecordCmd::Texture { texture, dst, .. } => {
                    let source = self.generated_textures.get(texture.index()).ok_or(
                        RecordingError::OutOfRange {
                            at,
                            table: "generatedTextures",
                            index: texture.0,
                            len: self.generated_textures.len(),
                        },
                    )?;
                    placements.push(GeneratedTexturePlacement {
                        provider_key: source.provider_key.clone(),
                        dst: *dst,
                        canvas_from_local: *stack.last().expect("root transform exists"),
                    });
                }
                other if other.depth_delta() == 1 => {
                    stack.push(*stack.last().expect("root transform exists"));
                }
                _ => {}
            }
        }
        if stack.len() != 1 {
            return Err(RecordingError::UnclosedGroups {
                depth: (stack.len() as i32) - 1,
            });
        }
        Ok(placements)
    }

    pub fn intern_shader_program(&mut self, program: ShaderProgram) -> ShaderProgramId {
        if let Some(i) = self
            .shader_programs
            .iter()
            .position(|item| *item == program)
        {
            return ShaderProgramId(i as u32);
        }
        self.shader_programs.push(program);
        ShaderProgramId(self.shader_programs.len() as u32 - 1)
    }

    pub fn intern_shader_uniforms(&mut self, values: &[ShaderUniformBinding]) -> Span {
        let start = self.shader_uniforms.len() as u32;
        self.shader_uniforms.extend_from_slice(values);
        Span {
            start,
            end: self.shader_uniforms.len() as u32,
        }
    }

    pub fn intern_shader_inputs(&mut self, values: &[ShaderTextureInput]) -> Span {
        let start = self.shader_inputs.len() as u32;
        self.shader_inputs.extend_from_slice(values);
        Span {
            start,
            end: self.shader_inputs.len() as u32,
        }
    }

    /// Append one contiguous batch and return its stable side-table span.
    pub fn intern_batch_instances(&mut self, instances: &[BatchInstance]) -> Span {
        let start = self.batch_instances.len() as u32;
        self.batch_instances.extend_from_slice(instances);
        Span {
            start,
            end: self.batch_instances.len() as u32,
        }
    }

    /// Append leaf gradient stops and return their offset for paint conversion.
    pub fn push_draw_gradients(&mut self, stops: &[GradientStop]) -> u32 {
        let offset = self.gradient_stops.len() as u32;
        self.gradient_stops.extend_from_slice(stops);
        offset
    }

    /// Validate group balance, references, path arity, and finite values before compilation.
    pub fn validate(&self) -> Result<(), RecordingError> {
        let mut depth: i32 = 0;
        for (at, cmd) in self.cmds.iter().enumerate() {
            depth += cmd.depth_delta();
            if depth < 0 {
                return Err(RecordingError::UnbalancedEnd { at });
            }
            self.check_cmd(at, cmd)?;
            if !finite::cmd(cmd) {
                return Err(RecordingError::NonFinite { at, table: "cmds" });
            }
        }
        if depth != 0 {
            return Err(RecordingError::UnclosedGroups { depth });
        }
        self.check_finite_tables()
    }

    /// Deterministic compact JSON snapshot for tests and diagnostics. Preserve declaration order
    /// and exact f64 round trips; execution consumes only the compiled packed DrawProgram.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, CanonicalRecordingError> {
        self.validate().map_err(CanonicalRecordingError::Invalid)?;
        serde_json::to_vec(self).map_err(CanonicalRecordingError::Serialize)
    }

    /// Check floating-point values in side tables; command fields are checked separately.
    fn check_finite_tables(&self) -> Result<(), RecordingError> {
        if self.batch_instances.len() > MAX_BATCH_INSTANCES_PER_RECORDING {
            return Err(RecordingError::TooManyBatchInstances {
                found: self.batch_instances.len(),
                max: MAX_BATCH_INSTANCES_PER_RECORDING,
            });
        }
        let bad = |at: usize, table: &'static str| RecordingError::NonFinite { at, table };
        if let Some(at) = self.points.iter().position(|p| !finite::point(p)) {
            return Err(bad(at, "points"));
        }
        if let Some(at) = self
            .gradient_stops
            .iter()
            .position(|s| !s.offset.is_finite())
        {
            return Err(bad(at, "gradientStops"));
        }
        if let Some(at) = self.filters.iter().position(|f| !finite::filter(f)) {
            return Err(bad(at, "filters"));
        }
        if let Some(at) = self
            .glyphs
            .iter()
            .position(|g| !(g.x.is_finite() && g.y.is_finite()))
        {
            return Err(bad(at, "glyphs"));
        }
        if let Some(at) = self.batch_instances.iter().position(|instance| {
            !finite::point(&instance.position)
                || !finite::point(&instance.size)
                || instance.size.x <= 0.0
                || instance.size.y <= 0.0
        }) {
            return Err(bad(at, "batchInstances"));
        }
        if let Some(at) = self.fonts.iter().position(|f| !f.size.is_finite()) {
            return Err(bad(at, "fonts"));
        }
        if let Some(at) = self
            .shader_uniforms
            .iter()
            .position(|binding| match &binding.value {
                ShaderUniformValue::Float { value } => !value.is_finite(),
                ShaderUniformValue::Float2 { value } => value.iter().any(|v| !v.is_finite()),
                ShaderUniformValue::Color { value } => value.iter().any(|v| !v.is_finite()),
                ShaderUniformValue::Bool { .. } => false,
            })
        {
            return Err(bad(at, "shaderUniforms"));
        }
        for (at, program) in self.shader_programs.iter().enumerate() {
            if program.uri.is_empty() {
                return Err(RecordingError::InvalidShaderLayer {
                    at,
                    reason: "program pin must contain a URI",
                });
            }
        }
        for (at, source) in self.generated_textures.iter().enumerate() {
            if source.provider_key.is_empty() {
                return Err(RecordingError::InvalidGeneratedTexture {
                    at,
                    reason: "providerKey must be non-empty",
                });
            }
            if source.width == 0
                || source.height == 0
                || source.width.checked_mul(source.height).is_none()
            {
                return Err(RecordingError::InvalidGeneratedTexture {
                    at,
                    reason: "dimensions must be positive and their pixel product must fit u32",
                });
            }
        }
        Ok(())
    }

    fn check_cmd(&self, at: usize, cmd: &RecordCmd) -> Result<(), RecordingError> {
        let span = |table: &'static str, s: Span, len: usize| -> Result<(), RecordingError> {
            if s.start > s.end || s.end as usize > len {
                return Err(RecordingError::OutOfRange {
                    at,
                    table,
                    index: s.end,
                    len,
                });
            }
            Ok(())
        };
        let paint = |p: &Paint, this: &ProgramRecording| -> Result<(), RecordingError> {
            match p.stops() {
                Some(s) => span("gradientStops", s, this.gradient_stops.len()),
                None => Ok(()),
            }
        };
        match cmd {
            RecordCmd::BeginClipPath { path, .. } => self.check_path(at, *path),
            RecordCmd::BeginFilter { filters, .. }
            | RecordCmd::BeginBackdropFilter { filters, .. } => {
                span("filters", *filters, self.filters.len())
            }
            RecordCmd::BeginShaderLayer {
                program,
                uniforms,
                inputs,
                bounds,
            } => {
                self.check_id(at, "shaderPrograms", program.0, self.shader_programs.len())?;
                span("shaderUniforms", *uniforms, self.shader_uniforms.len())?;
                span("shaderInputs", *inputs, self.shader_inputs.len())?;
                if bounds.x != 0.0 || bounds.y != 0.0 || bounds.width <= 0.0 || bounds.height <= 0.0
                {
                    return Err(RecordingError::InvalidShaderLayer {
                        at,
                        reason: "bounds must be an origin-anchored local box with positive axes",
                    });
                }
                for input in &self.shader_inputs[inputs.range()] {
                    self.check_id(at, "images", input.image.0, self.images.len())?;
                }
                Ok(())
            }
            RecordCmd::BeginMotionGlass { program } => program
                .validate()
                .map_err(|_| RecordingError::InvalidMotionGlass { at }),
            RecordCmd::BeginMotionGlassForeground { program } => program
                .validate()
                .map_err(|_| RecordingError::InvalidMotionGlassForeground { at }),
            RecordCmd::BeginMask { source, .. } => match source {
                MaskSource::Image { image } => {
                    self.check_id(at, "images", image.0, self.images.len())
                }
                MaskSource::Paint { paint: p } => paint(p, self),
            },
            RecordCmd::Path {
                path, fill, stroke, ..
            } => {
                self.check_path(at, *path)?;
                if let Some(p) = fill {
                    paint(p, self)?;
                }
                if let Some(s) = stroke {
                    paint(&s.paint, self)?;
                }
                Ok(())
            }
            RecordCmd::GeometryBatch { instances, .. } => {
                span("batchInstances", *instances, self.batch_instances.len())
            }
            RecordCmd::GlyphRun {
                font,
                glyphs,
                paint: p,
                stroke: st,
                source,
            } => {
                self.check_id(at, "fonts", font.0, self.fonts.len())?;
                span("glyphs", *glyphs, self.glyphs.len())?;
                if let Some(source) = source {
                    self.check_id(at, "textSources", source.node.0, self.text_sources.len())?;
                    span(
                        "glyphSourceRanges",
                        source.ranges,
                        self.glyph_source_ranges.len(),
                    )?;
                    // Require one source range per glyph before execution to prevent silent
                    // source-address misalignment.
                    if source.ranges.end - source.ranges.start != glyphs.end - glyphs.start {
                        return Err(RecordingError::GlyphSourceArity {
                            at,
                            glyphs: glyphs.end - glyphs.start,
                            source_ranges: source.ranges.end - source.ranges.start,
                        });
                    }
                }
                paint(p, self)?;
                match st {
                    Some(s) => paint(&s.paint, self),
                    None => Ok(()),
                }
            }
            RecordCmd::Image { image, .. } => {
                self.check_id(at, "images", image.0, self.images.len())
            }
            RecordCmd::Texture {
                texture,
                dst,
                alpha,
            } => {
                self.check_id(
                    at,
                    "generatedTextures",
                    texture.0,
                    self.generated_textures.len(),
                )?;
                if dst.width <= 0.0 || dst.height <= 0.0 {
                    return Err(RecordingError::InvalidTextureCommand {
                        at,
                        reason: "destination dimensions must be positive",
                    });
                }
                if !(0.0..=1.0).contains(alpha) {
                    return Err(RecordingError::InvalidTextureCommand {
                        at,
                        reason: "alpha must be in 0..=1",
                    });
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn check_id(
        &self,
        at: usize,
        table: &'static str,
        index: u32,
        len: usize,
    ) -> Result<(), RecordingError> {
        if index as usize >= len {
            return Err(RecordingError::OutOfRange {
                at,
                table,
                index,
                len,
            });
        }
        Ok(())
    }

    fn check_path(&self, at: usize, p: PathRef) -> Result<(), RecordingError> {
        if p.verbs.start > p.verbs.end || p.verbs.end as usize > self.verbs.len() {
            return Err(RecordingError::OutOfRange {
                at,
                table: "verbs",
                index: p.verbs.end,
                len: self.verbs.len(),
            });
        }
        if p.points.start > p.points.end || p.points.end as usize > self.points.len() {
            return Err(RecordingError::OutOfRange {
                at,
                table: "points",
                index: p.points.end,
                len: self.points.len(),
            });
        }
        let expected: usize = self.verbs[p.verbs.range()]
            .iter()
            .map(|v| v.point_count())
            .sum();
        let found = p.points.range().len();
        if expected != found {
            return Err(RecordingError::PathArity {
                at,
                expected,
                found,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affine_compose_matches_manual_application() {
        let t = Affine::translate(10.0, 0.0);
        let s = Affine::scale(2.0, 2.0);
        // Translate, then scale: (1,0) becomes (11,0), then (22,0).
        let p = t.then(s).apply(Point::new(1.0, 0.0));
        assert_eq!((p.x, p.y), (22.0, 0.0));
        // Scale, then translate: (1,0) becomes (2,0), then (12,0).
        let q = s.then(t).apply(Point::new(1.0, 0.0));
        assert_eq!((q.x, q.y), (12.0, 0.0));
        assert_eq!(
            Affine::IDENTITY.apply(Point::new(3.0, 4.0)),
            Point::new(3.0, 4.0)
        );
    }

    #[test]
    fn affine_inverse_round_trips_and_rejects_zero_scale() {
        let transform = Affine::translate(17.0, -8.0)
            .then(Affine::rotate(23.0))
            .then(Affine::scale(1.5, 0.75));
        let source = Point::new(31.0, 19.0);
        let restored = transform.inverse().unwrap().apply(transform.apply(source));
        assert!((restored.x - source.x).abs() < 1e-12);
        assert!((restored.y - source.y).abs() < 1e-12);
        assert!(Affine::scale(0.0, 1.0).inverse().is_none());
    }

    #[test]
    fn rotate_is_clockwise_on_screen_axes() {
        // With downward-positive y, a clockwise 90-degree rotation maps +x to +y.
        let p = Affine::rotate(90.0).apply(Point::new(1.0, 0.0));
        assert!(
            (p.x - 0.0).abs() < 1e-12 && (p.y - 1.0).abs() < 1e-12,
            "{p:?}"
        );
    }

    fn sample() -> ProgramRecording {
        let mut d = ProgramRecording::new();
        d.push(RecordCmd::BeginOpacity { alpha: 0.5 });
        let path = d
            .begin_path()
            .rect(Rect::new(0.0, 0.0, 10.0, 10.0))
            .finish();
        let stops = d.intern_stops(&[(0.0, Rgba::rgb(255, 0, 0)), (1.0, Rgba::rgb(0, 0, 255))]);
        d.push(RecordCmd::Path {
            path,
            fill_rule: FillRule::NonZero,
            fill: Some(Paint::Radial(RadialGradient {
                center: Point::new(5.0, 5.0),
                radii: Point::new(5.0, 5.0),
                stops,
                spread: SpreadMode::Pad,
                alpha: 1.0,
            })),
            stroke: None,
        });
        d.push(RecordCmd::End);
        d
    }

    #[test]
    fn validate_accepts_a_balanced_list() {
        assert_eq!(sample().validate(), Ok(()));
    }

    #[test]
    fn validate_catches_unbalanced_groups() {
        let mut d = sample();
        d.cmds.pop();
        assert_eq!(
            d.validate(),
            Err(RecordingError::UnclosedGroups { depth: 1 })
        );

        let mut d = sample();
        d.push(RecordCmd::End);
        assert_eq!(d.validate(), Err(RecordingError::UnbalancedEnd { at: 3 }));
    }

    #[test]
    fn validate_catches_out_of_range_side_tables() {
        let mut d = sample();
        d.push(RecordCmd::Image {
            image: ImageId(7),
            dst: Rect::new(0.0, 0.0, 1.0, 1.0),
            src: None,
            alpha: 1.0,
            fit: Default::default(),
        });
        assert!(matches!(
            d.validate(),
            Err(RecordingError::OutOfRange {
                table: "images",
                ..
            })
        ));

        let mut d = sample();
        d.push(RecordCmd::Texture {
            texture: GeneratedTextureId(7),
            dst: Rect::new(0.0, 0.0, 1.0, 1.0),
            alpha: 1.0,
        });
        assert!(matches!(
            d.validate(),
            Err(RecordingError::OutOfRange {
                table: "generatedTextures",
                ..
            })
        ));

        let mut d = sample();
        d.gradient_stops.clear();
        assert!(matches!(
            d.validate(),
            Err(RecordingError::OutOfRange {
                table: "gradientStops",
                ..
            })
        ));
    }

    #[test]
    fn validate_rejects_malformed_generated_texture_sources() {
        let mut d = sample();
        let id = d.intern_generated_texture(GeneratedTextureSource {
            provider_key: String::new(),
            width: 1,
            height: 1,
        });
        d.push(RecordCmd::Texture {
            texture: id,
            dst: Rect::new(0.0, 0.0, 1.0, 1.0),
            alpha: 1.0,
        });
        assert!(matches!(
            d.validate(),
            Err(RecordingError::InvalidGeneratedTexture { .. })
        ));

        d.generated_textures[0].provider_key = "scene3d://stage".into();
        d.generated_textures[0].width = 0;
        assert!(matches!(
            d.validate(),
            Err(RecordingError::InvalidGeneratedTexture { .. })
        ));
    }

    #[test]
    fn validate_catches_path_arity_mismatch() {
        let mut d = sample();
        // Reject mismatched path arity even when both spans are in bounds.
        d.push(RecordCmd::Path {
            path: PathRef {
                verbs: Span {
                    start: 0,
                    end: d.verbs.len() as u32,
                },
                points: Span {
                    start: 0,
                    end: d.points.len() as u32 - 1,
                },
            },
            fill_rule: FillRule::NonZero,
            fill: None,
            stroke: None,
        });
        assert!(matches!(
            d.validate(),
            Err(RecordingError::PathArity { .. })
        ));
    }

    #[test]
    fn interning_dedupes_fonts_images_and_generated_textures() {
        let mut d = ProgramRecording::new();
        let face = FontFace {
            family: "Inter".into(),
            weight: 400,
            italic: false,
            size: 16.0,
        };
        assert_eq!(d.intern_font(face.clone()), FontId(0));
        assert_eq!(d.intern_font(face), FontId(0));
        assert_eq!(d.fonts.len(), 1);

        let img = ImageSource {
            asset: "asset:test-recording".into(),
            width: 4,
            height: 4,
            source_time_s: None,
        };
        assert_eq!(d.intern_image(img.clone()), ImageId(0));
        assert_eq!(d.intern_image(img), ImageId(0));

        let generated = GeneratedTextureSource {
            provider_key: "scene3d://stage".into(),
            width: 64,
            height: 32,
        };
        assert_eq!(
            d.intern_generated_texture(generated.clone()),
            GeneratedTextureId(0)
        );
        assert_eq!(d.intern_generated_texture(generated), GeneratedTextureId(0));
    }

    #[test]
    fn clear_keeps_capacity() {
        let mut d = sample();
        let cap = d.points.capacity();
        d.clear();
        assert!(d.is_empty());
        assert_eq!(d.points.capacity(), cap, "clear must retain capacity");
    }

    /// Clear every side table to prevent stale entries accumulating across reused frames.
    #[test]
    fn clear_empties_every_side_table() {
        let mut d = sample();
        d.intern_glyphs(&[Glyph {
            id: 1,
            x: 0.0,
            y: 0.0,
        }]);
        d.intern_glyph_source_ranges(&[Span { start: 0, end: 3 }]);
        d.intern_font(FontFace {
            family: "f".into(),
            weight: 400,
            italic: false,
            size: 16.0,
        });
        d.intern_image(ImageSource {
            asset: "a".into(),
            width: 1,
            height: 1,
            source_time_s: None,
        });
        d.intern_generated_texture(GeneratedTextureSource {
            provider_key: "scene3d://clear-probe".into(),
            width: 1,
            height: 1,
        });
        d.intern_filters(&[FilterOp::Blur { sigma: 1.0 }]);
        d.intern_stops(&[(0.0, Rgba::rgb(0, 0, 0))]);
        d.intern_shader_program(ShaderProgram {
            uri: "shader://clear-probe@1".into(),
            content_hash: DigestBytes::from_bytes([0x11; 32]),
            abi_hash: DigestBytes::from_bytes([0x22; 32]),
        });
        d.intern_shader_uniforms(&[ShaderUniformBinding {
            name: "amount".into(),
            value: ShaderUniformValue::Float { value: 0.5 },
        }]);
        d.intern_shader_inputs(&[ShaderTextureInput {
            name: "noise".into(),
            image: ImageId(0),
        }]);

        d.clear();

        assert_eq!(
            d,
            ProgramRecording::new(),
            "clear must produce the state of a new ProgramRecording"
        );
    }

    #[test]
    fn layer_homography_foreshortens_the_far_edge() {
        let h = Homography::layer(100.0, 80.0, 40.0, 0.0, 800.0).expect("rotateY");
        let left = h.apply(Point::new(0.0, 80.0));
        let right = h.apply(Point::new(200.0, 80.0));
        let left_height =
            (h.apply(Point::new(0.0, 0.0)).y - h.apply(Point::new(0.0, 160.0)).y).abs();
        let right_height =
            (h.apply(Point::new(200.0, 0.0)).y - h.apply(Point::new(200.0, 160.0)).y).abs();
        assert!(right.x - left.x < 200.0, "card should get thinner");
        assert!(
            right_height < left_height,
            "far (right) edge must be shorter than the near edge: {right_height} vs {left_height}"
        );
        assert!(Homography::layer(100.0, 80.0, 0.0, 0.0, 800.0).is_none());
    }

    #[test]
    fn positive_rotate_x_matches_css_near_edge_direction() {
        let h = Homography::layer(100.0, 80.0, 0.0, 30.0, 800.0).expect("rotateX");
        let top_width = (h.apply(Point::new(200.0, 0.0)).x - h.apply(Point::new(0.0, 0.0)).x).abs();
        let bottom_width =
            (h.apply(Point::new(200.0, 160.0)).x - h.apply(Point::new(0.0, 160.0)).x).abs();
        assert!(
            bottom_width > top_width,
            "CSS positive rotateX brings the bottom edge closer: {bottom_width} vs {top_width}"
        );
    }

    #[test]
    fn map_rect_is_identity_on_a_matching_quad() {
        let src = Rect::new(10.0, 20.0, 40.0, 30.0);
        let dst = [
            Point::new(10.0, 20.0),
            Point::new(50.0, 20.0),
            Point::new(50.0, 50.0),
            Point::new(10.0, 50.0),
        ];
        let h = Homography::map_rect(src, dst).expect("identity quad");
        let mid = h.apply(Point::new(30.0, 35.0));
        assert!((mid.x - 30.0).abs() < 1e-9);
        assert!((mid.y - 35.0).abs() < 1e-9);
    }

    #[test]
    fn map_rect_recovers_a_layer_homography() {
        let src = Rect::new(0.0, 0.0, 190.0, 190.0);
        let layer = Homography::layer(95.0, 95.0, -38.0, -22.0, 760.0).expect("layer");
        let dst = [
            layer.apply(Point::new(0.0, 0.0)),
            layer.apply(Point::new(190.0, 0.0)),
            layer.apply(Point::new(190.0, 190.0)),
            layer.apply(Point::new(0.0, 190.0)),
        ];
        let mapped = Homography::map_rect(src, dst).expect("quad");
        let sample = Point::new(42.0, 117.0);
        let a = layer.apply(sample);
        let b = mapped.apply(sample);
        assert!((a.x - b.x).abs() < 1e-6, "{a:?} vs {b:?}");
        assert!((a.y - b.y).abs() < 1e-6, "{a:?} vs {b:?}");
    }

    #[test]
    fn map_rect_rejects_a_bowtie_or_line() {
        let src = Rect::new(0.0, 0.0, 10.0, 10.0);
        let line = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(20.0, 0.0),
            Point::new(30.0, 0.0),
        ];
        assert!(Homography::map_rect(src, line).is_none());
        let bowtie = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(10.0, 0.0),
            Point::new(0.0, 10.0),
        ];
        assert!(Homography::map_rect(src, bowtie).is_none());
    }

    #[test]
    fn combined_rotate_x_and_y_shears_a_vertical_point() {
        let only_y = Homography::layer(0.0, 0.0, 40.0, 0.0, 800.0).expect("rotateY");
        let only_x = Homography::layer(0.0, 0.0, 0.0, 25.0, 800.0).expect("rotateX");
        let both = Homography::layer(0.0, 0.0, 40.0, 25.0, 800.0).expect("rotateX+Y");
        let local = Point::new(0.0, 40.0);
        assert_eq!(only_y.apply(local).x, 0.0, "rotateY alone keeps x = 0");
        assert_eq!(only_x.apply(local).x, 0.0, "rotateX alone keeps x = 0");
        assert!(
            both.apply(local).x.abs() > 1.0,
            "rotateX then rotateY must pull (0, y) off the y-axis: {:?}",
            both.apply(local)
        );

        let (sy, cy_y) = crate::math::sin_cos(40.0_f64.to_radians());
        let (sx, cx_x) = crate::math::sin_cos(25.0_f64.to_radians());
        let w = 1.0 - cy_y * sx / 800.0 * local.y;
        let expected = Point::new((sy * sx * local.y) / w, (cx_x * local.y) / w);
        let got = both.apply(local);
        assert!((got.x - expected.x).abs() < 1e-12);
        assert!((got.y - expected.y).abs() < 1e-12);
    }

    #[test]
    fn homography_inverse_round_trips_a_perspective_layer() {
        let h = Homography::layer(100.0, 80.0, 32.0, 18.0, 900.0).expect("layer");
        let inverse = h.inverse().expect("invertible");
        let sample = Point::new(40.0, 30.0);
        let back = inverse.try_apply(h.apply(sample)).expect("finite");
        assert!((back.x - sample.x).abs() < 1e-9);
        assert!((back.y - sample.y).abs() < 1e-9);

        let affine = Affine::translate(12.0, -8.0).then(Affine::rotate(20.0));
        let as_h = Homography::from_affine(affine);
        let p = Point::new(15.0, 9.0);
        let via_affine = affine.apply(p);
        let via_h = as_h.apply(p);
        assert!((via_affine.x - via_h.x).abs() < 1e-12);
        assert!((via_affine.y - via_h.y).abs() < 1e-12);
        let composed = as_h.then(h);
        let inverted = composed.inverse().expect("invertible stack");
        let round = inverted.try_apply(composed.apply(p)).expect("finite");
        assert!((round.x - p.x).abs() < 1e-9);
        assert!((round.y - p.y).abs() < 1e-9);
    }

    #[test]
    fn generated_texture_placements_invert_the_perspective_stack() {
        let mut list = ProgramRecording::new();
        let texture = list.intern_generated_texture(GeneratedTextureSource {
            provider_key: "scene3d://card".into(),
            width: 40,
            height: 20,
        });
        let dst = Rect::new(0.0, 0.0, 40.0, 20.0);
        let matrix = Homography::layer(20.0, 10.0, 30.0, 15.0, 400.0).expect("layer");
        list.push(RecordCmd::BeginTransform {
            transform: Affine::translate(80.0, 40.0),
        });
        list.push(RecordCmd::BeginPerspective { matrix });
        list.push(RecordCmd::Texture {
            texture,
            dst,
            alpha: 1.0,
        });
        list.push(RecordCmd::End);
        list.push(RecordCmd::End);
        list.validate().expect("valid");

        let placements = list.generated_texture_placements().expect("stack");
        assert_eq!(placements.len(), 1);
        let canvas_from_local = placements[0].canvas_from_local;
        let local = Point::new(30.0, 5.0);
        let canvas = canvas_from_local.apply(local);
        let recovered = canvas_from_local
            .inverse()
            .and_then(|inverse| inverse.try_apply(canvas))
            .expect("pickable");
        assert!((recovered.x - local.x).abs() < 1e-9);
        assert!((recovered.y - local.y).abs() < 1e-9);
        // Affine-only pick (ignore BeginPerspective) lands somewhere else.
        let affine_only = Affine::translate(80.0, 40.0).apply(local);
        assert!((affine_only.x - canvas.x).abs() > 0.5);
    }

    #[test]
    fn leaf_paint_lifts_with_stop_offset() {
        let mut leaf = crate::draw::DrawList::new();
        let g = leaf.linear_gradient(
            Point::default(),
            Point::new(0.0, 1.0),
            &[(0.0, Rgba::rgb(1, 1, 1)), (1.0, Rgba::rgb(2, 2, 2))],
        );
        let mut d = ProgramRecording::new();
        d.intern_stops(&[(0.0, Rgba::rgb(9, 9, 9))]); // Reserve an entry to exercise a nonzero offset.
        let offset = d.push_draw_gradients(&leaf.gradient_stops);
        assert_eq!(offset, 1);
        let Paint::Linear(lifted) = Paint::from_leaf(g, offset) else {
            panic!("linear must stay linear")
        };
        assert_eq!(lifted.stops, Span { start: 1, end: 3 });
        assert_eq!(
            d.gradient_stops_of(lifted.stops)[0].color,
            Rgba::rgb(1, 1, 1)
        );
    }
}
