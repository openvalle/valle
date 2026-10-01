//! Immutable Scene3D geometry built from authored paths during scene preparation.

use geo::{Coord, LineString, Polygon, TriangulateEarcut, Validation};
use serde::{Deserialize, Serialize};
use valle_draw::{Point, math};

use crate::geometry::PathData;

use super::{
    AdmittedModel, ContractErrors, MAX_ABS_POSITION, MAX_MODEL_BYTES, MAX_TRIANGLES, MAX_VERTICES,
    ModelInstance, ModelMesh, ModelNode, ModelVertex, PrimitiveRange, asset::Affine,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ProceduralGeometry {
    Extrude {
        path: PathData,
        depth: f32,
        bevel: f32,
    },
    Lathe {
        path: PathData,
        segments: u32,
    },
    Tube {
        path: PathData,
        radius: f32,
        sides: u32,
    },
}

impl ProceduralGeometry {
    pub fn validate(&self) -> Result<(), ContractErrors> {
        let mut errors = ContractErrors::default();
        let path = match self {
            Self::Extrude { path, .. } | Self::Lathe { path, .. } | Self::Tube { path, .. } => path,
        };
        if let Err(error) = path.validate() {
            errors.push("/geometry/path", error.to_string());
        }
        if path.points.iter().any(|point| {
            !point.x.is_finite()
                || !point.y.is_finite()
                || point.x.abs() > f64::from(MAX_ABS_POSITION)
                || point.y.abs() > f64::from(MAX_ABS_POSITION)
        }) {
            errors.push(
                "/geometry/path",
                "path coordinates exceed the Scene3D position budget",
            );
        }
        match self {
            Self::Extrude { depth, bevel, .. } => {
                if !depth.is_finite() || *depth <= 0.0 || *depth > MAX_ABS_POSITION {
                    errors.push(
                        "/geometry/depth",
                        "extrude depth must be finite in 0..=10000",
                    );
                }
                if !bevel.is_finite() || *bevel < 0.0 || *bevel > *depth * 0.5 {
                    errors.push(
                        "/geometry/bevel",
                        "extrude bevel must be finite in 0..=depth/2",
                    );
                }
            }
            Self::Lathe { segments, .. } => {
                if !(3..=256).contains(segments) {
                    errors.push("/geometry/segments", "lathe segments must be in 3..=256");
                }
            }
            Self::Tube { radius, sides, .. } => {
                if !radius.is_finite() || *radius <= 0.0 || *radius > MAX_ABS_POSITION {
                    errors.push(
                        "/geometry/radius",
                        "tube radius must be finite in 0..=10000",
                    );
                }
                if !(3..=128).contains(sides) {
                    errors.push("/geometry/sides", "tube sides must be in 3..=128");
                }
            }
        }
        errors.finish()
    }

    pub fn admit_model(&self) -> Result<AdmittedModel, ContractErrors> {
        self.validate()?;
        let mut triangles = Vec::new();
        match self {
            Self::Extrude { path, depth, bevel } => extrude(path, *depth, *bevel, &mut triangles)?,
            Self::Lathe { path, segments } => lathe(path, *segments, &mut triangles)?,
            Self::Tube {
                path,
                radius,
                sides,
            } => tube(path, *radius, *sides, &mut triangles)?,
        }
        if triangles.is_empty() || triangles.len() > MAX_TRIANGLES as usize {
            return Err(error(
                "/geometry",
                "procedural triangle count exceeds the Scene3D budget",
            ));
        }
        let (vertices, indices) = indexed_smooth_mesh(&triangles)?;
        let vertex_count = vertices.len();
        if vertex_count > MAX_VERTICES as usize {
            return Err(error(
                "/geometry",
                "procedural vertex count exceeds the Scene3D budget",
            ));
        }
        if vertices.iter().any(|vertex| {
            vertex
                .position
                .iter()
                .any(|value| !value.is_finite() || value.abs() > MAX_ABS_POSITION)
        }) {
            return Err(error(
                "/geometry",
                "generated positions exceed the Scene3D position budget",
            ));
        }
        let source_bytes = (vertices.len() * std::mem::size_of::<ModelVertex>()
            + indices.len() * std::mem::size_of::<u32>()) as u64;
        if source_bytes > MAX_MODEL_BYTES {
            return Err(error(
                "/geometry",
                "procedural mesh storage exceeds the model budget",
            ));
        }
        let spec_bytes = serde_json::to_vec(self).expect("finite procedural spec serializes");
        let identity_matrix = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let identity = Affine::identity();
        Ok(AdmittedModel {
            content_digest: crate::ContentDigest::of_bytes(&spec_bytes),
            source_bytes,
            primitives: vec![PrimitiveRange {
                first_vertex: 0,
                vertex_count: vertex_count as u32,
                first_index: 0,
                index_count: indices.len() as u32,
                material_index: None,
                has_uv: true,
                has_normal: true,
            }],
            meshes: vec![ModelMesh {
                first_primitive: 0,
                primitive_count: 1,
                vertex_count: vertex_count as u32,
                triangle_count: (indices.len() / 3) as u32,
                morph_target_count: 0,
            }],
            nodes: vec![ModelNode {
                id: 0,
                name: None,
                parent: None,
                mesh: Some(0),
                transform: identity_matrix,
                active: true,
            }],
            instances: vec![ModelInstance {
                node: 0,
                mesh: 0,
                skin: None,
                transform: identity,
                weights: Vec::new(),
                joint_palette: Vec::new(),
            }],
            vertices,
            indices,
            morph_targets: vec![Vec::new()],
            skin_vertices: vec![None],
            skins: Vec::new(),
            materials: Vec::new(),
            images: Vec::new(),
            animations: Vec::new(),
            source_trs: vec![None],
        })
    }
}

type Vertex = ([f32; 3], [f32; 2]);
type Triangle = [Vertex; 3];

// Angle-weighted normals share smooth edges up to 60 degrees. UV seams retain separate
// vertices but use the same positional neighborhood; caps and sharp corners stay hard.
fn indexed_smooth_mesh(
    triangles: &[Triangle],
) -> Result<(Vec<ModelVertex>, Vec<u32>), ContractErrors> {
    use std::collections::BTreeMap;
    fn bits(v: f32) -> u32 {
        if v == 0.0 { 0 } else { v.to_bits() }
    }
    let normals = triangles
        .iter()
        .map(|t| {
            face_normal(t[0].0, t[1].0, t[2].0).ok_or_else(|| {
                error(
                    "/geometry",
                    "procedural triangle is degenerate after float conversion",
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut neighbors = BTreeMap::<[u32; 3], BTreeMap<[u32; 3], ([f32; 3], f64)>>::new();
    for (triangle, normal) in triangles.iter().zip(&normals) {
        for corner in 0..3 {
            let p = triangle[corner].0;
            let a: [f64; 3] = std::array::from_fn(|i| {
                f64::from(triangle[(corner + 1) % 3].0[i]) - f64::from(p[i])
            });
            let b: [f64; 3] = std::array::from_fn(|i| {
                f64::from(triangle[(corner + 2) % 3].0[i]) - f64::from(p[i])
            });
            let cross = [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ];
            let angle = libm::atan2(
                libm::sqrt(cross.iter().map(|v| v * v).sum()),
                a.iter().zip(b).map(|(a, b)| a * b).sum(),
            );
            neighbors
                .entry(p.map(bits))
                .or_default()
                .entry(normal.map(bits))
                .or_insert((*normal, 0.0))
                .1 += angle;
        }
    }
    let mut vertices = Vec::new();
    let mut indices = Vec::with_capacity(triangles.len() * 3);
    let mut unique = BTreeMap::<[u32; 8], u32>::new();
    let mut work = 0usize;
    for (triangle, face) in triangles.iter().zip(normals) {
        for &(position, uv) in triangle {
            let mut sum = [0.0f64; 3];
            for (normal, weight) in neighbors[&position.map(bits)].values() {
                work += 1;
                if work > 16_777_216 {
                    return Err(error(
                        "/geometry",
                        "procedural normal adjacency exceeds the geometry budget",
                    ));
                }
                if normal
                    .iter()
                    .zip(face)
                    .map(|(a, b)| f64::from(*a) * f64::from(b))
                    .sum::<f64>()
                    >= 0.5
                {
                    for axis in 0..3 {
                        sum[axis] += f64::from(normal[axis]) * weight;
                    }
                }
            }
            let length = libm::sqrt(sum.iter().map(|v| v * v).sum());
            let normal = sum.map(|v| (v / length) as f32);
            let key = [
                bits(position[0]),
                bits(position[1]),
                bits(position[2]),
                bits(normal[0]),
                bits(normal[1]),
                bits(normal[2]),
                bits(uv[0]),
                bits(uv[1]),
            ];
            let index = *unique.entry(key).or_insert_with(|| {
                let index = vertices.len() as u32;
                vertices.push(ModelVertex {
                    position,
                    normal,
                    uv,
                });
                index
            });
            indices.push(index);
        }
    }
    Ok((vertices, indices))
}

struct Ring {
    points: Vec<Point>,
    inset: Vec<Point>,
    parent: Option<usize>,
    depth: usize,
}

fn extrude(
    path: &PathData,
    depth: f32,
    bevel: f32,
    output: &mut Vec<Triangle>,
) -> Result<(), ContractErrors> {
    let contours = path
        .nested_closed_contours()
        .map_err(|issue| error("/geometry/path", &issue.to_string()))?;
    let mut rings = Vec::with_capacity(contours.len());
    for contour in contours {
        // Motion PathData uses screen-down Y. Scene3D is world-up Y, so flip here to
        // preserve the authored outline's visible orientation under a level camera.
        let mut points = contour
            .points
            .into_iter()
            .map(|point| Point::new(point.x, -point.y))
            .collect::<Vec<_>>();
        let area = signed_area(&points);
        if area.abs() <= 1e-9 {
            return Err(error("/geometry/path", "extrude contour has zero area"));
        }
        if (area > 0.0) != (contour.depth % 2 == 0) {
            points.reverse();
        }
        let inset = if bevel > 0.0 {
            inset_ring(&points, bevel as f64)?
        } else {
            points.clone()
        };
        rings.push(Ring {
            points,
            inset,
            parent: contour.parent,
            depth: contour.depth,
        });
    }
    let bounds = rings.iter().flat_map(|ring| ring.points.iter()).fold(
        (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ),
        |(min_x, min_y, max_x, max_y), point| {
            (
                min_x.min(point.x),
                min_y.min(point.y),
                max_x.max(point.x),
                max_y.max(point.y),
            )
        },
    );
    let span_x = (bounds.2 - bounds.0).max(f64::EPSILON);
    let span_y = (bounds.3 - bounds.1).max(f64::EPSILON);
    let cap_uv = |point: [f64; 2]| {
        [
            ((point[0] - bounds.0) / span_x) as f32,
            ((point[1] - bounds.1) / span_y) as f32,
        ]
    };
    for (index, ring) in rings.iter().enumerate() {
        if ring.depth % 2 != 0 {
            continue;
        }
        let holes = rings
            .iter()
            .filter(|hole| hole.parent == Some(index))
            .map(|hole| line_string(&hole.inset))
            .collect();
        let polygon = Polygon::new(line_string(&ring.inset), holes);
        if !polygon.is_valid() {
            return Err(error(
                "/geometry/bevel",
                "bevel collapses or intersects the path contours",
            ));
        }
        let triangles = polygon.earcut_triangles_raw();
        if triangles.triangle_indices.is_empty() {
            return Err(error(
                "/geometry/path",
                "extrude cap triangulation is empty",
            ));
        }
        for indices in triangles.triangle_indices.chunks_exact(3) {
            let mut xy = [
                triangles.vertices[indices[0]],
                triangles.vertices[indices[1]],
                triangles.vertices[indices[2]],
            ];
            if cross2(xy[0], xy[1], xy[2]) < 0.0 {
                xy.swap(1, 2);
            }
            let front = xy.map(|point| {
                (
                    [point[0] as f32, point[1] as f32, depth * 0.5],
                    cap_uv(point),
                )
            });
            let back = xy.map(|point| {
                (
                    [point[0] as f32, point[1] as f32, -depth * 0.5],
                    cap_uv(point),
                )
            });
            output.push(front);
            output.push([back[0], back[2], back[1]]);
        }
    }
    for ring in &rings {
        let mut perimeter = 0.0;
        let lengths = (0..ring.points.len())
            .map(|at| {
                let a = ring.points[at];
                let b = ring.points[(at + 1) % ring.points.len()];
                let length = math::hypot(b.x - a.x, b.y - a.y);
                perimeter += length;
                length
            })
            .collect::<Vec<_>>();
        let mut traversed = 0.0;
        for (at, length) in lengths.into_iter().enumerate() {
            let next = (at + 1) % ring.points.len();
            let profile = if bevel > 0.0 {
                [
                    (ring.inset[at], ring.inset[next], depth * 0.5),
                    (ring.points[at], ring.points[next], depth * 0.5 - bevel),
                    (ring.points[at], ring.points[next], -depth * 0.5 + bevel),
                    (ring.inset[at], ring.inset[next], -depth * 0.5),
                ]
            } else {
                [
                    (ring.points[at], ring.points[next], depth * 0.5),
                    (ring.points[at], ring.points[next], -depth * 0.5),
                    (ring.points[at], ring.points[next], -depth * 0.5),
                    (ring.points[at], ring.points[next], -depth * 0.5),
                ]
            };
            let segments = if bevel > 0.0 { 3 } else { 1 };
            for part in 0..segments {
                let (top_a, top_b, top_z) = profile[part];
                let (bottom_a, bottom_b, bottom_z) = profile[part + 1];
                if top_z == bottom_z && top_a == bottom_a && top_b == bottom_b {
                    continue;
                }
                let u0 = (traversed / perimeter) as f32;
                let u1 = ((traversed + length) / perimeter) as f32;
                let v0 = (top_z / depth + 0.5).clamp(0.0, 1.0);
                let v1 = (bottom_z / depth + 0.5).clamp(0.0, 1.0);
                let a = ([top_a.x as f32, top_a.y as f32, top_z], [u0, v0]);
                let b = ([top_b.x as f32, top_b.y as f32, top_z], [u1, v0]);
                let c = ([bottom_b.x as f32, bottom_b.y as f32, bottom_z], [u1, v1]);
                let d = ([bottom_a.x as f32, bottom_a.y as f32, bottom_z], [u0, v1]);
                output.push([a, c, b]);
                output.push([a, d, c]);
            }
            traversed += length;
        }
    }
    Ok(())
}

fn mesh_budget(triangles: usize) -> Result<(), ContractErrors> {
    if triangles > MAX_TRIANGLES as usize {
        Err(error(
            "/geometry",
            "procedural triangle count exceeds the Scene3D budget",
        ))
    } else {
        Ok(())
    }
}

fn lathe(path: &PathData, segments: u32, output: &mut Vec<Triangle>) -> Result<(), ContractErrors> {
    let (mut profile, closed) = path
        .single_flattened_contour()
        .map_err(|issue| error("/geometry/path", &issue.to_string()))?;
    for point in &mut profile {
        point.y = -point.y;
    }
    if closed {
        if profile.first() == profile.last() {
            profile.pop();
        }
        if profile.len() < 3 || profile.iter().any(|p| p.x <= 0.0) {
            return Err(error(
                "/geometry/path",
                "closed lathe profile needs at least three points at positive radii",
            ));
        }
        let ring = LineString::new(profile.iter().map(|p| Coord { x: p.x, y: p.y }).collect());
        let polygon = Polygon::new(ring, vec![]);
        polygon
            .check_validation()
            .map_err(|_| error("/geometry/path", "lathe profile must be simple"))?;
        let area: f64 = profile
            .iter()
            .zip(profile.iter().cycle().skip(1))
            .take(profile.len())
            .map(|(a, b)| a.x * b.y - b.x * a.y)
            .sum();
        if area < 0.0 {
            profile.reverse();
        }
        profile.push(profile[0]);
    } else {
        if profile[0].y > profile[profile.len() - 1].y {
            profile.reverse();
        }
        if profile[profile.len() - 1].y - profile[0].y <= 1e-9 {
            return Err(error(
                "/geometry/path",
                "lathe profile must span the Y axis",
            ));
        }
        // Close the profile through the axis only for validation: this also admits returning
        // lips and bowls while rejecting self intersections and overlapping radial caps.
        let mut ring: Vec<_> = profile.iter().map(|p| Coord { x: p.x, y: p.y }).collect();
        if profile.last().unwrap().x != 0.0 {
            ring.push(Coord {
                x: 0.0,
                y: profile.last().unwrap().y,
            });
        }
        if profile[0].x != 0.0 {
            ring.push(Coord {
                x: 0.0,
                y: profile[0].y,
            });
        }
        Polygon::new(LineString::new(ring), vec![])
            .check_validation()
            .map_err(|_| {
                error(
                    "/geometry/path",
                    "lathe profile and axis caps must form a simple region",
                )
            })?;
    }
    if profile.iter().enumerate().any(|(index, point)| {
        point.x < 0.0 || (point.x == 0.0 && index > 0 && index + 1 < profile.len())
    }) {
        return Err(error(
            "/geometry/path",
            "lathe radius must be nonnegative and may reach zero only at an endpoint",
        ));
    }
    let mut lengths = Vec::with_capacity(profile.len() - 1);
    let mut total = 0.0;
    for pair in profile.windows(2) {
        let length = math::hypot(pair[1].x - pair[0].x, pair[1].y - pair[0].y);
        if length <= 1e-9 {
            return Err(error(
                "/geometry/path",
                "lathe profile has a zero-length edge",
            ));
        }
        lengths.push(length);
        total += length;
    }
    if !profile.iter().any(|point| point.x > 0.0) {
        return Err(error(
            "/geometry/path",
            "lathe profile needs a positive radius",
        ));
    }
    let sides = segments as usize;
    mesh_budget(2 * (profile.len() - 1) * sides + 2 * sides)?;
    let mut traversed = 0.0;
    for (at, pair) in profile.windows(2).enumerate() {
        let (lower, upper) = (pair[0], pair[1]);
        let v0 = (traversed / total) as f32;
        traversed += lengths[at];
        let v1 = (traversed / total) as f32;
        for side in 0..sides {
            let theta0 = core::f64::consts::TAU * (side % sides) as f64 / sides as f64;
            let theta1 = core::f64::consts::TAU * ((side + 1) % sides) as f64 / sides as f64;
            let u0 = side as f32 / sides as f32;
            let u1 = (side + 1) as f32 / sides as f32;
            let a = (revolved(lower, theta0), [u0, v0]);
            let b = (revolved(upper, theta0), [u0, v1]);
            let c = (revolved(upper, theta1), [u1, v1]);
            let d = (revolved(lower, theta1), [u1, v0]);
            if lower.x == 0.0 {
                output.push([a, b, c]);
            } else if upper.x == 0.0 {
                output.push([a, b, d]);
            } else {
                output.push([a, b, c]);
                output.push([a, c, d]);
            }
        }
    }
    if closed {
        return Ok(());
    }
    for (at, profile_point) in [profile[0], profile[profile.len() - 1]]
        .into_iter()
        .enumerate()
    {
        if profile_point.x == 0.0 {
            continue;
        }
        let center = ([0.0, profile_point.y as f32, 0.0], [0.5, 0.5]);
        for side in 0..sides {
            let theta0 = core::f64::consts::TAU * (side % sides) as f64 / sides as f64;
            let theta1 = core::f64::consts::TAU * ((side + 1) % sides) as f64 / sides as f64;
            let cap_vertex = |theta: f64| {
                (
                    revolved(profile_point, theta),
                    [
                        (0.5 + 0.5 * libm::cos(theta)) as f32,
                        (0.5 + 0.5 * libm::sin(theta)) as f32,
                    ],
                )
            };
            let a = cap_vertex(theta0);
            let b = cap_vertex(theta1);
            output.push(if at == 0 {
                [center, a, b]
            } else {
                [center, b, a]
            });
        }
    }
    Ok(())
}

fn revolved(point: Point, theta: f64) -> [f32; 3] {
    let theta = if theta == core::f64::consts::TAU {
        0.0
    } else {
        theta
    };
    [
        (point.x * libm::cos(theta)) as f32,
        point.y as f32,
        (point.x * libm::sin(theta)) as f32,
    ]
}

fn tube(
    path: &PathData,
    radius: f32,
    sides: u32,
    output: &mut Vec<Triangle>,
) -> Result<(), ContractErrors> {
    let (mut points, closed) = path
        .single_flattened_contour()
        .map_err(|issue| error("/geometry/path", &issue.to_string()))?;
    for point in &mut points {
        point.y = -point.y;
    }
    let edge_count = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    let mut directions = Vec::with_capacity(edge_count);
    let mut lengths = Vec::with_capacity(edge_count);
    let mut total = 0.0;
    for at in 0..edge_count {
        let a = points[at];
        let b = points[(at + 1) % points.len()];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let length = math::hypot(dx, dy);
        if length <= 1e-9 {
            return Err(error(
                "/geometry/path",
                "tube centerline has a zero-length edge",
            ));
        }
        directions.push([dx / length, dy / length]);
        lengths.push(length);
        total += length;
    }
    let mut normals = Vec::with_capacity(points.len());
    for at in 0..points.len() {
        let tangent = if !closed && at == 0 {
            directions[0]
        } else if !closed && at + 1 == points.len() {
            directions[edge_count - 1]
        } else {
            let before = directions[(at + edge_count - 1) % edge_count];
            let after = directions[at];
            let sum = [before[0] + after[0], before[1] + after[1]];
            let length = math::hypot(sum[0], sum[1]);
            if length <= 1e-9 {
                return Err(error(
                    "/geometry/path",
                    "tube centerline reverses direction at a corner",
                ));
            }
            [sum[0] / length, sum[1] / length]
        };
        normals.push([-tangent[1], tangent[0]]);
    }
    let sides = sides as usize;
    mesh_budget(2 * edge_count * sides + if closed { 0 } else { 2 * sides })?;
    let ring = |at: usize, side: usize| -> [f32; 3] {
        let theta = core::f64::consts::TAU * (side % sides) as f64 / sides as f64;
        let outward = libm::cos(theta);
        let depth = libm::sin(theta);
        [
            (points[at].x + f64::from(radius) * normals[at][0] * outward) as f32,
            (points[at].y + f64::from(radius) * normals[at][1] * outward) as f32,
            (f64::from(radius) * depth) as f32,
        ]
    };
    let mut traversed = 0.0;
    for at in 0..edge_count {
        let next = (at + 1) % points.len();
        let v0 = (traversed / total) as f32;
        traversed += lengths[at];
        let v1 = (traversed / total) as f32;
        for side in 0..sides {
            let u0 = side as f32 / sides as f32;
            let u1 = (side + 1) as f32 / sides as f32;
            let a = (ring(at, side), [u0, v0]);
            let b = (ring(at, side + 1), [u1, v0]);
            let c = (ring(next, side + 1), [u1, v1]);
            let d = (ring(next, side), [u0, v1]);
            output.push([a, b, c]);
            output.push([a, c, d]);
        }
    }
    if !closed {
        for at in [0, points.len() - 1] {
            let center = ([points[at].x as f32, points[at].y as f32, 0.0], [0.5, 0.5]);
            for side in 0..sides {
                let cap_vertex = |side: usize| {
                    let theta = core::f64::consts::TAU * (side % sides) as f64 / sides as f64;
                    (
                        ring(at, side),
                        [
                            (0.5 + 0.5 * libm::cos(theta)) as f32,
                            (0.5 + 0.5 * libm::sin(theta)) as f32,
                        ],
                    )
                };
                let a = cap_vertex(side);
                let b = cap_vertex(side + 1);
                output.push(if at == 0 {
                    [center, b, a]
                } else {
                    [center, a, b]
                });
            }
        }
    }
    Ok(())
}

fn line_string(points: &[Point]) -> LineString<f64> {
    let mut coords = points
        .iter()
        .map(|point| Coord {
            x: point.x,
            y: point.y,
        })
        .collect::<Vec<_>>();
    coords.push(coords[0]);
    LineString::new(coords)
}

fn signed_area(points: &[Point]) -> f64 {
    (0..points.len())
        .map(|at| {
            let a = points[at];
            let b = points[(at + 1) % points.len()];
            a.x * b.y - a.y * b.x
        })
        .sum::<f64>()
        * 0.5
}

fn inset_ring(points: &[Point], amount: f64) -> Result<Vec<Point>, ContractErrors> {
    let mut inset = Vec::with_capacity(points.len());
    for at in 0..points.len() {
        let prev = points[(at + points.len() - 1) % points.len()];
        let curr = points[at];
        let next = points[(at + 1) % points.len()];
        let edge0 = [curr.x - prev.x, curr.y - prev.y];
        let edge1 = [next.x - curr.x, next.y - curr.y];
        let len0 = math::hypot(edge0[0], edge0[1]);
        let len1 = math::hypot(edge1[0], edge1[1]);
        if len0 <= 1e-9 || len1 <= 1e-9 {
            return Err(error(
                "/geometry/path",
                "extrude contour has a zero-length edge",
            ));
        }
        let normal0 = [-edge0[1] / len0, edge0[0] / len0];
        let normal1 = [-edge1[1] / len1, edge1[0] / len1];
        let dot = normal0[0] * normal1[0] + normal0[1] * normal1[1];
        if dot <= -0.999_999 {
            return Err(error(
                "/geometry/bevel",
                "bevel cannot offset a reversing corner",
            ));
        }
        let scale = amount / (1.0 + dot);
        let mut shift = [
            (normal0[0] + normal1[0]) * scale,
            (normal0[1] + normal1[1]) * scale,
        ];
        let magnitude = math::hypot(shift[0], shift[1]);
        if magnitude > amount * 2.0 {
            shift[0] *= amount * 2.0 / magnitude;
            shift[1] *= amount * 2.0 / magnitude;
        }
        inset.push(Point::new(curr.x + shift[0], curr.y + shift[1]));
    }
    Ok(inset)
}

fn cross2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Option<[f32; 3]> {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let length = math::hypot_f32(math::hypot_f32(cross[0], cross[1]), cross[2]);
    (length > 1e-12 && length.is_finite()).then(|| cross.map(|value| value / length))
}

fn error(path: &str, message: &str) -> ContractErrors {
    let mut errors = ContractErrors::default();
    errors.push(path, message);
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use valle_draw::PathVerb;

    fn square_with_hole() -> PathData {
        PathData::new(
            vec![
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Close,
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Close,
            ],
            vec![
                Point::new(-2.0, -2.0),
                Point::new(2.0, -2.0),
                Point::new(2.0, 2.0),
                Point::new(-2.0, 2.0),
                Point::new(-0.5, -0.5),
                Point::new(-0.5, 0.5),
                Point::new(0.5, 0.5),
                Point::new(0.5, -0.5),
            ],
        )
        .unwrap()
    }

    #[test]
    fn extrusion_has_caps_hole_walls_and_bevel_normals() {
        let path = square_with_hole();
        let flat = ProceduralGeometry::Extrude {
            path: path.clone(),
            depth: 2.0,
            bevel: 0.0,
        }
        .admit_model()
        .unwrap();
        let beveled = ProceduralGeometry::Extrude {
            path,
            depth: 2.0,
            bevel: 0.2,
        }
        .admit_model()
        .unwrap();
        assert_eq!(flat.primitives().len(), 1);
        assert_eq!(flat.instances().len(), 1);
        assert!(beveled.triangle_count() > flat.triangle_count());
        assert!(
            flat.vertices()
                .iter()
                .any(|vertex| vertex.normal == [0.0, 0.0, 1.0])
        );
        assert!(
            flat.vertices()
                .iter()
                .any(|vertex| vertex.normal == [0.0, 0.0, -1.0])
        );
        assert!(flat.vertices().iter().any(|vertex| vertex.normal[0] < -0.9));
        assert!(flat.vertices().iter().any(|vertex| vertex.normal[0] > 0.9));
        assert!(
            beveled
                .vertices()
                .iter()
                .any(|vertex| vertex.normal[2] > 0.1 && vertex.normal[2] < 0.9)
        );
        assert!(
            beveled
                .vertices()
                .iter()
                .all(|vertex| vertex.uv.iter().all(|value| (0.0..=1.0).contains(value)))
        );
        let min_z = beveled
            .vertices()
            .iter()
            .map(|vertex| vertex.position[2])
            .fold(f32::INFINITY, f32::min);
        let max_z = beveled
            .vertices()
            .iter()
            .map(|vertex| vertex.position[2])
            .fold(f32::NEG_INFINITY, f32::max);
        assert_eq!((min_z, max_z), (-1.0, 1.0));
        let front_hole_covers_center = flat.vertices().chunks_exact(3).any(|triangle| {
            triangle.iter().all(|vertex| vertex.position[2] == 1.0)
                && point_in_triangle(
                    [0.0, 0.0],
                    [
                        [triangle[0].position[0], triangle[0].position[1]],
                        [triangle[1].position[0], triangle[1].position[1]],
                        [triangle[2].position[0], triangle[2].position[1]],
                    ],
                )
        });
        assert!(!front_hole_covers_center, "the cap must preserve the hole");
    }

    #[test]
    fn maximum_bevel_omits_the_zero_height_wall_band() {
        let path = PathData::new(
            vec![
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Close,
            ],
            vec![
                Point::new(-2.0, -2.0),
                Point::new(2.0, -2.0),
                Point::new(2.0, 2.0),
                Point::new(-2.0, 2.0),
            ],
        )
        .unwrap();
        let model = ProceduralGeometry::Extrude {
            path,
            depth: 2.0,
            bevel: 1.0,
        }
        .admit_model()
        .unwrap();
        assert!(model.triangle_count() > 0);
        assert!(
            model
                .vertices()
                .iter()
                .all(|vertex| vertex.normal.iter().all(|value| value.is_finite()))
        );
    }

    #[test]
    fn screen_down_path_y_becomes_world_up_geometry() {
        let path = PathData::new(
            vec![
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Close,
            ],
            vec![
                Point::new(0.0, 0.0),
                Point::new(2.0, 0.0),
                Point::new(0.0, 3.0),
            ],
        )
        .unwrap();
        let model = ProceduralGeometry::Extrude {
            path,
            depth: 1.0,
            bevel: 0.0,
        }
        .admit_model()
        .unwrap();
        let min_y = model
            .vertices()
            .iter()
            .map(|vertex| vertex.position[1])
            .fold(f32::INFINITY, f32::min);
        let max_y = model
            .vertices()
            .iter()
            .map(|vertex| vertex.position[1])
            .fold(f32::NEG_INFINITY, f32::max);
        assert_eq!((min_y, max_y), (-3.0, 0.0));
    }

    #[test]
    fn lathe_builds_outward_wall_caps_and_axis_tips() {
        let cylinder = PathData::new(
            vec![PathVerb::Move, PathVerb::Line],
            vec![Point::new(1.0, 1.0), Point::new(1.0, -1.0)],
        )
        .unwrap();
        let model = ProceduralGeometry::Lathe {
            path: cylinder,
            segments: 16,
        }
        .admit_model()
        .unwrap();
        assert_eq!(model.triangle_count(), 64);
        assert!(model.vertices().iter().any(|vertex| vertex.normal[0] > 0.9));
        assert!(
            model
                .vertices()
                .iter()
                .any(|vertex| vertex.normal[0] < -0.9)
        );
        assert!(
            model
                .vertices()
                .iter()
                .any(|vertex| vertex.normal == [0.0, 1.0, 0.0])
        );
        assert!(
            model
                .vertices()
                .iter()
                .any(|vertex| vertex.normal == [0.0, -1.0, 0.0])
        );
        assert!(
            model
                .vertices()
                .iter()
                .all(|vertex| vertex.uv.iter().all(|value| (0.0..=1.0).contains(value)))
        );
        let pointed = PathData::new(
            vec![PathVerb::Move, PathVerb::Line, PathVerb::Line],
            vec![
                Point::new(0.0, 1.0),
                Point::new(1.0, 0.0),
                Point::new(0.0, -1.0),
            ],
        )
        .unwrap();
        assert_eq!(
            ProceduralGeometry::Lathe {
                path: pointed,
                segments: 16,
            }
            .admit_model()
            .unwrap()
            .triangle_count(),
            32
        );
    }

    #[test]
    fn tube_builds_open_caps_and_seamless_closed_centerline() {
        let open = PathData::new(
            vec![PathVerb::Move, PathVerb::Line],
            vec![Point::new(-1.0, 0.0), Point::new(1.0, 0.0)],
        )
        .unwrap();
        let model = ProceduralGeometry::Tube {
            path: open,
            radius: 0.25,
            sides: 16,
        }
        .admit_model()
        .unwrap();
        assert_eq!(model.triangle_count(), 64);
        assert!(
            model
                .vertices()
                .iter()
                .any(|vertex| vertex.normal == [1.0, 0.0, 0.0])
        );
        assert!(
            model
                .vertices()
                .iter()
                .any(|vertex| vertex.normal == [-1.0, 0.0, 0.0])
        );
        assert!(model.vertices().iter().any(|vertex| vertex.normal[1] > 0.9));
        // The UV seam has two vertices, but must have one exact position and normal.
        for v in model
            .vertices()
            .iter()
            .filter(|v| v.uv[0] == 0.0 && v.normal[1] > 0.9)
        {
            let seam = model
                .vertices()
                .iter()
                .find(|other| other.uv == [1.0, v.uv[1]] && other.position == v.position)
                .expect("tube seam position must close exactly");
            assert_eq!(v.normal, seam.normal);
        }
        assert!(
            model
                .vertices()
                .iter()
                .all(|vertex| vertex.uv.iter().all(|value| (0.0..=1.0).contains(value)))
        );
        let closed = PathData::new(
            vec![
                PathVerb::Move,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Line,
                PathVerb::Close,
            ],
            vec![
                Point::new(-1.0, -1.0),
                Point::new(1.0, -1.0),
                Point::new(1.0, 1.0),
                Point::new(-1.0, 1.0),
            ],
        )
        .unwrap();
        assert_eq!(
            ProceduralGeometry::Tube {
                path: closed,
                radius: 0.2,
                sides: 16,
            }
            .admit_model()
            .unwrap()
            .triangle_count(),
            128
        );
    }

    #[test]
    fn closed_lathe_is_indexed_smooth_and_keeps_the_torus_hole() {
        let profile =
            PathData::arc(Point::new(3.0, 0.0), 1.0, 0.0, core::f64::consts::TAU).unwrap();
        let model = ProceduralGeometry::Lathe {
            path: profile,
            segments: 64,
        }
        .admit_model()
        .unwrap();
        assert!(model.vertices.len() < model.indices.len() / 2);
        for vertex in &model.vertices {
            let [x, y, z] = vertex.position;
            let r = libm::sqrtf(x * x + z * z);
            assert!(r >= 1.99, "torus hole filled by a cap");
            let ideal = [x * (r - 3.0) / r, y, z * (r - 3.0) / r];
            let norm = libm::sqrtf(ideal.iter().map(|v| v * v).sum());
            let dot: f32 = ideal
                .iter()
                .zip(vertex.normal)
                .map(|(a, b)| a * b / norm)
                .sum();
            assert!(dot > 0.98, "facet normal {dot}: {vertex:?}");
        }
        let bowl = PathData::line(vec![
            Point::new(0.0, 2.0),
            Point::new(2.0, 2.0),
            Point::new(2.5, -2.0),
            Point::new(2.0, -1.8),
        ])
        .unwrap();
        assert!(
            ProceduralGeometry::Lathe {
                path: bowl,
                segments: 32
            }
            .admit_model()
            .is_ok()
        );
    }

    #[test]
    fn lathe_and_tube_reject_invalid_profiles_and_options() {
        let backwards_radius = PathData::new(
            vec![PathVerb::Move, PathVerb::Line],
            vec![Point::new(-1.0, 1.0), Point::new(1.0, -1.0)],
        )
        .unwrap();
        assert!(
            ProceduralGeometry::Lathe {
                path: backwards_radius,
                segments: 16
            }
            .admit_model()
            .is_err()
        );
        let non_monotone = PathData::new(
            vec![PathVerb::Move, PathVerb::Line, PathVerb::Line],
            vec![
                Point::new(1.0, 1.0),
                Point::new(1.0, -1.0),
                Point::new(1.0, 0.0),
            ],
        )
        .unwrap();
        assert!(
            ProceduralGeometry::Lathe {
                path: non_monotone,
                segments: 16
            }
            .admit_model()
            .is_err()
        );
        let line = PathData::new(
            vec![PathVerb::Move, PathVerb::Line],
            vec![Point::new(0.0, 0.0), Point::new(1.0, 0.0)],
        )
        .unwrap();
        assert!(
            ProceduralGeometry::Tube {
                path: line.clone(),
                radius: 0.0,
                sides: 16
            }
            .admit_model()
            .is_err()
        );
        assert!(
            ProceduralGeometry::Tube {
                path: line.clone(),
                radius: 0.1,
                sides: 2
            }
            .admit_model()
            .is_err()
        );
        assert!(
            ProceduralGeometry::Lathe {
                path: line,
                segments: 257
            }
            .admit_model()
            .is_err()
        );
    }

    fn point_in_triangle(point: [f32; 2], triangle: [[f32; 2]; 3]) -> bool {
        let cross = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
        };
        (0..3).all(|at| cross(triangle[at], triangle[(at + 1) % 3], point) >= -1e-6)
    }
}
