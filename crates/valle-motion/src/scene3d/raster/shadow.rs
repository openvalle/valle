//! Orthographic depth maps for directional lights, retained only by exact caster content.
use super::{
    AlphaMode, FixedPoint, FrameMesh, SUBPIXEL_BITS, SUBPIXEL_SCALE, SUBPIXEL_STEP, V2, V3,
    WorldVertex, center_range, covered_span, is_top_left, orient_fixed, round_fixed,
};
mod cache;
use crate::ContentDigest;
pub(super) use cache::{ShadowCache, ShadowMaps};
use sha2::{Digest, Sha256};

pub(super) const SHADOW_SIZE: usize = 1024;

pub(super) fn render_maps(
    directions: &[(V3, [f32; 3])],
    casters: &ShadowCasters,
    meshes: &[FrameMesh<'_>],
    workers: usize,
) -> Vec<Option<ShadowMap>> {
    directions
        .iter()
        .map(|(direction, _)| ShadowMap::render(*direction, casters, meshes, workers))
        .collect()
}

/// World-space shadow casters. Each transformed vertex is stored once and triangles name their
/// corners by index, so every map projects and snaps a vertex once instead of once per corner.
#[derive(Default)]
pub(super) struct ShadowCasters {
    positions: Vec<V3>,
    uvs: Vec<V2>,
    triangles: Vec<ShadowTriangle>,
}

#[derive(Clone, Copy)]
struct ShadowTriangle {
    vertices: [u32; 3],
    mesh_index: usize,
    material_index: usize,
}

impl ShadowCasters {
    /// One source identity for CPU, Native GPU and WebGL shadows. Camera, exposure, light
    /// color and PBR shading do not change these maps. Every input to projection, face
    /// selection and alpha sampling does. Hash exact post-morph/skin world positions;
    /// a model/animation name alone is not enough for arbitrary seeking.
    pub(super) fn content_key(
        &self,
        directions: &[(V3, [f32; 3])],
        meshes: &[FrameMesh<'_>],
    ) -> ContentDigest {
        let mut hash = Sha256::new();
        hash.update(b"valle.scene3d.shadow-source@1");
        hash.update((SHADOW_SIZE as u32).to_le_bytes());
        hash.update((directions.len() as u64).to_le_bytes());
        for (direction, _) in directions {
            for value in [direction.x, direction.y, direction.z] {
                hash.update(value.to_le_bytes());
            }
        }
        hash.update((self.positions.len() as u64).to_le_bytes());
        for (position, uv) in self.positions.iter().zip(&self.uvs) {
            for value in [position.x, position.y, position.z, uv.x, uv.y] {
                hash.update(value.to_le_bytes());
            }
        }
        hash.update((self.triangles.len() as u64).to_le_bytes());
        for triangle in &self.triangles {
            for index in triangle.vertices {
                hash.update(index.to_le_bytes());
            }
            hash.update((triangle.mesh_index as u64).to_le_bytes());
            hash.update((triangle.material_index as u64).to_le_bytes());
        }
        hash.update((meshes.len() as u64).to_le_bytes());
        for mesh in meshes {
            hash.update((mesh.materials.len() as u64).to_le_bytes());
            for material in &mesh.materials {
                let material = &material.values;
                hash.update([
                    match material.alpha_mode {
                        AlphaMode::Opaque => 0,
                        AlphaMode::Mask => 1,
                        AlphaMode::Blend => 2,
                    },
                    u8::from(material.double_sided),
                ]);
                hash.update(material.base_color[3].to_le_bytes());
                hash.update([u8::from(material.alpha_cutoff.is_some())]);
                if let Some(cutoff) = material.alpha_cutoff {
                    hash.update(cutoff.to_le_bytes());
                }
                hash.update([u8::from(material.base_color_texture.is_some())]);
                if let Some(binding) = material.base_color_texture {
                    let image = &mesh.mesh.images[binding.image];
                    hash.update(image.content_digest().as_bytes());
                    hash.update([match image.role() {
                        crate::scene3d::TextureRole::Color => 0,
                        crate::scene3d::TextureRole::Data => 1,
                    }]);
                    hash.update(binding.gpu_sampler().to_le_bytes());
                    for value in binding.uv_transform.into_iter().flatten() {
                        hash.update(value.to_le_bytes());
                    }
                }
            }
        }
        ContentDigest::from_bytes(hash.finalize().into())
    }

    pub fn push_vertex(&mut self, vertex: WorldVertex) -> u32 {
        let index = u32::try_from(self.positions.len())
            .expect("Scene3D triangle budget bounds shadow caster vertices");
        self.positions.push(vertex.position);
        self.uvs.push(vertex.uv);
        index
    }

    pub fn push_triangle(&mut self, vertices: [u32; 3], mesh_index: usize, material_index: usize) {
        self.triangles.push(ShadowTriangle {
            vertices,
            mesh_index,
            material_index,
        });
    }
}

#[derive(Debug)]
pub(super) struct ShadowMap {
    right: V3,
    up: V3,
    forward: V3,
    min_x: f32,
    max_y: f32,
    scale_x: f32,
    scale_y: f32,
    depth_span: f32,
    min_depth: f32,
    depth: Vec<f32>,
}

#[derive(Clone, Copy)]
struct ShadowVertex {
    x: f32,
    y: f32,
    depth: f32,
    uv: V2,
}

impl ShadowMap {
    pub fn render(
        direction: V3,
        casters: &ShadowCasters,
        meshes: &[FrameMesh<'_>],
        workers: usize,
    ) -> Option<Self> {
        let (mut map, vertices, fixed) = Self::prepare(direction, casters, meshes)?;
        map.depth = vec![f32::MAX; SHADOW_SIZE * SHADOW_SIZE];
        let rows = |first_y, depth: &mut [f32]| {
            // Each band sees triangles in submission order, retaining exact coplanar and
            // masked depth semantics. Only disjoint output rows execute concurrently.
            for triangle in &casters.triangles {
                let mesh = &meshes[triangle.mesh_index];
                let material = &mesh.materials[triangle.material_index].values;
                if material.alpha_mode != AlphaMode::Blend {
                    let corners = triangle.vertices.map(|index| index as usize);
                    Self::raster_triangle(
                        depth,
                        first_y,
                        corners.map(|index| vertices[index]),
                        corners.map(|index| fixed[index]),
                        mesh,
                        material,
                    );
                }
            }
        };
        let workers = if cfg!(target_arch = "wasm32") {
            1
        } else {
            workers.clamp(1, super::MAX_RASTER_WORKERS)
        };
        crate::cpu::parallel_scope(workers, |scope, workers| {
            let band = SHADOW_SIZE.div_ceil(workers).next_multiple_of(32);
            let mut chunks = map.depth.chunks_mut(band * SHADOW_SIZE).enumerate();
            let (_, first) = chunks.next().unwrap();
            for (index, depth) in chunks {
                let rows = &rows;
                scope.spawn(move || rows(index * band, depth));
            }
            rows(0, first);
        });
        Some(map)
    }

    fn prepare(
        direction: V3,
        casters: &ShadowCasters,
        meshes: &[FrameMesh<'_>],
    ) -> Option<(Self, Vec<ShadowVertex>, Vec<FixedPoint>)> {
        let forward = direction * -1.0;
        let reference_up = if forward.y.abs() > 0.99 {
            V3::new(0.0, 0.0, 1.0)
        } else {
            V3::Y
        };
        let right = forward.cross(reference_up).normalized();
        let up = right.cross(forward).normalized();
        let light = casters
            .positions
            .iter()
            .map(|position| [position.dot(right), position.dot(up), position.dot(forward)])
            .collect::<Vec<_>>();
        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        let mut min_depth = f32::INFINITY;
        let mut max_depth = f32::NEG_INFINITY;
        for triangle in &casters.triangles {
            if meshes[triangle.mesh_index].materials[triangle.material_index]
                .values
                .alpha_mode
                == AlphaMode::Blend
            {
                continue;
            }
            for index in triangle.vertices {
                let [x, y, depth] = light[index as usize];
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                min_depth = min_depth.min(depth);
                max_depth = max_depth.max(depth);
            }
        }
        if !min_x.is_finite() {
            return None;
        }
        let span_x = (max_x - min_x).max(0.001);
        let span_y = (max_y - min_y).max(0.001);
        let margin_x = span_x * 0.01 + 0.0001;
        let margin_y = span_y * 0.01 + 0.0001;
        min_x -= margin_x;
        max_y += margin_y;
        let span_x = span_x + 2.0 * margin_x;
        let span_y = span_y + 2.0 * margin_y;
        let map = Self {
            right,
            up,
            forward,
            min_x,
            max_y,
            scale_x: SHADOW_SIZE as f32 / span_x,
            scale_y: SHADOW_SIZE as f32 / span_y,
            depth_span: (max_depth - min_depth).max(0.001),
            min_depth,
            depth: Vec::new(),
        };
        let vertices = light
            .iter()
            .zip(&casters.uvs)
            .map(|(&[x, y, depth], &uv)| ShadowVertex {
                x: (x - map.min_x) * map.scale_x,
                y: (map.max_y - y) * map.scale_y,
                depth,
                uv,
            })
            .collect::<Vec<_>>();
        let fixed = vertices
            .iter()
            .map(|vertex| FixedPoint {
                x: round_fixed(vertex.x * SUBPIXEL_SCALE),
                y: round_fixed(vertex.y * SUBPIXEL_SCALE),
            })
            .collect::<Vec<_>>();
        Some((map, vertices, fixed))
    }

    fn raster_triangle(
        depth_plane: &mut [f32],
        first_y: usize,
        vertices: [ShadowVertex; 3],
        fixed: [FixedPoint; 3],
        mesh: &FrameMesh<'_>,
        material: &super::ModelMaterial,
    ) {
        let Some((vertices, fixed, area)) = orient_triangle(vertices, fixed, material.double_sided)
        else {
            return;
        };
        // Only texels whose centers lie within the snapped corners can be covered.
        let Some((min_x, max_x)) = center_range(fixed.map(|point| point.x), SHADOW_SIZE as u32)
        else {
            return;
        };
        let Some((min_y, max_y)) = center_range(fixed.map(|point| point.y), SHADOW_SIZE as u32)
        else {
            return;
        };
        let [min_x, max_x, min_y, max_y] = [min_x, max_x, min_y, max_y].map(|value| value as usize);
        let min_y = min_y.max(first_y);
        let max_y = max_y.min(first_y + depth_plane.len() / SHADOW_SIZE - 1);
        if min_y > max_y {
            return;
        }
        let top_left = [
            is_top_left(fixed[1], fixed[2]),
            is_top_left(fixed[2], fixed[0]),
            is_top_left(fixed[0], fixed[1]),
        ];
        let inv_area = 1.0 / area as f32;
        let first = FixedPoint {
            x: ((min_x as i64) << SUBPIXEL_BITS) + (SUBPIXEL_STEP / 2),
            y: ((min_y as i64) << SUBPIXEL_BITS) + (SUBPIXEL_STEP / 2),
        };
        let corner_edge = [
            orient_fixed(fixed[1], fixed[2], first),
            orient_fixed(fixed[2], fixed[0], first),
            orient_fixed(fixed[0], fixed[1], first),
        ];
        let step_x = [
            -(fixed[2].y - fixed[1].y) * SUBPIXEL_STEP,
            -(fixed[0].y - fixed[2].y) * SUBPIXEL_STEP,
            -(fixed[1].y - fixed[0].y) * SUBPIXEL_STEP,
        ];
        let step_y = [
            (fixed[2].x - fixed[1].x) * SUBPIXEL_STEP,
            (fixed[0].x - fixed[2].x) * SUBPIXEL_STEP,
            (fixed[1].x - fixed[0].x) * SUBPIXEL_STEP,
        ];
        let bounds = [min_x, max_x, min_y, max_y];
        covered_texels(
            bounds,
            corner_edge,
            [step_x, step_y],
            top_left,
            |index, edge| {
                let index = index - first_y * SHADOW_SIZE;
                let bary = edge.map(|v| v as f32 * inv_area);
                let depth = vertices[0].depth * bary[0]
                    + vertices[1].depth * bary[1]
                    + vertices[2].depth * bary[2];
                if depth >= depth_plane[index] {
                    return;
                }
                if material.alpha_mode == AlphaMode::Mask {
                    let uv = V2 {
                        x: vertices[0].uv.x * bary[0]
                            + vertices[1].uv.x * bary[1]
                            + vertices[2].uv.x * bary[2],
                        y: vertices[0].uv.y * bary[0]
                            + vertices[1].uv.y * bary[1]
                            + vertices[2].uv.y * bary[2],
                    };
                    let alpha = material.base_color[3]
                        * material.base_color_texture.map_or(1.0, |binding| {
                            mesh.mesh.images[binding.image].sample(binding, [uv.x, uv.y], 0.0)[3]
                        });
                    if alpha < material.alpha_cutoff.unwrap_or(0.5) {
                        return;
                    }
                }
                depth_plane[index] = depth;
            },
        );
    }

    pub fn visibility(&self, position: V3, normal: V3) -> f32 {
        let x = (position.dot(self.right) - self.min_x) * self.scale_x;
        let y = (self.max_y - position.dot(self.up)) * self.scale_y;
        if x < 0.0 || x >= SHADOW_SIZE as f32 || y < 0.0 || y >= SHADOW_SIZE as f32 {
            return 1.0;
        }
        let receiver_depth = position.dot(self.forward);
        // A 3×3 PCF tap may be 1.5 texels from the receiver. Bound the change in
        // light-space depth over that footprint using the receiver plane's slope.
        let denominator = normal.dot(self.forward).abs().max(0.05);
        let slope_x = normal.dot(self.right).abs() / denominator;
        let slope_y = normal.dot(self.up).abs() / denominator;
        let bias = (1.5 * (slope_x / self.scale_x + slope_y / self.scale_y)
            + self.depth_span * 0.00001)
            .min(self.depth_span * 0.02)
            .max(0.0001);
        let ix = x.floor() as i32;
        let iy = y.floor() as i32;
        pcf(&self.depth, ix, iy, receiver_depth - bias)
    }
}

/// Calls `visit` once for every texel of the inclusive `[min_x, max_x, min_y, max_y]` bounds that
/// all three edges cover under the top-left rule, with the texel's index and edge values.
/// `corner_edge` holds the edge values at the first texel and `steps` their exact changes per
/// texel along x and y. A triangle tests each texel once and the test of a texel depends only on
/// its own edge values and stored depth, so the order is free: the bounds are walked in lines along
/// their longer side, and a tall triangle takes a few long columns instead of many rows that cover
/// one texel each.
fn covered_texels(
    [min_x, max_x, min_y, max_y]: [usize; 4],
    corner_edge: [i64; 3],
    [step_x, step_y]: [[i64; 3]; 2],
    top_left: [bool; 3],
    mut visit: impl FnMut(usize, [i64; 3]),
) {
    let (lines, length, line_step, texel_step, line_stride, texel_stride) =
        if max_y - min_y > max_x - min_x {
            (max_x - min_x, max_y - min_y, step_x, step_y, 1, SHADOW_SIZE)
        } else {
            (max_y - min_y, max_x - min_x, step_y, step_x, SHADOW_SIZE, 1)
        };
    let mut line_edge = corner_edge;
    let mut line_start = min_y * SHADOW_SIZE + min_x;
    for _ in 0..=lines {
        if let Some((first, last)) = covered_span(line_edge, texel_step, top_left, length as i64) {
            let mut edge: [i64; 3] =
                std::array::from_fn(|axis| line_edge[axis] + first * texel_step[axis]);
            let mut index = line_start + first as usize * texel_stride;
            for _ in first..=last {
                visit(index, edge);
                edge[0] += texel_step[0];
                edge[1] += texel_step[1];
                edge[2] += texel_step[2];
                index += texel_stride;
            }
        }
        for axis in 0..3 {
            line_edge[axis] += line_step[axis];
        }
        line_start += line_stride;
    }
}

fn pcf(depth: &[f32], ix: i32, iy: i32, receiver: f32) -> f32 {
    if ix > 0 && iy > 0 && ix < SHADOW_SIZE as i32 - 1 && iy < SHADOW_SIZE as i32 - 1 {
        // Most samples are interior: validate the footprint once instead of
        // checking four edges separately for each of the nine taps.
        let first = (iy as usize - 1) * SHADOW_SIZE + ix as usize - 1;
        let mut visible = 0_u32;
        for offset in [0, SHADOW_SIZE, 2 * SHADOW_SIZE] {
            let row = &depth[first + offset..first + offset + 3];
            visible += u32::from(receiver <= row[0])
                + u32::from(receiver <= row[1])
                + u32::from(receiver <= row[2]);
        }
        return visible as f32 / 9.0;
    }
    let mut visible = 0.0;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let sx = ix + dx;
            let sy = iy + dy;
            if sx < 0
                || sy < 0
                || sx >= SHADOW_SIZE as i32
                || sy >= SHADOW_SIZE as i32
                || receiver <= depth[sy as usize * SHADOW_SIZE + sx as usize]
            {
                visible += 1.0;
            }
        }
    }
    visible / 9.0
}

fn orient(a: ShadowVertex, b: ShadowVertex, c: ShadowVertex) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn orient_triangle(
    mut vertices: [ShadowVertex; 3],
    mut fixed: [FixedPoint; 3],
    double_sided: bool,
) -> Option<([ShadowVertex; 3], [FixedPoint; 3], i64)> {
    let area_f32 = orient(vertices[0], vertices[1], vertices[2]);
    if area_f32.abs() <= 1e-8 || (area_f32 > 0.0 && !double_sided) {
        return None;
    }
    if area_f32 < 0.0 {
        vertices.swap(1, 2);
        fixed.swap(1, 2);
    }
    let area = orient_fixed(fixed[0], fixed[1], fixed[2]);
    (area > 0).then_some((vertices, fixed, area))
}

/// CPU and GPU share projection, caster selection, culling and snapped coverage.
pub(super) fn gpu_maps(
    directions: &[(V3, [f32; 3])],
    casters: &ShadowCasters,
    meshes: &[FrameMesh<'_>],
    shading: &super::GpuRasterShading,
) -> Vec<super::gpu::GpuRasterShadow> {
    directions
        .iter()
        .enumerate()
        .filter_map(|(light, (direction, _))| {
            let (map, vertices, fixed) = ShadowMap::prepare(*direction, casters, meshes)?;
            let mut output = Vec::new();
            for triangle in &casters.triangles {
                let material =
                    &meshes[triangle.mesh_index].materials[triangle.material_index].values;
                if material.alpha_mode == AlphaMode::Blend {
                    continue;
                }
                let corners = triangle.vertices.map(|i| i as usize);
                let Some((vertices, fixed, area)) = orient_triangle(
                    corners.map(|i| vertices[i]),
                    corners.map(|i| fixed[i]),
                    material.double_sided,
                ) else {
                    continue;
                };
                for (vertex, point) in vertices.into_iter().zip(fixed) {
                    output.push(super::gpu::GpuRasterShadowVertex {
                        position: [
                            point.x as f32 / SUBPIXEL_SCALE,
                            point.y as f32 / SUBPIXEL_SCALE,
                        ],
                        depth: vertex.depth,
                        uv: [vertex.uv.x, vertex.uv.y],
                        material: shading.material_id(triangle.mesh_index, triangle.material_index),
                        inv_area: 1.0 / area as f32,
                    });
                }
            }
            let mut scale = 1.0;
            while map.depth_span * scale >= 1.0 {
                scale *= 0.5;
            }
            let v = |v: V3, w| [v.x, v.y, v.z, w];
            Some(super::gpu::GpuRasterShadow {
                light: light as u32,
                vertices: output,
                values: [
                    v(map.right, map.min_x),
                    v(map.up, map.max_y),
                    v(map.forward, map.depth_span),
                    [map.scale_x, map.scale_y, map.min_depth, scale],
                ],
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covered_texels_visit_each_texel_the_per_texel_fill_rule_covers_once() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = |range: i64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % (2 * range as u64 + 1)) as i64 - range
        };
        for _ in 0..20_000 {
            // Corners in a small window of the map, in 1/256 texel units. Snapping them to half or
            // whole texels makes axis-aligned edges and edges through texel centers.
            let snap = [1, 128, 256][(next(1) + 1) as usize];
            let mut corner = || FixedPoint {
                x: (600 << SUBPIXEL_BITS) + next(24 << SUBPIXEL_BITS) / snap * snap,
                y: (600 << SUBPIXEL_BITS) + next(24 << SUBPIXEL_BITS) / snap * snap,
            };
            let fixed = [corner(), corner(), corner()];
            let min_x = (fixed.iter().map(|p| p.x).min().unwrap() >> SUBPIXEL_BITS) as usize;
            let max_x = ((fixed.iter().map(|p| p.x).max().unwrap() >> SUBPIXEL_BITS) + 1) as usize;
            let min_y = (fixed.iter().map(|p| p.y).min().unwrap() >> SUBPIXEL_BITS) as usize;
            let max_y = ((fixed.iter().map(|p| p.y).max().unwrap() >> SUBPIXEL_BITS) + 1) as usize;
            let top_left = [
                is_top_left(fixed[1], fixed[2]),
                is_top_left(fixed[2], fixed[0]),
                is_top_left(fixed[0], fixed[1]),
            ];
            let center = |x: usize, y: usize| FixedPoint {
                x: ((x as i64) << SUBPIXEL_BITS) + SUBPIXEL_STEP / 2,
                y: ((y as i64) << SUBPIXEL_BITS) + SUBPIXEL_STEP / 2,
            };
            let edges = |point| {
                [
                    orient_fixed(fixed[1], fixed[2], point),
                    orient_fixed(fixed[2], fixed[0], point),
                    orient_fixed(fixed[0], fixed[1], point),
                ]
            };
            let mut expected = Vec::new();
            for y in min_y..=max_y {
                for x in min_x..=max_x {
                    let edge = edges(center(x, y));
                    if (0..3).all(|i| edge[i] > 0 || (edge[i] == 0 && top_left[i])) {
                        expected.push((y * SHADOW_SIZE + x, edge));
                    }
                }
            }
            let step_x = [
                -(fixed[2].y - fixed[1].y) * SUBPIXEL_STEP,
                -(fixed[0].y - fixed[2].y) * SUBPIXEL_STEP,
                -(fixed[1].y - fixed[0].y) * SUBPIXEL_STEP,
            ];
            let step_y = [
                (fixed[2].x - fixed[1].x) * SUBPIXEL_STEP,
                (fixed[0].x - fixed[2].x) * SUBPIXEL_STEP,
                (fixed[1].x - fixed[0].x) * SUBPIXEL_STEP,
            ];
            let mut visited = Vec::new();
            covered_texels(
                [min_x, max_x, min_y, max_y],
                edges(center(min_x, min_y)),
                [step_x, step_y],
                top_left,
                |index, edge| visited.push((index, edge)),
            );
            visited.sort();
            assert_eq!(visited, expected);
        }
    }

    #[test]
    fn pcf_matches_reference_at_edges_and_interior() {
        let depth = (0..SHADOW_SIZE * SHADOW_SIZE)
            .map(|i| ((i * 19) % 31) as f32 / 16.0)
            .collect::<Vec<_>>();
        for iy in [0, 1, 2, 511, 1022, 1023] {
            for ix in [0, 1, 2, 511, 1022, 1023] {
                for receiver in [0.0, 0.5, 1.0, 1.5, 2.0, f32::MAX] {
                    let mut reference = 0.0;
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let (x, y) = (ix + dx, iy + dy);
                            if x < 0
                                || y < 0
                                || x >= SHADOW_SIZE as i32
                                || y >= SHADOW_SIZE as i32
                                || receiver <= depth[y as usize * SHADOW_SIZE + x as usize]
                            {
                                reference += 1.0;
                            }
                        }
                    }
                    assert_eq!(pcf(&depth, ix, iy, receiver), reference / 9.0);
                }
            }
        }
    }
}
