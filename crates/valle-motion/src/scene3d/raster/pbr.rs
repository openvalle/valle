//! Opaque and alpha-mask glTF shading. Inputs belong to this frame or admitted resources.
use super::{Lights, PreparedMesh, ScreenVertex, V2, V3, WorldVertex};
use crate::scene3d::{MaterialKind, ModelMaterial, asset::ModelTextureBinding};

pub(super) struct Triangle<'a> {
    pub kind: MaterialKind,
    pub material: &'a ModelMaterial,
    tangent: V3,
    bitangent: V3,
    has_basis: bool,
}

impl<'a> Triangle<'a> {
    pub fn new(
        kind: MaterialKind,
        material: &'a ModelMaterial,
        vertices: [WorldVertex; 3],
    ) -> Self {
        let e1 = vertices[1].position - vertices[0].position;
        let e2 = vertices[2].position - vertices[0].position;
        let u1 = vertices[1].uv.x - vertices[0].uv.x;
        let v1 = vertices[1].uv.y - vertices[0].uv.y;
        let u2 = vertices[2].uv.x - vertices[0].uv.x;
        let v2 = vertices[2].uv.y - vertices[0].uv.y;
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
    dx: [f32; 3], // d(u/z), d(v/z), d(1/z)
    dy: [f32; 3],
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
    let dx2 = (d[0] * w) * (d[0] * w) + (d[1] * h) * (d[1] * h);
    let dy2 = (d[2] * w) * (d[2] * w) + (d[3] * h) * (d[3] * h);
    let lod = 0.5 * libm::log2f(dx2.max(dy2).max(1.0e-16));
    image.sample(binding, [uv.x, uv.y], lod)
}

// Isotropic GGX with height-correlated Smith visibility and Schlick Fresnel.
fn direct(base: [f32; 3], metallic: f32, roughness: f32, n: V3, v: V3, l: V3) -> [f32; 3] {
    let nl = n.dot(l).max(0.0);
    let nv = n.dot(v).max(0.0);
    if nl == 0.0 || nv == 0.0 {
        return [0.0; 3];
    }
    let h = (v + l).normalized();
    let nh = n.dot(h).max(0.0);
    let vh = v.dot(h).clamp(0.0, 1.0);
    let alpha = roughness * roughness;
    let a2 = alpha * alpha;
    let denom = nh * nh * (a2 - 1.0) + 1.0;
    let distribution = a2 / (core::f32::consts::PI * denom * denom);
    let visibility = 0.5
        / (nl * libm::sqrtf(nv * nv * (1.0 - a2) + a2)
            + nv * libm::sqrtf(nl * nl * (1.0 - a2) + a2))
        .max(1.0e-8);
    let fresnel = libm::powf(1.0 - vh, 5.0);
    let fab = super::lighting::dfg(roughness, nv);
    let energy = (fab[0] + fab[1]).max(1e-6);
    std::array::from_fn(|i| {
        let f0 = 0.04 * (1.0 - metallic) + base[i] * metallic;
        let f = f0 + (1.0 - f0) * fresnel;
        let compensation = 1.0 + f0 * (1.0 / energy - 1.0);
        let dielectric_fresnel = 0.04 + 0.96 * fresnel;
        nl * (base[i] * (1.0 - metallic) * (1.0 - dielectric_fresnel) / core::f32::consts::PI
            + f * distribution * visibility * compensation)
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn shade(
    mesh: &PreparedMesh,
    triangle: &Triangle<'_>,
    mut n: V3,
    v: V3,
    uv: V2,
    derivatives: [f32; 4],
    lights: &Lights,
) -> Option<[u8; 4]> {
    let m = triangle.material;
    let tex = |binding| sample(mesh, binding, uv, derivatives);
    let base_tex = m.base_color_texture.map(tex).unwrap_or([1.0; 4]);
    if m.alpha_cutoff
        .is_some_and(|cutoff| m.base_color[3] * base_tex[3] < cutoff)
    {
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
    let mut radiance = match triangle.kind {
        MaterialKind::Pbr => indirect(base, metallic, roughness, n, v, ao, lights),
        MaterialKind::Lambert => {
            let factor = lights.factor(n, ao);
            std::array::from_fn(|i| base[i] * factor[i])
        }
        MaterialKind::Unlit => base,
    };
    for i in 0..3 {
        radiance[i] += m.emissive[i] * m.emissive_intensity * emission[i];
    }
    if triangle.kind == MaterialKind::Pbr {
        for &(direction, intensity) in &lights.directional {
            let reflected = direct(base, metallic, roughness, n, v, direction);
            for i in 0..3 {
                radiance[i] += reflected[i] * intensity[i];
            }
        }
    }
    Some(super::output_color(
        radiance,
        1.0,
        lights.pbr.tone_mapping,
        lights.exposure,
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

fn indirect(
    base: [f32; 3],
    metallic: f32,
    roughness: f32,
    n: V3,
    v: V3,
    ao: f32,
    lights: &Lights,
) -> [f32; 3] {
    let nv = n.dot(v).clamp(0.0, 1.0);
    let fab = super::lighting::dfg(roughness, nv);
    let dielectric = scattering(0.04, fab);
    let diffuse_energy = 1.0 - dielectric[0] - dielectric[1];
    let mut irradiance = lights.ambient.map(|v| v / core::f32::consts::PI);
    for &(direction, sky, ground) in &lights.hemisphere {
        let t = n.dot(direction) * 0.5 + 0.5;
        for i in 0..3 {
            irradiance[i] += (ground[i] * (1.0 - t) + sky[i] * t) / core::f32::consts::PI;
        }
    }
    let mut result =
        std::array::from_fn(|i| base[i] * (1.0 - metallic) * diffuse_energy * irradiance[i] * ao);
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
                + base[i] * (1.0 - metallic) * diffuse_energy * diffuse[i] * ao;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
