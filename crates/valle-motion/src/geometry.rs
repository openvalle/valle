//! Typed, deterministic geometry shared by Motion expression evaluation and layout emission.
//!
//! Curves are measured with an exact-version fixed arc-length table while sampling and trimming
//! stay on the authored Line/Quad/Cubic segments. This is deliberately not a renderer service:
//! Native and WASM must derive trim, length, points and tangents from the same [`PathData`] bytes
//! before either backend sees a ProgramRecording.

use geo::algorithm::kernels::{Kernel, Orientation, RobustKernel};
use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon};
use serde::{Deserialize, Serialize};
use valle_draw::{PathVerb, Point, Vec2};

mod morph_anchor;
mod morph_auto_pairs;
mod morph_check;
mod morph_compatible;
mod morph_pairs;
mod stroke;

pub use morph_compatible::CompatibleBarycentricMorph;

/// Maximum authored points in one path value. This protects Artifact decoding and trajectory size.
pub const MAX_PATH_POINTS: usize = 16_384;
/// Maximum points admitted in a per-frame path expression such as morph.
pub const MAX_FRAME_GEOMETRY_POINTS: usize = 2_048;
/// Angular samples for guaranteed star-shaped morphs prepared before frame evaluation.
pub const POLAR_MORPH_POINTS: usize = 256;
/// Fixed samples per contour for frame-evaluated shape modifiers.
pub const PATH_MODIFIER_SAMPLES: usize = 256;
/// Fixed centerline samples for stroke expansion (two offset sides per contour).
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MorphMethod {
    Auto,
    Convex,
    Polar,
    ArcLength,
}

/// A nonfatal zero-width contact found over one adjacent key-shape interval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MorphContact {
    pub segment: usize,
    pub detail: String,
}

/// Corresponding boundary and interior vertices with the same triangle connectivity.
/// This is an endpoint mesh; interpolating its coordinates linearly does not
/// itself guarantee a simple polygon at intermediate times.
#[derive(Debug, Clone, PartialEq)]
pub struct CompatibleMorphMesh {
    /// Boundary vertices come first, in counterclockwise order.
    pub boundary_vertices: usize,
    pub from_vertices: Vec<Point>,
    pub to_vertices: Vec<Point>,
    pub triangles: Vec<[usize; 3]>,
}

/// A compatible triangulation of the full plane region inside a fixed convex
/// outer triangle. The shape outline is an interior cycle of this mesh.
/// This supplies the fixed boundary needed for barycentric morph interpolation;
/// the interpolation itself is not encoded here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnclosedCompatibleMorphMesh {
    pub from_vertices: Vec<Point>,
    pub to_vertices: Vec<Point>,
    /// Indices of the fixed convex outer triangle, counterclockwise.
    pub outer_vertices: [usize; 3],
    /// Indices of the shape outline, counterclockwise.
    pub outline_vertices: Vec<usize>,
    pub triangles: Vec<[usize; 3]>,
}

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

#[derive(Debug)]
pub(crate) struct NestedClosedContour {
    pub points: Vec<Point>,
    pub parent: Option<usize>,
    pub depth: usize,
}

impl PathData {
    /// Find a common triangulation using only corresponding boundary vertices.
    /// `None` includes unsupported inputs, geometry budgets, and pairs that need
    /// Steiner vertices or a different correspondence.
    pub fn direct_compatible_morph_mesh(
        &self,
        other: &Self,
    ) -> Result<Option<CompatibleMorphMesh>, GeometryError> {
        morph_compatible::prepare_direct(self, other)
    }

    /// Build a compatible endpoint mesh for two single, simple, closed polygons
    /// with the same boundary vertex count. When necessary, refine their
    /// triangulations in a common convex reference polygon using exact rational
    /// intersections. Refinement currently accepts up to 64 boundary vertices;
    /// the direct path accepts up to 256.
    pub fn compatible_morph_mesh(
        &self,
        other: &Self,
    ) -> Result<Option<CompatibleMorphMesh>, GeometryError> {
        morph_compatible::prepare_refined(self, other)
    }

    /// Embed two compatible outlines inside a shared fixed convex boundary.
    /// The outlines must be single simple closed polygons. Shorter boundaries
    /// gain collinear edge points to match vertex counts without changing their
    /// geometry. The method may return `None` when triangulation or geometry
    /// budgets cannot be met. It prepares a mesh; no morph trajectory is
    /// guaranteed until barycentric interpolation is applied.
    pub fn enclosed_compatible_morph_mesh(
        &self,
        other: &Self,
    ) -> Result<Option<EnclosedCompatibleMorphMesh>, GeometryError> {
        morph_compatible::prepare_enclosed(self, other)
    }

    /// Prepare a fixed-boundary convex-combination morph for two single simple
    /// closed polygons. Returns `None` if
    /// triangulation or numerical conditioning prevents a supported result.
    pub fn prepare_compatible_barycentric_morph(
        &self,
        other: &Self,
    ) -> Result<Option<CompatibleBarycentricMorph>, GeometryError> {
        Ok(self
            .enclosed_compatible_morph_mesh(other)?
            .and_then(CompatibleBarycentricMorph::from_mesh))
    }

    /// Prepare adjacent compatible morphs on one shared boundary vertex count
    /// for a key-shape sequence.
    pub fn prepare_compatible_barycentric_sequence(
        paths: &[Self],
    ) -> Result<Option<Vec<CompatibleBarycentricMorph>>, GeometryError> {
        morph_compatible::prepare_sequence(paths)
    }

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

    /// Resample every contour at uniform arc-length intervals. Closed contours use `count`
    /// distinct points and a Close verb; open contours include both endpoints. Curve flattening
    /// subdivides until the control-polygon length bounds the chord error below 0.025%.
    pub fn resample(&self, count: usize) -> Result<Self, GeometryError> {
        let contours = adaptive_flatten(self)?;
        if contours.is_empty()
            || count < 2
            || contours.iter().any(|contour| contour.closed && count < 3)
            || count.saturating_mul(contours.len()) > MAX_PATH_POINTS
        {
            return Err(GeometryError::TooComplex);
        }
        let mut verbs = Vec::with_capacity(count * contours.len() + contours.len());
        let mut points = Vec::with_capacity(count * contours.len());
        for contour in &contours {
            let samples = resample_contour(contour, count)?;
            verbs.push(PathVerb::Move);
            verbs.extend(std::iter::repeat_n(PathVerb::Line, count - 1));
            if contour.closed {
                verbs.push(PathVerb::Close);
            }
            points.extend(samples);
        }
        Self::new(verbs, points)
    }

    /// Reverse the drawing direction of every contour without flattening its curves.
    pub fn reverse(&self) -> Result<Self, GeometryError> {
        self.validate()?;
        let mut verbs = Vec::with_capacity(self.verbs.len());
        let mut points = Vec::with_capacity(self.points.len());
        let mut verb_at = 0usize;
        let mut point_at = 0usize;
        while verb_at < self.verbs.len() {
            if self.verbs[verb_at] != PathVerb::Move {
                return Err(GeometryError::MissingMove);
            }
            let start = self.points[point_at];
            verb_at += 1;
            point_at += 1;
            let segment_verb_start = verb_at;
            let segment_point_start = point_at;
            while verb_at < self.verbs.len()
                && !matches!(self.verbs[verb_at], PathVerb::Move | PathVerb::Close)
            {
                point_at += self.verbs[verb_at].point_count();
                verb_at += 1;
            }
            let rest_verbs = &self.verbs[segment_verb_start..verb_at];
            let rest_points = &self.points[segment_point_start..point_at];
            let end = rest_points.last().copied().unwrap_or(start);
            verbs.push(PathVerb::Move);
            points.push(end);
            let reversed = reverse_open_segments(start, rest_points, rest_verbs)?;
            verbs.extend(reversed.verbs);
            points.extend(reversed.points);
            if self.verbs.get(verb_at) == Some(&PathVerb::Close) {
                verbs.push(PathVerb::Close);
                verb_at += 1;
            }
        }
        Self::new(verbs, points)
    }

    /// Round the vertices of a line-only path. Every input corner emits the same Line/Cubic
    /// commands at all radii, including zero, so animating the radius keeps fixed topology.
    pub fn round_corners(&self, radius: f64) -> Result<Self, GeometryError> {
        self.validate()?;
        if !radius.is_finite() || radius < 0.0 {
            return Err(GeometryError::InvalidModifier);
        }
        let mut verbs = Vec::new();
        let mut points = Vec::new();
        let mut verb_at = 0;
        let mut point_at = 0;
        while verb_at < self.verbs.len() {
            if self.verbs[verb_at] != PathVerb::Move {
                return Err(GeometryError::InvalidModifier);
            }
            let mut contour = vec![self.points[point_at]];
            point_at += 1;
            verb_at += 1;
            while self.verbs.get(verb_at) == Some(&PathVerb::Line) {
                contour.push(self.points[point_at]);
                point_at += 1;
                verb_at += 1;
            }
            let closed = self.verbs.get(verb_at) == Some(&PathVerb::Close);
            if closed {
                verb_at += 1;
                if contour.len() > 1 && contour[0] == *contour.last().unwrap() {
                    contour.pop();
                }
            }
            if contour.len() < if closed { 3 } else { 2 } {
                return Err(GeometryError::TooFewPoints);
            }
            let corner = |index: usize| {
                let n = contour.len();
                let before = contour[(index + n - 1) % n];
                let vertex = contour[index];
                let after = contour[(index + 1) % n];
                let left = distance(before, vertex);
                let right = distance(vertex, after);
                let dot = if left > 0.0 && right > 0.0 {
                    ((before.x - vertex.x) * (after.x - vertex.x)
                        + (before.y - vertex.y) * (after.y - vertex.y))
                        / (left * right)
                } else {
                    -1.0
                };
                let angle = libm::acos(dot.clamp(-1.0, 1.0));
                let tangent = libm::tan(angle * 0.5);
                let cut = if tangent > 1e-9 {
                    (radius / tangent).min(left * 0.5).min(right * 0.5)
                } else {
                    0.0
                };
                let handle_ratio = if cut > 0.0 {
                    (4.0 / 3.0) * libm::tan((core::f64::consts::PI - angle) * 0.25) * tangent
                } else {
                    0.0
                };
                let incoming = if left > 0.0 {
                    Point::new(
                        vertex.x + (before.x - vertex.x) * cut / left,
                        vertex.y + (before.y - vertex.y) * cut / left,
                    )
                } else {
                    vertex
                };
                let outgoing = if right > 0.0 {
                    Point::new(
                        vertex.x + (after.x - vertex.x) * cut / right,
                        vertex.y + (after.y - vertex.y) * cut / right,
                    )
                } else {
                    vertex
                };
                let control1 = Point::new(
                    incoming.x + (vertex.x - incoming.x) * handle_ratio,
                    incoming.y + (vertex.y - incoming.y) * handle_ratio,
                );
                let control2 = Point::new(
                    outgoing.x + (vertex.x - outgoing.x) * handle_ratio,
                    outgoing.y + (vertex.y - outgoing.y) * handle_ratio,
                );
                (incoming, control1, control2, outgoing)
            };
            if closed {
                let first = corner(0);
                verbs.push(PathVerb::Move);
                points.push(first.3);
                for index in 1..contour.len() {
                    let (incoming, control1, control2, outgoing) = corner(index);
                    verbs.extend([PathVerb::Line, PathVerb::Cubic]);
                    points.extend([incoming, control1, control2, outgoing]);
                }
                verbs.extend([PathVerb::Line, PathVerb::Cubic, PathVerb::Close]);
                points.extend([first.0, first.1, first.2, first.3]);
            } else {
                verbs.push(PathVerb::Move);
                points.push(contour[0]);
                for index in 1..contour.len() - 1 {
                    let (incoming, control1, control2, outgoing) = corner(index);
                    verbs.extend([PathVerb::Line, PathVerb::Cubic]);
                    points.extend([incoming, control1, control2, outgoing]);
                }
                verbs.push(PathVerb::Line);
                points.push(*contour.last().unwrap());
            }
        }
        Self::new(verbs, points)
    }

    /// Replace one open contour with an alternating sequence of ridge peaks. Resampling fixes
    /// the number of output points independently of the input segment lengths.
    pub fn zigzag(&self, size: f64, ridges: usize) -> Result<Self, GeometryError> {
        if !size.is_finite() || size < 0.0 || ridges == 0 || ridges > 1023 {
            return Err(GeometryError::InvalidModifier);
        }
        let contours = adaptive_flatten(self)?;
        if contours.len() != 1 || contours[0].closed {
            return Err(GeometryError::InvalidModifier);
        }
        let samples = resample_contour(&contours[0], 2 * ridges + 1)?;
        let mut points = Vec::with_capacity(samples.len());
        for index in 0..samples.len() {
            let mut point = samples[index];
            if index % 2 == 1 {
                let from = samples[index - 1];
                let to = samples[index + 1];
                let dx = to.x - from.x;
                let dy = to.y - from.y;
                let length = valle_draw::math::sqrt(dx * dx + dy * dy);
                if length > 0.0 {
                    let side = if (index / 2) % 2 == 0 { 1.0 } else { -1.0 };
                    point.x -= side * size * dy / length;
                    point.y += side * size * dx / length;
                }
            }
            points.push(point);
        }
        let mut verbs = vec![PathVerb::Move];
        verbs.extend(std::iter::repeat_n(PathVerb::Line, points.len() - 1));
        Self::new(verbs, points)
    }

    /// Displace uniformly sampled vertices using the shared deterministic 2D value-noise field.
    /// Recenter each contour after displacement so phase animation does not add a rigid drift.
    pub fn noise_displace(
        &self,
        seed: u64,
        amount: f64,
        frequency: f64,
        phase: f64,
    ) -> Result<Self, GeometryError> {
        if !amount.is_finite()
            || amount < 0.0
            || !frequency.is_finite()
            || frequency <= 0.0
            || !phase.is_finite()
        {
            return Err(GeometryError::InvalidModifier);
        }
        let contours = adaptive_flatten(self)?;
        if contours.is_empty() || contours.len() * POLAR_MORPH_POINTS > MAX_FRAME_GEOMETRY_POINTS {
            return Err(GeometryError::TooComplex);
        }
        let mut verbs = Vec::new();
        let mut points = Vec::new();
        for contour in &contours {
            let source = resample_contour(contour, POLAR_MORPH_POINTS)?;
            let center = contour_centroid(&source, contour.closed);
            let extent = source
                .iter()
                .map(|point| distance(center, *point))
                .fold(0.0_f64, f64::max);
            if extent <= 0.0 {
                return Err(GeometryError::DegenerateContour);
            }
            let mut displaced = source
                .iter()
                .map(|point| {
                    let dx = point.x - center.x;
                    let dy = point.y - center.y;
                    let radial = valle_draw::math::sqrt(dx * dx + dy * dy);
                    let noise = crate::compute::noise::value_noise_2d(
                        seed,
                        dx / extent * frequency + phase * 0.61,
                        dy / extent * frequency + phase * 0.83,
                    );
                    let offset = amount * (noise * 2.0 - 1.0);
                    if radial > 0.0 {
                        Point::new(
                            point.x + dx / radial * offset,
                            point.y + dy / radial * offset,
                        )
                    } else {
                        *point
                    }
                })
                .collect::<Vec<_>>();
            let shifted = contour_centroid(&displaced, contour.closed);
            for point in &mut displaced {
                point.x += center.x - shifted.x;
                point.y += center.y - shifted.y;
            }
            verbs.push(PathVerb::Move);
            verbs.extend(std::iter::repeat_n(PathVerb::Line, displaced.len() - 1));
            if contour.closed {
                verbs.push(PathVerb::Close);
            }
            points.extend(displaced);
        }
        Self::new(verbs, points)
    }

    /// Pull radial extrema toward the mean radius for negative amounts and amplify them for
    /// positive amounts. Zero keeps the sampled outline; the output topology is frame-stable.
    pub fn pucker_bloat(&self, amount: f64) -> Result<Self, GeometryError> {
        if !amount.is_finite() || !(-1.0..=1.0).contains(&amount) {
            return Err(GeometryError::InvalidModifier);
        }
        let contours = fixed_modifier_contours(self, PATH_MODIFIER_SAMPLES)?;
        let mut output = Vec::with_capacity(contours.len());
        for (source, closed) in contours {
            if !closed {
                return Err(GeometryError::ClosedContourRequired);
            }
            let center = contour_centroid(&source, true);
            let radii = source
                .iter()
                .map(|point| distance(center, *point))
                .collect::<Vec<_>>();
            let mean = radii.iter().sum::<f64>() / radii.len() as f64;
            let points = source
                .iter()
                .zip(radii)
                .map(|(point, radius)| {
                    if radius <= 0.0 {
                        return *point;
                    }
                    let target = (mean + (radius - mean) * (1.0 + amount)).max(0.001);
                    Point::new(
                        center.x + (point.x - center.x) * target / radius,
                        center.y + (point.y - center.y) * target / radius,
                    )
                })
                .collect();
            output.push((points, true));
        }
        path_from_sampled_contours(output)
    }

    /// Rotate each sampled vertex about the contour center by an angle that grows with radius.
    pub fn twist(&self, angle: f64) -> Result<Self, GeometryError> {
        if !angle.is_finite() {
            return Err(GeometryError::InvalidModifier);
        }
        let contours = fixed_modifier_contours(self, PATH_MODIFIER_SAMPLES)?;
        let mut output = Vec::with_capacity(contours.len());
        for (source, closed) in contours {
            let center = contour_centroid(&source, closed);
            let extent = source
                .iter()
                .map(|point| distance(center, *point))
                .fold(0.0_f64, f64::max);
            if extent <= 0.0 {
                return Err(GeometryError::DegenerateContour);
            }
            let points = source
                .iter()
                .map(|point| {
                    let dx = point.x - center.x;
                    let dy = point.y - center.y;
                    let radial = valle_draw::math::sqrt(dx * dx + dy * dy) / extent;
                    let (sin, cos) = valle_draw::math::sin_cos(angle * radial);
                    Point::new(
                        center.x + dx * cos - dy * sin,
                        center.y + dx * sin + dy * cos,
                    )
                })
                .collect();
            output.push((points, closed));
        }
        path_from_sampled_contours(output)
    }

    /// Simplify a contour with a distance tolerance, then sample the resulting outline back to
    /// a fixed point count. This simplifies its visible shape without changing frame topology.
    pub fn simplify(&self, tolerance: f64) -> Result<Self, GeometryError> {
        if !tolerance.is_finite() || tolerance < 0.0 {
            return Err(GeometryError::InvalidModifier);
        }
        let contours = fixed_modifier_contours(self, PATH_MODIFIER_SAMPLES)?;
        let mut output = Vec::with_capacity(contours.len());
        for (source, closed) in contours {
            let reduced = simplify_contour(&source, closed, tolerance)?;
            let resampled = resample_contour(
                &FlattenedContour {
                    points: reduced,
                    closed,
                },
                PATH_MODIFIER_SAMPLES,
            )?;
            output.push((resampled, closed));
        }
        path_from_sampled_contours(output)
    }

    /// Expand a centerline to the union of its stroked segments with butt caps and
    /// miter joins. Overlapping inner offsets never introduce spurious holes.
    pub fn stroke_to_path(&self, width: f64) -> Result<Self, GeometryError> {
        stroke::expand(self, width)
    }

    /// Align two single, closed contours around a point inside both star-shaped kernels.
    /// Interpolating corresponding radii keeps every intermediate contour star-shaped and
    /// prevents self-intersection for this admitted class of inputs.
    pub fn polar_morph_pair(&self, other: &Self) -> Result<(Self, Self), GeometryError> {
        let left = adaptive_flatten(self)?;
        let right = adaptive_flatten(other)?;
        if left.len() != 1 || right.len() != 1 {
            return Err(GeometryError::SingleContourRequired);
        }
        if !left[0].closed || !right[0].closed {
            return Err(GeometryError::ClosedContourRequired);
        }
        let center = shared_star_kernel_point(&[&left[0].points, &right[0].points])?;
        let left = polar_samples(&left[0].points, center, POLAR_MORPH_POINTS)?;
        let right = polar_samples(&right[0].points, center, POLAR_MORPH_POINTS)?;
        let path = |points| {
            let mut verbs = vec![PathVerb::Move];
            verbs.extend(std::iter::repeat_n(PathVerb::Line, POLAR_MORPH_POINTS - 1));
            verbs.push(PathVerb::Close);
            Self::new(verbs, points)
        };
        Ok((path(left)?, path(right)?))
    }

    /// Align convex contours by the union of their edge directions. Each intermediate polygon
    /// is the boundary of `(1-t) * from + t * to`, a Minkowski convex combination, so it stays
    /// convex for all t in [0,1]. Missing edges are represented by repeated endpoint vertices.
    pub fn convex_morph_pair(&self, other: &Self) -> Result<(Self, Self), GeometryError> {
        let mut aligned = Self::convex_morph_sequence(&[self.clone(), other.clone()])?;
        let right = aligned.pop().unwrap();
        let left = aligned.pop().unwrap();
        Ok((left, right))
    }

    /// Use one edge-direction event grid for every convex key shape, preserving exact polygon
    /// corners at each keyframe while guaranteeing convexity throughout each interpolation span.
    pub fn convex_morph_sequence(paths: &[Self]) -> Result<Vec<Self>, GeometryError> {
        if !(2..=16).contains(&paths.len()) {
            return Err(GeometryError::TooComplex);
        }
        let contours = paths
            .iter()
            .map(convex_contour)
            .collect::<Result<Vec<_>, _>>()?;
        align_convex_contours(&contours)
    }

    /// Align one or more simple contours by arc length, choose cyclic starting points with
    /// the least displacement, and certify each linear interpolation over the whole [0,1].
    pub fn arc_length_morph_pair(&self, other: &Self) -> Result<(Self, Self), GeometryError> {
        self.arc_length_morph_pair_with_options(other, &[], false)
    }

    /// Prepare explicit contour correspondence, including contours born from or
    /// shrinking to a point at key shapes where their pair entry is `None`.
    pub fn paired_morph_sequence_with_options(
        paths: &[Self],
        pairs: &[Vec<Option<usize>>],
        anchors: &[Vec<Point>],
        method: MorphMethod,
        allow_self_intersection: bool,
    ) -> Result<Vec<Self>, GeometryError> {
        morph_pairs::prepare(paths, pairs, anchors, method, allow_self_intersection)
    }

    pub fn paired_morph_sequence_with_contacts(
        paths: &[Self],
        pairs: &[Vec<Option<usize>>],
        anchors: &[Vec<Point>],
        method: MorphMethod,
        allow_self_intersection: bool,
        contact_is_error: bool,
    ) -> Result<(Vec<Self>, Vec<MorphContact>), GeometryError> {
        morph_pairs::prepare_with_contacts(
            paths,
            pairs,
            anchors,
            method,
            allow_self_intersection,
            contact_is_error,
        )
    }

    /// Infer contour tracks through all key shapes, then apply the same endpoint
    /// and continuous-time checks used for explicitly authored `pairs`.
    pub fn automatic_multi_morph_sequence_with_options(
        paths: &[Self],
        anchors: &[Vec<Point>],
        method: MorphMethod,
        allow_self_intersection: bool,
    ) -> Result<Vec<Self>, GeometryError> {
        let pairs = morph_auto_pairs::infer(paths, anchors)?;
        morph_pairs::prepare(paths, &pairs, anchors, method, allow_self_intersection)
    }

    pub fn automatic_multi_morph_sequence_with_contacts(
        paths: &[Self],
        anchors: &[Vec<Point>],
        method: MorphMethod,
        allow_self_intersection: bool,
        contact_is_error: bool,
    ) -> Result<(Vec<Self>, Vec<MorphContact>), GeometryError> {
        let pairs = morph_auto_pairs::infer(paths, anchors)?;
        Self::paired_morph_sequence_with_contacts(
            paths,
            &pairs,
            anchors,
            method,
            allow_self_intersection,
            contact_is_error,
        )
    }

    pub fn arc_length_morph_pair_with_options(
        &self,
        other: &Self,
        anchors: &[Vec<Point>],
        allow_self_intersection: bool,
    ) -> Result<(Self, Self), GeometryError> {
        let mut aligned = Self::arc_length_morph_sequence_with_options(
            &[self.clone(), other.clone()],
            anchors,
            allow_self_intersection,
        )?;
        let right = aligned.pop().unwrap();
        let left = aligned.pop().unwrap();
        Ok((left, right))
    }

    pub fn arc_length_morph_pair_with_contacts(
        &self,
        other: &Self,
        anchors: &[Vec<Point>],
        allow_self_intersection: bool,
        contact_is_error: bool,
    ) -> Result<((Self, Self), Vec<MorphContact>), GeometryError> {
        let (mut aligned, contacts) = Self::arc_length_morph_sequence_with_contacts(
            &[self.clone(), other.clone()],
            anchors,
            allow_self_intersection,
            contact_is_error,
        )?;
        let right = aligned.pop().unwrap();
        let left = aligned.pop().unwrap();
        Ok(((left, right), contacts))
    }

    pub fn arc_length_morph_sequence(paths: &[Self]) -> Result<Vec<Self>, GeometryError> {
        Self::arc_length_morph_sequence_with_options(paths, &[], false)
    }

    pub fn arc_length_morph_sequence_with_options(
        paths: &[Self],
        anchors: &[Vec<Point>],
        allow_self_intersection: bool,
    ) -> Result<Vec<Self>, GeometryError> {
        Self::arc_length_morph_sequence_with_contacts(paths, anchors, allow_self_intersection, true)
            .map(|(paths, _)| paths)
    }

    pub fn arc_length_morph_sequence_with_contacts(
        paths: &[Self],
        anchors: &[Vec<Point>],
        allow_self_intersection: bool,
        contact_is_error: bool,
    ) -> Result<(Vec<Self>, Vec<MorphContact>), GeometryError> {
        if !(2..=16).contains(&paths.len()) {
            return Err(GeometryError::TooComplex);
        }
        let mut contours = Vec::with_capacity(paths.len());
        for path in paths {
            let mut flattened = adaptive_flatten(path)?;
            if flattened.len() != 1 {
                return Err(GeometryError::SingleContourRequired);
            }
            let mut contour = flattened.pop().unwrap();
            if !contour.closed {
                return Err(GeometryError::ClosedContourRequired);
            }
            let winding = morph_check::closed_contour_winding(&contour.points)
                .ok_or(GeometryError::DegenerateContour)?;
            if winding == core::cmp::Ordering::Less {
                contour.points.reverse();
            }
            contours.push(contour);
        }
        let anchored_samples = if anchors.is_empty() {
            None
        } else {
            Some(morph_anchor::sample_anchored_contours(&contours, anchors)?)
        };
        let mut aligned: Vec<Self> = Vec::with_capacity(paths.len());
        let mut contacts = Vec::new();
        for (index, contour) in contours.iter().enumerate() {
            let mut samples = if let Some(anchored) = &anchored_samples {
                anchored[index].clone()
            } else {
                resample_contour(contour, POLAR_MORPH_POINTS)?
            };
            if anchored_samples.is_none()
                && let Some(previous) = aligned.last()
            {
                rotate_to_minimum_displacement(&mut samples, &previous.points);
            }
            if !morph_check::simple_closed_contour(&samples) {
                return Err(GeometryError::DegenerateContour);
            }
            let mut verbs = vec![PathVerb::Move];
            verbs.extend(std::iter::repeat_n(PathVerb::Line, samples.len() - 1));
            verbs.push(PathVerb::Close);
            let prepared = Self::new(verbs, samples)?;
            if !allow_self_intersection && let Some(previous) = aligned.last() {
                if let Some(issue) =
                    morph_check::scan_linear_morph(previous, &prepared, &[], &[], contact_is_error)
                        .map_err(|issue| GeometryError::UnsafeMorph(issue.to_string()))?
                {
                    contacts.push(MorphContact {
                        segment: index - 1,
                        detail: issue.to_string(),
                    });
                }
            }
            aligned.push(prepared);
        }
        Ok((aligned, contacts))
    }

    /// Prefer the convex guarantee when it applies, then use the shared-kernel star guarantee.
    pub fn automatic_morph_pair(&self, other: &Self) -> Result<(Self, Self), GeometryError> {
        self.automatic_morph_pair_with_options(other, false)
    }

    pub fn automatic_morph_pair_with_options(
        &self,
        other: &Self,
        allow_self_intersection: bool,
    ) -> Result<(Self, Self), GeometryError> {
        match self.convex_morph_pair(other) {
            Ok(pair) => Ok(pair),
            Err(convex_reason) => match self.polar_morph_pair(other) {
                Ok(pair) => Ok(pair),
                Err(polar_reason) => self
                    .arc_length_morph_pair_with_options(other, &[], allow_self_intersection)
                    .or_else(|arc_reason| {
                        Err(if matches!(arc_reason, GeometryError::UnsafeMorph(_)) {
                            arc_reason
                        } else if convex_reason == GeometryError::NonConvexContour {
                            polar_reason
                        } else {
                            convex_reason
                        })
                    }),
            },
        }
    }

    pub fn automatic_morph_pair_with_contacts(
        &self,
        other: &Self,
        allow_self_intersection: bool,
        contact_is_error: bool,
    ) -> Result<((Self, Self), Vec<MorphContact>), GeometryError> {
        match self.convex_morph_pair(other) {
            Ok(pair) => Ok((pair, Vec::new())),
            Err(convex_reason) => match self.polar_morph_pair(other) {
                Ok(pair) => Ok((pair, Vec::new())),
                Err(polar_reason) => self
                    .arc_length_morph_pair_with_contacts(
                        other,
                        &[],
                        allow_self_intersection,
                        contact_is_error,
                    )
                    .or_else(|arc_reason| {
                        Err(if matches!(arc_reason, GeometryError::UnsafeMorph(_)) {
                            arc_reason
                        } else if convex_reason == GeometryError::NonConvexContour {
                            polar_reason
                        } else {
                            convex_reason
                        })
                    }),
            },
        }
    }

    pub fn automatic_morph_sequence(paths: &[Self]) -> Result<Vec<Self>, GeometryError> {
        Self::automatic_morph_sequence_with_options(paths, false)
    }

    pub fn automatic_morph_sequence_with_options(
        paths: &[Self],
        allow_self_intersection: bool,
    ) -> Result<Vec<Self>, GeometryError> {
        match Self::convex_morph_sequence(paths) {
            Ok(aligned) => Ok(aligned),
            Err(convex_reason) => match Self::polar_morph_sequence(paths) {
                Ok(aligned) => Ok(aligned),
                Err(polar_reason) => Self::arc_length_morph_sequence_with_options(
                    paths,
                    &[],
                    allow_self_intersection,
                )
                .or_else(|arc_reason| {
                    Err(if matches!(arc_reason, GeometryError::UnsafeMorph(_)) {
                        arc_reason
                    } else if convex_reason == GeometryError::NonConvexContour {
                        polar_reason
                    } else {
                        convex_reason
                    })
                }),
            },
        }
    }

    pub fn automatic_morph_sequence_with_contacts(
        paths: &[Self],
        allow_self_intersection: bool,
        contact_is_error: bool,
    ) -> Result<(Vec<Self>, Vec<MorphContact>), GeometryError> {
        match Self::convex_morph_sequence(paths) {
            Ok(aligned) => Ok((aligned, Vec::new())),
            Err(convex_reason) => match Self::polar_morph_sequence(paths) {
                Ok(aligned) => Ok((aligned, Vec::new())),
                Err(polar_reason) => Self::arc_length_morph_sequence_with_contacts(
                    paths,
                    &[],
                    allow_self_intersection,
                    contact_is_error,
                )
                .or_else(|arc_reason| {
                    Err(if matches!(arc_reason, GeometryError::UnsafeMorph(_)) {
                        arc_reason
                    } else if convex_reason == GeometryError::NonConvexContour {
                        polar_reason
                    } else {
                        convex_reason
                    })
                }),
            },
        }
    }

    /// Align a sequence at one shared kernel point. Vertex angles join the uniform angular grid
    /// so polygon keyframes retain every authored corner exactly when sampled.
    pub fn polar_morph_sequence(paths: &[Self]) -> Result<Vec<Self>, GeometryError> {
        if !(2..=16).contains(&paths.len()) {
            return Err(GeometryError::TooComplex);
        }
        let flattened = paths
            .iter()
            .map(adaptive_flatten)
            .collect::<Result<Vec<_>, _>>()?;
        if flattened.iter().any(|contours| contours.len() != 1) {
            return Err(GeometryError::SingleContourRequired);
        }
        if flattened.iter().any(|contours| !contours[0].closed) {
            return Err(GeometryError::ClosedContourRequired);
        }
        let contours = flattened
            .iter()
            .map(|contours| contours[0].points.as_slice())
            .collect::<Vec<_>>();
        let center = shared_star_kernel_point(&contours)?;
        let mut angles = (0..POLAR_MORPH_POINTS)
            .map(|index| core::f64::consts::TAU * index as f64 / POLAR_MORPH_POINTS as f64)
            .collect::<Vec<_>>();
        for point in contours.iter().flat_map(|contour| contour.iter()) {
            let vector = displacement(center, *point);
            let mut angle = valle_draw::math::atan2(vector.y, vector.x);
            if angle < 0.0 {
                angle += core::f64::consts::TAU;
            }
            if (core::f64::consts::TAU - angle).abs() < 1e-12 {
                angle = 0.0;
            }
            angles.push(angle);
        }
        angles.sort_by(f64::total_cmp);
        angles.dedup_by(|left, right| (*left - *right).abs() < 1e-12);
        if angles.len() > MAX_FRAME_GEOMETRY_POINTS {
            return Err(GeometryError::TooComplex);
        }
        contours
            .iter()
            .map(|contour| {
                let points = polar_samples_at_angles(contour, center, &angles)?;
                let mut verbs = vec![PathVerb::Move];
                verbs.extend(std::iter::repeat_n(PathVerb::Line, points.len() - 1));
                verbs.push(PathVerb::Close);
                Self::new(verbs, points)
            })
            .collect()
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

    /// Flatten a closed path for solid mesh construction, retaining exact contour nesting.
    /// Reject touching or self-intersecting boundaries before triangulation.
    pub(crate) fn nested_closed_contours(&self) -> Result<Vec<NestedClosedContour>, GeometryError> {
        let contours = adaptive_flatten(self)?;
        if contours.is_empty() || contours.iter().any(|contour| !contour.closed) {
            return Err(GeometryError::ClosedContourRequired);
        }
        let points = contours
            .into_iter()
            .map(|mut contour| {
                if contour.points.len() > 1 && contour.points.first() == contour.points.last() {
                    contour.points.pop();
                }
                contour.points
            })
            .collect::<Vec<_>>();
        let hierarchy =
            morph_check::contour_hierarchy(&points).ok_or(GeometryError::DegenerateContour)?;
        Ok(points
            .into_iter()
            .enumerate()
            .map(|(index, points)| NestedClosedContour {
                points,
                parent: hierarchy.parent[index],
                depth: hierarchy.depth[index],
            })
            .collect())
    }

    /// Flatten one authored contour for prepared 3D profiles and centerlines.
    /// A closing point is implicit when `closed` is true.
    pub(crate) fn single_flattened_contour(&self) -> Result<(Vec<Point>, bool), GeometryError> {
        let mut contours = adaptive_flatten(self)?;
        if contours.len() != 1 {
            return Err(GeometryError::SingleContourRequired);
        }
        let contour = contours.pop().unwrap();
        let minimum = if contour.closed { 3 } else { 2 };
        if contour.points.len() < minimum {
            return Err(GeometryError::DegenerateContour);
        }
        Ok((contour.points, contour.closed))
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
    PointBudget { limit: usize, actual: usize },
    TooFewPoints,
    TopologyMismatch,
    BadTrajectory,
    InvalidArc,
    InvalidSector,
    SingleContourRequired,
    ClosedContourRequired,
    DegenerateContour,
    NoSharedStarKernel,
    NonConvexContour,
    InvalidMorphAnchors,
    InvalidMorphPairs(String),
    InvalidCompatibleMorph,
    UnsafeMorph(String),
    AreaBandMismatch,
    InvalidModifier,
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
            GeometryError::PointBudget { limit, actual } => write!(
                f, "path needs {actual} points but exceeds the geometry budget of {limit} points"
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
            GeometryError::NoSharedStarKernel => f.write_str(
                "automatic polar morph needs two simple closed contours with a shared star-shaped kernel",
            ),
            GeometryError::NonConvexContour => f.write_str("convex morph needs simple closed convex contours"),
            GeometryError::InvalidMorphAnchors => f.write_str("morph anchors must be distinct points on each contour in the same cyclic order"),
            GeometryError::InvalidMorphPairs(reason) => write!(f, "invalid morph pairs: {reason}"),
            GeometryError::InvalidCompatibleMorph => f.write_str(
                "compatible morph cannot produce a valid mesh at this progress value",
            ),
            GeometryError::UnsafeMorph(issue) => write!(f, "unsafe morph correspondence: {issue}; try `convex` or `polar`, or revise the shapes"),
            GeometryError::AreaBandMismatch => f.write_str(
                "areaBand needs two open single-contour paths with matching segment structure and strictly increasing X nodes",
            ),
            GeometryError::InvalidModifier => f.write_str("path modifier parameters or input topology are invalid"),
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

    fn to(self) -> Point {
        match self {
            Self::Line { to, .. } | Self::Quad { to, .. } | Self::Cubic { to, .. } => to,
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

fn adaptive_flatten(path: &PathData) -> Result<Vec<FlattenedContour>, GeometryError> {
    path.validate()?;
    let mut contours = Vec::new();
    let mut points = Vec::new();
    let mut closed = false;
    let mut budget = MAX_PATH_POINTS;
    let mut at = 0;
    let mut current = Point::default();
    let mut start = current;
    for verb in &path.verbs {
        match verb {
            PathVerb::Move => {
                if !points.is_empty() {
                    let ends_at_start = contour_ends_at_start(&points);
                    closed |= ends_at_start;
                    if ends_at_start {
                        points.pop();
                    }
                    contours.push(FlattenedContour {
                        points: std::mem::take(&mut points),
                        closed,
                    });
                }
                closed = false;
                current = path.points[at];
                start = current;
                push_adaptive_point(&mut points, current, &mut budget)?;
                at += 1;
            }
            PathVerb::Line => {
                current = path.points[at];
                push_adaptive_point(&mut points, current, &mut budget)?;
                at += 1;
            }
            PathVerb::Quad => {
                let control = path.points[at];
                let to = path.points[at + 1];
                flatten_curve(
                    SegmentGeometry::Quad {
                        from: current,
                        control,
                        to,
                    },
                    &mut points,
                    &mut budget,
                    0,
                )?;
                current = to;
                at += 2;
            }
            PathVerb::Cubic => {
                let control_1 = path.points[at];
                let control_2 = path.points[at + 1];
                let to = path.points[at + 2];
                flatten_curve(
                    SegmentGeometry::Cubic {
                        from: current,
                        control_1,
                        control_2,
                        to,
                    },
                    &mut points,
                    &mut budget,
                    0,
                )?;
                current = to;
                at += 3;
            }
            PathVerb::Close => {
                closed = true;
                current = start;
            }
        }
    }
    if !points.is_empty() {
        let ends_at_start = contour_ends_at_start(&points);
        closed |= ends_at_start;
        if ends_at_start {
            points.pop();
        }
        contours.push(FlattenedContour { points, closed });
    }
    Ok(contours)
}

fn contour_ends_at_start(points: &[Point]) -> bool {
    if points.len() < 3 {
        return false;
    }
    let (first, last) = (points[0], points[points.len() - 1]);
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (first.x, first.x, first.y, first.y);
    for point in points {
        min_x = min_x.min(point.x);
        max_x = max_x.max(point.x);
        min_y = min_y.min(point.y);
        max_y = max_y.max(point.y);
    }
    let scale = (max_x - min_x).max(max_y - min_y);
    distance(first, last) <= scale * 1e-12
}

fn push_adaptive_point(
    points: &mut Vec<Point>,
    point: Point,
    budget: &mut usize,
) -> Result<(), GeometryError> {
    if *budget == 0 {
        return Err(GeometryError::TooComplex);
    }
    points.push(point);
    *budget -= 1;
    Ok(())
}

fn flatten_curve(
    curve: SegmentGeometry,
    points: &mut Vec<Point>,
    budget: &mut usize,
    depth: usize,
) -> Result<(), GeometryError> {
    let (upper, chord, left, right) = match curve {
        SegmentGeometry::Line { from, to } => {
            if !distance(from, to).is_finite() {
                return Err(GeometryError::NonFinite);
            }
            return push_adaptive_point(points, to, budget);
        }
        SegmentGeometry::Quad { from, control, to } => {
            let halves = split_quad(from, control, to, 0.5);
            (
                distance(from, control) + distance(control, to),
                distance(from, to),
                SegmentGeometry::Quad {
                    from: halves.0[0],
                    control: halves.0[1],
                    to: halves.0[2],
                },
                SegmentGeometry::Quad {
                    from: halves.1[0],
                    control: halves.1[1],
                    to: halves.1[2],
                },
            )
        }
        SegmentGeometry::Cubic {
            from,
            control_1,
            control_2,
            to,
        } => {
            let halves = split_cubic(from, control_1, control_2, to, 0.5);
            (
                distance(from, control_1)
                    + distance(control_1, control_2)
                    + distance(control_2, to),
                distance(from, to),
                SegmentGeometry::Cubic {
                    from: halves.0[0],
                    control_1: halves.0[1],
                    control_2: halves.0[2],
                    to: halves.0[3],
                },
                SegmentGeometry::Cubic {
                    from: halves.1[0],
                    control_1: halves.1[1],
                    control_2: halves.1[2],
                    to: halves.1[3],
                },
            )
        }
    };
    if !upper.is_finite() || !chord.is_finite() {
        return Err(GeometryError::NonFinite);
    }
    // The true curve length lies between the chord and the control-polygon length.
    // A relative bound composes across all accepted subsegments of the path.
    if upper - chord <= upper * 0.00025 {
        return push_adaptive_point(points, curve.to(), budget);
    }
    if depth >= 16 {
        return Err(GeometryError::TooComplex);
    }
    flatten_curve(left, points, budget, depth + 1)?;
    flatten_curve(right, points, budget, depth + 1)
}

fn resample_contour(contour: &FlattenedContour, count: usize) -> Result<Vec<Point>, GeometryError> {
    let source = &contour.points;
    if source.len() < 2 {
        return Err(GeometryError::DegenerateContour);
    }
    let edge_count = if contour.closed {
        source.len()
    } else {
        source.len() - 1
    };
    let mut lengths = Vec::with_capacity(edge_count + 1);
    lengths.push(0.0);
    for edge in 0..edge_count {
        let next = source[(edge + 1) % source.len()];
        lengths.push(lengths[edge] + distance(source[edge], next));
    }
    let total = lengths[edge_count];
    if !total.is_finite() || total <= 0.0 {
        return Err(GeometryError::DegenerateContour);
    }
    let divisor = if contour.closed { count } else { count - 1 };
    let mut output = Vec::with_capacity(count);
    for index in 0..count {
        let target = total * index as f64 / divisor as f64;
        let edge_end = lengths
            .partition_point(|length| *length < target)
            .clamp(1, edge_count);
        let edge = edge_end - 1;
        let span = lengths[edge_end] - lengths[edge];
        let t = if span <= 0.0 {
            0.0
        } else {
            (target - lengths[edge]) / span
        };
        output.push(point_lerp(
            source[edge],
            source[(edge + 1) % source.len()],
            t,
        ));
    }
    Ok(output)
}

fn fixed_modifier_contours(
    path: &PathData,
    count: usize,
) -> Result<Vec<(Vec<Point>, bool)>, GeometryError> {
    let contours = adaptive_flatten(path)?;
    if contours.is_empty() || contours.len().saturating_mul(count) > MAX_FRAME_GEOMETRY_POINTS {
        return Err(GeometryError::TooComplex);
    }
    contours
        .iter()
        .map(|contour| Ok((resample_contour(contour, count)?, contour.closed)))
        .collect()
}

fn path_from_sampled_contours(
    contours: Vec<(Vec<Point>, bool)>,
) -> Result<PathData, GeometryError> {
    let mut verbs = Vec::new();
    let mut points = Vec::new();
    for (samples, closed) in contours {
        if samples.len() < if closed { 3 } else { 2 } {
            return Err(GeometryError::TooFewPoints);
        }
        verbs.push(PathVerb::Move);
        verbs.extend(std::iter::repeat_n(PathVerb::Line, samples.len() - 1));
        if closed {
            verbs.push(PathVerb::Close);
        }
        points.extend(samples);
    }
    if points.len() > MAX_FRAME_GEOMETRY_POINTS {
        return Err(GeometryError::PointBudget {
            limit: MAX_FRAME_GEOMETRY_POINTS,
            actual: points.len(),
        });
    }
    PathData::new(verbs, points)
}

fn simplify_contour(
    source: &[Point],
    closed: bool,
    tolerance: f64,
) -> Result<Vec<Point>, GeometryError> {
    if !closed {
        return Ok(simplify_open_points(source, tolerance));
    }
    let opposite = source.len() / 2;
    let first = simplify_open_points(&source[..=opposite], tolerance);
    let mut second_source = source[opposite..].to_vec();
    second_source.push(source[0]);
    let second = simplify_open_points(&second_source, tolerance);
    let mut output = first;
    output.extend(
        second
            .into_iter()
            .skip(1)
            .take_while(|point| *point != source[0]),
    );
    if output.len() < 3 {
        let (index, distance) = source
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 0 && *index != opposite)
            .map(|(index, point)| {
                (
                    index,
                    point_segment_distance(*point, source[0], source[opposite]),
                )
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .ok_or(GeometryError::DegenerateContour)?;
        if distance <= 1e-12 {
            return Err(GeometryError::DegenerateContour);
        }
        output.insert(if index < opposite { 1 } else { 2 }, source[index]);
    }
    Ok(output)
}

fn simplify_open_points(source: &[Point], tolerance: f64) -> Vec<Point> {
    let mut keep = vec![false; source.len()];
    keep[0] = true;
    keep[source.len() - 1] = true;
    let mut spans = vec![(0, source.len() - 1)];
    while let Some((start, end)) = spans.pop() {
        if end <= start + 1 {
            continue;
        }
        let (index, distance) = (start + 1..end)
            .map(|index| {
                (
                    index,
                    point_segment_distance(source[index], source[start], source[end]),
                )
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        if distance > tolerance {
            keep[index] = true;
            spans.push((start, index));
            spans.push((index, end));
        }
    }
    source
        .iter()
        .zip(keep)
        .filter_map(|(point, keep)| keep.then_some(*point))
        .collect()
}

fn point_segment_distance(point: Point, from: Point, to: Point) -> f64 {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= 0.0 {
        return distance(point, from);
    }
    let t = ((point.x - from.x) * dx + (point.y - from.y) * dy) / length_squared;
    distance(
        point,
        Point::new(
            from.x + dx * t.clamp(0.0, 1.0),
            from.y + dy * t.clamp(0.0, 1.0),
        ),
    )
}

fn cross(a: Vec2, b: Vec2) -> f64 {
    a.x * b.y - a.y * b.x
}

fn displacement(from: Point, to: Point) -> Vec2 {
    Vec2::new(to.x - from.x, to.y - from.y)
}

fn signed_double_area(points: &[Point]) -> f64 {
    (0..points.len())
        .map(|index| {
            let next = points[(index + 1) % points.len()];
            points[index].x * next.y - points[index].y * next.x
        })
        .sum()
}

fn point_on_segment(point: Point, from: Point, to: Point, tolerance: f64) -> bool {
    point.x >= from.x.min(to.x) - tolerance
        && point.x <= from.x.max(to.x) + tolerance
        && point.y >= from.y.min(to.y) - tolerance
        && point.y <= from.y.max(to.y) + tolerance
}

fn segments_touch(a: Point, b: Point, c: Point, d: Point, tolerance: f64) -> bool {
    let (ab, cd) = (displacement(a, b), displacement(c, d));
    let (ac, ad, ca, cb) = (
        displacement(a, c),
        displacement(a, d),
        displacement(c, a),
        displacement(c, b),
    );
    let (o1, o2, o3, o4) = (cross(ab, ac), cross(ab, ad), cross(cd, ca), cross(cd, cb));
    let crossing = |first: f64, second: f64| {
        (first > tolerance && second < -tolerance) || (first < -tolerance && second > tolerance)
    };
    (crossing(o1, o2) && crossing(o3, o4))
        || (o1.abs() <= tolerance && point_on_segment(c, a, b, tolerance))
        || (o2.abs() <= tolerance && point_on_segment(d, a, b, tolerance))
        || (o3.abs() <= tolerance && point_on_segment(a, c, d, tolerance))
        || (o4.abs() <= tolerance && point_on_segment(b, c, d, tolerance))
}

fn robust_turn(a: Point, b: Point, c: Point) -> Orientation {
    RobustKernel::orient2d(
        Coord { x: a.x, y: a.y },
        Coord { x: b.x, y: b.y },
        Coord { x: c.x, y: c.y },
    )
}

fn rotate_to_minimum_displacement(points: &mut [Point], reference: &[Point]) {
    let count = points.len();
    let mut best = (f64::INFINITY, 0usize);
    for offset in 0..count {
        let cost = (0..count)
            .map(|index| {
                let a = reference[index];
                let b = points[(index + offset) % count];
                let dx = a.x - b.x;
                let dy = a.y - b.y;
                dx * dx + dy * dy
            })
            .sum::<f64>();
        if cost < best.0 {
            best = (cost, offset);
        }
    }
    points.rotate_left(best.1);
}

fn convex_contour(path: &PathData) -> Result<Vec<Point>, GeometryError> {
    let contours = adaptive_flatten(path)?;
    if contours.len() != 1 {
        return Err(GeometryError::SingleContourRequired);
    }
    let contour = &contours[0];
    if !contour.closed {
        return Err(GeometryError::ClosedContourRequired);
    }
    let mut points = contour.points.clone();
    if points.len() < 3 || !morph_check::simple_closed_contour(&points) {
        return Err(GeometryError::NonConvexContour);
    }
    let winding = (0..points.len())
        .find_map(|index| {
            let turn = robust_turn(
                points[index],
                points[(index + 1) % points.len()],
                points[(index + 2) % points.len()],
            );
            (turn != Orientation::Collinear).then_some(turn)
        })
        .ok_or(GeometryError::DegenerateContour)?;
    if winding == Orientation::Clockwise {
        points.reverse();
    }
    let mut clean = Vec::with_capacity(points.len());
    for index in 0..points.len() {
        match robust_turn(
            points[(index + points.len() - 1) % points.len()],
            points[index],
            points[(index + 1) % points.len()],
        ) {
            Orientation::Clockwise => return Err(GeometryError::NonConvexContour),
            Orientation::CounterClockwise => clean.push(points[index]),
            Orientation::Collinear => {}
        }
    }
    if clean.len() < 3 || !morph_check::simple_closed_contour(&clean) {
        return Err(GeometryError::NonConvexContour);
    }
    let start = (0..clean.len())
        .min_by(|a, b| {
            clean[*a]
                .y
                .total_cmp(&clean[*b].y)
                .then_with(|| clean[*a].x.total_cmp(&clean[*b].x))
        })
        .unwrap();
    clean.rotate_left(start);
    let angles = (0..clean.len())
        .map(|index| {
            let from = clean[index];
            let to = clean[(index + 1) % clean.len()];
            let mut angle = valle_draw::math::atan2(to.y - from.y, to.x - from.x);
            if angle < 0.0 {
                angle += core::f64::consts::TAU;
            }
            if angle == 0.0 {
                angle = 0.0;
            }
            angle
        })
        .collect::<Vec<_>>();
    if angles.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(GeometryError::NonConvexContour);
    }
    Ok(clean)
}

struct ConvexEdgeEvent {
    angle: f64,
    path: usize,
    edge: usize,
    vector: Vec2,
}

fn parallel_forward(left: Vec2, right: Vec2) -> bool {
    robust_turn(
        Point::new(0.0, 0.0),
        Point::new(left.x, left.y),
        Point::new(right.x, right.y),
    ) == Orientation::Collinear
        && left.x * right.x + left.y * right.y > 0.0
}

fn align_convex_contours(contours: &[Vec<Point>]) -> Result<Vec<PathData>, GeometryError> {
    let mut events = Vec::new();
    for (path, contour) in contours.iter().enumerate() {
        for edge in 0..contour.len() {
            let vector = displacement(contour[edge], contour[(edge + 1) % contour.len()]);
            let mut angle = valle_draw::math::atan2(vector.y, vector.x);
            if angle < 0.0 {
                angle += core::f64::consts::TAU;
            }
            if angle == 0.0 {
                angle = 0.0;
            }
            events.push(ConvexEdgeEvent {
                angle,
                path,
                edge,
                vector,
            });
        }
    }
    events.sort_by(|a, b| {
        a.angle
            .total_cmp(&b.angle)
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut cursors = vec![0usize; contours.len()];
    let mut aligned = vec![Vec::new(); contours.len()];
    let mut at = 0;
    while at < events.len() {
        let mut end = at + 1;
        while end < events.len() && parallel_forward(events[at].vector, events[end].vector) {
            end += 1;
        }
        for (output, (contour, cursor)) in aligned.iter_mut().zip(contours.iter().zip(&cursors)) {
            output.push(contour[*cursor % contour.len()]);
        }
        if aligned[0].len() > MAX_FRAME_GEOMETRY_POINTS {
            return Err(GeometryError::TooComplex);
        }
        for event in &events[at..end] {
            if event.edge != cursors[event.path] {
                return Err(GeometryError::NonConvexContour);
            }
            cursors[event.path] += 1;
        }
        at = end;
    }
    if cursors
        .iter()
        .zip(contours)
        .any(|(cursor, contour)| *cursor != contour.len())
    {
        return Err(GeometryError::NonConvexContour);
    }
    aligned
        .into_iter()
        .map(|points| {
            let mut verbs = vec![PathVerb::Move];
            verbs.extend(std::iter::repeat_n(PathVerb::Line, points.len() - 1));
            verbs.push(PathVerb::Close);
            PathData::new(verbs, points)
        })
        .collect()
}

fn simple_polygon(points: &[Point], tolerance: f64) -> bool {
    if points.len() < 3 {
        return false;
    }
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        if distance(a, b) <= tolerance {
            return false;
        }
        for j in (i + 2)..points.len() {
            if i == 0 && j == points.len() - 1 {
                continue;
            }
            if segments_touch(a, b, points[j], points[(j + 1) % points.len()], tolerance) {
                return false;
            }
        }
    }
    true
}

fn shared_star_kernel_point(contours: &[&[Point]]) -> Result<Point, GeometryError> {
    let mut min = Point::new(f64::INFINITY, f64::INFINITY);
    let mut max = Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    for point in contours.iter().flat_map(|contour| contour.iter()) {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
    }
    let span = (max.x - min.x).max(max.y - min.y);
    if !span.is_finite() || span <= 0.0 {
        return Err(GeometryError::DegenerateContour);
    }
    let tolerance = span * span * 1e-12;
    if contours
        .iter()
        .any(|contour| !simple_polygon(contour, span * 1e-12))
    {
        return Err(GeometryError::NoSharedStarKernel);
    }
    let mut kernel = vec![
        Point::new(min.x - span, min.y - span),
        Point::new(max.x + span, min.y - span),
        Point::new(max.x + span, max.y + span),
        Point::new(min.x - span, max.y + span),
    ];
    for contour in contours {
        let area = signed_double_area(contour);
        if !area.is_finite() || area.abs() <= tolerance {
            return Err(GeometryError::DegenerateContour);
        }
        let sign = area.signum();
        for index in 0..contour.len() {
            let a = contour[index];
            let edge = displacement(a, contour[(index + 1) % contour.len()]);
            let side = |point: Point| sign * cross(edge, displacement(a, point));
            let mut clipped = Vec::new();
            for next_index in 0..kernel.len() {
                let before = kernel[next_index];
                let after = kernel[(next_index + 1) % kernel.len()];
                let (before_side, after_side) = (side(before), side(after));
                let (before_inside, after_inside) =
                    (before_side >= -tolerance, after_side >= -tolerance);
                if before_inside != after_inside {
                    let ratio = before_side / (before_side - after_side);
                    clipped.push(point_lerp(before, after, ratio));
                }
                if after_inside {
                    clipped.push(after);
                }
            }
            kernel = clipped;
            if kernel.len() < 3 {
                return Err(GeometryError::NoSharedStarKernel);
            }
        }
    }
    let center = Point::new(
        kernel.iter().map(|point| point.x).sum::<f64>() / kernel.len() as f64,
        kernel.iter().map(|point| point.y).sum::<f64>() / kernel.len() as f64,
    );
    for contour in contours {
        let sign = signed_double_area(contour).signum();
        for index in 0..contour.len() {
            let a = contour[index];
            let edge = displacement(a, contour[(index + 1) % contour.len()]);
            if sign * cross(edge, displacement(a, center)) <= tolerance {
                return Err(GeometryError::NoSharedStarKernel);
            }
        }
    }
    Ok(center)
}

fn polar_samples(
    contour: &[Point],
    center: Point,
    count: usize,
) -> Result<Vec<Point>, GeometryError> {
    let angles = (0..count)
        .map(|index| core::f64::consts::TAU * index as f64 / count as f64)
        .collect::<Vec<_>>();
    polar_samples_at_angles(contour, center, &angles)
}

fn polar_samples_at_angles(
    contour: &[Point],
    center: Point,
    angles: &[f64],
) -> Result<Vec<Point>, GeometryError> {
    let mut output = Vec::with_capacity(angles.len());
    for &angle in angles {
        let (sin, cos) = valle_draw::math::sin_cos(angle);
        let ray = Vec2::new(cos, sin);
        let mut radius = 0.0_f64;
        for edge_index in 0..contour.len() {
            let a = contour[edge_index];
            let next = contour[(edge_index + 1) % contour.len()];
            let edge = displacement(a, next);
            let denominator = cross(ray, edge);
            if denominator.abs() <= 1e-14 * distance(a, next) {
                continue;
            }
            let center_to_a = displacement(center, a);
            let along_ray = cross(center_to_a, edge) / denominator;
            let along_edge = cross(center_to_a, ray) / denominator;
            if along_ray > 0.0 && (-1e-10..=1.0 + 1e-10).contains(&along_edge) {
                radius = radius.max(along_ray);
            }
        }
        if radius <= 0.0 || !radius.is_finite() {
            return Err(GeometryError::NoSharedStarKernel);
        }
        output.push(Point::new(
            center.x + radius * ray.x,
            center.y + radius * ray.y,
        ));
    }
    Ok(output)
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

fn contour_centroid(points: &[Point], closed: bool) -> Point {
    if closed {
        let mut twice_area = 0.0;
        let mut x = 0.0;
        let mut y = 0.0;
        for index in 0..points.len() {
            let from = points[index];
            let to = points[(index + 1) % points.len()];
            let cross = from.x * to.y - to.x * from.y;
            twice_area += cross;
            x += (from.x + to.x) * cross;
            y += (from.y + to.y) * cross;
        }
        if twice_area.abs() > 1e-12 {
            return Point::new(x / (3.0 * twice_area), y / (3.0 * twice_area));
        }
    }
    let n = points.len() as f64;
    Point::new(
        points.iter().map(|point| point.x).sum::<f64>() / n,
        points.iter().map(|point| point.y).sum::<f64>() / n,
    )
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
    fn corner_radius_sets_tangent_distance_for_non_right_angles() {
        let path = PathData::line(vec![
            Point::new(-100.0, 0.0),
            Point::new(0.0, 0.0),
            Point::new(50.0, 86.60254037844386),
        ])
        .unwrap();
        let rounded = path.round_corners(10.0).unwrap();
        // Interior angle is 120 degrees: tangent distance = r / tan(60 degrees).
        assert!((rounded.points[1].x + 10.0 / libm::sqrt(3.0)).abs() < 1e-9);
        assert_eq!(rounded.verbs, path.round_corners(0.0).unwrap().verbs);
    }

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
    fn resample_uses_equal_arc_lengths_and_preserves_contour_closure() {
        let open = PathData::line(vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(10.0, 0.0),
        ])
        .unwrap();
        let sampled = open.resample(5).unwrap();
        assert_eq!(
            sampled.verbs,
            vec![
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line
            ]
        );
        for (index, point) in sampled.points.iter().enumerate() {
            assert!((point.x - index as f64 * 2.5).abs() < 1e-12);
            assert_eq!(point.y, 0.0);
        }

        let closed = PathData::new(
            vec![
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Close,
            ],
            vec![
                Point::new(0.0, 0.0),
                Point::new(10.0, 0.0),
                Point::new(10.0, 10.0),
                Point::new(0.0, 10.0),
            ],
        )
        .unwrap();
        let sampled = closed.resample(8).unwrap();
        assert_eq!(sampled.points.len(), 8);
        assert_eq!(sampled.verbs.last(), Some(&PathVerb::Close));
        assert_eq!(sampled.points[0], Point::new(0.0, 0.0));
        assert_eq!(sampled.points[4], Point::new(10.0, 10.0));
        assert_ne!(sampled.points[0], sampled.points[7]);
        assert_eq!(sampled.path_length().unwrap(), 40.0);
    }

    #[test]
    fn adaptive_resample_keeps_arc_length_within_one_tenth_percent() {
        let arc = PathData::arc(Point::new(0.0, 0.0), 100.0, 0.0, core::f64::consts::TAU).unwrap();
        let sampled = arc.resample(256).unwrap();
        let expected = 200.0 * core::f64::consts::PI;
        let actual = sampled.path_length().unwrap();
        assert!(
            ((actual - expected) / expected).abs() < 0.001,
            "{actual} vs {expected}"
        );
        assert_eq!(
            sampled
                .verbs
                .iter()
                .filter(|verb| **verb == PathVerb::Close)
                .count(),
            1
        );
        assert_eq!(sampled.points.len(), 256);
    }

    #[test]
    fn reversing_paths_preserves_curves_and_flips_travel_direction() {
        let cubic = PathData::cubic(
            Point::new(0.0, 0.0),
            Point::new(10.0, 30.0),
            Point::new(40.0, 20.0),
            Point::new(50.0, 0.0),
        )
        .unwrap();
        let reversed = cubic.reverse().unwrap();
        assert_eq!(reversed.verbs, cubic.verbs);
        assert_eq!(
            reversed.points,
            vec![
                Point::new(50.0, 0.0),
                Point::new(40.0, 20.0),
                Point::new(10.0, 30.0),
                Point::new(0.0, 0.0),
            ]
        );
        assert_eq!(reversed.reverse().unwrap(), cubic);

        let orbit =
            PathData::arc(Point::new(320.0, 180.0), 140.0, 0.0, core::f64::consts::TAU).unwrap();
        let backward = orbit.reverse().unwrap();
        for progress in [0.1, 0.25, 0.6, 0.9] {
            let actual = backward.point_at(progress).unwrap();
            let expected = orbit.point_at(1.0 - progress).unwrap();
            assert!(
                distance(actual, expected) < 1e-6,
                "{progress}: {actual:?} vs {expected:?}"
            );
        }
        assert_eq!(backward.reverse().unwrap(), orbit);
    }

    #[test]
    fn path_modifiers_keep_topology_and_localize_displacement() {
        let rectangle = PathData::new(
            vec![
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Close,
            ],
            vec![
                Point::new(40.0, 40.0),
                Point::new(240.0, 40.0),
                Point::new(240.0, 200.0),
                Point::new(40.0, 200.0),
            ],
        )
        .unwrap();
        let square = rectangle.round_corners(0.0).unwrap();
        let rounded = rectangle.round_corners(36.0).unwrap();
        assert_eq!(square.verbs, rounded.verbs);
        assert!(rounded.verbs.contains(&PathVerb::Cubic));
        assert!(rounded.points.iter().all(|point| (40.0..=240.0).contains(&point.x) && (40.0..=200.0).contains(&point.y)));

        let route = PathData::line(vec![Point::new(300.0, 80.0), Point::new(600.0, 80.0)]).unwrap();
        let zig = route.zigzag(10.0, 24).unwrap();
        assert_eq!(zig.points.len(), 49);
        assert_eq!(zig.points[0], Point::new(300.0, 80.0));
        assert_eq!(*zig.points.last().unwrap(), Point::new(600.0, 80.0));
        assert!(zig.points.iter().any(|point| point.y >= 89.9));
        assert!(zig.points.iter().any(|point| point.y <= 70.1));

        let circle =
            PathData::arc(Point::new(470.0, 250.0), 70.0, 0.0, core::f64::consts::TAU).unwrap();
        let first = circle.noise_displace(3, 16.0, 3.0, 10.0 / 30.0).unwrap();
        let later = circle.noise_displace(3, 16.0, 3.0, 40.0 / 30.0).unwrap();
        assert_eq!(first.verbs, later.verbs);
        assert_eq!(first.points.len(), POLAR_MORPH_POINTS);
        assert!(
            first
                .points
                .iter()
                .zip(&later.points)
                .any(|(a, b)| distance(*a, *b) > 1.0)
        );
        for shape in [&first, &later] {
            let center = contour_centroid(&shape.points, true);
            assert!(distance(center, Point::new(470.0, 250.0)) < 0.01);
        }
    }

    #[test]
    fn radial_simplification_and_stroke_expansion_have_fixed_output_topology() {
        let irregular = PathData::new(
            vec![
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Close,
            ],
            vec![
                Point::new(0.0, -80.0),
                Point::new(15.0, -15.0),
                Point::new(80.0, 0.0),
                Point::new(15.0, 15.0),
                Point::new(0.0, 80.0),
                Point::new(-70.0, 0.0),
            ],
        )
        .unwrap();
        let inward = irregular.pucker_bloat(-0.8).unwrap();
        let neutral = irregular.pucker_bloat(0.0).unwrap();
        let outward = irregular.pucker_bloat(0.8).unwrap();
        assert_eq!(inward.verbs, neutral.verbs);
        assert_eq!(neutral.verbs, outward.verbs);
        let radial_spread = |path: &PathData| {
            let center = contour_centroid(&path.points, true);
            let radii = path
                .points
                .iter()
                .map(|point| distance(center, *point))
                .collect::<Vec<_>>();
            radii.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                - radii.iter().copied().fold(f64::INFINITY, f64::min)
        };
        assert!(radial_spread(&inward) < radial_spread(&neutral));
        assert!(radial_spread(&neutral) < radial_spread(&outward));
        let center = contour_centroid(&neutral.points, true);
        let mean_radius = |path: &PathData| {
            path.points
                .iter()
                .map(|point| distance(center, *point))
                .sum::<f64>()
                / path.points.len() as f64
        };
        assert!((mean_radius(&inward) - mean_radius(&neutral)).abs() < 1e-9);
        let untwisted = irregular.twist(0.0).unwrap();
        let twisted = irregular.twist(1.0).unwrap();
        assert_eq!(untwisted.verbs, twisted.verbs);
        assert!(
            untwisted
                .points
                .iter()
                .zip(&twisted.points)
                .any(|(a, b)| distance(*a, *b) > 10.0)
        );
        let center = contour_centroid(&untwisted.points, true);
        let radii = untwisted
            .points
            .iter()
            .map(|point| distance(center, *point))
            .collect::<Vec<_>>();
        let nearest = (0..radii.len())
            .min_by(|a, b| radii[*a].total_cmp(&radii[*b]))
            .unwrap();
        let farthest = (0..radii.len())
            .max_by(|a, b| radii[*a].total_cmp(&radii[*b]))
            .unwrap();
        let rotated_angle = |index: usize| {
            let from = untwisted.points[index];
            let to = twisted.points[index];
            let (ax, ay) = (from.x - center.x, from.y - center.y);
            let (bx, by) = (to.x - center.x, to.y - center.y);
            valle_draw::math::atan2(ax * by - ay * bx, ax * bx + ay * by)
        };
        assert!(rotated_angle(farthest) - rotated_angle(nearest) > 0.1);

        let line = PathData::line(vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)]).unwrap();
        let noisy = line.zigzag(10.0, 12).unwrap();
        let smoothed = noisy.simplify(15.0).unwrap();
        assert_eq!(smoothed.points.len(), PATH_MODIFIER_SAMPLES);
        assert!(smoothed.points.iter().all(|point| point.y.abs() < 1e-9));
        let ribbon = line.stroke_to_path(20.0).unwrap();
        assert_eq!(ribbon.points.len(), 4);
        assert_eq!(ribbon.verbs.last(), Some(&PathVerb::Close));
        let area = signed_double_area(&ribbon.points).abs() / 2.0;
        assert!((area - 2000.0).abs() < 1e-6, "{area}");

        let circle =
            PathData::arc(Point::new(0.0, 0.0), 80.0, 0.0, core::f64::consts::TAU).unwrap();
        let ring = circle.stroke_to_path(20.0).unwrap();
        assert_eq!(
            ring.verbs
                .iter()
                .filter(|verb| **verb == PathVerb::Move)
                .count(),
            2
        );
        let contours = FlattenedPath::from_path(&ring).unwrap().contours;
        let outer = signed_double_area(&contours[0].points);
        let inner = signed_double_area(&contours[1].points);
        assert!(outer * inner < 0.0);
        let mean_radius = |points: &[Point]| {
            points
                .iter()
                .map(|point| distance(*point, Point::new(0.0, 0.0)))
                .sum::<f64>()
                / points.len() as f64
        };
        assert!((mean_radius(&contours[0].points) - 90.0).abs() < 1.0);
        assert!((mean_radius(&contours[1].points) - 70.0).abs() < 1.0);
    }

    #[test]
    fn polar_morph_aligns_star_and_circle_without_intermediate_crossings() {
        let star = PathData::new(
            vec![
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Close,
            ],
            vec![
                Point::new(320.0, 50.0),
                Point::new(352.3, 135.5),
                Point::new(443.6, 139.8),
                Point::new(372.3, 197.0),
                Point::new(396.4, 285.2),
                Point::new(320.0, 235.0),
                Point::new(243.6, 285.2),
                Point::new(267.7, 197.0),
                Point::new(196.4, 139.8),
                Point::new(287.7, 135.5),
            ],
        )
        .unwrap();
        let circle =
            PathData::arc(Point::new(320.0, 180.0), 120.0, 0.0, core::f64::consts::TAU).unwrap();
        let (from, to) = star.polar_morph_pair(&circle).unwrap();
        assert!(from.has_same_topology(&to));
        assert_eq!(from.points.len(), POLAR_MORPH_POINTS);
        morph_check::check_linear_morph(&from, &to, &[], &[]).unwrap();
        let mut previous_area = 0.0;
        for sample in 0..=20 {
            let shape = from.morph(&to, sample as f64 / 20.0).unwrap();
            assert!(simple_polygon(&shape.points, 1e-9), "sample {sample}");
            let area = signed_double_area(&shape.points).abs() / 2.0;
            assert!(
                area >= previous_area,
                "sample {sample}: {area} < {previous_area}"
            );
            previous_area = area;
        }
    }

    #[test]
    fn convex_morph_is_the_minkowski_combination_even_without_a_shared_kernel() {
        let square = PathData::new(
            vec![
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Close,
            ],
            vec![
                Point::new(0.0, 0.0),
                Point::new(40.0, 0.0),
                Point::new(40.0, 40.0),
                Point::new(0.0, 40.0),
            ],
        )
        .unwrap();
        let triangle = PathData::new(
            vec![
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Close,
            ],
            vec![
                Point::new(100.0, 10.0),
                Point::new(160.0, 20.0),
                Point::new(120.0, 80.0),
            ],
        )
        .unwrap();
        assert_eq!(
            square.polar_morph_pair(&triangle),
            Err(GeometryError::NoSharedStarKernel)
        );
        let (from, to) = square
            .convex_morph_pair(&triangle.reverse().unwrap())
            .unwrap();
        assert!(from.has_same_topology(&to));
        assert_eq!(from.points.len(), 7);
        for step in 1..20 {
            let progress = step as f64 / 20.0;
            let shape = from.morph(&to, progress).unwrap();
            assert!(simple_polygon(&shape.points, 0.0), "step {step}");
            assert!(convex_contour(&shape).is_ok(), "step {step}");
            for direction in 0..36 {
                let angle = direction as f64 * core::f64::consts::TAU / 36.0;
                let (sin, cos) = valle_draw::math::sin_cos(angle);
                let support = |points: &[Point]| {
                    points
                        .iter()
                        .map(|point| point.x * cos + point.y * sin)
                        .fold(f64::NEG_INFINITY, f64::max)
                };
                let expected = (1.0 - progress) * support(&square.points)
                    + progress * support(&triangle.points);
                assert!(
                    (support(&shape.points) - expected).abs() < 1e-9,
                    "step {step} direction {direction}"
                );
            }
        }
        let circle =
            PathData::arc(Point::new(230.0, 50.0), 30.0, 0.0, core::f64::consts::TAU).unwrap();
        let sequence = PathData::convex_morph_sequence(&[square, triangle, circle]).unwrap();
        assert_eq!(sequence.len(), 3);
        assert!(
            sequence
                .windows(2)
                .all(|pair| pair[0].has_same_topology(&pair[1]))
        );
    }

    #[test]
    fn randomized_convex_morphs_preserve_support_functions() {
        let mut seed = 0x6f34_a912_8cde_310bu64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            (seed >> 32) as f64 / u32::MAX as f64
        };
        let mut hull = |offset: f64| {
            let mut points = (0..20)
                .map(|_| Point::new(offset + (next() * 200.0).round(), (next() * 200.0).round()))
                .collect::<Vec<_>>();
            points.sort_by(|a, b| a.x.total_cmp(&b.x).then_with(|| a.y.total_cmp(&b.y)));
            points.dedup_by(|a, b| a.x == b.x && a.y == b.y);
            let chain = |ordered: &[Point]| {
                let mut result = Vec::new();
                for point in ordered {
                    while result.len() >= 2
                        && robust_turn(result[result.len() - 2], result[result.len() - 1], *point)
                            != Orientation::CounterClockwise
                    {
                        result.pop();
                    }
                    result.push(*point);
                }
                result
            };
            let mut boundary = chain(&points);
            points.reverse();
            let mut upper = chain(&points);
            boundary.pop();
            upper.pop();
            boundary.extend(upper);
            PathData::new(
                std::iter::once(PathVerb::Move)
                    .chain(std::iter::repeat_n(PathVerb::Line, boundary.len() - 1))
                    .chain(std::iter::once(PathVerb::Close))
                    .collect(),
                boundary,
            )
            .unwrap()
        };
        for case in 0..32 {
            let from = hull(0.0);
            let to = hull(300.0);
            let to = if case % 2 == 0 {
                to.reverse().unwrap()
            } else {
                to
            };
            let (aligned_from, aligned_to) = from.convex_morph_pair(&to).unwrap();
            for step in 1..6 {
                let t = step as f64 / 6.0;
                let shape = aligned_from.morph(&aligned_to, t).unwrap();
                assert!(convex_contour(&shape).is_ok(), "case {case} step {step}");
                for direction in 0..24 {
                    let angle = direction as f64 * core::f64::consts::TAU / 24.0;
                    let (sin, cos) = valle_draw::math::sin_cos(angle);
                    let support = |points: &[Point]| {
                        points
                            .iter()
                            .map(|point| point.x * cos + point.y * sin)
                            .fold(f64::NEG_INFINITY, f64::max)
                    };
                    let expected = (1.0 - t) * support(&from.points) + t * support(&to.points);
                    assert!(
                        (support(&shape.points) - expected).abs() < 1e-8,
                        "case {case} step {step} direction {direction}"
                    );
                }
            }
        }
    }

    #[test]
    fn arc_length_morph_certifies_disjoint_concave_contours() {
        let coordinates = [
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 20.0),
            (25.0, 20.0),
            (25.0, 60.0),
            (100.0, 60.0),
            (100.0, 80.0),
            (0.0, 80.0),
        ];
        let make = |dx: f64, widen: f64| {
            PathData::new(
                std::iter::once(PathVerb::Move)
                    .chain(std::iter::repeat_n(PathVerb::Line, coordinates.len() - 1))
                    .chain(std::iter::once(PathVerb::Close))
                    .collect(),
                coordinates
                    .iter()
                    .map(|(x, y)| Point::new(x * widen + dx, *y))
                    .collect(),
            )
            .unwrap()
        };
        let from = make(0.0, 1.0);
        let to = make(300.0, 1.1).reverse().unwrap();
        assert_eq!(
            from.convex_morph_pair(&to),
            Err(GeometryError::NonConvexContour)
        );
        assert_eq!(
            from.polar_morph_pair(&to),
            Err(GeometryError::NoSharedStarKernel)
        );
        let (aligned_from, aligned_to) = from.automatic_morph_pair(&to).unwrap();
        assert_eq!(aligned_from.points.len(), POLAR_MORPH_POINTS);
        assert!(aligned_from.has_same_topology(&aligned_to));
        for sample in 0..=20 {
            let shape = aligned_from
                .morph(&aligned_to, sample as f64 / 20.0)
                .unwrap();
            assert!(simple_polygon(&shape.points, 0.0), "sample {sample}");
        }
    }

    #[test]
    fn arc_length_morph_rejects_an_intermediate_collision() {
        let polygon = |points: &[(f64, f64)]| {
            PathData::new(
                std::iter::once(PathVerb::Move)
                    .chain(std::iter::repeat_n(PathVerb::Line, points.len() - 1))
                    .chain(std::iter::once(PathVerb::Close))
                    .collect(),
                points.iter().map(|(x, y)| Point::new(*x, *y)).collect(),
            )
            .unwrap()
        };
        let from = polygon(&[
            (26.0, 6.0),
            (5.0, 18.0),
            (23.0, 17.0),
            (14.0, 15.0),
            (28.0, 16.0),
        ]);
        let to = polygon(&[
            (7.0, 5.0),
            (19.0, 2.0),
            (28.0, 20.0),
            (28.0, 28.0),
            (15.0, 24.0),
        ]);
        let result = from.arc_length_morph_pair(&to);
        assert!(
            matches!(result, Err(GeometryError::UnsafeMorph(_))),
            "{:?}",
            result.map(|_| ())
        );
        let allowed = from
            .arc_length_morph_pair_with_options(&to, &[], true)
            .unwrap();
        assert!(allowed.0.has_same_topology(&allowed.1));
    }

    #[test]
    fn arc_length_anchors_preserve_authored_points_and_reject_bad_order() {
        let c_shape = |left: f64, width: f64, top: f64| {
            let points = [
                Point::new(left, top),
                Point::new(left + width, top),
                Point::new(left + width, top + 20.0),
                Point::new(left + width * 0.25, top + 20.0),
                Point::new(left + width * 0.25, top + 60.0),
                Point::new(left + width, top + 60.0),
                Point::new(left + width, top + 80.0),
                Point::new(left, top + 80.0),
            ];
            PathData::new(
                std::iter::once(PathVerb::Move)
                    .chain(std::iter::repeat_n(PathVerb::Line, points.len() - 1))
                    .chain(std::iter::once(PathVerb::Close))
                    .collect(),
                points.to_vec(),
            )
            .unwrap()
        };
        let from = c_shape(40.0, 100.0, 40.0);
        let to = c_shape(340.0, 110.0, 40.0);
        let anchors = vec![
            vec![Point::new(140.0, 40.0), Point::new(450.0, 40.0)],
            vec![Point::new(140.0, 120.0), Point::new(450.0, 120.0)],
        ];
        let (aligned_from, aligned_to) = from
            .arc_length_morph_pair_with_options(&to, &anchors, false)
            .unwrap();
        assert_eq!(aligned_from.points.len(), POLAR_MORPH_POINTS);
        for row in &anchors {
            let index = aligned_from
                .points
                .iter()
                .position(|point| *point == row[0])
                .expect("source anchor is a sample");
            assert_eq!(aligned_to.points[index], row[1]);
        }
        let (single_from, single_to) = from
            .arc_length_morph_pair_with_options(&to, &anchors[..1], false)
            .unwrap();
        let single_index = single_from
            .points
            .iter()
            .position(|point| *point == anchors[0][0])
            .unwrap();
        assert_eq!(single_to.points[single_index], anchors[0][1]);
        let third = c_shape(500.0, 120.0, 40.0).reverse().unwrap();
        let sequence = PathData::arc_length_morph_sequence_with_options(
            &[from.clone(), to.clone(), third],
            &[
                vec![
                    Point::new(140.0, 40.0),
                    Point::new(450.0, 40.0),
                    Point::new(620.0, 40.0),
                ],
                vec![
                    Point::new(140.0, 120.0),
                    Point::new(450.0, 120.0),
                    Point::new(620.0, 120.0),
                ],
            ],
            false,
        )
        .unwrap();
        assert!(
            sequence
                .windows(2)
                .all(|pair| pair[0].has_same_topology(&pair[1]))
        );
        let off_path = vec![vec![Point::new(140.0, 40.0), Point::new(400.0, 80.0)]];
        assert_eq!(
            from.arc_length_morph_pair_with_options(&to, &off_path, false),
            Err(GeometryError::InvalidMorphAnchors)
        );
        let duplicate = vec![anchors[0].clone(), anchors[0].clone()];
        assert_eq!(
            from.arc_length_morph_pair_with_options(&to, &duplicate, false),
            Err(GeometryError::InvalidMorphAnchors)
        );
        let crossed_order = vec![
            vec![Point::new(40.0, 40.0), Point::new(340.0, 40.0)],
            vec![Point::new(140.0, 40.0), Point::new(450.0, 120.0)],
            vec![Point::new(140.0, 120.0), Point::new(450.0, 40.0)],
        ];
        assert_eq!(
            from.arc_length_morph_pair_with_options(&to, &crossed_order, false),
            Err(GeometryError::InvalidMorphAnchors)
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

#[cfg(test)]
mod stroke_corner_regressions {
    use super::*;
    #[test]
    fn rectangle_stroke_keeps_exact_miter_corners_and_straight_edges() {
        let rectangle = PathData::new(
            vec![
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Close,
            ],
            vec![
                Point::new(0.0, 0.0),
                Point::new(100.0, 0.0),
                Point::new(100.0, 50.0),
                Point::new(0.0, 50.0),
            ],
        )
        .unwrap();
        let stroke = rectangle.stroke_to_path(10.0).unwrap();
        assert_eq!(stroke.points.len(), 8);
        for point in [
            Point::new(-5.0, -5.0),
            Point::new(105.0, -5.0),
            Point::new(105.0, 55.0),
            Point::new(-5.0, 55.0),
            Point::new(5.0, 5.0),
            Point::new(95.0, 5.0),
            Point::new(95.0, 45.0),
            Point::new(5.0, 45.0),
        ] {
            assert!(
                stroke
                    .points
                    .iter()
                    .any(|p| (p.x - point.x).abs() < 1e-9 && (p.y - point.y).abs() < 1e-9),
                "missing {point:?}"
            );
        }
        assert_eq!(stroke.verbs, rectangle.stroke_to_path(20.0).unwrap().verbs);
    }
}
