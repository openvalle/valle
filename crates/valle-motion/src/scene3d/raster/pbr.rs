//! glTF shading for opaque, mask and blended surfaces. Inputs belong to this frame or admitted resources.
use super::{Lights, PreparedMesh, ScreenVertex, V2, V3, WorldVertex};
use crate::scene3d::{AlphaMode, MaterialKind, ModelMaterial, asset::ModelTextureBinding};

pub(super) struct Triangle<'a> {
    pub kind: MaterialKind,
    pub material: &'a ModelMaterial,
    pub(super) tangent: V3,
    pub(super) bitangent: V3,
    pub(super) has_basis: bool,
}

impl<'a> Triangle<'a> {
    pub fn new(
        kind: MaterialKind,
        material: &'a ModelMaterial,
        vertices: [WorldVertex; 3],
    ) -> Self {
        if material.normal_texture.is_none() {
            return Self {
                kind,
                material,
                tangent: V3::new(0.0, 0.0, 0.0),
                bitangent: V3::new(0.0, 0.0, 0.0),
                has_basis: false,
            };
        }
        let e1 = vertices[1].position - vertices[0].position;
        let e2 = vertices[2].position - vertices[0].position;
        let binding = material
            .normal_texture
            .expect("normal texture checked above");
        let uv = vertices.map(|v| binding.map_uv([v.uv.x, v.uv.y]));
        let u1 = uv[1][0] - uv[0][0];
        let v1 = uv[1][1] - uv[0][1];
        let u2 = uv[2][0] - uv[0][0];
        let v2 = uv[2][1] - uv[0][1];
        let det = u1 * v2 - u2 * v1;
        let has_basis = det.abs() > 1.0e-12;
        let inverse = if has_basis { 1.0 / det } else { 0.0 };
        Self {
            kind,
            material,
            tangent: (e1 * v2 - e2 * v1) * inverse,
            bitangent: (e2 * u1 - e1 * u2) * inverse,
            has_basis,
        }
    }

    fn normal(&self, normal: V3, texel: [f32; 4]) -> V3 {
        if !self.has_basis {
            return normal;
        }
        let t = (self.tangent - normal * normal.dot(self.tangent)).normalized();
        let cross = normal.cross(t);
        let handedness = if cross.dot(self.bitangent) < 0.0 {
            -1.0
        } else {
            1.0
        };
        let b = cross * handedness;
        (t * ((texel[0] * 2.0 - 1.0) * self.material.normal_scale)
            + b * ((texel[1] * 2.0 - 1.0) * self.material.normal_scale)
            + normal * (texel[2] * 2.0 - 1.0))
            .normalized()
    }
}

/// Gradients of the perspective-correct UV quotient, in one output pixel. No neighboring frame
/// or mutable sampling state is needed to choose a mip level.
pub(super) struct Gradients {
    pub(super) dx: [f32; 3], // d(u/z), d(v/z), d(1/z)
    pub(super) dy: [f32; 3],
}

impl Gradients {
    pub fn new(v: [ScreenVertex; 3]) -> Self {
        let area = super::orient_f32(v[0], v[1], v[2]);
        let derivative = |weights: [f32; 3]| {
            let mut result = [0.0; 3];
            for i in 0..3 {
                let w = weights[i] * v[i].inv_z / area;
                result[0] += w * v[i].uv.x;
                result[1] += w * v[i].uv.y;
                result[2] += w;
            }
            result
        };
        Self {
            dx: derivative([v[1].y - v[2].y, v[2].y - v[0].y, v[0].y - v[1].y]),
            dy: derivative([v[2].x - v[1].x, v[0].x - v[2].x, v[1].x - v[0].x]),
        }
    }

    pub fn at(&self, uv: V2, inverse_z: f32) -> [f32; 4] {
        [
            (self.dx[0] - uv.x * self.dx[2]) / inverse_z,
            (self.dx[1] - uv.y * self.dx[2]) / inverse_z,
            (self.dy[0] - uv.x * self.dy[2]) / inverse_z,
            (self.dy[1] - uv.y * self.dy[2]) / inverse_z,
        ]
    }
}

fn sample(mesh: &PreparedMesh, binding: ModelTextureBinding, uv: V2, d: [f32; 4]) -> [f32; 4] {
    let image = &mesh.images[binding.image];
    let [w, h] = image.size().map(|n| n as f32);
    let [dx, dy] = [[d[0], d[1]], [d[2], d[3]]].map(|d| binding.map_derivative(d));
    let d = [dx[0], dx[1], dy[0], dy[1]];
    let dx2 = (d[0] * w) * (d[0] * w) + (d[1] * h) * (d[1] * h);
    let dy2 = (d[2] * w) * (d[2] * w) + (d[3] * h) * (d[3] * h);
    let lod = 0.5 * libm::log2f(dx2.max(dy2).max(1.0e-16));
    image.sample(binding, [uv.x, uv.y], lod)
}

// Isotropic GGX. Material/view terms are shared by every light at this pixel;
// keep their original operation order to preserve the reference f32 results.
struct DirectBrdf {
    nv: f32,
    a2: f32,
    view_visibility: f32,
    diffuse: [f32; 3],
    f0: [f32; 3],
    compensation: [f32; 3],
    transmission: f32,
}

impl DirectBrdf {
    fn new(base: [f32; 3], metallic: f32, roughness: f32, nv: f32, fab: [f32; 2]) -> Self {
        let alpha = roughness * roughness;
        let a2 = alpha * alpha;
        let energy = (fab[0] + fab[1]).max(1e-6);
        let f0 = base.map(|base| 0.04 * (1.0 - metallic) + base * metallic);
        Self {
            nv,
            a2,
            transmission: 0.0,
            view_visibility: libm::sqrtf(nv * nv * (1.0 - a2) + a2),
            diffuse: base.map(|base| base * (1.0 - metallic)),
            f0,
            compensation: f0.map(|f0| 1.0 + f0 * (1.0 / energy - 1.0)),
        }
    }

    fn reflect(&self, n: V3, v: V3, l: V3) -> [f32; 3] {
        let nl = n.dot(l).max(0.0);
        if nl == 0.0 || self.nv == 0.0 {
            return [0.0; 3];
        }
        let h = (v + l).normalized();
        let nh = n.dot(h).max(0.0);
        let vh = v.dot(h).clamp(0.0, 1.0);
        let denom = nh * nh * (self.a2 - 1.0) + 1.0;
        let distribution = self.a2 / (core::f32::consts::PI * denom * denom);
        let visibility = 0.5
            / (nl * self.view_visibility
                + self.nv * libm::sqrtf(nl * nl * (1.0 - self.a2) + self.a2))
            .max(1.0e-8);
        let (dielectric_fresnel, fresnel) = super::fresnel::coefficients(vh, self.f0);
        std::array::from_fn(|i| {
            let f = fresnel[i];
            nl * (self.diffuse[i] * (1.0 - self.transmission) * (1.0 - dielectric_fresnel)
                / core::f32::consts::PI
                + f * distribution * visibility * self.compensation[i])
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn shade(
    mesh: &PreparedMesh,
    triangle: &Triangle<'_>,
    mut n: V3,
    v: V3,
    uv: V2,
    derivatives: [f32; 4],
    position: V3,
    lights: &Lights,
) -> Option<([f32; 3], f32, Option<super::transmission::Sample>)> {
    let m = triangle.material;
    let tex = |binding| sample(mesh, binding, uv, derivatives);
    let base_tex = m.base_color_texture.map(tex).unwrap_or([1.0; 4]);
    let coverage = m.base_color[3] * base_tex[3];
    if m.alpha_cutoff.is_some_and(|cutoff| coverage < cutoff) {
        return None;
    }
    let base = std::array::from_fn::<_, 3, _>(|i| m.base_color[i] * base_tex[i]);
    let mr = m.metallic_roughness_texture.map(tex).unwrap_or([1.0; 4]);
    let metallic = m.metallic * mr[2];
    let roughness = (m.roughness * mr[1]).max(0.0525);
    if let Some(binding) = m.normal_texture {
        n = triangle.normal(n, tex(binding));
    }
    let ao = m
        .occlusion_texture
        .map(|b| 1.0 + m.occlusion_strength * (tex(b)[0] - 1.0))
        .unwrap_or(1.0);
    let emission = m.emissive_texture.map(tex).unwrap_or([1.0; 4]);
    let fab = if triangle.kind == MaterialKind::Pbr {
        super::lighting::dfg(roughness, n.dot(v).clamp(0.0, 1.0))
    } else {
        [0.0; 2]
    };
    let mut radiance = match triangle.kind {
        MaterialKind::Pbr => indirect(
            base,
            metallic,
            roughness,
            n,
            v,
            ao,
            lights,
            fab,
            m.transmission,
        ),
        MaterialKind::Lambert => {
            let factor = lights.factor(n, ao, position);
            std::array::from_fn(|i| base[i] * factor[i])
        }
        MaterialKind::Unlit => base,
    };
    for i in 0..3 {
        radiance[i] += m.emissive[i] * m.emissive_intensity * emission[i];
    }
    if triangle.kind == MaterialKind::Pbr && !lights.directional.is_empty() {
        let mut brdf = DirectBrdf::new(base, metallic, roughness, n.dot(v).max(0.0), fab);
        brdf.transmission = m.transmission;
        for (index, &(direction, intensity)) in lights.directional.iter().enumerate() {
            let reflected = brdf.reflect(n, v, direction);
            // A back-facing light contributes zero regardless of its shadow map.
            if reflected == [0.0; 3] {
                continue;
            }
            let visibility = lights.shadow_visibility(index, position, n);
            for i in 0..3 {
                radiance[i] += reflected[i] * intensity[i] * visibility;
            }
        }
    }
    Some((
        radiance,
        if m.alpha_mode == AlphaMode::Blend {
            coverage
        } else {
            1.0
        },
        (m.transmission > 0.0)
            .then(|| super::transmission::Sample::new(m, n, v, base, metallic, roughness)),
    ))
}

fn scattering(f0: f32, fab: [f32; 2]) -> [f32; 2] {
    let single = f0 * fab[0] + fab[1];
    let missing = 1.0 - fab[0] - fab[1];
    let average = f0 + (1.0 - f0) / 21.0;
    [
        single,
        single * average / (1.0 - missing * average) * missing,
    ]
}

#[allow(clippy::too_many_arguments)]
fn indirect(
    base: [f32; 3],
    metallic: f32,
    roughness: f32,
    n: V3,
    v: V3,
    ao: f32,
    lights: &Lights,
    fab: [f32; 2],
    transmission: f32,
) -> [f32; 3] {
    let nv = n.dot(v).clamp(0.0, 1.0);
    let dielectric = scattering(0.04, fab);
    let diffuse_energy = 1.0 - dielectric[0] - dielectric[1];
    let mut irradiance = lights.ambient.map(|v| v / core::f32::consts::PI);
    for &(direction, sky, ground) in &lights.hemisphere {
        let t = n.dot(direction) * 0.5 + 0.5;
        for i in 0..3 {
            irradiance[i] += (ground[i] * (1.0 - t) + sky[i] * t) / core::f32::consts::PI;
        }
    }
    let mut result = std::array::from_fn(|i| {
        base[i] * (1.0 - transmission) * (1.0 - metallic) * diffuse_energy * irradiance[i] * ao
    });
    if let Some(environment) = &lights.environment {
        let reflection = n * (2.0 * n.dot(v)) - v;
        let r4 = roughness * roughness * roughness * roughness;
        let reflection = (reflection * (1.0 - r4) + n * r4).normalized();
        let radiance = environment
            .specular(lights.environment_direction(reflection), roughness)
            .map(|v| v * lights.environment_intensity);
        let diffuse = environment
            .diffuse(lights.environment_direction(n))
            .map(|v| v * lights.environment_intensity);
        let specular_ao =
            (libm::powf(nv + ao, libm::exp2f(-16.0 * roughness - 1.0)) - 1.0 + ao).clamp(0.0, 1.0);
        for i in 0..3 {
            let metal = scattering(base[i], fab);
            let single = dielectric[0] * (1.0 - metallic) + metal[0] * metallic;
            let multi = dielectric[1] * (1.0 - metallic) + metal[1] * metallic;
            result[i] += (radiance[i] * single + diffuse[i] * multi) * specular_ao
                + base[i]
                    * (1.0 - transmission)
                    * (1.0 - metallic)
                    * diffuse_energy
                    * diffuse[i]
                    * ao;
        }
    }
    result
}

#[cfg(test)]
#[path = "pbr_reference.rs"]
mod reference;

#[cfg(test)]
mod tests {
    use super::*;

    fn direct(base: [f32; 3], metallic: f32, roughness: f32, n: V3, v: V3, l: V3) -> [f32; 3] {
        let nv = n.dot(v).max(0.0);
        DirectBrdf::new(
            base,
            metallic,
            roughness,
            nv,
            super::super::lighting::dfg(roughness, nv),
        )
        .reflect(n, v, l)
    }

    #[test]
    fn shared_brdf_terms_match_reference_before_color_quantization() {
        let mut seed = 41_u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / 16_777_215.0
        };
        for _ in 0..10_000 {
            let base = [next(), next(), next()];
            let metallic = next();
            let roughness = next().max(0.0525);
            let [n, v, l] = std::array::from_fn(|_| {
                V3::new(next() * 2.0 - 1.0, next() * 2.0 - 1.0, next() * 2.0 - 1.0).normalized()
            });
            assert_eq!(
                direct(base, metallic, roughness, n, v, l).map(f32::to_bits),
                reference::direct(base, metallic, roughness, n, v, l).map(f32::to_bits),
            );
        }
    }

    #[test]
    fn ggx_known_normal_incidence_and_grazing_are_finite() {
        let z = V3::new(0.0, 0.0, 1.0);
        // At N=V=L and roughness=1, D=1/pi, V=1/4 and F=F0. Expected values
        // include the independently tabulated DFG white-furnace energy 0.3452853560447693.
        let metal = direct([0.8, 0.4, 0.2], 1.0, 1.0, z, z, z);
        for i in 0..3 {
            assert!((metal[i] - [0.16023237, 0.05597359, 0.02195114][i]).abs() < 1e-6);
        }
        let dielectric = direct([0.8, 0.4, 0.2], 0.0, 1.0, z, z, z);
        for i in 0..3 {
            assert!((dielectric[i] - [0.24788652, 0.12565552, 0.06454002][i]).abs() < 1e-6);
        }
        assert_eq!(direct([1.0; 3], 1.0, 0.0525, z, V3::Y, z), [0.0; 3]);
    }

    #[test]
    fn uv_quotient_gradients_match_finite_differences() {
        let v = [
            (0.0, 0.0, 1.0, 0.0, 0.0),
            (8.0, 0.0, 0.5, 1.0, 0.0),
            (0.0, 8.0, 0.25, 0.0, 1.0),
        ]
        .map(|(x, y, inv_z, u, v)| ScreenVertex {
            x,
            y,
            inv_z,
            view_z: 1.0 / inv_z,
            normal: V3::Y,
            uv: V2 { x: u, y: v },
        });
        let evaluate = |x: f32, y: f32| {
            let bary = [1.0 - x / 8.0 - y / 8.0, x / 8.0, y / 8.0];
            let sum = (0..3).map(|i| bary[i] * v[i].inv_z).sum::<f32>();
            (
                super::super::interpolated_uv(v, super::super::perspective_weights(bary, v, sum)),
                sum,
            )
        };
        let (uv, s) = evaluate(2.0, 2.0);
        let d = Gradients::new(v).at(uv, s);
        let step = 0.001;
        let (x, _) = evaluate(2.0 + step, 2.0);
        let (y, _) = evaluate(2.0, 2.0 + step);
        for (actual, expected) in d.into_iter().zip([
            (x.x - uv.x) / step,
            (x.y - uv.y) / step,
            (y.x - uv.x) / step,
            (y.y - uv.y) / step,
        ]) {
            assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
        }
    }
}
