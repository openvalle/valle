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

    /// Close one flattened line/curve against a horizontal baseline.
    pub fn area(&self, baseline_y: f64) -> Result<Self, GeometryError> {
        if !baseline_y.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        let flattened = FlattenedPath::from_path(self)?;
        if flattened.contours.len() != 1 || flattened.contours[0].points.len() < 2 {
            return Err(GeometryError::SingleContourRequired);
        }
        let contour = &flattened.contours[0].points;
        let mut points = Vec::with_capacity(contour.len() + 2);
        points.push(Point::new(contour[0].x, baseline_y));
        points.extend(contour.iter().copied());
        points.push(Point::new(contour[contour.len() - 1].x, baseline_y));
        let mut verbs = Vec::with_capacity(points.len() + 1);
        verbs.push(PathVerb::Move);
        verbs.extend(std::iter::repeat_n(PathVerb::Line, points.len() - 1));
        verbs.push(PathVerb::Close);
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
    SingleContourRequired,
    ClosedContourRequired,
    DegenerateContour,
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
            GeometryError::SingleContourRequired => {
                f.write_str("geometry operation requires exactly one contour")
            }
            GeometryError::ClosedContourRequired => {
                f.write_str("path boolean requires one closed contour per operand")
            }
            GeometryError::DegenerateContour => f.write_str("geometry contour is degenerate"),
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
