//! Single-layer screen-space dielectric transmission. Opaque color/depth is immutable for the
//! whole pass, including across CPU tiles. No preceding frame or recursive transparent feedback.
use super::{Camera, ModelMaterial, RasterFrame, V3, f16_bits_to_f32};
use crate::scene3d::asset::srgb_to_linear;

#[derive(Clone, Copy)]
pub(super) enum Pass<'a> {
    All,
    Opaque,
    Transparent(&'a Backdrop),
}

pub(super) struct Backdrop {
    width: u32,
    height: u32,
    color: Vec<u16>,
    depth: Vec<f32>,
}
impl Backdrop {
    pub fn new(frame: &RasterFrame) -> Self {
        Self {
            width: frame.width,
            height: frame.height,
            color: frame.premul_rgba16f.clone(),
            depth: frame.depth.clone(),
        }
    }

    fn pixel(&self, x: i32, y: i32, near: f32) -> [f32; 4] {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return [0.0; 4];
        }
        let index = y as usize * self.width as usize + x as usize;
        // Never pull a foreground object through the glass; missing screen-space information is
        // transparent, so an outer compositor backdrop remains visible.
        if self.depth[index] <= near {
            return [0.0; 4];
        }
        let rgba: [f32; 4] = std::array::from_fn(|i| f16_bits_to_f32(self.color[index * 4 + i]));
        let a = rgba[3];
        if a == 0.0 {
            return [0.0; 4];
        }
        [
            srgb_to_linear(rgba[0] / a) * a,
            srgb_to_linear(rgba[1] / a) * a,
            srgb_to_linear(rgba[2] / a) * a,
            a,
        ]
    }

    fn sample(&self, x: f32, y: f32, near: f32) -> [f32; 4] {
        let x = x - 0.5;
        let y = y - 0.5;
        let ix = libm::floorf(x) as i32;
        let iy = libm::floorf(y) as i32;
        let tx = x - ix as f32;
        let ty = y - iy as f32;
        let a = self.pixel(ix, iy, near);
        let b = self.pixel(ix + 1, iy, near);
        let c = self.pixel(ix, iy + 1, near);
        let d = self.pixel(ix + 1, iy + 1, near);
        std::array::from_fn(|i| {
            (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty) + (c[i] * (1.0 - tx) + d[i] * tx) * ty
        })
    }
}
pub(super) struct Sample {
    direction: V3,
    distance: f32,
    weight: f32,
    tint: [f32; 3],
    roughness: f32,
}
impl Sample {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        m: &ModelMaterial,
        n: V3,
        v: V3,
        base: [f32; 3],
        metallic: f32,
        roughness: f32,
        coat_attenuation: f32,
    ) -> Self {
        let cosine = n.dot(v).clamp(0.0, 1.0);
        let eta = 1.0 / m.ior;
        let direction = (v * -eta
            + n * (eta * cosine
                - libm::sqrtf((1.0 - eta * eta * (1.0 - cosine * cosine)).max(0.0))))
        .normalized();
        let distance = m.thickness / (-direction.dot(n)).max(0.05);
        let (f0, f90) = m.dielectric();
        let f0 = f0.into_iter().fold(0.0, f32::max);
        let fresnel = f0 + (f90 - f0) * libm::powf(1.0 - cosine, 5.0);
        let tint = std::array::from_fn(|i| {
            base[i]
                * m.attenuation_distance
                    .filter(|_| distance > 0.0)
                    .map_or(1.0, |d| libm::powf(m.attenuation_color[i], distance / d))
        });
        Self {
            direction,
            distance,
            weight: m.transmission * (1.0 - metallic) * (1.0 - fresnel) * coat_attenuation,
            tint,
            roughness,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resolve(
        self,
        reflected: [f32; 3],
        coverage: f32,
        position: V3,
        depth: f32,
        camera: &Camera,
        backdrop: &Backdrop,
    ) -> ([f32; 3], f32) {
        let exit = position + self.direction * self.distance - camera.eye;
        let z = exit.dot(camera.forward);
        if z < camera.near {
            let alpha = 1.0 - self.weight;
            return (
                reflected.map(|v| (v / alpha.max(1e-6)).clamp(0.0, 1.0)),
                coverage * alpha,
            );
        }
        let divisor = camera.projection_divisor(z);
        let screen_x = (exit.dot(camera.right) * camera.focal / camera.aspect / divisor * 0.5
            + 0.5)
            * backdrop.width as f32;
        let screen_y =
            (-exit.dot(camera.up) * camera.focal / divisor * 0.5 + 0.5) * backdrop.height as f32;
        let radius = (self.roughness
            * self.roughness
            * self.distance
            * camera.focal
            * backdrop.height as f32
            / (2.0 * camera.projection_divisor(depth)))
        .min(32.0);
        let mut sampled = [0.0; 4];
        for (x, y, w) in [
            (0.0, 0.0, 0.4),
            (-radius, 0.0, 0.15),
            (radius, 0.0, 0.15),
            (0.0, -radius, 0.15),
            (0.0, radius, 0.15),
        ] {
            let tap = backdrop.sample(screen_x + x, screen_y + y, depth);
            for i in 0..4 {
                sampled[i] += tap[i] * w;
            }
        }
        let alpha = 1.0 - self.weight + self.weight * sampled[3];
        let color = std::array::from_fn(|i| {
            ((reflected[i] + self.weight * self.tint[i] * sampled[i]) / alpha.max(1e-6))
                .clamp(0.0, 1.0)
        });
        (color, coverage * alpha)
    }
}
