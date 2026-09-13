//! Typed, deterministic geometry shared by Motion expression evaluation and layout emission.
//!
//! Curves are measured with an exact-version fixed arc-length table while sampling and trimming
//! stay on the authored Line/Quad/Cubic segments. This is deliberately not a renderer service:
//! Native and WASM must derive trim, length, points and tangents from the same [`PathData`] bytes
//! before either backend sees a ProgramRecording.

use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon};
use serde::{Deserialize, Serialize};
use valle_draw::{PathVerb, Point, Vec2};

/// Maximum authored points in one path value. This protects Artifact decoding and trajectory size.
pub const MAX_PATH_POINTS: usize = 16_384;
/// Maximum points admitted in a per-frame path expression such as morph.
pub const MAX_FRAME_GEOMETRY_POINTS: usize = 2_048;
/// Maximum number of frames embedded in one precomputed geometry trajectory.
pub const MAX_GEOMETRY_TRAJECTORY_FRAMES: usize = 18_000;
/// Exact-version curve subdivision count used to build the shared arc-length lookup table.
pub const PATH_CURVE_STEPS: usize = 16;
/// Exact-version number of cubic segments emitted by [`PathData::arc`].
pub const PATH_ARC_SEGMENTS: usize = 4;
/// Frozen sector table: inner-start corner, start radial, outer-start corner, outer arc,
/// outer-end corner, end radial, inner-end corner, inner arc, close. Degenerate radii and
/// corners keep these commands and this point count.
pub const PATH_SECTOR_ARC_SEGMENTS: usize = PATH_ARC_SEGMENTS;
pub const PATH_SECTOR_VERBS: usize = 8 + 2 * PATH_SECTOR_ARC_SEGMENTS;
pub const PATH_SECTOR_POINTS: usize = 15 + 6 * PATH_SECTOR_ARC_SEGMENTS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PathBooleanOp {
    Union,
    Intersection,
    Difference,
    Xor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathData {
    pub verbs: Vec<PathVerb>,
    pub points: Vec<Point>,
}

impl PathData {
    pub fn new(verbs: Vec<PathVerb>, points: Vec<Point>) -> Result<Self, GeometryError> {
        let value = Self { verbs, points };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), GeometryError> {
        if self.verbs.is_empty() {
            return if self.points.is_empty() {
                Ok(())
            } else {
                Err(GeometryError::PointCount {
                    expected: 0,
                    actual: self.points.len(),
                })
            };
        }
        let expected = self
            .verbs
            .iter()
            .try_fold(0usize, |sum, verb| sum.checked_add(verb.point_count()))
            .ok_or(GeometryError::TooComplex)?;
        if expected != self.points.len() {
            return Err(GeometryError::PointCount {
                expected,
                actual: self.points.len(),
            });
        }
        if self.points.len() > MAX_PATH_POINTS {
            return Err(GeometryError::TooComplex);
        }
        if self
            .points
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
        {
            return Err(GeometryError::NonFinite);
        }
        if !matches!(self.verbs.first(), Some(PathVerb::Move)) {
            return Err(GeometryError::MissingMove);
        }
        Ok(())
    }

    pub fn has_same_topology(&self, other: &Self) -> bool {
        self.verbs == other.verbs && self.points.len() == other.points.len()
    }

    pub fn morph(&self, other: &Self, progress: f64) -> Result<Self, GeometryError> {
        self.validate()?;
        other.validate()?;
        if !self.has_same_topology(other) {
            return Err(GeometryError::TopologyMismatch);
        }
        if !progress.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        let t = progress.clamp(0.0, 1.0);
        let points = self
            .points
            .iter()
            .zip(&other.points)
            .map(|(from, to)| Point::new(lerp(from.x, to.x, t), lerp(from.y, to.y, t)))
            .collect();
        PathData::new(self.verbs.clone(), points)
    }

    pub fn line(points: Vec<Point>) -> Result<Self, GeometryError> {
        if points.len() < 2 {
            return Err(GeometryError::TooFewPoints);
        }
        let mut verbs = Vec::with_capacity(points.len());
        verbs.push(PathVerb::Move);
        verbs.extend(std::iter::repeat_n(PathVerb::Line, points.len() - 1));
        Self::new(verbs, points)
    }

    pub fn cubic(
        from: Point,
        control_1: Point,
        control_2: Point,
        to: Point,
    ) -> Result<Self, GeometryError> {
        Self::new(
            vec![PathVerb::Move, PathVerb::Cubic],
            vec![from, control_1, control_2, to],
        )
    }

    /// Build one arc with fixed topology. Angles use the Canvas/geometry convention: radians,
    /// zero on +x, positive clockwise in Valle's y-down coordinate space.
    pub fn arc(
        center: Point,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Result<Self, GeometryError> {
        if !center.x.is_finite()
            || !center.y.is_finite()
            || !radius.is_finite()
            || radius <= 0.0
            || !start_angle.is_finite()
            || !end_angle.is_finite()
        {
            return Err(GeometryError::NonFinite);
        }
        let sweep = end_angle - start_angle;
        if sweep.abs() > core::f64::consts::TAU {
            return Err(GeometryError::InvalidArc);
        }
        let step = sweep / PATH_ARC_SEGMENTS as f64;
        let mut verbs = Vec::with_capacity(PATH_ARC_SEGMENTS + 1);
        let mut points = Vec::with_capacity(1 + PATH_ARC_SEGMENTS * 3);
        let at = |angle: f64| {
            let (sin, cos) = valle_draw::math::sin_cos(angle);
            Point::new(center.x + radius * cos, center.y + radius * sin)
        };
        verbs.push(PathVerb::Move);
        points.push(at(start_angle));
        for segment in 0..PATH_ARC_SEGMENTS {
            let from_angle = start_angle + step * segment as f64;
            let to_angle = from_angle + step;
            let (from_sin, from_cos) = valle_draw::math::sin_cos(from_angle);
            let (to_sin, to_cos) = valle_draw::math::sin_cos(to_angle);
            let k = 4.0 / 3.0 * valle_draw::math::tan(step / 4.0) * radius;
            points.extend([
                Point::new(
                    center.x + radius * from_cos - k * from_sin,
                    center.y + radius * from_sin + k * from_cos,
                ),
                Point::new(
                    center.x + radius * to_cos + k * to_sin,
                    center.y + radius * to_sin - k * to_cos,
                ),
                at(to_angle),
            ]);
            verbs.push(PathVerb::Cubic);
        }
        Self::new(verbs, points)
    }

    /// Close an open single-contour path against a horizontal baseline, keeping Line/Quad/Cubic
    /// verbs so a filled area shares the stroke's curve.
    pub fn area(&self, baseline_y: f64) -> Result<Self, GeometryError> {
        if !baseline_y.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        let (first, last, rest) = open_contour(self)?;
        let mut verbs = Vec::with_capacity(self.verbs.len() + 3);
        let mut points = Vec::with_capacity(self.points.len() + 2);
        verbs.push(PathVerb::Move);
        points.push(Point::new(first.x, baseline_y));
        verbs.push(PathVerb::Line);
        points.push(first);
        verbs.extend(rest);
        points.extend(self.points.iter().copied().skip(1));
        verbs.push(PathVerb::Line);
        points.push(Point::new(last.x, baseline_y));
        verbs.push(PathVerb::Close);
        Self::new(verbs, points)
    }

    /// Join two aligned open contours into a closed band: upper forward, lower reversed.
    pub fn area_band(upper: &Self, lower: &Self) -> Result<Self, GeometryError> {
        let (upper_start, _upper_end, upper_rest) = open_contour(upper)?;
        let (lower_start, lower_end, lower_rest) = open_contour(lower)?;
        if upper_rest != lower_rest {
            return Err(GeometryError::AreaBandMismatch);
        }
        let upper_nodes = on_curve_nodes(upper)?;
        let lower_nodes = on_curve_nodes(lower)?;
        if upper_nodes.len() != lower_nodes.len()
            || !strictly_increasing_x(&upper_nodes)
            || !strictly_increasing_x(&lower_nodes)
            || upper_nodes
                .iter()
                .zip(&lower_nodes)
                .any(|(upper, lower)| upper.x != lower.x)
        {
            return Err(GeometryError::AreaBandMismatch);
        }
        let reversed = reverse_open_segments(lower_start, &lower.points[1..], &lower_rest)?;
        let mut verbs = Vec::with_capacity(upper.verbs.len() + lower.verbs.len());
        let mut points = Vec::with_capacity(upper.points.len() + lower.points.len());
        verbs.push(PathVerb::Move);
        points.push(upper_start);
        verbs.extend(upper_rest);
        points.extend(upper.points.iter().copied().skip(1));
        verbs.push(PathVerb::Line);
        points.push(lower_end);
        verbs.extend(reversed.verbs);
        points.extend(reversed.points);
        verbs.push(PathVerb::Close);
        Self::new(verbs, points)
    }

    /// Filled annular sector with a frozen verb/point table. Angles are radians, zero on +X,
    /// positive clockwise. Interpolation must happen on these parameters, not on cubic controls.
    pub fn sector(
        center: Point,
        inner: f64,
        outer: f64,
        start: f64,
        end: f64,
        corner_radius: f64,
    ) -> Result<Self, GeometryError> {
        if ![center.x, center.y, inner, outer, start, end, corner_radius]
            .iter()
            .all(|value| value.is_finite())
            || inner < 0.0
            || outer < inner
            || corner_radius < 0.0
        {
            return Err(GeometryError::InvalidSector);
        }
        let sweep = end - start;
        if sweep.abs() > core::f64::consts::TAU {
            return Err(GeometryError::InvalidSector);
        }
        let s = if sweep < 0.0 { -1.0 } else { 1.0 };
        let half = sweep.abs() / 2.0;
        let sh = if half >= core::f64::consts::FRAC_PI_2 {
            1.0
        } else {
            valle_draw::math::sin(half).max(0.0)
        };
        let thickness = outer - inner;
        // Limit rounding by both the visible wedge and the remaining gap. As the gap closes,
        // corners collapse continuously, leaving a complete circle with the same verb table.
        let corner_span = sweep.abs().min(core::f64::consts::TAU - sweep.abs());
        let corner_sin = valle_draw::math::sin(corner_span / 2.0);
        let cr = corner_radius
            .min(thickness / 2.0)
            .min(outer * (corner_sin / (1.0 + corner_sin)));
        // The inner corners may meet before the outer ones. Shrinking the hole must only
        // shrink its own corners, otherwise the outer boundary jumps at inner == 0.
        let inner_cr = if inner == 0.0 {
            0.0
        } else if sh < 1.0 {
            cr.min(inner * (sh / (1.0 - sh)))
        } else {
            cr
        };
        let outer_ro = (outer - cr).max(0.0);
        let inner_ri = inner + inner_cr;
        let d_out = if cr > 0.0 && outer_ro > 0.0 {
            valle_draw::math::asin((cr / outer_ro).clamp(-1.0, 1.0))
        } else {
            0.0
        };
        let d_in = if inner_cr > 0.0 && inner_ri > 0.0 {
            valle_draw::math::asin((inner_cr / inner_ri).clamp(-1.0, 1.0))
        } else {
            0.0
        };
        let foot_out = if cr > 0.0 {
            valle_draw::math::sqrt((outer_ro * outer_ro - cr * cr).max(0.0))
        } else {
            outer
        };
        let foot_in = if inner_cr > 0.0 {
            valle_draw::math::sqrt((inner_ri * inner_ri - inner_cr * inner_cr).max(0.0))
        } else {
            inner
        };
        let at = |radius: f64, angle: f64| polar(center, radius, angle);
        let ci0 = at(inner_ri, start + s * d_in);
        let ci1 = at(inner_ri, end - s * d_in);
        let co0 = at(outer_ro, start + s * d_out);
        let co1 = at(outer_ro, end - s * d_out);
        let mut verbs = Vec::with_capacity(PATH_SECTOR_VERBS);
        let mut points = Vec::with_capacity(PATH_SECTOR_POINTS);
        let inner_start = at(inner, start + s * d_in);
        verbs.push(PathVerb::Move);
        points.push(inner_start);
        append_corner_cubic(
            &mut verbs,
            &mut points,
            ci0,
            inner_cr,
            start + s * d_in + core::f64::consts::PI,
            start - s * core::f64::consts::FRAC_PI_2,
        );
        verbs.push(PathVerb::Line);
        points.push(at(foot_out, start));
        append_corner_cubic(
            &mut verbs,
            &mut points,
            co0,
            cr,
            start - s * core::f64::consts::FRAC_PI_2,
            start + s * d_out,
        );
        append_arc_cubics(
            &mut verbs,
            &mut points,
            center,
            outer,
            start + s * d_out,
            end - s * d_out,
            PATH_SECTOR_ARC_SEGMENTS,
        );
        append_corner_cubic(
            &mut verbs,
            &mut points,
            co1,
            cr,
            end - s * d_out,
            end + s * core::f64::consts::FRAC_PI_2,
        );
        verbs.push(PathVerb::Line);
        points.push(at(foot_in, end));
        append_corner_cubic(
            &mut verbs,
            &mut points,
            ci1,
            inner_cr,
            end + s * core::f64::consts::FRAC_PI_2,
            end - s * d_in + core::f64::consts::PI,
        );
        append_arc_cubics(
            &mut verbs,
            &mut points,
            center,
            inner,
            end - s * d_in,
            start + s * d_in,
            PATH_SECTOR_ARC_SEGMENTS,
        );
        verbs.push(PathVerb::Close);
        debug_assert_eq!(verbs.len(), PATH_SECTOR_VERBS);
        debug_assert_eq!(points.len(), PATH_SECTOR_POINTS);
        Self::new(verbs, points)
    }

    /// Deterministically flatten one closed contour for backend-neutral material kernels.
    /// The repeated closing point is removed because packed polygon edges close implicitly.
    pub fn closed_polygon_points(&self) -> Result<Vec<Point>, GeometryError> {
        let flattened = FlattenedPath::from_path(self)?;
        if flattened.contours.len() != 1 || !flattened.contours[0].closed {
            return Err(GeometryError::SingleContourRequired);
        }
        let mut points = flattened.contours[0].points.clone();
        if points.len() > 1 && points.first() == points.last() {
            points.pop();
        }
        if points.len() < 3 {
            return Err(GeometryError::DegenerateContour);
        }
        Ok(points)
    }

    /// Deterministic one-sided polyline offset. Curves first use the shared fixed flattening; joins
    /// use a bounded miter and preserve contour count, so the result is safe for frame expressions.
    pub fn offset_path(&self, distance: f64) -> Result<Self, GeometryError> {
        if !distance.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        if distance == 0.0 {
            return Ok(self.clone());
        }
        let flattened = FlattenedPath::from_path(self)?;
        let mut verbs = Vec::new();
        let mut points = Vec::new();
        for contour in &flattened.contours {
            let offset = offset_contour(contour, distance)?;
            if offset.len() < 2 {
                continue;
            }
            verbs.push(PathVerb::Move);
            points.push(offset[0]);
            for point in offset.iter().skip(1) {
                verbs.push(PathVerb::Line);
                points.push(*point);
            }
            if contour.closed {
                verbs.push(PathVerb::Close);
            }
        }
        Self::new(verbs, points)
    }

    /// Prepare-time polygon boolean. Inputs may contain curves, which are flattened by the same
    /// exact-version geometry used by trim/sampling. One closed contour per operand is required;
    /// variable-topology output is frozen as a static PathData before frame evaluation.
    pub fn boolean(&self, other: &Self, op: PathBooleanOp) -> Result<Self, GeometryError> {
        let left = single_polygon(self)?;
        let right = single_polygon(other)?;
        let output = match op {
            PathBooleanOp::Union => left.union(&right),
            PathBooleanOp::Intersection => left.intersection(&right),
            PathBooleanOp::Difference => left.difference(&right),
            PathBooleanOp::Xor => left.xor(&right),
        };
        path_from_multi_polygon(&output)
    }

    pub fn path_length(&self) -> Result<f64, GeometryError> {
        Ok(MeasuredPath::from_path(self)?.total)
    }

    pub fn point_at(&self, progress: f64) -> Result<Point, GeometryError> {
        Ok(MeasuredPath::from_path(self)?.sample(progress).0)
    }

    pub fn tangent_at(&self, progress: f64) -> Result<Vec2, GeometryError> {
        Ok(MeasuredPath::from_path(self)?.sample(progress).1)
    }

    pub fn angle_at(&self, progress: f64) -> Result<f64, GeometryError> {
        let tangent = self.tangent_at(progress)?;
        Ok(valle_draw::math::atan2(tangent.y, tangent.x).to_degrees())
    }

    /// Sample points and tangents with one shared arc-length table. Results correspond to
    /// `progresses` and use the same geometry as individual sampling calls.
    pub fn samples_at(&self, progresses: &[f64]) -> Result<Vec<(Point, Vec2)>, GeometryError> {
        let measured = MeasuredPath::from_path(self)?;
        Ok(progresses
            .iter()
            .map(|progress| measured.sample(*progress))
            .collect())
    }

    pub fn to_svg_path(&self) -> String {
        let mut output = String::new();
        let mut at = 0usize;
        for verb in &self.verbs {
            if !output.is_empty() {
                output.push(' ');
            }
            let (name, count) = match verb {
                PathVerb::Move => ('M', 1),
                PathVerb::Line => ('L', 1),
                PathVerb::Quad => ('Q', 2),
                PathVerb::Cubic => ('C', 3),
                PathVerb::Close => ('Z', 0),
            };
            output.push(name);
            for point in &self.points[at..at + count] {
                output.push(' ');
                output.push_str(&geometry_number(point.x));
                output.push(' ');
                output.push_str(&geometry_number(point.y));
            }
            at += count;
        }
        output
    }

    /// Return the arc-length interval `[start, end]` while preserving authored curve verbs.
    /// Boundary segments are split with de Casteljau using the same measured parameters that drive
    /// [`Self::point_at`] and [`Self::tangent_at`].
    pub fn trim(&self, start: f64, end: f64) -> Result<Self, GeometryError> {
        self.validate()?;
        if !start.is_finite() || !end.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        let start = start.clamp(0.0, 1.0);
        let end = end.clamp(0.0, 1.0);
        if start <= 0.0 && end >= 1.0 {
            return Ok(self.clone());
        }
        if end <= start {
            return Ok(Self {
                verbs: Vec::new(),
                points: Vec::new(),
            });
        }
        MeasuredPath::from_path(self)?.trim(start, end)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GeometryEvalPolicy {
    PrepareCached,
    FrameExpr,
    PrecomputedTrajectory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeometryError {
    MissingMove,
    PointCount { expected: usize, actual: usize },
    NonFinite,
    TooComplex,
    TooFewPoints,
    TopologyMismatch,
    BadTrajectory,
    InvalidArc,
    InvalidSector,
    SingleContourRequired,
    ClosedContourRequired,
    DegenerateContour,
    AreaBandMismatch,
}

impl core::fmt::Display for GeometryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GeometryError::MissingMove => f.write_str("path must begin with move"),
            GeometryError::PointCount { expected, actual } => {
                write!(f, "path expects {expected} points but has {actual}")
            }
            GeometryError::NonFinite => f.write_str("path geometry must be finite"),
            GeometryError::TooComplex => write!(
                f,
                "path exceeds the geometry budget of {MAX_PATH_POINTS} points"
            ),
            GeometryError::TooFewPoints => {
                f.write_str("path constructor needs at least two points")
            }
            GeometryError::TopologyMismatch => {
                f.write_str("path morph inputs must have identical verbs and point counts")
            }
            GeometryError::BadTrajectory => {
                f.write_str("path trajectory frames must be non-empty and topology-compatible")
            }
            GeometryError::InvalidArc => {
                f.write_str("arc needs a positive radius and a sweep no larger than one turn")
            }
            GeometryError::InvalidSector => f.write_str(
                "sector needs finite inner/outer radii with 0 <= inner <= outer, a non-negative corner radius, and a sweep of at most one turn",
            ),
            GeometryError::SingleContourRequired => {
                f.write_str("geometry operation requires exactly one contour")
            }
            GeometryError::ClosedContourRequired => {
                f.write_str("path boolean requires one closed contour per operand")
            }
            GeometryError::DegenerateContour => f.write_str("geometry contour is degenerate"),
            GeometryError::AreaBandMismatch => f.write_str(
                "areaBand needs two open single-contour paths with matching segment structure and strictly increasing X nodes",
            ),
        }
    }
}

impl std::error::Error for GeometryError {}

#[derive(Debug)]
struct MeasuredPath {
    contours: Vec<MeasuredContour>,
    total: f64,
    first: Point,
}

#[derive(Debug)]
struct MeasuredContour {
    segments: Vec<MeasuredSegment>,
}

#[derive(Debug)]
struct MeasuredSegment {
    geometry: SegmentGeometry,
    samples: Vec<ArcSample>,
    length: f64,
}

#[derive(Debug, Clone, Copy)]
struct ArcSample {
    t: f64,
    length: f64,
}

#[derive(Debug, Clone, Copy)]
enum SegmentGeometry {
    Line {
        from: Point,
        to: Point,
    },
    Quad {
        from: Point,
        control: Point,
        to: Point,
    },
    Cubic {
        from: Point,
        control_1: Point,
        control_2: Point,
        to: Point,
    },
}

impl MeasuredPath {
    fn from_path(path: &PathData) -> Result<Self, GeometryError> {
        path.validate()?;
        let first = path.points.first().copied().unwrap_or_default();
        let mut contours = Vec::new();
        let mut segments = Vec::new();
        let mut at = 0usize;
        let mut current = Point::default();
        let mut contour_start = current;

        for verb in &path.verbs {
            match verb {
                PathVerb::Move => {
                    if !segments.is_empty() {
                        contours.push(MeasuredContour {
                            segments: std::mem::take(&mut segments),
                        });
                    }
                    current = path.points[at];
                    contour_start = current;
                    at += 1;
                }
                PathVerb::Line => {
                    let to = path.points[at];
                    segments.push(MeasuredSegment::new(SegmentGeometry::Line {
                        from: current,
                        to,
                    }));
                    current = to;
                    at += 1;
                }
                PathVerb::Quad => {
                    let control = path.points[at];
                    let to = path.points[at + 1];
                    segments.push(MeasuredSegment::new(SegmentGeometry::Quad {
                        from: current,
                        control,
                        to,
                    }));
                    current = to;
                    at += 2;
                }
                PathVerb::Cubic => {
                    let control_1 = path.points[at];
                    let control_2 = path.points[at + 1];
                    let to = path.points[at + 2];
                    segments.push(MeasuredSegment::new(SegmentGeometry::Cubic {
                        from: current,
                        control_1,
                        control_2,
                        to,
                    }));
                    current = to;
                    at += 3;
                }
                PathVerb::Close => {
                    if current != contour_start {
                        segments.push(MeasuredSegment::new(SegmentGeometry::Line {
                            from: current,
                            to: contour_start,
                        }));
                    }
                    current = contour_start;
                }
            }
        }
        if !segments.is_empty() {
            contours.push(MeasuredContour { segments });
        }
        let total = contours
            .iter()
            .flat_map(|contour| &contour.segments)
            .map(|segment| segment.length)
            .sum();
        Ok(Self {
            contours,
            total,
            first,
        })
    }

    fn sample(&self, progress: f64) -> (Point, Vec2) {
        if self.total <= 0.0 || !progress.is_finite() {
            return (self.first, Vec2::RIGHT);
        }
        let target = self.total * progress.clamp(0.0, 1.0);
        let mut consumed = 0.0;
        let mut last = (self.first, Vec2::RIGHT);
        for segment in self.contours.iter().flat_map(|contour| &contour.segments) {
            if segment.length <= 0.0 {
                continue;
            }
            last = segment.sample(segment.length);
            if target <= consumed + segment.length {
                return segment.sample(target - consumed);
            }
            consumed += segment.length;
        }
        last
    }

    fn trim(&self, start: f64, end: f64) -> Result<PathData, GeometryError> {
        if self.total <= 0.0 {
            return Ok(PathData {
                verbs: Vec::new(),
                points: Vec::new(),
            });
        }
        let from = self.total * start;
        let through = self.total * end;
        let mut consumed = 0.0;
        let mut verbs = Vec::new();
        let mut points = Vec::new();
        for contour in &self.contours {
            let mut open = false;
            for segment in &contour.segments {
                let segment_start = consumed;
                let segment_end = consumed + segment.length;
                consumed = segment_end;
                if segment.length <= 0.0 {
                    continue;
                }
                let overlap_start = from.max(segment_start);
                let overlap_end = through.min(segment_end);
                if overlap_end <= overlap_start {
                    continue;
                }
                let t0 = segment.parameter_at_length(overlap_start - segment_start);
                let t1 = segment.parameter_at_length(overlap_end - segment_start);
                let piece = segment.geometry.subsegment(t0, t1);
                if !open {
                    verbs.push(PathVerb::Move);
                    points.push(piece.from());
                    open = true;
                }
                piece.push_verb_and_points(&mut verbs, &mut points);
            }
        }
        PathData::new(verbs, points)
    }
}

impl MeasuredSegment {
    fn new(geometry: SegmentGeometry) -> Self {
        let steps = if matches!(geometry, SegmentGeometry::Line { .. }) {
            1
        } else {
            PATH_CURVE_STEPS
        };
        let mut samples = Vec::with_capacity(steps + 1);
        let mut previous = geometry.point(0.0);
        let mut length = 0.0;
        samples.push(ArcSample { t: 0.0, length });
        for step in 1..=steps {
            let t = step as f64 / steps as f64;
            let point = geometry.point(t);
            length += distance(previous, point);
            samples.push(ArcSample { t, length });
            previous = point;
        }
        Self {
            geometry,
            samples,
            length,
        }
    }

    fn parameter_at_length(&self, length: f64) -> f64 {
        if self.length <= 0.0 {
            return 0.0;
        }
        let target = length.clamp(0.0, self.length);
        let index = self
            .samples
            .partition_point(|sample| sample.length < target)
            .clamp(1, self.samples.len() - 1);
        let before = self.samples[index - 1];
        let after = self.samples[index];
        let span = after.length - before.length;
        if span <= 0.0 {
            before.t
        } else {
            lerp(before.t, after.t, (target - before.length) / span)
        }
    }

    fn sample(&self, length: f64) -> (Point, Vec2) {
        let t = self.parameter_at_length(length);
        (self.geometry.point(t), self.geometry.tangent(t))
    }
}

impl SegmentGeometry {
    fn from(self) -> Point {
        match self {
            Self::Line { from, .. } | Self::Quad { from, .. } | Self::Cubic { from, .. } => from,
        }
    }

    fn point(self, t: f64) -> Point {
        let t = t.clamp(0.0, 1.0);
        let u = 1.0 - t;
        match self {
            Self::Line { from, to } => point_lerp(from, to, t),
            Self::Quad { from, control, to } => Point::new(
                u * u * from.x + 2.0 * u * t * control.x + t * t * to.x,
                u * u * from.y + 2.0 * u * t * control.y + t * t * to.y,
            ),
            Self::Cubic {
                from,
                control_1,
                control_2,
                to,
            } => Point::new(
                u * u * u * from.x
                    + 3.0 * u * u * t * control_1.x
                    + 3.0 * u * t * t * control_2.x
                    + t * t * t * to.x,
                u * u * u * from.y
                    + 3.0 * u * u * t * control_1.y
                    + 3.0 * u * t * t * control_2.y
                    + t * t * t * to.y,
            ),
        }
    }

    fn tangent(self, t: f64) -> Vec2 {
        let t = t.clamp(0.0, 1.0);
        let u = 1.0 - t;
        let (dx, dy) = match self {
            Self::Line { from, to } => (to.x - from.x, to.y - from.y),
            Self::Quad { from, control, to } => (
                2.0 * (u * (control.x - from.x) + t * (to.x - control.x)),
                2.0 * (u * (control.y - from.y) + t * (to.y - control.y)),
            ),
            Self::Cubic {
                from,
                control_1,
                control_2,
                to,
            } => (
                3.0 * (u * u * (control_1.x - from.x)
                    + 2.0 * u * t * (control_2.x - control_1.x)
                    + t * t * (to.x - control_2.x)),
                3.0 * (u * u * (control_1.y - from.y)
                    + 2.0 * u * t * (control_2.y - control_1.y)
                    + t * t * (to.y - control_2.y)),
            ),
        };
        let length = valle_draw::math::sqrt(dx * dx + dy * dy);
        if length > 0.0 {
            Vec2::new(dx / length, dy / length)
        } else {
            Vec2::RIGHT
        }
    }

    fn subsegment(self, t0: f64, t1: f64) -> Self {
        let t0 = t0.clamp(0.0, 1.0);
        let t1 = t1.clamp(t0, 1.0);
        match self {
            Self::Line { from, to } => Self::Line {
                from: point_lerp(from, to, t0),
                to: point_lerp(from, to, t1),
            },
            Self::Quad { from, control, to } => {
                let left = split_quad(from, control, to, t1).0;
                if t0 <= 0.0 {
                    Self::Quad {
                        from: left[0],
                        control: left[1],
                        to: left[2],
                    }
                } else {
                    let local = if t1 > 0.0 { t0 / t1 } else { 0.0 };
                    let piece = split_quad(left[0], left[1], left[2], local).1;
                    Self::Quad {
                        from: piece[0],
                        control: piece[1],
                        to: piece[2],
                    }
                }
            }
            Self::Cubic {
                from,
                control_1,
                control_2,
                to,
            } => {
                let left = split_cubic(from, control_1, control_2, to, t1).0;
                if t0 <= 0.0 {
                    Self::Cubic {
                        from: left[0],
                        control_1: left[1],
                        control_2: left[2],
                        to: left[3],
                    }
                } else {
                    let local = if t1 > 0.0 { t0 / t1 } else { 0.0 };
                    let piece = split_cubic(left[0], left[1], left[2], left[3], local).1;
                    Self::Cubic {
                        from: piece[0],
                        control_1: piece[1],
                        control_2: piece[2],
                        to: piece[3],
                    }
                }
            }
        }
    }

    fn push_verb_and_points(self, verbs: &mut Vec<PathVerb>, points: &mut Vec<Point>) {
        match self {
            Self::Line { to, .. } => {
                verbs.push(PathVerb::Line);
                points.push(to);
            }
            Self::Quad { control, to, .. } => {
                verbs.push(PathVerb::Quad);
                points.extend([control, to]);
            }
            Self::Cubic {
                control_1,
                control_2,
                to,
                ..
            } => {
                verbs.push(PathVerb::Cubic);
                points.extend([control_1, control_2, to]);
            }
        }
    }
}

fn split_quad(from: Point, control: Point, to: Point, t: f64) -> ([Point; 3], [Point; 3]) {
    let a = point_lerp(from, control, t);
    let b = point_lerp(control, to, t);
    let middle = point_lerp(a, b, t);
    ([from, a, middle], [middle, b, to])
}

fn split_cubic(
    from: Point,
    control_1: Point,
    control_2: Point,
    to: Point,
    t: f64,
) -> ([Point; 4], [Point; 4]) {
    let a = point_lerp(from, control_1, t);
    let b = point_lerp(control_1, control_2, t);
    let c = point_lerp(control_2, to, t);
    let d = point_lerp(a, b, t);
    let e = point_lerp(b, c, t);
    let middle = point_lerp(d, e, t);
    ([from, a, d, middle], [middle, e, c, to])
}

fn point_lerp(from: Point, to: Point, progress: f64) -> Point {
    Point::new(lerp(from.x, to.x, progress), lerp(from.y, to.y, progress))
}

#[derive(Debug)]
struct FlattenedPath {
    contours: Vec<FlattenedContour>,
}

#[derive(Debug)]
struct FlattenedContour {
    points: Vec<Point>,
    closed: bool,
}

impl FlattenedPath {
    fn from_path(path: &PathData) -> Result<Self, GeometryError> {
        path.validate()?;
        let mut contours = Vec::<FlattenedContour>::new();
        let mut contour = Vec::<Point>::new();
        let mut contour_closed = false;
        let mut at = 0usize;
        let mut current = Point::new(0.0, 0.0);
        let mut contour_start = current;

        for verb in &path.verbs {
            match verb {
                PathVerb::Move => {
                    if !contour.is_empty() {
                        contours.push(FlattenedContour {
                            points: std::mem::take(&mut contour),
                            closed: contour_closed,
                        });
                    }
                    contour_closed = false;
                    current = path.points[at];
                    contour_start = current;
                    contour.push(current);
                    at += 1;
                }
                PathVerb::Line => {
                    current = path.points[at];
                    contour.push(current);
                    at += 1;
                }
                PathVerb::Quad => {
                    let from = current;
                    let control = path.points[at];
                    let to = path.points[at + 1];
                    for step in 1..=PATH_CURVE_STEPS {
                        let t = step as f64 / PATH_CURVE_STEPS as f64;
                        let u = 1.0 - t;
                        contour.push(Point::new(
                            u * u * from.x + 2.0 * u * t * control.x + t * t * to.x,
                            u * u * from.y + 2.0 * u * t * control.y + t * t * to.y,
                        ));
                    }
                    current = to;
                    at += 2;
                }
                PathVerb::Cubic => {
                    let from = current;
                    let control_1 = path.points[at];
                    let control_2 = path.points[at + 1];
                    let to = path.points[at + 2];
                    for step in 1..=PATH_CURVE_STEPS {
                        let t = step as f64 / PATH_CURVE_STEPS as f64;
                        let u = 1.0 - t;
                        contour.push(Point::new(
                            u * u * u * from.x
                                + 3.0 * u * u * t * control_1.x
                                + 3.0 * u * t * t * control_2.x
                                + t * t * t * to.x,
                            u * u * u * from.y
                                + 3.0 * u * u * t * control_1.y
                                + 3.0 * u * t * t * control_2.y
                                + t * t * t * to.y,
                        ));
                    }
                    current = to;
                    at += 3;
                }
                PathVerb::Close => {
                    if current != contour_start {
                        contour.push(contour_start);
                    }
                    current = contour_start;
                    contour_closed = true;
                }
            }
        }
        if !contour.is_empty() {
            contours.push(FlattenedContour {
                points: contour,
                closed: contour_closed,
            });
        }
        Ok(Self { contours })
    }
}

fn offset_contour(contour: &FlattenedContour, distance: f64) -> Result<Vec<Point>, GeometryError> {
    let mut source = contour.points.as_slice();
    if contour.closed && source.len() > 1 && source.first() == source.last() {
        source = &source[..source.len() - 1];
    }
    if source.len() < 2 {
        return Err(GeometryError::DegenerateContour);
    }
    let segment_normal = |from: Point, to: Point| -> Option<Vec2> {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let length = valle_draw::math::sqrt(dx * dx + dy * dy);
        (length > 0.0).then(|| Vec2::new(-dy / length, dx / length))
    };
    let mut output = Vec::with_capacity(source.len());
    for index in 0..source.len() {
        let previous = if index > 0 {
            segment_normal(source[index - 1], source[index])
        } else if contour.closed {
            segment_normal(source[source.len() - 1], source[index])
        } else {
            None
        };
        let next = if index + 1 < source.len() {
            segment_normal(source[index], source[index + 1])
        } else if contour.closed {
            segment_normal(source[index], source[0])
        } else {
            None
        };
        let normal = match (previous, next) {
            (Some(previous), Some(next)) => {
                let x = previous.x + next.x;
                let y = previous.y + next.y;
                let length = valle_draw::math::sqrt(x * x + y * y);
                if length <= 1e-12 {
                    next
                } else {
                    let unit = Vec2::new(x / length, y / length);
                    let projection = (unit.x * next.x + unit.y * next.y).abs().max(0.25);
                    Vec2::new(unit.x / projection, unit.y / projection)
                }
            }
            (Some(normal), None) | (None, Some(normal)) => normal,
            (None, None) => return Err(GeometryError::DegenerateContour),
        };
        output.push(Point::new(
            source[index].x + normal.x * distance,
            source[index].y + normal.y * distance,
        ));
    }
    Ok(output)
}

fn single_polygon(path: &PathData) -> Result<Polygon<f64>, GeometryError> {
    let flattened = FlattenedPath::from_path(path)?;
    if flattened.contours.len() != 1 {
        return Err(GeometryError::SingleContourRequired);
    }
    let contour = &flattened.contours[0];
    if !contour.closed {
        return Err(GeometryError::ClosedContourRequired);
    }
    if contour.points.len() < 4 {
        return Err(GeometryError::DegenerateContour);
    }
    let ring = contour
        .points
        .iter()
        .map(|point| Coord {
            x: point.x,
            y: point.y,
        })
        .collect::<Vec<_>>();
    Ok(Polygon::new(LineString::new(ring), Vec::new()))
}

fn path_from_multi_polygon(value: &MultiPolygon<f64>) -> Result<PathData, GeometryError> {
    let mut verbs = Vec::new();
    let mut points = Vec::new();
    let mut push_ring = |ring: &LineString<f64>| {
        let coords = &ring.0;
        if coords.len() < 4 {
            return;
        }
        verbs.push(PathVerb::Move);
        points.push(Point::new(coords[0].x, coords[0].y));
        for coord in coords.iter().skip(1).take(coords.len() - 2) {
            verbs.push(PathVerb::Line);
            points.push(Point::new(coord.x, coord.y));
        }
        verbs.push(PathVerb::Close);
    };
    for polygon in &value.0 {
        push_ring(polygon.exterior());
        for interior in polygon.interiors() {
            push_ring(interior);
        }
    }
    PathData::new(verbs, points)
}

fn distance(from: Point, to: Point) -> f64 {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    valle_draw::math::sqrt(dx * dx + dy * dy)
}

fn polar(center: Point, radius: f64, angle: f64) -> Point {
    let (sin, cos) = valle_draw::math::sin_cos(angle);
    Point::new(center.x + radius * cos, center.y + radius * sin)
}

fn wrap_delta(from: f64, to: f64) -> f64 {
    let mut delta = to - from;
    while delta > core::f64::consts::PI {
        delta -= core::f64::consts::TAU;
    }
    while delta < -core::f64::consts::PI {
        delta += core::f64::consts::TAU;
    }
    delta
}

fn append_arc_cubics(
    verbs: &mut Vec<PathVerb>,
    points: &mut Vec<Point>,
    center: Point,
    radius: f64,
    from: f64,
    to: f64,
    segments: usize,
) {
    let radius = radius.max(0.0);
    let step = (to - from) / segments as f64;
    for segment in 0..segments {
        let a0 = from + step * segment as f64;
        let a1 = a0 + step;
        let k = 4.0 / 3.0 * valle_draw::math::tan(step / 4.0) * radius;
        let (s0, c0) = valle_draw::math::sin_cos(a0);
        let (s1, c1) = valle_draw::math::sin_cos(a1);
        points.push(Point::new(
            center.x + radius * c0 - k * s0,
            center.y + radius * s0 + k * c0,
        ));
        points.push(Point::new(
            center.x + radius * c1 + k * s1,
            center.y + radius * s1 - k * c1,
        ));
        points.push(polar(center, radius, a1));
        verbs.push(PathVerb::Cubic);
    }
}

fn append_corner_cubic(
    verbs: &mut Vec<PathVerb>,
    points: &mut Vec<Point>,
    corner: Point,
    radius: f64,
    from: f64,
    to: f64,
) {
    if radius <= 0.0 {
        points.extend([corner, corner, corner]);
        verbs.push(PathVerb::Cubic);
        return;
    }
    let end = from + wrap_delta(from, to);
    append_arc_cubics(verbs, points, corner, radius, from, end, 1);
}

fn open_contour(path: &PathData) -> Result<(Point, Point, Vec<PathVerb>), GeometryError> {
    path.validate()?;
    if path.verbs.first() != Some(&PathVerb::Move)
        || path.verbs.iter().any(|verb| *verb == PathVerb::Close)
        || path
            .verbs
            .iter()
            .filter(|verb| **verb == PathVerb::Move)
            .count()
            != 1
        || path.points.len() < 2
    {
        return Err(GeometryError::SingleContourRequired);
    }
    Ok((
        path.points[0],
        *path.points.last().expect("open contour has an end point"),
        path.verbs[1..].to_vec(),
    ))
}

fn on_curve_nodes(path: &PathData) -> Result<Vec<Point>, GeometryError> {
    let mut index = 0usize;
    let mut nodes = Vec::new();
    for verb in &path.verbs {
        match verb {
            PathVerb::Move | PathVerb::Line => {
                nodes.push(path.points[index]);
                index += 1;
            }
            PathVerb::Quad => {
                index += 1;
                nodes.push(path.points[index]);
                index += 1;
            }
            PathVerb::Cubic => {
                index += 2;
                nodes.push(path.points[index]);
                index += 1;
            }
            PathVerb::Close => return Err(GeometryError::AreaBandMismatch),
        }
    }
    Ok(nodes)
}

fn strictly_increasing_x(nodes: &[Point]) -> bool {
    nodes
        .windows(2)
        .all(|pair| pair[0].x.is_finite() && pair[1].x.is_finite() && pair[1].x > pair[0].x)
}

struct ReversedOpen {
    verbs: Vec<PathVerb>,
    points: Vec<Point>,
}

fn reverse_open_segments(
    start: Point,
    rest_points: &[Point],
    rest_verbs: &[PathVerb],
) -> Result<ReversedOpen, GeometryError> {
    let mut nodes = vec![start];
    let mut index = 0usize;
    let mut segments = Vec::new();
    for verb in rest_verbs {
        let count = verb.point_count();
        let slice = rest_points
            .get(index..index + count)
            .ok_or(GeometryError::AreaBandMismatch)?;
        nodes.push(*slice.last().expect("segment ends on a point"));
        segments.push((*verb, slice.to_vec()));
        index += count;
    }
    if index != rest_points.len() {
        return Err(GeometryError::AreaBandMismatch);
    }
    let mut verbs = Vec::new();
    let mut points = Vec::new();
    for (segment_index, (verb, segment_points)) in segments.into_iter().enumerate().rev() {
        let dest = nodes[segment_index];
        match verb {
            PathVerb::Line => {
                verbs.push(PathVerb::Line);
                points.push(dest);
            }
            PathVerb::Quad => {
                verbs.push(PathVerb::Quad);
                points.push(segment_points[0]);
                points.push(dest);
            }
            PathVerb::Cubic => {
                verbs.push(PathVerb::Cubic);
                points.push(segment_points[1]);
                points.push(segment_points[0]);
                points.push(dest);
            }
            PathVerb::Move | PathVerb::Close => return Err(GeometryError::AreaBandMismatch),
        }
    }
    Ok(ReversedOpen { verbs, points })
}

fn lerp(from: f64, to: f64, progress: f64) -> f64 {
    from + (to - from) * progress
}

fn geometry_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> PathData {
        PathData::new(
            vec![PathVerb::Move, PathVerb::Line, PathVerb::Line],
            vec![
                Point::new(0.0, 0.0),
                Point::new(10.0, 0.0),
                Point::new(10.0, 10.0),
            ],
        )
        .unwrap()
    }

    /// Batch sampling must match individual point and tangent queries exactly.
    #[test]
    fn batch_sampling_is_pointwise_identical_to_sampling_one_at_a_time() {
        let path = line();
        let progresses = [0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0];
        let batch = path.samples_at(&progresses).unwrap();
        assert_eq!(batch.len(), progresses.len());
        for (progress, (point, tangent)) in progresses.iter().zip(batch) {
            assert_eq!(point, path.point_at(*progress).unwrap());
            assert_eq!(tangent, path.tangent_at(*progress).unwrap());
        }
    }

    #[test]
    fn one_arc_length_truth_drives_length_point_tangent_and_trim() {
        let path = line();
        assert_eq!(path.path_length().unwrap(), 20.0);
        assert_eq!(path.point_at(0.25).unwrap(), Point::new(5.0, 0.0));
        assert_eq!(path.tangent_at(0.25).unwrap(), Vec2::RIGHT);
        let trimmed = path.trim(0.25, 0.75).unwrap();
        assert_eq!(
            trimmed.verbs,
            vec![PathVerb::Move, PathVerb::Line, PathVerb::Line]
        );
        assert_eq!(
            trimmed.points,
            vec![
                Point::new(5.0, 0.0),
                Point::new(10.0, 0.0),
                Point::new(10.0, 5.0)
            ]
        );
    }

    #[test]
    fn morph_requires_identical_topology_and_clamps_progress() {
        let from = line();
        let mut to = line();
        to.points[1].y = 20.0;
        assert_eq!(
            from.morph(&to, 0.5).unwrap().points[1],
            Point::new(10.0, 10.0)
        );
        let incompatible = PathData::new(
            vec![PathVerb::Move, PathVerb::Line],
            vec![Point::new(0.0, 0.0), Point::new(1.0, 1.0)],
        )
        .unwrap();
        assert_eq!(
            from.morph(&incompatible, 0.5),
            Err(GeometryError::TopologyMismatch)
        );
    }

    #[test]
    fn curves_use_the_same_exact_version_fixed_sampling() {
        let path = PathData::new(
            vec![PathVerb::Move, PathVerb::Cubic],
            vec![
                Point::new(0.0, 0.0),
                Point::new(0.0, 10.0),
                Point::new(10.0, 10.0),
                Point::new(10.0, 0.0),
            ],
        )
        .unwrap();
        let middle = path.point_at(0.5).unwrap();
        assert!((middle.x - 5.0).abs() < 1e-9, "{middle:?}");
        assert!((middle.y - 7.5).abs() < 1e-9, "{middle:?}");
        assert!(path.path_length().unwrap() > 19.9);
    }

    #[test]
    fn partial_curve_trim_preserves_bezier_and_shares_its_endpoint_with_sampling() {
        let path = PathData::cubic(
            Point::new(0.0, 0.0),
            Point::new(0.0, 10.0),
            Point::new(10.0, 10.0),
            Point::new(10.0, 0.0),
        )
        .unwrap();
        let progress = 0.37;
        let trimmed = path.trim(0.0, progress).unwrap();
        let sampled = path.point_at(progress).unwrap();
        let endpoint = trimmed.points.last().unwrap();

        assert_eq!(trimmed.verbs, vec![PathVerb::Move, PathVerb::Cubic]);
        assert!((endpoint.x - sampled.x).abs() < 1e-12);
        assert!((endpoint.y - sampled.y).abs() < 1e-12);
        let trimmed_tangent = trimmed.tangent_at(1.0).unwrap();
        let sampled_tangent = path.tangent_at(progress).unwrap();
        assert!((trimmed_tangent.x - sampled_tangent.x).abs() < 1e-12);
        assert!((trimmed_tangent.y - sampled_tangent.y).abs() < 1e-12);
    }

    #[test]
    fn curve_trim_does_not_switch_to_a_different_geometry_at_completion() {
        let path = PathData::cubic(
            Point::new(0.0, 0.0),
            Point::new(0.0, 10.0),
            Point::new(10.0, 10.0),
            Point::new(10.0, 0.0),
        )
        .unwrap();

        assert_eq!(path.trim(0.0, 0.999).unwrap().verbs, path.verbs);
        assert_eq!(path.trim(0.0, 1.0).unwrap().verbs, path.verbs);
        assert_eq!(path.tangent_at(0.5).unwrap(), Vec2::RIGHT);
    }

    #[test]
    fn constructors_area_offset_and_motion_angle_are_frame_pure() {
        let line = PathData::line(vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)]).unwrap();
        let area = line.area(10.0).unwrap();
        assert_eq!(area.verbs.last(), Some(&PathVerb::Close));
        assert_eq!(area.points.first(), Some(&Point::new(0.0, 10.0)));
        assert_eq!(area.points.last(), Some(&Point::new(10.0, 10.0)));

        let offset = line.offset_path(2.0).unwrap();
        assert_eq!(
            offset.points,
            vec![Point::new(0.0, 2.0), Point::new(10.0, 2.0)]
        );
        assert_eq!(line.angle_at(0.5).unwrap(), 0.0);

        let cubic = PathData::cubic(
            Point::new(0.0, 0.0),
            Point::new(0.0, 10.0),
            Point::new(10.0, 10.0),
            Point::new(10.0, 0.0),
        )
        .unwrap();
        assert_eq!(cubic.verbs, vec![PathVerb::Move, PathVerb::Cubic]);

        let arc = PathData::arc(Point::new(0.0, 0.0), 10.0, 0.0, core::f64::consts::PI).unwrap();
        assert_eq!(arc.verbs.len(), PATH_ARC_SEGMENTS + 1);
        assert_eq!(arc.points.len(), 1 + PATH_ARC_SEGMENTS * 3);
        assert!((arc.points.last().unwrap().x + 10.0).abs() < 1e-12);
    }

    fn sector_verbs() -> Vec<PathVerb> {
        let mut verbs = vec![
            PathVerb::Move,
            PathVerb::Cubic,
            PathVerb::Line,
            PathVerb::Cubic,
        ];
        verbs.extend(std::iter::repeat_n(
            PathVerb::Cubic,
            PATH_SECTOR_ARC_SEGMENTS,
        ));
        verbs.extend([PathVerb::Cubic, PathVerb::Line, PathVerb::Cubic]);
        verbs.extend(std::iter::repeat_n(
            PathVerb::Cubic,
            PATH_SECTOR_ARC_SEGMENTS,
        ));
        verbs.push(PathVerb::Close);
        verbs
    }

    #[test]
    fn sector_keeps_one_verb_table_across_degeneracies() {
        let center = Point::new(40.0, 50.0);
        let expected = sector_verbs();
        assert_eq!(expected.len(), PATH_SECTOR_VERBS);
        let cases = [
            PathData::sector(center, 0.0, 20.0, 0.0, 1.2, 0.0).unwrap(),
            PathData::sector(center, 8.0, 20.0, 0.0, 1.2, 4.0).unwrap(),
            PathData::sector(center, 8.0, 20.0, 0.0, 0.0, 4.0).unwrap(),
            PathData::sector(center, 8.0, 8.0, 0.0, 1.2, 0.0).unwrap(),
            PathData::sector(center, 0.0, 20.0, 0.0, core::f64::consts::TAU, 0.0).unwrap(),
            PathData::sector(center, 6.0, 20.0, 0.0, core::f64::consts::TAU, 3.0).unwrap(),
            PathData::sector(center, 0.0, 0.0, 0.2, 1.1, 0.0).unwrap(),
            PathData::sector(center, 4.0, 18.0, 1.1, 0.2, 2.0).unwrap(),
        ];
        for path in &cases {
            assert_eq!(path.verbs, expected);
            assert_eq!(path.points.len(), PATH_SECTOR_POINTS);
            assert!(
                path.points
                    .iter()
                    .all(|point| point.x.is_finite() && point.y.is_finite())
            );
        }
        assert!(cases[0].has_same_topology(&cases[1]));
        assert!(cases[0].has_same_topology(&cases[4]));
        let pie = &cases[0];
        assert_eq!(pie.points[0], center);
        let ring = &cases[1];
        let hole = ring.points[0];
        let dx = hole.x - center.x;
        let dy = hole.y - center.y;
        let inner = (dx * dx + dy * dy).sqrt();
        assert!((inner - 8.0).abs() < 1e-6, "{inner}");
    }

    #[test]
    fn area_keeps_cubic_verbs_from_the_source_curve() {
        let curve = PathData::cubic(
            Point::new(0.0, 4.0),
            Point::new(4.0, 0.0),
            Point::new(8.0, 8.0),
            Point::new(12.0, 4.0),
        )
        .unwrap();
        let area = curve.area(10.0).unwrap();
        assert_eq!(
            area.verbs,
            vec![
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Cubic,
                PathVerb::Line,
                PathVerb::Close,
            ]
        );
        assert_eq!(area.points[0], Point::new(0.0, 10.0));
        assert_eq!(area.points[1], Point::new(0.0, 4.0));
        assert_eq!(area.points[2], Point::new(4.0, 0.0));
        assert_eq!(area.points[3], Point::new(8.0, 8.0));
        assert_eq!(area.points[4], Point::new(12.0, 4.0));
        assert_eq!(area.points[5], Point::new(12.0, 10.0));
        let trimmed = area.trim(0.0, 0.4).unwrap();
        let trimmed_end = trimmed.point_at(1.0).unwrap();
        let sampled = area.point_at(0.4).unwrap();
        assert!((trimmed_end.x - sampled.x).abs() < 1e-12);
        assert!((trimmed_end.y - sampled.y).abs() < 1e-12);
    }

    #[test]
    fn rounded_sectors_are_continuous_at_animation_boundaries() {
        let epsilon = 1e-6;
        let tau = core::f64::consts::TAU;
        for direction in [-1.0, 1.0] {
            for start in [0.0, -core::f64::consts::FRAC_PI_2] {
                // (inner, outer, sweep, corner): holes, thickness, sweeps and corners can
                // all reach zero during an ordinary animation without changing the outline.
                for (from, to) in [
                    ((0.0, 100.0, 0.2, 10.0), (epsilon, 100.0, 0.2, 10.0)),
                    ((0.0, 100.0, 1.5, 10.0), (epsilon, 100.0, 1.5, 10.0)),
                    ((0.0, 100.0, 4.5, 10.0), (epsilon, 100.0, 4.5, 10.0)),
                    ((50.0, 50.0, 1.5, 10.0), (50.0, 50.0 + epsilon, 1.5, 10.0)),
                    ((50.0, 100.0, 0.0, 10.0), (50.0, 100.0, epsilon, 10.0)),
                    ((50.0, 100.0, tau, 10.0), (50.0, 100.0, tau - epsilon, 10.0)),
                    ((0.0, 100.0, tau, 10.0), (0.0, 100.0, tau - epsilon, 10.0)),
                    ((50.0, 100.0, 1.5, 0.0), (50.0, 100.0, 1.5, epsilon)),
                ] {
                    let make = |(inner, outer, sweep, corner)| {
                        PathData::sector(
                            Point::new(0.0, 0.0),
                            inner,
                            outer,
                            start,
                            start + direction * sweep,
                            corner,
                        )
                        .unwrap()
                    };
                    let a = make(from);
                    let b = make(to);
                    assert!(a.has_same_topology(&b));
                    let displacement = a
                        .points
                        .iter()
                        .zip(&b.points)
                        .map(|(a, b)| distance(*a, *b))
                        .fold(0.0, f64::max);
                    assert!(
                        displacement < 0.01,
                        "jump of {displacement}: {from:?} -> {to:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn rounded_full_turn_has_no_seam_notch_and_preserves_the_hole() {
        use geo::Contains;
        for inner in [0.0, 50.0] {
            for direction in [-1.0, 1.0] {
                let make = |corner| {
                    PathData::sector(
                        Point::new(0.0, 0.0),
                        inner,
                        100.0,
                        0.0,
                        direction * core::f64::consts::TAU,
                        corner,
                    )
                    .unwrap()
                };
                let rounded = make(10.0);
                assert_eq!(rounded, make(0.0));
                let polygon = single_polygon(&rounded).unwrap();
                for y in [-0.1, 0.1] {
                    assert!(polygon.contains(&geo::Point::new(95.0, y)));
                    assert!(polygon.contains(&geo::Point::new(51.0, y)));
                }
                if inner > 0.0 {
                    assert!(!polygon.contains(&geo::Point::new(1.0, 1.0)));
                }
            }
        }
    }

    #[test]
    fn area_band_reverses_cubic_controls_and_rejects_misaligned_inputs() {
        let upper = PathData::cubic(
            Point::new(0.0, 2.0),
            Point::new(4.0, 0.0),
            Point::new(8.0, 0.0),
            Point::new(12.0, 2.0),
        )
        .unwrap();
        let lower = PathData::cubic(
            Point::new(0.0, 8.0),
            Point::new(4.0, 10.0),
            Point::new(8.0, 10.0),
            Point::new(12.0, 8.0),
        )
        .unwrap();
        let band = PathData::area_band(&upper, &lower).unwrap();
        assert_eq!(
            band.verbs,
            vec![
                PathVerb::Move,
                PathVerb::Cubic,
                PathVerb::Line,
                PathVerb::Cubic,
                PathVerb::Close,
            ]
        );
        assert_eq!(band.points[0], Point::new(0.0, 2.0));
        assert_eq!(band.points[4], Point::new(12.0, 8.0));
        assert_eq!(band.points[5], Point::new(8.0, 10.0));
        assert_eq!(band.points[6], Point::new(4.0, 10.0));
        assert_eq!(band.points[7], Point::new(0.0, 8.0));
        let flat = PathData::line(vec![
            Point::new(0.0, 2.0),
            Point::new(6.0, 2.0),
            Point::new(12.0, 2.0),
        ])
        .unwrap();
        assert_eq!(
            PathData::area_band(&upper, &flat),
            Err(GeometryError::AreaBandMismatch)
        );
        let decreasing = PathData::line(vec![Point::new(12.0, 2.0), Point::new(0.0, 2.0)]).unwrap();
        assert_eq!(
            PathData::area_band(&decreasing, &decreasing),
            Err(GeometryError::AreaBandMismatch)
        );
        let shifted = PathData::line(vec![
            Point::new(100.0, 4.0),
            Point::new(106.0, 4.0),
            Point::new(112.0, 4.0),
        ])
        .unwrap();
        let middle_shifted = PathData::line(vec![
            Point::new(0.0, 4.0),
            Point::new(7.0, 4.0),
            Point::new(12.0, 4.0),
        ])
        .unwrap();
        for lower in [&shifted, &middle_shifted] {
            assert_eq!(
                PathData::area_band(&flat, lower),
                Err(GeometryError::AreaBandMismatch)
            );
        }
        let mut shifted_curve = lower.clone();
        shifted_curve.points.last_mut().unwrap().x += 1.0;
        assert_eq!(
            PathData::area_band(&upper, &shifted_curve),
            Err(GeometryError::AreaBandMismatch)
        );
        assert!(
            PathData::area_band(&upper, &upper).is_ok(),
            "zero bandwidth is an animation endpoint"
        );
    }

    #[test]
    fn prepare_time_boolean_freezes_variable_topology_as_path_data() {
        let rectangle = |x0: f64, y0: f64, x1: f64, y1: f64| {
            PathData::new(
                vec![
                    PathVerb::Move,
                    PathVerb::Line,
                    PathVerb::Line,
                    PathVerb::Line,
                    PathVerb::Close,
                ],
                vec![
                    Point::new(x0, y0),
                    Point::new(x1, y0),
                    Point::new(x1, y1),
                    Point::new(x0, y1),
                ],
            )
            .unwrap()
        };
        let left = rectangle(0.0, 0.0, 10.0, 10.0);
        let right = rectangle(5.0, 0.0, 15.0, 10.0);
        for op in [
            PathBooleanOp::Union,
            PathBooleanOp::Intersection,
            PathBooleanOp::Difference,
            PathBooleanOp::Xor,
        ] {
            let result = left.boolean(&right, op).unwrap();
            result.validate().unwrap();
            assert!(!result.verbs.is_empty());
        }

        let disjoint = rectangle(20.0, 20.0, 30.0, 30.0);
        let empty = left
            .boolean(&disjoint, PathBooleanOp::Intersection)
            .unwrap();
        assert!(empty.verbs.is_empty());
        empty.validate().unwrap();
    }
}
