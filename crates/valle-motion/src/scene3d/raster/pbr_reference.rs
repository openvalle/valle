// Original per-light GGX evaluation, retained only as an exact numerical test oracle.
use super::V3;

pub(super) fn direct(
    base: [f32; 3],
    metallic: f32,
    roughness: f32,
    n: V3,
    v: V3,
    l: V3,
) -> [f32; 3] {
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
    let fab = super::super::lighting::dfg(roughness, nv);
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
