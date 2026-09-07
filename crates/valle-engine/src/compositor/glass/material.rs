//! Author intent to physical Motion Glass material.
//!
//! `clarity` and `depth` remain the compact author surface. They resolve once into a bounded,
//! backend-neutral optical description; neither Native nor Web is allowed to reinterpret them.

use super::reference::materialization;
use valle_draw::program::glass::{PackedGlassLight, PackedGlassMaterial};

const REFERENCE_RADIUS: f64 = 40.0;

// Canonical device-space optical ratios shared by material resolution, sampling bounds, and the
// reference raster. Native/Web kernels mirror these constants byte-for-byte in motionglass.sksl.
pub(super) const MAX_REFRACTION_BEVEL_FRACTION: f64 = 0.30;
pub(super) const OPTICAL_PATH_THICKNESS_FACTOR: f64 = 1.50;
const DIFFUSION_THICKNESS_FACTOR: f64 = 0.75;
const INTERFACE_THICKNESS_FACTOR: f64 = 0.13;

pub(super) fn diffusion_radius(roughness: f64, thickness: f64) -> f64 {
    roughness.max(0.0) * thickness.max(0.0) * DIFFUSION_THICKNESS_FACTOR
}

pub(super) fn interface_width(thickness: f64) -> f64 {
    thickness.max(0.0) * INTERFACE_THICKNESS_FACTOR
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedGlassMaterialBase {
    pub bevel_width: f64,
    pub thickness: f64,
    pub refractive_index: f64,
    pub roughness: f64,
    pub dispersion: f64,
    pub tint_linear: [f32; 4],
    pub specular_strength: f64,
    pub shadow_strength: f64,
    pub foreground_gain: f64,
}

fn thickness_for_radius(radius: f64, depth: f64) -> f64 {
    // Optical displacement must stay below the width of the curved transition. Otherwise the
    // backdrop coordinate folds back on itself and produces a caustic-like line where the bevel
    // meets the flat center. Keep enough path length to read as glass without turning a compact
    // control into a magnifying lens.
    radius.max(0.0) * (0.115 + 0.185 * depth)
}

fn bevel_width_for_radius(radius: f64, depth: f64) -> f64 {
    // Spread the edge normal over a wide-enough C2 rolloff that the refracted sampling map stays
    // monotone. This is the optical transition inside the outline, not an extra visible border.
    // Let the rounded normal field span nearly the whole short axis. A narrow bevel leaves a large
    // dead center that reads as a blurred translucent panel rather than a continuous lens.
    radius.max(0.0) * (0.780 + 0.190 * depth)
}

pub fn resolve_material_base(
    clarity: f64,
    depth: f64,
    tint: [f32; 4],
    presence: f64,
    protection: f64,
) -> Result<ResolvedGlassMaterialBase, &'static str> {
    if ![clarity, depth, presence, protection]
        .into_iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
        || !tint
            .into_iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
    {
        return Err("clarity, depth, tint, presence, and protection must be finite in 0..=1");
    }

    let presence_m = materialization(presence);
    let optical_depth = 0.18 + 0.82 * depth;
    let clarity_ior = 0.82 + 0.18 * (1.0 - clarity);
    let refractive_index = 1.0 + (0.06 + 0.22 * optical_depth * clarity_ior) * presence_m;
    // Clear materials should preserve real backdrop detail. Squaring the author response gives the
    // lower half of the control useful frosted range without making clarity 0.8-0.95 look milky.
    let roughness = (1.0 - clarity).powf(1.75) * (0.48 + 0.42 * depth);
    let dispersion = (0.0015 + 0.009 * depth * (0.25 + 0.75 * (1.0 - clarity))) * presence_m;
    let specular_strength = (0.12 + 0.40 * depth * (0.60 + 0.40 * clarity)) * presence_m;
    let shadow_strength = (0.018 + 0.055 * depth) * presence_m;
    let mut tint_linear = tint;
    // Alpha is the only tint-energy authority. A zero-alpha tint is therefore exactly dormant.
    tint_linear[3] *= ((0.18 + 0.52 * depth) * presence_m) as f32;

    Ok(ResolvedGlassMaterialBase {
        bevel_width: bevel_width_for_radius(REFERENCE_RADIUS, depth) * presence_m,
        thickness: thickness_for_radius(REFERENCE_RADIUS, depth) * presence_m,
        refractive_index,
        roughness,
        dispersion,
        tint_linear,
        specular_strength,
        shadow_strength,
        foreground_gain: (0.2 + 0.8 * protection) * presence_m,
    })
}

/// Backend-neutral conversion of the resolved material into typed execution data.
pub fn pack_material_base(resolved: &ResolvedGlassMaterialBase) -> PackedGlassMaterial {
    PackedGlassMaterial {
        bevel_width: resolved.bevel_width as f32,
        thickness: resolved.thickness as f32,
        refractive_index: resolved.refractive_index as f32,
        roughness: resolved.roughness as f32,
        dispersion: resolved.dispersion as f32,
        tint_linear: resolved.tint_linear,
        specular_strength: resolved.specular_strength as f32,
        shadow_strength: resolved.shadow_strength as f32,
        foreground_gain: resolved.foreground_gain as f32,
        light: PackedGlassLight::DEFAULT,
    }
}

/// Resolves the lens profile in device pixels. Thickness is a fraction of the actual surface
/// radius, so small controls do not inherit a desktop-sized optical boundary and large lenses do
/// not become visually flat.
pub fn resolve_material_base_with_geometry(
    clarity: f64,
    depth: f64,
    tint: [f32; 4],
    presence: f64,
    protection: f64,
    device_radius: f64,
) -> Result<ResolvedGlassMaterialBase, &'static str> {
    if !device_radius.is_finite() || device_radius <= 0.0 {
        return Err("device radius must be finite and positive");
    }
    let mut resolved = resolve_material_base(clarity, depth, tint, presence, protection)?;
    resolved.bevel_width = bevel_width_for_radius(device_radius, depth) * materialization(presence);
    resolved.thickness = thickness_for_radius(device_radius, depth) * materialization(presence);
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispersion_stays_inside_the_admitted_ior_range() {
        for clarity in [0.0, 0.5, 1.0] {
            for depth in [0.0, 0.5, 1.0] {
                let resolved =
                    resolve_material_base(clarity, depth, [0.1, 0.2, 0.3, 0.5], 1.0, 0.65).unwrap();
                assert!(resolved.refractive_index - resolved.dispersion >= 1.0);
                assert!(resolved.refractive_index + resolved.dispersion <= 1.6);
            }
        }
    }

    #[test]
    fn presence_zero_is_an_exact_dormant_material() {
        let resolved = resolve_material_base(0.82, 0.52, [0.2, 0.3, 0.4, 0.5], 0.0, 0.65).unwrap();
        assert_eq!(resolved.thickness, 0.0);
        assert_eq!(resolved.bevel_width, 0.0);
        assert_eq!(resolved.refractive_index, 1.0);
        assert_eq!(resolved.dispersion, 0.0);
        assert_eq!(resolved.tint_linear[3], 0.0);
        assert_eq!(resolved.specular_strength, 0.0);
        assert_eq!(resolved.foreground_gain, 0.0);
    }

    #[test]
    fn zero_alpha_tint_cannot_create_color_energy() {
        let resolved = resolve_material_base(0.82, 0.52, [1.0, 1.0, 1.0, 0.0], 1.0, 0.65).unwrap();
        assert_eq!(resolved.tint_linear[3], 0.0);
    }

    #[test]
    fn geometry_sets_a_monotone_profile_width() {
        let small =
            resolve_material_base_with_geometry(0.82, 0.52, [0.0; 4], 1.0, 0.65, 12.0).unwrap();
        let large =
            resolve_material_base_with_geometry(0.82, 0.52, [0.0; 4], 1.0, 0.65, 80.0).unwrap();
        assert!(small.thickness > 0.0);
        assert!(small.bevel_width > 0.0);
        assert!(large.thickness > small.thickness);
        assert!(large.bevel_width > small.bevel_width);
        assert!(large.bevel_width > large.thickness);
        assert!(large.thickness / large.bevel_width < 0.45);
    }

    #[test]
    fn material_lengths_scale_linearly_from_720p_to_4k() {
        let at_720 =
            resolve_material_base_with_geometry(0.68, 0.72, [0.0; 4], 1.0, 0.96, 48.0).unwrap();
        let at_4k =
            resolve_material_base_with_geometry(0.68, 0.72, [0.0; 4], 1.0, 0.96, 144.0).unwrap();
        assert!((at_4k.bevel_width - at_720.bevel_width * 3.0).abs() < 1.0e-12);
        assert!((at_4k.thickness - at_720.thickness * 3.0).abs() < 1.0e-12);
        assert!(
            (diffusion_radius(at_4k.roughness, at_4k.thickness)
                - diffusion_radius(at_720.roughness, at_720.thickness) * 3.0)
                .abs()
                < 1.0e-12
        );
        assert!(
            (interface_width(at_4k.thickness) - interface_width(at_720.thickness) * 3.0).abs()
                < 1.0e-12
        );
        assert_eq!(at_4k.roughness, at_720.roughness);
        assert_eq!(at_4k.refractive_index, at_720.refractive_index);
    }
}
