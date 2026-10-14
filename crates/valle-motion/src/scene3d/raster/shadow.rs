//! Orthographic depth maps for directional lights. Built anew from each random-access frame.
use super::{
    AlphaMode, FixedPoint, FrameMesh, SUBPIXEL_BITS, SUBPIXEL_SCALE, SUBPIXEL_STEP, V2, V3,
    WorldVertex, covered_span, is_top_left, orient_fixed, round_fixed,
};

const SHADOW_SIZE: usize = 1024;

pub(super) fn render_maps(
    directions: &[(V3, [f32; 3])],
    casters: &ShadowCasters,
    meshes: &[FrameMesh<'_>],
    workers: usize,
) -> Vec<Option<ShadowMap>> {
    let workers = if cfg!(target_arch = "wasm32") {
        1
    } else {
        workers.clamp(1, super::MAX_RASTER_WORKERS)
    }
    .min(directions.len());
    if workers <= 1 {
        return directions
            .iter()
            .map(|(direction, _)| ShadowMap::render(*direction, casters, meshes))
            .collect();
    }
    // Lights are independent. Keep each result in its original slot so shadow
    // lookup and light accumulation retain their deterministic ordering.
    let mut maps = (0..directions.len()).map(|_| None).collect::<Vec<_>>();
    crate::cpu::parallel_scope(workers, |scope, workers| {
        let chunk_size = directions.len().div_ceil(workers);
        let mut chunks = maps
            .chunks_mut(chunk_size)
            .zip(directions.chunks(chunk_size));
        let (caller_maps, caller_directions) = chunks.next().unwrap();
        for (maps, directions) in chunks {
            scope.spawn(move || {
                for (map, (direction, _)) in maps.iter_mut().zip(directions) {
                    *map = ShadowMap::render(*direction, casters, meshes);
                }
            });
        }
        for (map, (direction, _)) in caller_maps.iter_mut().zip(caller_directions) {
            *map = ShadowMap::render(*direction, casters, meshes);
        }
    });
    maps
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

pub(super) struct ShadowMap {
    right: V3,
    up: V3,
    forward: V3,
    min_x: f32,
    max_y: f32,
    scale_x: f32,
    scale_y: f32,
    depth_span: f32,
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
    ) -> Option<Self> {
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
        let mut map = Self {
            right,
            up,
            forward,
            min_x,
            max_y,
            scale_x: SHADOW_SIZE as f32 / span_x,
            scale_y: SHADOW_SIZE as f32 / span_y,
            depth_span: (max_depth - min_depth).max(0.001),
            depth: vec![f32::MAX; SHADOW_SIZE * SHADOW_SIZE],
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
        for triangle in &casters.triangles {
            let mesh = &meshes[triangle.mesh_index];
            let material = &mesh.materials[triangle.material_index].values;
            if material.alpha_mode != AlphaMode::Blend {
                let corners = triangle.vertices.map(|index| index as usize);
                map.raster_triangle(
                    corners.map(|index| vertices[index]),
                    corners.map(|index| fixed[index]),
                    mesh,
                    material,
                );
            }
        }
        Some(map)
    }

    fn raster_triangle(
        &mut self,
        mut vertices: [ShadowVertex; 3],
        mut fixed: [FixedPoint; 3],
        mesh: &FrameMesh<'_>,
        material: &super::ModelMaterial,
    ) {
        let area_f32 = orient(vertices[0], vertices[1], vertices[2]);
        if area_f32.abs() <= 1.0e-8 || (area_f32 > 0.0 && !material.double_sided) {
            return;
        }
        if area_f32 < 0.0 {
            vertices.swap(1, 2);
            fixed.swap(1, 2);
        }
        let area = orient_fixed(fixed[0], fixed[1], fixed[2]);
        if area <= 0 {
            return;
        }
        let min_x = vertices
            .iter()
            .map(|v| v.x)
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as usize;
        let max_x = vertices
            .iter()
            .map(|v| v.x)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .min(SHADOW_SIZE as f32 - 1.0) as usize;
        let min_y = vertices
            .iter()
            .map(|v| v.y)
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as usize;
        let max_y = vertices
            .iter()
            .map(|v| v.y)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .min(SHADOW_SIZE as f32 - 1.0) as usize;
        if min_x > max_x || min_y > max_y {
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
        let mut row_edge = [
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
        for y in min_y..=max_y {
            let Some((first, last)) =
                covered_span(row_edge, step_x, top_left, (max_x - min_x) as i64)
            else {
                for axis in 0..3 {
                    row_edge[axis] += step_y[axis];
                }
                continue;
            };
            let mut edge: [i64; 3] = std::array::from_fn(|i| row_edge[i] + first * step_x[i]);
            for x in min_x + first as usize..=min_x + last as usize {
                let current = edge;
                edge[0] += step_x[0];
                edge[1] += step_x[1];
                edge[2] += step_x[2];
                let bary = current.map(|v| v as f32 * inv_area);
                let depth = vertices[0].depth * bary[0]
                    + vertices[1].depth * bary[1]
                    + vertices[2].depth * bary[2];
                let index = y * SHADOW_SIZE + x;
                if depth >= self.depth[index] {
                    continue;
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
                        continue;
                    }
                }
                self.depth[index] = depth;
            }
            for axis in 0..3 {
                row_edge[axis] += step_y[axis];
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
