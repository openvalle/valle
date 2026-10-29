//! Rectangle area lighting using linearly transformed cosines (Heitz et al.).
//! The fit data and edge approximation retain their MIT attribution in assets/THREE-LICENSE.txt.
use super::{V3, degrees, lighting};

pub(super) struct Area {
    pub position: V3,
    pub half_width: V3,
    pub half_height: V3,
    pub color: [f32; 3],
}
impl Area {
    pub fn new(
        position: V3,
        target: V3,
        width: f32,
        height: f32,
        roll: f32,
        color: [f32; 3],
    ) -> Self {
        let direction = (target - position).normalized();
        let seed = if direction.y.abs() > 0.99 {
            V3::new(0.0, 0.0, 1.0)
        } else {
            V3::Y
        };
        let right = direction.cross(seed).normalized();
        let up = right.cross(direction).normalized();
        let sine = libm::sinf(degrees(roll));
        let cosine = libm::cosf(degrees(roll));
        Self {
            position,
            half_width: (right * cosine + up * sine) * (width * 0.5),
            half_height: (up * cosine - right * sine) * (height * 0.5),
            color,
        }
    }
    fn corners(&self) -> [V3; 4] {
        [
            self.position + self.half_width - self.half_height,
            self.position - self.half_width - self.half_height,
            self.position - self.half_width + self.half_height,
            self.position + self.half_width + self.half_height,
        ]
    }
    pub fn diffuse(&self, n: V3, position: V3) -> f32 {
        self.integral(n, n, position, [1.0, 0.0, 0.0, 1.0])
    }
    /// Diffuse and GGX form factors, followed by the two fitted Fresnel amplitudes.
    pub fn evaluate(&self, n: V3, v: V3, position: V3, roughness: f32) -> (f32, f32, [f32; 2]) {
        let coord = [
            roughness.clamp(0.0, 1.0) * 63.0,
            libm::sqrtf((1.0 - n.dot(v).clamp(0.0, 1.0)).max(0.0)) * 63.0,
        ];
        let matrix = lookup(0, coord);
        let amplitude = lookup(1, coord);
        (
            self.diffuse(n, position),
            self.integral(n, v, position, matrix),
            [amplitude[0], amplitude[1]],
        )
    }
    fn integral(&self, n: V3, v: V3, position: V3, m: [f32; 4]) -> f32 {
        let corners = self.corners();
        let light_normal = (corners[1] - corners[0]).cross(corners[3] - corners[0]);
        if light_normal.dot(position - corners[0]) < 0.0 {
            return 0.0;
        }
        let mut tangent = v - n * n.dot(v);
        if tangent.dot(tangent) < 1e-12 {
            let axis = if n.x.abs() < 0.9 {
                V3::new(1.0, 0.0, 0.0)
            } else {
                V3::Y
            };
            tangent = axis - n * n.dot(axis);
        }
        let tangent = tangent.normalized();
        let bitangent = n.cross(tangent) * -1.0;
        let q = corners.map(|p| {
            let delta = p - position;
            let x = delta.dot(tangent);
            let y = delta.dot(bitangent);
            let z = delta.dot(n);
            V3::new(m[0] * x + m[2] * z, y, m[1] * x + m[3] * z)
        });
        // Clip the transformed polygon to the cosine lobe's horizon before normalization.
        // Fixed storage avoids allocating per pixel; clipping a convex quad needs at most 5 vertices.
        let mut clipped = [V3::new(0.0, 0.0, 0.0); 6];
        let mut count = 0;
        let mut previous = q[3];
        for current in q {
            if (current.z >= 0.0) != (previous.z >= 0.0) {
                clipped[count] = (previous
                    + (current - previous) * (previous.z / (previous.z - current.z)))
                    .normalized();
                count += 1;
            }
            if current.z >= 0.0 {
                clipped[count] = current.normalized();
                count += 1;
            }
            previous = current;
        }
        if count < 3 {
            return 0.0;
        }
        let mut form = V3::new(0.0, 0.0, 0.0);
        for i in 0..count {
            form = form + edge(clipped[i], clipped[(i + 1) % count]);
        }
        form.z.max(0.0)
    }
}
fn edge(a: V3, b: V3) -> V3 {
    let x = a.dot(b);
    let y = x.abs();
    let ratio =
        (0.854_398_5 + (0.496_515_5 + 0.014_520_6 * y) * y) / (3.417_594 + (4.161_672_4 + y) * y);
    let theta = if x > 0.0 {
        ratio
    } else {
        0.5 / libm::sqrtf((1.0 - x * x).max(1e-7)) - ratio
    };
    a.cross(b) * theta
}
static LUT: [[[f32; 4]; 4096]; 2] = {
    let mut out = [[[0.0; 4]; 4096]; 2];
    let mut table = 0;
    while table < 2 {
        let mut y = 0;
        while y < 64 {
            let mut x = 0;
            while x < 64 {
                let mut c = 0;
                while c < 4 {
                    let at = ((y + 16) * 128 + x + table * 64) * 8 + c * 2;
                    out[table][y * 64 + x][c] = lighting::half(u16::from_le_bytes([
                        lighting::LIGHTING_BYTES[at],
                        lighting::LIGHTING_BYTES[at + 1],
                    ]));
                    c += 1;
                }
                x += 1;
            }
            y += 1;
        }
        table += 1;
    }
    out
};
fn lookup(table: usize, coord: [f32; 2]) -> [f32; 4] {
    let x = libm::floorf(coord[0]) as usize;
    let y = libm::floorf(coord[1]) as usize;
    let tx = coord[0] - x as f32;
    let ty = coord[1] - y as f32;
    let p = |x: usize, y: usize| LUT[table][y.min(63) * 64 + x.min(63)];
    let a = p(x, y);
    let b = p(x + 1, y);
    let c = p(x, y + 1);
    let d = p(x + 1, y + 1);
    std::array::from_fn(|i| {
        (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty) + (c[i] * (1.0 - tx) + d[i] * tx) * ty
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rectangle_diffuse_matches_numerical_area_integration() {
        let light = Area::new(
            V3::new(0.0, 0.0, 3.0),
            V3::new(0.0, 0.0, 0.0),
            2.0,
            1.5,
            0.0,
            [1.0; 3],
        );
        for (position, n) in [
            (V3::new(0.0, 0.0, 0.0), V3::new(0.0, 0.0, 1.0)),
            (V3::new(2.0, 0.0, 0.0), V3::new(0.0, 0.0, 1.0)),
            (V3::new(0.0, 0.0, 0.0), V3::new(0.9, 0.0, 0.3).normalized()),
        ] {
            let mut reference = 0.0;
            for y in 0..128 {
                for x in 0..128 {
                    let p = light.position
                        + light.half_width * (2.0 * (x as f32 + 0.5) / 128.0 - 1.0)
                        + light.half_height * (2.0 * (y as f32 + 0.5) / 128.0 - 1.0);
                    let delta = p - position;
                    let squared = delta.dot(delta);
                    let l = delta.normalized();
                    reference += n.dot(l).max(0.0) * l.z.max(0.0) / squared
                        * (3.0 / (128.0 * 128.0 * core::f32::consts::PI));
                }
            }
            let actual = light.diffuse(n, position);
            assert!((actual - reference).abs() < 0.0001, "{actual}/{reference}");
        }
    }
    #[test]
    fn ltc_is_finite_single_sided_and_rotation_invariant_at_normal_incidence() {
        let origin = V3::new(0.0, 0.0, 0.0);
        let n = V3::new(0.0, 0.0, 1.0);
        let light = Area::new(V3::new(0.0, 0.0, 3.0), origin, 1.0, 3.0, 0.0, [1.0; 3]);
        let rotated = Area::new(V3::new(0.0, 0.0, 3.0), origin, 1.0, 3.0, 90.0, [1.0; 3]);
        for roughness in [0.0525, 0.1, 0.4, 1.0] {
            let (diffuse, specular, amplitude) = light.evaluate(n, n, origin, roughness);
            let other = rotated.evaluate(n, n, origin, roughness);
            assert!(diffuse > 0.0 && specular > 0.0 && specular.is_finite());
            assert!(
                amplitude
                    .into_iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(&v))
            );
            assert!((diffuse - other.0).abs() < 1e-5 && (specular - other.1).abs() < 1e-5);
            assert_eq!(
                light.evaluate(n, n, V3::new(0.0, 0.0, 4.0), roughness).1,
                0.0
            );
        }
        // Looking straight down has a deterministic basis, including at the lamp plane.
        let down = Area::new(V3::Y, origin, 1.0, 1.0, 0.0, [1.0; 3]);
        assert!(down.diffuse(V3::Y, origin) > 0.0);
        assert!(
            down.evaluate(V3::Y, V3::Y, down.position, 0.1)
                .1
                .is_finite()
        );
    }
}
