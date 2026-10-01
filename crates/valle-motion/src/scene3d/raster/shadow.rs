//! Orthographic depth maps for directional lights. Built anew from each random-access frame.
use super::{
    AlphaMode, FixedPoint, FrameMesh, SUBPIXEL_BITS, SUBPIXEL_SCALE, SUBPIXEL_STEP, V2, V3,
    WorldVertex, is_top_left, orient_fixed,
};

const SHADOW_SIZE: usize = 1024;

#[derive(Clone, Copy)]
pub(super) struct ShadowTriangle {
    pub vertices: [WorldVertex; 3],
    pub mesh_index: usize,
    pub material_index: usize,
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
        triangles: &[ShadowTriangle],
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
        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        let mut min_depth = f32::INFINITY;
        let mut max_depth = f32::NEG_INFINITY;
        for triangle in triangles {
            if meshes[triangle.mesh_index].materials[triangle.material_index]
                .values
                .alpha_mode
                == AlphaMode::Blend
            {
                continue;
            }
            for vertex in triangle.vertices {
                let position = vertex.position;
                let x = position.dot(right);
                let y = position.dot(up);
                let depth = position.dot(forward);
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
        for triangle in triangles {
            let mesh = &meshes[triangle.mesh_index];
            let material = &mesh.materials[triangle.material_index].values;
            if material.alpha_mode != AlphaMode::Blend {
                map.raster_triangle(triangle.vertices, mesh, material);
            }
        }
        Some(map)
    }

    fn project(&self, vertex: WorldVertex) -> ShadowVertex {
        let position = vertex.position;
        ShadowVertex {
            x: (position.dot(self.right) - self.min_x) * self.scale_x,
            y: (self.max_y - position.dot(self.up)) * self.scale_y,
            depth: position.dot(self.forward),
            uv: vertex.uv,
        }
    }

    fn raster_triangle(
        &mut self,
        world: [WorldVertex; 3],
        mesh: &FrameMesh<'_>,
        material: &super::ModelMaterial,
    ) {
        let mut vertices = world.map(|vertex| self.project(vertex));
        let area_f32 = orient(vertices[0], vertices[1], vertices[2]);
        if area_f32.abs() <= 1.0e-8 || (area_f32 > 0.0 && !material.double_sided) {
            return;
        }
        if area_f32 < 0.0 {
            vertices.swap(1, 2);
        }
        let fixed = vertices.map(|vertex| FixedPoint {
            x: libm::roundf(vertex.x * SUBPIXEL_SCALE) as i64,
            y: libm::roundf(vertex.y * SUBPIXEL_SCALE) as i64,
        });
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
            let mut edge = row_edge;
            for x in min_x..=max_x {
                let current = edge;
                edge[0] += step_x[0];
                edge[1] += step_x[1];
                edge[2] += step_x[2];
                if current
                    .iter()
                    .zip(top_left)
                    .any(|(v, owns)| *v < 0 || (*v == 0 && !owns))
                {
                    continue;
                }
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
        let ix = libm::floorf(x) as i32;
        let iy = libm::floorf(y) as i32;
        let mut visible = 0.0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let sx = ix + dx;
                let sy = iy + dy;
                if sx < 0
                    || sy < 0
                    || sx >= SHADOW_SIZE as i32
                    || sy >= SHADOW_SIZE as i32
                    || receiver_depth - bias <= self.depth[sy as usize * SHADOW_SIZE + sx as usize]
                {
                    visible += 1.0;
                }
            }
        }
        visible / 9.0
    }
}

fn orient(a: ShadowVertex, b: ShadowVertex, c: ShadowVertex) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}
