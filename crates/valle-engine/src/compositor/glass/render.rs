//! G1.5 canonical RGBA32F raster for an independent Motion Glass surface.
//!
//! Executes the typed Glass program against a working-color premultiplied backdrop: analytic
//! shape/height/normal, bounded refraction, diffusion with detail preservation, tint, edge and
//! shadow, materialization coverage, and the packed current response as a bounded refraction
//! bias. Output is a transparent local contribution (premul) that the compositor source-overs
//! once; the raster never writes back to the backdrop and never reads a previous frame.
//!
//! This is the F32 reference against which the G1.6 Native/Web kernels are compared; it is not
//! a production fast path.

use thiserror::Error;
use valle_draw::Rect;
use valle_draw::program::glass::{
    GlassOwnerKind, MotionGlassForegroundProgram, MotionGlassProgram, PackedGlassMotion,
    PackedGlassShapeKind, verify_kernel,
};

use crate::compositor::glass::material::{
    MAX_REFRACTION_BEVEL_FRACTION, OPTICAL_PATH_THICKNESS_FACTOR, diffusion_radius, interface_width,
};
use crate::compositor::glass::reference::materialization;
use crate::compositor::glass::sampling::sample_bilinear;
use crate::compositor::glass::{
    field_pseudo_distance, potential_w, validate_motion_glass_backdrop_bounds,
};
use crate::compositor::reference::{PremulRgba32, ReferenceImage};

const IDENTITY_MATRIX: [f64; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
/// Reference raster sample budget (same order of magnitude as the compositor reference).
const MAX_RASTER_PIXELS: u64 = 64 * 1024 * 1024;
const FOREGROUND_COVERAGE_SAMPLES_PER_AXIS: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GlassRenderError {
    #[error("Glass raster requires an independent program")]
    NotIndependent,
    #[error("Glass field raster requires a field program")]
    NotField,
    #[error("Glass raster requires exactly one surface")]
    SurfaceCount,
    #[error("Glass program carries a foreign or mismatched kernel")]
    KernelMismatch,
    #[error("Glass program is structurally invalid")]
    InvalidProgram,
    #[error("backdrop does not cover the Glass sample bounds")]
    BackdropCoverage,
    #[error("Glass raster budget exceeded")]
    RasterBudget,
    #[error("Glass raster produced a non-finite pixel")]
    NonFinitePixel,
}

/// Rasterizes the Glass contribution over `backdrop` (same extent, mostly transparent).
///
/// The surface's device rect and radius are in the backdrop's pixel coordinate space. The
/// backdrop covers the larger sampling footprint, but raster work is restricted to the visible
/// output ROI. The returned image is the local contribution `G`; use
/// [`composite_glass_contribution`] to source-over it onto the backdrop.
pub fn render_glass_contribution(
    program: &MotionGlassProgram,
    backdrop: &ReferenceImage,
) -> Result<ReferenceImage, GlassRenderError> {
    render_glass_contribution_transformed(program, IDENTITY_MATRIX, backdrop)
}

/// Transform-aware reference path used by DrawProgram. Geometry stays in the material owner's
/// local space while response and optical sampling are device-qualified.
pub fn render_glass_contribution_transformed(
    program: &MotionGlassProgram,
    owner_to_device: [f64; 9],
    backdrop: &ReferenceImage,
) -> Result<ReferenceImage, GlassRenderError> {
    if program.owner_kind != GlassOwnerKind::Independent {
        return Err(GlassRenderError::NotIndependent);
    }
    if program.surfaces.len() != 1 {
        return Err(GlassRenderError::SurfaceCount);
    }
    // G1.6 binding checks: capability/ABI/kernel identity must match before any pixel work.
    verify_kernel(program).map_err(|_| GlassRenderError::KernelMismatch)?;
    program
        .validate()
        .map_err(|_| GlassRenderError::InvalidProgram)?;
    validate_motion_glass_backdrop_bounds(program, owner_to_device)
        .map_err(|_| GlassRenderError::BackdropCoverage)?;
    let surface = &program.surfaces[0];
    let extent = backdrop.extent();
    let output = transformed_rect_bounds(program.backdrop.output_bounds, owner_to_device)
        .ok_or(GlassRenderError::BackdropCoverage)?;
    let left = output.left().floor().max(0.0) as u32;
    let top = output.top().floor().max(0.0) as u32;
    let right = (output.right().ceil() as i64).min(i64::from(extent.width())) as u32;
    let bottom = (output.bottom().ceil() as i64).min(i64::from(extent.height())) as u32;
    if left >= right || top >= bottom {
        return ReferenceImage::transparent(extent).map_err(|_| GlassRenderError::NonFinitePixel);
    }
    let pixel_count = u64::from(right - left) * u64::from(bottom - top);
    if pixel_count > MAX_RASTER_PIXELS {
        return Err(GlassRenderError::RasterBudget);
    }

    let width = extent.width() as usize;
    let prepared_surface = PreparedTransformedSurface::new(surface, owner_to_device)
        .ok_or(GlassRenderError::BackdropCoverage)?;
    let mut pixels = vec![PremulRgba32::TRANSPARENT; width * extent.height() as usize];
    for y in top..bottom {
        for x in left..right {
            let p = [f64::from(x) + 0.5, f64::from(y) + 0.5];
            let pixel = pixel_contribution(program, &prepared_surface, p, backdrop)?;
            pixels[y as usize * width + x as usize] = pixel;
        }
    }
    ReferenceImage::new(extent, pixels).map_err(|_| GlassRenderError::NonFinitePixel)
}

/// Clips an already-rendered, ordinary foreground subtree to its Glass surface in device space.
///
/// The input remains the source of truth for foreground pixels: this pass only applies the
/// materialization/presence coverage described by `foreground`. Keeping the operation here makes
/// Reference, Native and Web execute the identical coverage rule instead of synthesizing a fake
/// foreground primitive in either backend.
pub fn apply_glass_foreground_transformed(
    foreground: &MotionGlassForegroundProgram,
    owner_to_device: [f64; 9],
    input: &ReferenceImage,
) -> Result<ReferenceImage, GlassRenderError> {
    foreground
        .validate()
        .map_err(|_| GlassRenderError::InvalidProgram)?;
    let extent = input.extent();
    let pixel_count = u64::from(extent.width()) * u64::from(extent.height());
    if pixel_count > MAX_RASTER_PIXELS {
        return Err(GlassRenderError::RasterBudget);
    }

    let presence = materialization(f64::from(foreground.presence));
    if presence <= 0.0 {
        return ReferenceImage::transparent(extent).map_err(|_| GlassRenderError::NonFinitePixel);
    }
    let local_to_device = matrix_mul(owner_to_device, foreground.local_to_owner.map(f64::from));
    if !rect_has_no_horizon(foreground.rect, local_to_device) {
        return Err(GlassRenderError::BackdropCoverage);
    }
    let device_to_local =
        matrix_inverse(local_to_device).ok_or(GlassRenderError::BackdropCoverage)?;

    let mut pixels = vec![PremulRgba32::TRANSPARENT; pixel_count as usize];
    let width = extent.width() as usize;
    for y in 0..extent.height() {
        for x in 0..extent.width() {
            let source = input.pixel(x, y).ok_or(GlassRenderError::NonFinitePixel)?;
            if source.alpha() == 0.0 {
                continue;
            }
            let mut inside = 0_u32;
            for sample_y in 0..FOREGROUND_COVERAGE_SAMPLES_PER_AXIS {
                for sample_x in 0..FOREGROUND_COVERAGE_SAMPLES_PER_AXIS {
                    let point = [
                        f64::from(x)
                            + (f64::from(sample_x) + 0.5)
                                / f64::from(FOREGROUND_COVERAGE_SAMPLES_PER_AXIS),
                        f64::from(y)
                            + (f64::from(sample_y) + 0.5)
                                / f64::from(FOREGROUND_COVERAGE_SAMPLES_PER_AXIS),
                    ];
                    let Some(sample) = transformed_shape_sample_prepared(
                        foreground.shape,
                        foreground.rect,
                        device_to_local,
                        f64::from(foreground.radius),
                        &foreground.path_points,
                        point,
                    ) else {
                        continue;
                    };
                    let effective_distance = sample.distance + (1.0 - presence) * sample.band;
                    inside += u32::from(effective_distance <= 0.0);
                }
            }
            let coverage = inside as f32
                / (FOREGROUND_COVERAGE_SAMPLES_PER_AXIS * FOREGROUND_COVERAGE_SAMPLES_PER_AXIS)
                    as f32;
            pixels[y as usize * width + x as usize] = source
                .scale_coverage(coverage)
                .map_err(|_| GlassRenderError::NonFinitePixel)?;
        }
    }
    ReferenceImage::new(extent, pixels).map_err(|_| GlassRenderError::NonFinitePixel)
}

/// Source-overs the Glass contribution onto the backdrop (the single composite).
pub fn composite_glass_contribution(
    contribution: &ReferenceImage,
    backdrop: &ReferenceImage,
) -> Result<ReferenceImage, GlassRenderError> {
    contribution
        .source_over(backdrop)
        .map_err(|_| GlassRenderError::NonFinitePixel)
}

// ---- G4.3 Native CPU F16 kernel: the same optics with RGBA16F transport ----

/// IEEE 754 half-precision round-trip of one f32 channel (round-to-nearest-even).
fn f16_round_trip(value: f32) -> f32 {
    if !value.is_finite() {
        return value;
    }
    let bits = value.to_bits();
    let sign = (bits >> 31) & 0x1;
    let exponent = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x7F_FFFF;
    if exponent == 0 {
        // f32 subnormal: below the f16 normal range; round to zero.
        return 0.0;
    }
    let mut half_exp = exponent - 127 + 15;
    if half_exp <= 0 {
        return 0.0;
    }
    if half_exp >= 0x1F {
        // Overflow: saturate to f16 max (65504.0).
        return 65504.0 * if sign == 1 { -1.0 } else { 1.0 };
    }
    let mut half_mantissa = mantissa >> 13;
    let rest = mantissa & 0x1FFF;
    if rest > 0x1000 || (rest == 0x1000 && (half_mantissa & 1) == 1) {
        half_mantissa += 1;
        if half_mantissa >= 0x400 {
            half_mantissa = 0;
            half_exp += 1;
            if half_exp >= 0x1F {
                return 65504.0 * if sign == 1 { -1.0 } else { 1.0 };
            }
        }
    }
    let half = (sign << 15) | ((half_exp as u32) << 10) | half_mantissa;
    f16_to_f32(half)
}

fn f16_to_f32(half: u32) -> f32 {
    let sign = (half >> 15) & 1;
    let exponent = (half >> 10) & 0x1F;
    let mantissa = half & 0x3FF;
    if exponent == 0 {
        // Subnormal half: value = mantissa * 2^-24.
        if mantissa == 0 {
            return 0.0;
        }
        let value = mantissa as f32 / 16_777_216.0;
        return if sign == 1 { -value } else { value };
    }
    let bits = (sign << 31) | ((exponent + 127 - 15) << 23) | (mantissa << 13);
    f32::from_bits(bits)
}

/// F16 transport round-trip of a working-color premul image (RGBA16F contract).
pub fn quantize_to_f16(image: &ReferenceImage) -> ReferenceImage {
    let extent = image.extent();
    let pixels = image
        .pixels()
        .iter()
        .map(|pixel| {
            let channels = pixel.channels();
            PremulRgba32::from_premultiplied([
                f16_round_trip(channels[0]),
                f16_round_trip(channels[1]),
                f16_round_trip(channels[2]),
                f16_round_trip(channels[3]),
            ])
            .unwrap_or(PremulRgba32::TRANSPARENT)
        })
        .collect::<Vec<_>>();
    ReferenceImage::new(extent, pixels).unwrap_or_else(|_| image.clone())
}

/// Native CPU F16 kernel: the shared optics over an RGBA16F-transported backdrop, with an
/// RGBA16F output. Math is computed in f32 (as GPUs do); transport and storage are f16.
pub fn render_glass_contribution_f16(
    program: &MotionGlassProgram,
    backdrop: &ReferenceImage,
) -> Result<ReferenceImage, GlassRenderError> {
    render_glass_contribution_f16_transformed(program, IDENTITY_MATRIX, backdrop)
}

/// Transform-aware F16 transport path used by ROI product executors and fixed runners.
pub fn render_glass_contribution_f16_transformed(
    program: &MotionGlassProgram,
    owner_to_device: [f64; 9],
    backdrop: &ReferenceImage,
) -> Result<ReferenceImage, GlassRenderError> {
    let quantized = quantize_to_f16(backdrop);
    let contribution = render_glass_contribution_transformed(program, owner_to_device, &quantized)?;
    Ok(quantize_to_f16(&contribution))
}

/// Applies a soft-edged rectangular mask (device pixel space) to a Glass contribution: outside
/// the mask the contribution is fully transparent, so the composite returns to the backdrop.
pub fn apply_rect_mask(image: &ReferenceImage, mask: Rect, softness: f64) -> ReferenceImage {
    if mask.is_empty() {
        return image.clone();
    }
    let extent = image.extent();
    let mut pixels = image.pixels().to_vec();
    let width = extent.width() as usize;
    for y in 0..extent.height() {
        for x in 0..extent.width() {
            let p = [f64::from(x) + 0.5, f64::from(y) + 0.5];
            let coverage = rect_coverage(p, mask, softness);
            if coverage < 1.0 {
                let index = y as usize * width + x as usize;
                let channels = pixels[index].channels();
                let factor = coverage as f32;
                if let Ok(masked) = PremulRgba32::from_premultiplied([
                    channels[0] * factor,
                    channels[1] * factor,
                    channels[2] * factor,
                    channels[3] * factor,
                ]) {
                    pixels[index] = masked;
                }
            }
        }
    }
    ReferenceImage::new(extent, pixels).unwrap_or_else(|_| image.clone())
}

fn rect_coverage(p: [f64; 2], rect: Rect, softness: f64) -> f64 {
    let dx = (rect.left() - p[0]).max(p[0] - rect.right()).max(0.0);
    let dy = (rect.top() - p[1]).max(p[1] - rect.bottom()).max(0.0);
    let distance = dx.hypot(dy);
    if distance <= 0.0 {
        1.0
    } else if softness <= 0.0 || distance >= softness {
        0.0
    } else {
        1.0 - distance / softness
    }
}

fn pixel_contribution(
    program: &MotionGlassProgram,
    surface: &PreparedTransformedSurface<'_>,
    p: [f64; 2],
    backdrop: &ReferenceImage,
) -> Result<PremulRgba32, GlassRenderError> {
    let presence_m = materialization(f64::from(surface.surface.presence));
    if presence_m <= 0.0 {
        return Ok(PremulRgba32::TRANSPARENT);
    }
    let Some(shape) = surface.sample(p) else {
        return Ok(PremulRgba32::TRANSPARENT);
    };
    let sdf = shape.distance;
    let band = shape.band;
    if band <= 0.0 || !sdf.is_finite() {
        return Ok(PremulRgba32::TRANSPARENT);
    }
    // Presence moves the material boundary inward. Height remains the smooth normalized lens
    // profile, while coverage is a separate one-pixel C2 edge. Coupling coverage to height made
    // the entire lens semi-transparent except at its center and disagreed with Field semantics.
    let effective_sdf = sdf + (1.0 - presence_m) * band;
    let coverage = materialization((0.5 - effective_sdf).clamp(0.0, 1.0));
    let material = if coverage > 0.0 {
        optical_pixel(
            program,
            backdrop,
            OpticalPixelInput {
                signed_distance: effective_sdf,
                normal: shape.normal,
                coverage,
                motion: surface.surface.response,
                strain: 0.0,
                device_point: p,
            },
        )?
    } else {
        PremulRgba32::TRANSPARENT
    };
    Ok(material)
}

/// G2.3 field raster: the C2 potential field (Σ W(sdf_i / mergeDistance)) is the shape; one
/// material pass over all members (ScopeEntry: no member re-refraction), order-independent.
pub fn render_field_contribution(
    program: &MotionGlassProgram,
    backdrop: &ReferenceImage,
) -> Result<ReferenceImage, GlassRenderError> {
    render_field_contribution_transformed(program, IDENTITY_MATRIX, backdrop)
}

pub fn render_field_contribution_transformed(
    program: &MotionGlassProgram,
    owner_to_device: [f64; 9],
    backdrop: &ReferenceImage,
) -> Result<ReferenceImage, GlassRenderError> {
    if program.owner_kind != GlassOwnerKind::Field {
        return Err(GlassRenderError::NotField);
    }
    let Some(field) = &program.field else {
        return Err(GlassRenderError::NotField);
    };
    verify_kernel(program).map_err(|_| GlassRenderError::KernelMismatch)?;
    program
        .validate()
        .map_err(|_| GlassRenderError::InvalidProgram)?;
    validate_motion_glass_backdrop_bounds(program, owner_to_device)
        .map_err(|_| GlassRenderError::BackdropCoverage)?;
    let extent = backdrop.extent();
    let output = transformed_rect_bounds(program.backdrop.output_bounds, owner_to_device)
        .ok_or(GlassRenderError::BackdropCoverage)?;
    let left = output.left().floor().max(0.0) as u32;
    let top = output.top().floor().max(0.0) as u32;
    let right = (output.right().ceil() as i64).min(i64::from(extent.width())) as u32;
    let bottom = (output.bottom().ceil() as i64).min(i64::from(extent.height())) as u32;
    if left >= right || top >= bottom {
        return ReferenceImage::transparent(extent).map_err(|_| GlassRenderError::NonFinitePixel);
    }
    let pixel_count = u64::from(right - left) * u64::from(bottom - top);
    if pixel_count > MAX_RASTER_PIXELS {
        return Err(GlassRenderError::RasterBudget);
    }
    let width = extent.width() as usize;
    let mut pixels = vec![PremulRgba32::TRANSPARENT; width * extent.height() as usize];
    let merge = f64::from(field.merge_distance);
    let device_to_owner =
        matrix_inverse(owner_to_device).ok_or(GlassRenderError::BackdropCoverage)?;
    let prepared_surfaces =
        prepare_surfaces(program, owner_to_device).ok_or(GlassRenderError::BackdropCoverage)?;
    for y in top..bottom {
        for x in left..right {
            let p = [f64::from(x) + 0.5, f64::from(y) + 0.5];
            let Some(sample) = field_sample_prepared(&prepared_surfaces, device_to_owner, p, merge)
            else {
                continue;
            };
            let material = if sample.coverage > 0.0 {
                optical_pixel(
                    program,
                    backdrop,
                    OpticalPixelInput {
                        signed_distance: sample.signed_distance,
                        normal: sample.normal,
                        coverage: sample.coverage,
                        motion: sample.motion,
                        strain: sample.strain,
                        device_point: p,
                    },
                )?
            } else {
                PremulRgba32::TRANSPARENT
            };
            pixels[y as usize * width + x as usize] = material;
        }
    }
    ReferenceImage::new(extent, pixels).map_err(|_| GlassRenderError::NonFinitePixel)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FieldSample {
    pub density: f64,
    pub signed_distance: f64,
    pub coverage: f64,
    pub normal: [f64; 2],
    pub motion: PackedGlassMotion,
    pub strain: f64,
}

/// Field shape sample. Presence shrinks each member's signed distance before it contributes;
/// zero-presence members are skipped exactly. The material boundary is the density isocontour
/// `rho=1`, converted back to a pseudo-distance for one-pixel antialiasing and stable normals.
pub(crate) fn field_sample(
    program: &MotionGlassProgram,
    p: [f64; 2],
    merge: f64,
) -> Option<FieldSample> {
    field_sample_transformed(program, IDENTITY_MATRIX, p, merge)
}

fn field_sample_transformed(
    program: &MotionGlassProgram,
    owner_to_device: [f64; 9],
    p: [f64; 2],
    merge: f64,
) -> Option<FieldSample> {
    let surfaces = prepare_surfaces(program, owner_to_device)?;
    let device_to_owner = matrix_inverse(owner_to_device)?;
    field_sample_prepared(&surfaces, device_to_owner, p, merge)
}

fn field_sample_prepared(
    surfaces: &[PreparedTransformedSurface<'_>],
    device_to_owner: [f64; 9],
    p: [f64; 2],
    merge: f64,
) -> Option<FieldSample> {
    let (density, motion, strain, active) =
        field_density_motion_at(surfaces, device_to_owner, p, merge);
    if density <= 0.0 {
        return None;
    }
    let epsilon = 0.5;
    let dx = field_density_at(surfaces, device_to_owner, [p[0] + epsilon, p[1]], merge)
        - field_density_at(surfaces, device_to_owner, [p[0] - epsilon, p[1]], merge);
    let dy = field_density_at(surfaces, device_to_owner, [p[0], p[1] + epsilon], merge)
        - field_density_at(surfaces, device_to_owner, [p[0], p[1] - epsilon], merge);
    let gradient = [dx / (2.0 * epsilon), dy / (2.0 * epsilon)];
    let length = gradient[0].hypot(gradient[1]);
    let normal = if length < 1e-9 {
        [0.0, 0.0]
    } else {
        // Density falls toward the exterior, so its negative gradient is the outward normal.
        [-gradient[0] / length, -gradient[1] / length]
    };
    let owner_metric = OwnerMetric::new(device_to_owner, p)?;
    let owner_per_device = owner_metric.device_distance_to_owner_scale(normal)?;
    let merge_device = merge / owner_per_device;
    if !merge_device.is_finite() || merge_device <= 0.0 {
        return None;
    }
    let pseudo_distance = field_pseudo_distance(density, length, merge_device);
    // At a multi-member topology critical point, opposing density gradients cancel even though
    // the merge is moving continuously. Use the frozen critical-point AA scale for coverage only;
    // the lens signed distance above keeps the distance-preserving 1/merge fallback.
    let coverage_floor = if active > 1 {
        8.0 / merge_device
    } else {
        1.0 / merge_device
    };
    let coverage_distance = (1.0 - density) / length.max(coverage_floor).max(1.0e-6);
    let coverage = materialization((0.5 - coverage_distance).clamp(0.0, 1.0));
    Some(FieldSample {
        density,
        signed_distance: pseudo_distance,
        coverage,
        normal,
        motion,
        strain,
    })
}

fn field_density_at(
    surfaces: &[PreparedTransformedSurface<'_>],
    device_to_owner: [f64; 9],
    p: [f64; 2],
    merge: f64,
) -> f64 {
    let Some(owner_metric) = OwnerMetric::new(device_to_owner, p) else {
        return 0.0;
    };
    let mut density = 0.0;
    for surface in surfaces {
        let presence = materialization(f64::from(surface.surface.presence));
        if presence <= 0.0 {
            continue;
        }
        let Some(sample) = surface.sample(p) else {
            continue;
        };
        let sdf = sample.distance;
        let band = sample.band;
        if band <= 0.0 || !sdf.is_finite() {
            continue;
        }
        let Some(owner_per_device) = owner_metric.device_distance_to_owner_scale(sample.normal)
        else {
            continue;
        };
        let effective_sdf_owner = (sdf + (1.0 - presence) * band) * owner_per_device;
        density += potential_w(effective_sdf_owner / merge);
    }
    density
}

fn field_density_motion_at(
    surfaces: &[PreparedTransformedSurface<'_>],
    device_to_owner: [f64; 9],
    p: [f64; 2],
    merge: f64,
) -> (f64, PackedGlassMotion, f64, usize) {
    let Some(owner_metric) = OwnerMetric::new(device_to_owner, p) else {
        return (0.0, PackedGlassMotion::ZERO, 0.0, 0);
    };
    let mut density = 0.0;
    let mut mixed = PackedGlassMotion::ZERO;
    let mut weighted_translation_sq = 0.0;
    let mut active = 0usize;
    for surface in surfaces {
        let presence = materialization(f64::from(surface.surface.presence));
        if presence <= 0.0 {
            continue;
        }
        let Some(sample) = surface.sample(p) else {
            continue;
        };
        let sdf = sample.distance;
        let band = sample.band;
        if band <= 0.0 || !sdf.is_finite() {
            continue;
        }
        let Some(owner_per_device) = owner_metric.device_distance_to_owner_scale(sample.normal)
        else {
            continue;
        };
        let effective_sdf_owner = (sdf + (1.0 - presence) * band) * owner_per_device;
        let rho = potential_w(effective_sdf_owner / merge);
        if rho <= 0.0 {
            continue;
        }
        density += rho;
        active += 1;
        let motion = surface.surface.response;
        add_motion_scaled(&mut mixed, motion, rho as f32);
        let tx = f64::from(motion.translation[0]);
        let ty = f64::from(motion.translation[1]);
        weighted_translation_sq += rho * (tx * tx + ty * ty);
    }
    if density <= 0.0 {
        return (0.0, PackedGlassMotion::ZERO, 0.0, 0);
    }
    scale_motion(&mut mixed, (1.0 / density) as f32);
    weighted_translation_sq /= density;
    let mean_sq = f64::from(mixed.translation[0]).powi(2) + f64::from(mixed.translation[1]).powi(2);
    let strain = if active > 1 {
        (weighted_translation_sq - mean_sq).max(0.0).sqrt()
    } else {
        0.0
    };
    (density, mixed, strain, active)
}

fn add_motion_scaled(target: &mut PackedGlassMotion, value: PackedGlassMotion, weight: f32) {
    target.translation[0] += value.translation[0] * weight;
    target.translation[1] += value.translation[1] * weight;
    target.acceleration[0] += value.acceleration[0] * weight;
    target.acceleration[1] += value.acceleration[1] * weight;
    target.angular += value.angular * weight;
    target.scale[0] += value.scale[0] * weight;
    target.scale[1] += value.scale[1] * weight;
    target.shear += value.shear * weight;
    target.area += value.area * weight;
    target.pressure += value.pressure * weight;
    target.twist += value.twist * weight;
}

fn scale_motion(target: &mut PackedGlassMotion, scale: f32) {
    target.translation = target.translation.map(|value| value * scale);
    target.acceleration = target.acceleration.map(|value| value * scale);
    target.angular *= scale;
    target.scale = target.scale.map(|value| value * scale);
    target.shear *= scale;
    target.area *= scale;
    target.pressure *= scale;
    target.twist *= scale;
}

#[derive(Debug, Clone, Copy)]
struct OpticalPixelInput {
    signed_distance: f64,
    normal: [f64; 2],
    coverage: f64,
    motion: PackedGlassMotion,
    strain: f64,
    device_point: [f64; 2],
}

#[derive(Debug, Clone, Copy)]
struct LensSurface {
    normal: [f64; 3],
    edge_curve: f64,
}

fn lens_surface(signed_distance: f64, boundary_normal: [f64; 2], bevel_width: f64) -> LensSurface {
    if bevel_width <= 1.0e-9 {
        return LensSurface {
            normal: [0.0, 0.0, 1.0],
            edge_curve: 0.0,
        };
    }
    let interior = (-signed_distance / bevel_width).clamp(0.0, 1.0);
    // This is the normal field of a rounded height profile, not a generic material mask. A gentle
    // power keeps curvature alive through most of the short axis while the smoothstep still reaches
    // both the interface and center with a zero derivative.
    let smooth = interior * interior * (3.0 - 2.0 * interior);
    let edge_curve = (1.0 - smooth).powf(1.35);
    let view_facing = (1.0 - edge_curve * edge_curve).max(0.0).sqrt();
    LensSurface {
        normal: [
            boundary_normal[0] * edge_curve,
            boundary_normal[1] * edge_curve,
            view_facing,
        ],
        edge_curve,
    }
}

fn snell_offset(surface: LensSurface, refractive_index: f64, optical_path: f64) -> [f64; 2] {
    // Snell's law gives the maximum lateral travel at the vertical interface as
    // `path * sqrt(ior² - 1)`. Scale that edge solution by the monotone height-field normal rather
    // than evaluating the angular ray independently at every pixel: the latter has an interior
    // maximum and folds a thin slice of the backdrop even when endpoint displacement is bounded.
    let edge_travel = optical_path * (refractive_index.max(1.0).powi(2) - 1.0).max(0.0).sqrt();
    [
        -surface.normal[0] * edge_travel,
        -surface.normal[1] * edge_travel,
    ]
}

/// Limits the optical path so the refracted backdrop coordinate remains one-to-one across the
/// bevel. Once the edge displacement approaches the bevel width the map folds, repeating a thin
/// slice of the backdrop as a bright/dark line. The 30% budget leaves headroom for the steepest
/// part of the rounded height-field normal profile over the full admitted IOR range.
fn fold_safe_optical_thickness(thickness: f64, bevel_width: f64, maximum_ior: f64) -> f64 {
    let edge_gain =
        OPTICAL_PATH_THICKNESS_FACTOR * (maximum_ior * maximum_ior - 1.0).max(0.0).sqrt();
    if edge_gain <= 1.0e-9 {
        return thickness;
    }
    thickness.min(bevel_width.max(0.0) * MAX_REFRACTION_BEVEL_FRACTION / edge_gain)
}

fn clamp_displacement(mut value: [f64; 2], bevel_width: f64) -> [f64; 2] {
    let magnitude = value[0].hypot(value[1]);
    let limit = bevel_width.max(0.0) * MAX_REFRACTION_BEVEL_FRACTION;
    if magnitude > limit && limit > 0.0 {
        let scale = limit / magnitude;
        value[0] *= scale;
        value[1] *= scale;
    }
    value
}

fn diffused_sample(image: &ReferenceImage, point: [f64; 2], radius: f64) -> [f64; 4] {
    let radius = radius.max(0.0);
    if radius < 0.5 {
        return sample_bilinear(image, point[0], point[1]);
    }
    // One center and four diagonal taps approximate a coherent Gaussian footprint while keeping
    // 4K Glass affordable. The 1.3 spread preserves the variance of the former 5x5 binomial kernel.
    let spread = radius * 1.3;
    let mut soft = sample_bilinear(image, point[0], point[1]).map(|channel| channel * 0.4);
    for [x, y] in [
        [-spread, -spread],
        [spread, -spread],
        [-spread, spread],
        [spread, spread],
    ] {
        let sample = sample_bilinear(image, point[0] + x, point[1] + y);
        for channel in 0..4 {
            soft[channel] += sample[channel] * 0.15;
        }
    }
    soft
}

fn glass_highlight_color(background: [f64; 3]) -> [f64; 3] {
    // Bright, colorful backdrops tint the interface while dark or neutral backdrops approach a
    // white reflection. This follows the content-adaptive lighting strategy used by the Flutter
    // reference and avoids painting the same opaque white strip over every scene.
    let luminance = background[0] * 0.2627 + background[1] * 0.6780 + background[2] * 0.0593;
    let maximum = background.into_iter().fold(0.0_f64, f64::max);
    let lum = luminance * 2.5;
    let saturation = maximum * 2.5;
    let influence = (lum / (1.0 + lum)) * (saturation / (1.0 + saturation));
    let denominator = maximum.max(0.001);
    std::array::from_fn(|channel| {
        (1.0 + (background[channel] / denominator - 1.0) * influence * 0.22).clamp(0.72, 1.08)
    })
}

/// Shared optics: rounded height field, Snell refraction, IOR dispersion, roughness,
/// alpha-gated tint, Schlick Fresnel and a subtle directional contact shadow.
fn optical_pixel(
    program: &MotionGlassProgram,
    backdrop: &ReferenceImage,
    input: OpticalPixelInput,
) -> Result<PremulRgba32, GlassRenderError> {
    let OpticalPixelInput {
        signed_distance,
        normal,
        coverage,
        motion,
        strain,
        device_point: p,
    } = input;
    let bevel_width = f64::from(program.material.bevel_width);
    let thickness = f64::from(program.material.thickness);
    let mut surface = lens_surface(signed_distance, normal, bevel_width);
    let tangent = [-normal[1], normal[0]];
    let normal_acceleration = f64::from(motion.acceleration[0]) * normal[0]
        + f64::from(motion.acceleration[1]) * normal[1];
    let anisotropic_scale =
        f64::from(motion.scale[0]) * normal[0].abs() + f64::from(motion.scale[1]) * normal[1].abs();
    let normal_delta = (-normal_acceleration * 0.000_001_5
        + anisotropic_scale * 0.035
        + f64::from(motion.area) * 0.016
        + f64::from(motion.pressure) * 0.022
        + strain * 0.000_15)
        * thickness;
    let tangent_delta =
        (f64::from(motion.angular) * 0.025 + f64::from(motion.twist) * 0.018) * thickness;
    let shear_delta = f64::from(motion.shear) * thickness * 0.02;
    let motion_envelope = 0.22 + 0.78 * surface.edge_curve;
    let deformation_delta = [
        normal[0] * normal_delta + tangent[0] * tangent_delta + normal[1] * shear_delta,
        normal[1] * normal_delta + tangent[1] * tangent_delta + normal[0] * shear_delta,
    ];
    if thickness > 1.0e-9 {
        let mut dynamic_slope = [
            surface.normal[0] + deformation_delta[0] / thickness * motion_envelope * 1.6,
            surface.normal[1] + deformation_delta[1] / thickness * motion_envelope * 1.6,
        ];
        let slope_length = dynamic_slope[0].hypot(dynamic_slope[1]);
        if slope_length > 1.0 {
            dynamic_slope = dynamic_slope.map(|value| value / slope_length);
        }
        let dynamic_length = dynamic_slope[0].hypot(dynamic_slope[1]);
        surface.normal = [
            dynamic_slope[0],
            dynamic_slope[1],
            (1.0 - dynamic_length * dynamic_length).max(0.0).sqrt(),
        ];
    }
    let motion_delta = [
        -f64::from(motion.translation[0]) * 0.000_75 * thickness * motion_envelope,
        -f64::from(motion.translation[1]) * 0.000_75 * thickness * motion_envelope,
    ];
    let ior = f64::from(program.material.refractive_index);
    let dispersion = f64::from(program.material.dispersion);
    let optical_thickness = fold_safe_optical_thickness(thickness, bevel_width, ior + dispersion);
    // The control is a constant-thickness plate; only its surface normal changes across the edge.
    // Modulating path length with the same profile created a second extremum in the sampling map.
    let optical_path = optical_thickness * OPTICAL_PATH_THICKNESS_FACTOR;
    let offset_for = |channel_ior: f64| {
        let optical = snell_offset(surface, channel_ior, optical_path);
        clamp_displacement(
            [optical[0] + motion_delta[0], optical[1] + motion_delta[1]],
            bevel_width,
        )
    };
    let has_optical_offset = surface.normal[0].hypot(surface.normal[1]) > 1.0e-9
        || motion_delta[0].hypot(motion_delta[1]) > 1.0e-9;
    let (red_offset, green_offset, blue_offset) = if !has_optical_offset {
        ([0.0; 2], [0.0; 2], [0.0; 2])
    } else if dispersion <= 1.0e-9 {
        let offset = offset_for(ior);
        (offset, offset, offset)
    } else {
        (
            offset_for((ior - dispersion).max(1.0)),
            offset_for(ior),
            offset_for(ior + dispersion),
        )
    };
    let green_point = [p[0] + green_offset[0], p[1] + green_offset[1]];
    let roughness = f64::from(program.material.roughness);
    // Blur is a uniform property of the transmitted backdrop. Geometry and surface slope may move
    // that backdrop, but must not silently change its blur radius or split the glass into bands.
    let blur_radius = diffusion_radius(roughness, thickness);
    let green_soft = diffused_sample(backdrop, green_point, blur_radius);
    // The stable center and a zero-dispersion material share one footprint. With dispersion, only
    // the red and blue soft channels move; no sharp layout-bound rectangle is mixed back in.
    let soft = if !has_optical_offset || dispersion <= 1.0e-9 {
        green_soft
    } else {
        let red_soft = diffused_sample(
            backdrop,
            [p[0] + red_offset[0], p[1] + red_offset[1]],
            blur_radius,
        );
        let blue_soft = diffused_sample(
            backdrop,
            [p[0] + blue_offset[0], p[1] + blue_offset[1]],
            blur_radius,
        );
        [red_soft[0], green_soft[1], blue_soft[2], green_soft[3]]
    };
    let mut rgb = [soft[0], soft[1], soft[2]];
    let highlight_color = glass_highlight_color(rgb);

    // Tint alpha is authoritative. RGB with alpha zero is exactly inert.
    let tint = program.material.tint_linear;
    let tint_mix = f64::from(tint[3]);
    rgb = [
        rgb[0] * (1.0 - tint_mix) + f64::from(tint[0]) * tint_mix,
        rgb[1] * (1.0 - tint_mix) + f64::from(tint[1]) * tint_mix,
        rgb[2] * (1.0 - tint_mix) + f64::from(tint[2]) * tint_mix,
    ];

    // Saturation belongs after tint in the transmission pipeline. Applying it to the backdrop
    // first made the subsequent white tint erase much of the optical color separation.
    let luma = rgb[0] * 0.2627 + rgb[1] * 0.6780 + rgb[2] * 0.0593;
    let saturation = 1.0 + (1.0 - roughness) * 0.08;
    rgb = rgb.map(|channel| (luma + (channel - luma) * saturation).clamp(0.0, 1.0));

    // Three-dimensional surface lighting. Schlick Fresnel follows view angle, while a tight key
    // lobe and a broad environment lobe keep the material legible over both flat and detailed
    // backdrops. The opposing edge receives transmission absorption before reflection, producing
    // the bright/dark interface pair that a real glass plate exhibits.
    let light = program.material.light;
    let light_z = f64::from(light.elevation).clamp(0.0, 1.0);
    let light_xy = (1.0 - light_z * light_z).sqrt();
    let light_vector = [
        f64::from(light.direction[0]) * light_xy,
        f64::from(light.direction[1]) * light_xy,
        light_z,
    ];
    let half_raw = [light_vector[0], light_vector[1], light_vector[2] + 1.0];
    let half_length = half_raw[0]
        .hypot(half_raw[1])
        .hypot(half_raw[2])
        .max(1.0e-9);
    let half_vector = half_raw.map(|value| value / half_length);
    let n_dot_h = (surface.normal[0] * half_vector[0]
        + surface.normal[1] * half_vector[1]
        + surface.normal[2] * half_vector[2])
        .max(0.0);
    let highlight_power = 18.0 + 84.0 * (1.0 - roughness).powi(2);
    let specular = n_dot_h.powf(highlight_power);
    let environment_power = 3.0 + 10.0 * (1.0 - roughness);
    let environment_specular = n_dot_h.powf(environment_power);
    let fill_power = 6.0 + 34.0 * (1.0 - roughness);
    let fill_specular = surface.normal[2].max(0.0).powf(fill_power);
    let f0 = ((ior - 1.0) / (ior + 1.0)).powi(2);
    let interface_width = interface_width(thickness).max(1.0e-6);
    let interface_rim = materialization((1.0 + signed_distance / interface_width).clamp(0.0, 1.0));
    // Strong grazing reflection belongs to the material interface itself. Extending it across the
    // whole bevel paints both straight sides as separate bright strips; keep only the small F0
    // base response in the interior and confine the grazing term to the outer interface.
    let fresnel = f0 + (1.0 - f0) * (1.0 - surface.normal[2]).powi(5) * interface_rim;
    let facing =
        normal[0] * f64::from(light.direction[0]) + normal[1] * f64::from(light.direction[1]);
    let key_rim = facing.max(0.0).powf(1.35);
    let fill_rim = (-facing).max(0.0).powf(1.35);
    // Absorption is confined to the physical interface. Reusing the whole curvature mask here
    // painted a broad dark strip opposite the key light even when refraction itself was smooth.
    let absorption =
        (f64::from(program.material.shadow_strength) * interface_rim * (0.02 + 0.13 * fill_rim))
            .clamp(0.0, 0.10);
    rgb = rgb.map(|channel| channel * (1.0 - absorption));

    let light_intensity = f64::from(light.intensity);
    let reflection = (f64::from(program.material.specular_strength)
        * (fresnel * (0.18 + light_intensity * (0.10 * key_rim + 0.035 * fill_rim))
            + light_intensity
                * (0.28 * specular + 0.13 * environment_specular + 0.06 * fill_specular)
            + interface_rim * light_intensity * (0.13 * key_rim + 0.02 * fill_rim)))
        .clamp(0.0, 0.48);
    rgb = std::array::from_fn(|channel| {
        rgb[channel] * (1.0 - reflection) + highlight_color[channel] * reflection
    });

    premul([
        (rgb[0] * coverage) as f32,
        (rgb[1] * coverage) as f32,
        (rgb[2] * coverage) as f32,
        coverage as f32,
    ])
}

#[derive(Debug, Clone, Copy)]
struct OwnerMetric {
    j00: f64,
    j01: f64,
    j10: f64,
    j11: f64,
    determinant: f64,
}

impl OwnerMetric {
    fn new(device_to_owner: [f64; 9], device_point: [f64; 2]) -> Option<Self> {
        let owner_point = project(device_to_owner, device_point)?;
        let denominator = device_to_owner[6] * device_point[0]
            + device_to_owner[7] * device_point[1]
            + device_to_owner[8];
        if !denominator.is_finite() || denominator.abs() <= 1e-12 {
            return None;
        }
        let j00 = (device_to_owner[0] - device_to_owner[6] * owner_point[0]) / denominator;
        let j01 = (device_to_owner[1] - device_to_owner[7] * owner_point[0]) / denominator;
        let j10 = (device_to_owner[3] - device_to_owner[6] * owner_point[1]) / denominator;
        let j11 = (device_to_owner[4] - device_to_owner[7] * owner_point[1]) / denominator;
        let determinant = j00 * j11 - j01 * j10;
        if !determinant.is_finite() || determinant.abs() <= 1e-12 {
            return None;
        }
        Some(Self {
            j00,
            j01,
            j10,
            j11,
            determinant,
        })
    }

    fn device_distance_to_owner_scale(self, device_normal: [f64; 2]) -> Option<f64> {
        let normal_length = device_normal[0].hypot(device_normal[1]);
        let scale = if normal_length <= 1e-12 {
            self.determinant.abs().sqrt()
        } else {
            let normal = [
                device_normal[0] / normal_length,
                device_normal[1] / normal_length,
            ];
            let owner_gradient = [
                (self.j11 * normal[0] - self.j10 * normal[1]) / self.determinant,
                (-self.j01 * normal[0] + self.j00 * normal[1]) / self.determinant,
            ];
            1.0 / owner_gradient[0].hypot(owner_gradient[1])
        };
        (scale.is_finite() && scale > 1e-12).then_some(scale)
    }
}

fn premul(channels: [f32; 4]) -> Result<PremulRgba32, GlassRenderError> {
    PremulRgba32::from_premultiplied(channels).map_err(|_| GlassRenderError::NonFinitePixel)
}

#[derive(Debug, Clone, Copy)]
struct TransformedShapeSample {
    distance: f64,
    band: f64,
    normal: [f64; 2],
}

struct PreparedTransformedSurface<'a> {
    surface: &'a valle_draw::program::glass::PackedGlassSurface,
    device_to_local: [f64; 9],
}

impl<'a> PreparedTransformedSurface<'a> {
    fn new(
        surface: &'a valle_draw::program::glass::PackedGlassSurface,
        owner_to_device: [f64; 9],
    ) -> Option<Self> {
        let local_to_device = matrix_mul(owner_to_device, surface.local_to_owner.map(f64::from));
        if !rect_has_no_horizon(surface.rect, local_to_device) {
            return None;
        }
        Some(Self {
            surface,
            device_to_local: matrix_inverse(local_to_device)?,
        })
    }

    fn sample(&self, point: [f64; 2]) -> Option<TransformedShapeSample> {
        transformed_shape_sample_prepared(
            self.surface.shape,
            self.surface.rect,
            self.device_to_local,
            f64::from(self.surface.radius),
            &self.surface.path_points,
            point,
        )
    }
}

fn prepare_surfaces(
    program: &MotionGlassProgram,
    owner_to_device: [f64; 9],
) -> Option<Vec<PreparedTransformedSurface<'_>>> {
    program
        .surfaces
        .iter()
        .map(|surface| PreparedTransformedSurface::new(surface, owner_to_device))
        .collect()
}

pub(crate) fn transformed_foreground_distance(
    foreground: &valle_draw::program::MotionGlassForegroundProgram,
    owner_to_device: [f64; 9],
    point: [f64; 2],
) -> Option<(f64, f64)> {
    transformed_shape_sample(
        foreground.shape,
        foreground.rect,
        matrix_mul(owner_to_device, foreground.local_to_owner.map(f64::from)),
        f64::from(foreground.radius),
        &foreground.path_points,
        point,
    )
    .map(|sample| (sample.distance, sample.band))
}

fn transformed_shape_sample(
    shape: PackedGlassShapeKind,
    rect: Rect,
    local_to_device: [f64; 9],
    radius: f64,
    points: &[[f32; 2]],
    point: [f64; 2],
) -> Option<TransformedShapeSample> {
    if !rect_has_no_horizon(rect, local_to_device) {
        return None;
    }
    let device_to_local = matrix_inverse(local_to_device)?;
    transformed_shape_sample_prepared(shape, rect, device_to_local, radius, points, point)
}

fn transformed_shape_sample_prepared(
    shape: PackedGlassShapeKind,
    rect: Rect,
    device_to_local: [f64; 9],
    radius: f64,
    points: &[[f32; 2]],
    point: [f64; 2],
) -> Option<TransformedShapeSample> {
    if device_to_local[6] == 0.0 && device_to_local[7] == 0.0 {
        let denominator = device_to_local[8];
        if !denominator.is_finite() || denominator.abs() <= 1e-12 {
            return None;
        }
        let local = [
            (device_to_local[0] * point[0] + device_to_local[1] * point[1] + device_to_local[2])
                / denominator,
            (device_to_local[3] * point[0] + device_to_local[4] * point[1] + device_to_local[5])
                / denominator,
        ];
        let (distance, band, local_normal) =
            shape_signed_distance_and_normal(shape, rect, radius, points, local);
        return finish_transformed_shape_sample(
            distance,
            band,
            local_normal,
            device_to_local[0] / denominator,
            device_to_local[1] / denominator,
            device_to_local[3] / denominator,
            device_to_local[4] / denominator,
        );
    }
    let local = project(device_to_local, point)?;
    let (distance, band, local_normal) =
        shape_signed_distance_and_normal(shape, rect, radius, points, local);
    let denominator =
        device_to_local[6] * point[0] + device_to_local[7] * point[1] + device_to_local[8];
    if !denominator.is_finite() || denominator.abs() <= 1e-12 {
        return None;
    }
    let j00 = (device_to_local[0] - device_to_local[6] * local[0]) / denominator;
    let j01 = (device_to_local[1] - device_to_local[7] * local[0]) / denominator;
    let j10 = (device_to_local[3] - device_to_local[6] * local[1]) / denominator;
    let j11 = (device_to_local[4] - device_to_local[7] * local[1]) / denominator;
    finish_transformed_shape_sample(distance, band, local_normal, j00, j01, j10, j11)
}

fn finish_transformed_shape_sample(
    distance: f64,
    band: f64,
    local_normal: [f64; 2],
    j00: f64,
    j01: f64,
    j10: f64,
    j11: f64,
) -> Option<TransformedShapeSample> {
    if !distance.is_finite() || !band.is_finite() || band <= 0.0 {
        return None;
    }
    let gradient = [
        j00 * local_normal[0] + j10 * local_normal[1],
        j01 * local_normal[0] + j11 * local_normal[1],
    ];
    let length = gradient[0].hypot(gradient[1]);
    if !length.is_finite() || length <= 1e-12 {
        let fallback = (j00 * j11 - j01 * j10).abs().sqrt();
        if !fallback.is_finite() || fallback <= 1e-12 {
            return None;
        }
        return Some(TransformedShapeSample {
            distance: distance / fallback,
            band: band / fallback,
            normal: [0.0, 0.0],
        });
    }
    Some(TransformedShapeSample {
        distance: distance / length,
        band: band / length,
        normal: [gradient[0] / length, gradient[1] / length],
    })
}

fn transformed_rect_bounds(rect: Rect, transform: [f64; 9]) -> Option<Rect> {
    if !rect_has_no_horizon(rect, transform) {
        return None;
    }
    let points = [
        project(transform, [rect.left(), rect.top()])?,
        project(transform, [rect.right(), rect.top()])?,
        project(transform, [rect.right(), rect.bottom()])?,
        project(transform, [rect.left(), rect.bottom()])?,
    ];
    let left = points
        .iter()
        .map(|point| point[0])
        .fold(f64::INFINITY, f64::min);
    let top = points
        .iter()
        .map(|point| point[1])
        .fold(f64::INFINITY, f64::min);
    let right = points
        .iter()
        .map(|point| point[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = points
        .iter()
        .map(|point| point[1])
        .fold(f64::NEG_INFINITY, f64::max);
    Some(Rect::from_edges(left, top, right, bottom))
}

fn rect_has_no_horizon(rect: Rect, transform: [f64; 9]) -> bool {
    let denominators = [
        [rect.left(), rect.top()],
        [rect.right(), rect.top()],
        [rect.right(), rect.bottom()],
        [rect.left(), rect.bottom()],
    ]
    .map(|point| transform[6] * point[0] + transform[7] * point[1] + transform[8]);
    let first = denominators[0];
    first.is_finite()
        && first.abs() > 1.0e-12
        && denominators.into_iter().all(|value| {
            value.is_finite()
                && value.abs() > 1.0e-12
                && value.is_sign_positive() == first.is_sign_positive()
        })
}

fn matrix_mul(left: [f64; 9], right: [f64; 9]) -> [f64; 9] {
    let mut output = [0.0; 9];
    for row in 0..3 {
        for column in 0..3 {
            output[row * 3 + column] = (0..3)
                .map(|index| left[row * 3 + index] * right[index * 3 + column])
                .sum();
        }
    }
    output
}

fn matrix_inverse(matrix: [f64; 9]) -> Option<[f64; 9]> {
    let [a, b, c, d, e, f, g, h, i] = matrix;
    let cofactors = [
        e * i - f * h,
        c * h - b * i,
        b * f - c * e,
        f * g - d * i,
        a * i - c * g,
        c * d - a * f,
        d * h - e * g,
        b * g - a * h,
        a * e - b * d,
    ];
    let determinant = a * cofactors[0] + b * cofactors[3] + c * cofactors[6];
    (determinant.is_finite() && determinant.abs() > 1e-12)
        .then(|| cofactors.map(|value| value / determinant))
}

fn project(matrix: [f64; 9], point: [f64; 2]) -> Option<[f64; 2]> {
    let denominator = matrix[6] * point[0] + matrix[7] * point[1] + matrix[8];
    if !denominator.is_finite() || denominator.abs() <= 1e-12 {
        return None;
    }
    let output = [
        (matrix[0] * point[0] + matrix[1] * point[1] + matrix[2]) / denominator,
        (matrix[3] * point[0] + matrix[4] * point[1] + matrix[5]) / denominator,
    ];
    output.into_iter().all(f64::is_finite).then_some(output)
}

fn shape_signed_distance_and_normal(
    shape: PackedGlassShapeKind,
    rect: Rect,
    radius: f64,
    points: &[[f32; 2]],
    p: [f64; 2],
) -> (f64, f64, [f64; 2]) {
    match shape {
        PackedGlassShapeKind::Circle => {
            let center = rect.center();
            let dx = p[0] - center.x;
            let dy = p[1] - center.y;
            let length = dx.hypot(dy);
            let normal = if length > 1e-12 {
                [dx / length, dy / length]
            } else {
                [0.0, 0.0]
            };
            (length - radius, radius, normal)
        }
        PackedGlassShapeKind::ContinuousRect | PackedGlassShapeKind::Capsule => {
            let center = rect.center();
            let half_w = (rect.right() - rect.left()) * 0.5;
            let half_h = (rect.bottom() - rect.top()) * 0.5;
            let r = radius.min(half_w).min(half_h);
            let center_delta = [p[0] - center.x, p[1] - center.y];
            let qx = center_delta[0].abs() - (half_w - r);
            let qy = center_delta[1].abs() - (half_h - r);
            let outside_delta = [qx.max(0.0), qy.max(0.0)];
            let outside = outside_delta[0].hypot(outside_delta[1]);
            let inside = qx.max(qy).min(0.0);
            let signs = [
                if center_delta[0] < 0.0 { -1.0 } else { 1.0 },
                if center_delta[1] < 0.0 { -1.0 } else { 1.0 },
            ];
            let normal = if outside > 1e-12 {
                [
                    outside_delta[0] / outside * signs[0],
                    outside_delta[1] / outside * signs[1],
                ]
            } else if center_delta[0].hypot(center_delta[1]) <= 1e-12 {
                [0.0, 0.0]
            } else if qx > qy {
                [signs[0], 0.0]
            } else {
                [0.0, signs[1]]
            };
            (outside + inside - r, r.max(1e-3), normal)
        }
        PackedGlassShapeKind::Path => {
            let (sdf, normal) = polygon_signed_distance_and_normal(points, p);
            let band = ((rect.right() - rect.left()) + (rect.bottom() - rect.top())) / 8.0;
            (sdf, band.max(1e-3), normal)
        }
    }
}

/// Closed-polygon signed distance and outward normal in the shape's local coordinates.
fn polygon_signed_distance_and_normal(points: &[[f32; 2]], p: [f64; 2]) -> (f64, [f64; 2]) {
    let count = points.len();
    if count < 3 {
        return (f64::INFINITY, [0.0, 0.0]);
    }
    let mut min_distance = f64::INFINITY;
    let mut closest_delta = [0.0, 0.0];
    let mut closest_edge = [1.0, 0.0];
    let mut signed_area_twice = 0.0;
    let mut inside = false;
    for index in 0..count {
        let a = [f64::from(points[index][0]), f64::from(points[index][1])];
        let b = [
            f64::from(points[(index + 1) % count][0]),
            f64::from(points[(index + 1) % count][1]),
        ];
        let ab = [b[0] - a[0], b[1] - a[1]];
        let ap = [p[0] - a[0], p[1] - a[1]];
        let length_sq = ab[0] * ab[0] + ab[1] * ab[1];
        let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / length_sq.max(1e-12)).clamp(0.0, 1.0);
        let closest = [a[0] + t * ab[0], a[1] + t * ab[1]];
        let delta = [p[0] - closest[0], p[1] - closest[1]];
        let distance = delta[0].hypot(delta[1]);
        if distance < min_distance {
            min_distance = distance;
            closest_delta = delta;
            closest_edge = ab;
        }
        signed_area_twice += a[0] * b[1] - a[1] * b[0];
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x_intersect = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if x_intersect > p[0] {
                inside = !inside;
            }
        }
    }
    let normal = if min_distance > 1e-12 {
        let sign = if inside { -1.0 } else { 1.0 };
        [
            closest_delta[0] / min_distance * sign,
            closest_delta[1] / min_distance * sign,
        ]
    } else {
        let edge_length = closest_edge[0].hypot(closest_edge[1]);
        if edge_length <= 1e-12 {
            [0.0, 0.0]
        } else if signed_area_twice >= 0.0 {
            [
                closest_edge[1] / edge_length,
                -closest_edge[0] / edge_length,
            ]
        } else {
            [
                -closest_edge[1] / edge_length,
                closest_edge[0] / edge_length,
            ]
        }
    };
    (if inside { -min_distance } else { min_distance }, normal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::Extent2d;
    use valle_draw::program::glass::{
        BackdropUse, MOTION_GLASS_KERNEL_ID, PackedGlassMaterial, PackedGlassMotion,
        PackedGlassSurface, motion_glass_kernel_digest, schema_digest,
    };

    fn checker_backdrop(extent: Extent2d) -> ReferenceImage {
        let mut pixels = Vec::with_capacity(extent.width() as usize * extent.height() as usize);
        for y in 0..extent.height() {
            for x in 0..extent.width() {
                let dark = (x / 16 + y / 16) % 2 == 0;
                let value = if dark { 0.08 } else { 0.35 };
                pixels.push(PremulRgba32::from_straight([value, value, value * 1.2], 1.0).unwrap());
            }
        }
        ReferenceImage::new(extent, pixels).unwrap()
    }

    fn solid_backdrop(extent: Extent2d, value: f32) -> ReferenceImage {
        ReferenceImage::solid(
            extent,
            PremulRgba32::from_straight([value, value, value], 1.0).unwrap(),
        )
        .unwrap()
    }

    fn refresh_bounds(program: &mut MotionGlassProgram, owner_to_device: [f64; 9]) {
        let bounds = crate::compositor::glass::resolve_motion_glass_backdrop_bounds(
            program,
            owner_to_device,
        )
        .unwrap();
        program.backdrop.output_bounds = bounds.output_owner;
        program.backdrop.sample_bounds = bounds.sample_owner;
    }

    fn program(presence: f32, response_tap: f32) -> MotionGlassProgram {
        let response_tap = PackedGlassMotion {
            translation: [response_tap, 0.0],
            ..PackedGlassMotion::ZERO
        };
        let mut program = MotionGlassProgram {
            owner_kind: GlassOwnerKind::Independent,
            owner_id: "hero-lens".into(),
            surfaces: vec![PackedGlassSurface {
                surface_id: "hero-lens".into(),
                shape: PackedGlassShapeKind::Circle,
                rect: Rect::new(32.0, 32.0, 64.0, 64.0),
                local_to_owner: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                radius: 32.0,
                presence,
                response: response_tap,
                path_points: vec![],
                foreground_tone: valle_draw::program::glass::PackedGlassForegroundTone::None,
                foreground_protection: 0.0,
                foreground_bounds: None,
                foreground_luma: None,
            }],
            field: None,
            material: PackedGlassMaterial {
                bevel_width: 8.0,
                thickness: 16.0,
                refractive_index: 1.2,
                roughness: 0.08,
                dispersion: 0.012,
                tint_linear: [0.3, 0.5, 0.9, 0.2],
                specular_strength: 0.5,
                shadow_strength: 0.1,
                foreground_gain: 0.7,
                light: valle_draw::program::PackedGlassLight::DEFAULT,
            },
            backdrop: BackdropUse {
                scope: valle_draw::program::BackdropScope::Current,
                sample_bounds: Rect::new(24.0, 24.0, 80.0, 80.0),
                output_bounds: Rect::new(32.0, 32.0, 64.0, 64.0),
            },
            kernel_id: MOTION_GLASS_KERNEL_ID.into(),
            kernel_digest: motion_glass_kernel_digest(),
            schema_digest: schema_digest(),
        };
        refresh_bounds(&mut program, IDENTITY_MATRIX);
        program
    }

    fn extent() -> Extent2d {
        Extent2d::new(128, 128).unwrap()
    }

    #[test]
    fn presence_zero_yields_no_contribution() {
        let backdrop = checker_backdrop(extent());
        let contribution = render_glass_contribution(&program(0.0, 0.0), &backdrop).unwrap();
        assert_eq!(contribution, ReferenceImage::transparent(extent()).unwrap());
    }

    #[test]
    fn rest_circle_covers_and_tints_inside() {
        let backdrop = checker_backdrop(extent());
        let contribution = render_glass_contribution(&program(1.0, 0.0), &backdrop).unwrap();
        // Center of the circle: full coverage, tinted.
        let center = contribution.pixel(64, 64).unwrap();
        assert!(center.alpha() > 0.99);
        assert!(center.channels()[0] < 0.4, "tint mixes blue in");
        assert!(center.channels()[2] > center.channels()[0]);
        // Far outside the sample ring: transparent.
        let outside = contribution.pixel(8, 8).unwrap();
        assert_eq!(outside.alpha(), 0.0);
    }

    #[test]
    fn independent_coverage_is_not_the_lens_height_profile() {
        let backdrop = checker_backdrop(extent());
        let contribution = render_glass_contribution(&program(1.0, 0.0), &backdrop).unwrap();
        // This sample is well inside the one-pixel edge but only about one quarter of the radius
        // into the height profile. The old coverage=height coupling made it mostly transparent.
        assert!(contribution.pixel(88, 64).unwrap().alpha() > 0.99);
    }

    #[test]
    fn static_height_field_has_strong_edge_refraction_and_a_stable_center() {
        let thickness = 20.0;
        let edge = lens_surface(-0.01, [1.0, 0.0], thickness);
        let middle = lens_surface(-10.0, [1.0, 0.0], thickness);
        let center = lens_surface(-20.0, [1.0, 0.0], thickness);
        let edge_offset = snell_offset(edge, 1.24, thickness * OPTICAL_PATH_THICKNESS_FACTOR);
        let middle_offset = snell_offset(middle, 1.24, thickness * OPTICAL_PATH_THICKNESS_FACTOR);
        let center_offset = snell_offset(center, 1.24, thickness * OPTICAL_PATH_THICKNESS_FACTOR);
        assert!(
            edge_offset[0].abs() > 15.0,
            "static edge refraction is too weak"
        );
        assert!(middle_offset[0].abs() > 1.0);
        assert!(edge_offset[0].abs() > middle_offset[0].abs());
        assert!(center_offset[0].abs() < 1.0e-9 && center_offset[1].abs() < 1.0e-9);
    }

    #[test]
    fn lens_profile_meets_the_flat_center_without_an_inner_seam() {
        let width = 20.0;
        let epsilon = 0.01;
        let just_inside = lens_surface(-width - epsilon, [1.0, 0.0], width);
        let join = lens_surface(-width, [1.0, 0.0], width);
        let just_outside = lens_surface(-width + epsilon, [1.0, 0.0], width);
        assert_eq!(just_inside.edge_curve, 0.0);
        assert_eq!(join.edge_curve, 0.0);
        assert!(
            just_outside.edge_curve < 1.0e-6,
            "{}",
            just_outside.edge_curve
        );
    }

    #[test]
    fn bounded_refraction_never_folds_the_backdrop_sampling_map() {
        let bevel_width = 20.0;
        for ior in [1.1, 1.2, 1.4, 1.6] {
            let thickness = fold_safe_optical_thickness(64.0, bevel_width, ior);
            let mut previous = snell_offset(
                lens_surface(0.0, [1.0, 0.0], bevel_width),
                ior,
                thickness * OPTICAL_PATH_THICKNESS_FACTOR,
            )[0]
            .abs();
            for step in 1..=4096 {
                let interior = bevel_width * f64::from(step) / 4096.0;
                let surface = lens_surface(-interior, [1.0, 0.0], bevel_width);
                let offset =
                    snell_offset(surface, ior, thickness * OPTICAL_PATH_THICKNESS_FACTOR)[0].abs();
                let sampled_depth = interior + offset;
                assert!(
                    sampled_depth > previous,
                    "sampling map folded at ior={ior}, step={step}: {sampled_depth} <= {previous}"
                );
                previous = sampled_depth;
            }
        }
    }

    #[test]
    fn optical_profile_is_covariant_from_720p_to_4k() {
        let scale = 3.0;
        let bevel = 24.08;
        let thickness = 9.504;
        let signed_distance = -bevel * 0.37;
        let ior = 1.22;
        let low = lens_surface(signed_distance, [0.0, -1.0], bevel);
        let high = lens_surface(signed_distance * scale, [0.0, -1.0], bevel * scale);
        for channel in 0..3 {
            assert!((low.normal[channel] - high.normal[channel]).abs() < 1.0e-12);
        }
        let low_path =
            fold_safe_optical_thickness(thickness, bevel, ior) * OPTICAL_PATH_THICKNESS_FACTOR;
        let high_path = fold_safe_optical_thickness(thickness * scale, bevel * scale, ior)
            * OPTICAL_PATH_THICKNESS_FACTOR;
        assert!((high_path - low_path * scale).abs() < 1.0e-12);
        let low_offset = snell_offset(low, ior, low_path);
        let high_offset = snell_offset(high, ior, high_path);
        for axis in 0..2 {
            assert!((high_offset[axis] - low_offset[axis] * scale).abs() < 1.0e-12);
        }
        let roughness = 0.158;
        let low_blur = diffusion_radius(roughness, thickness);
        let high_blur = diffusion_radius(roughness, thickness * scale);
        assert!((high_blur - low_blur * scale).abs() < 1.0e-12);
        let low_interface = interface_width(thickness);
        let high_interface = interface_width(thickness * scale);
        assert!((high_interface - low_interface * scale).abs() < 1.0e-12);
    }

    #[test]
    fn zero_alpha_tint_rgb_is_exactly_dormant() {
        let backdrop = checker_backdrop(extent());
        let mut white = program(1.0, 0.0);
        white.material.tint_linear = [1.0, 1.0, 1.0, 0.0];
        let mut black = white.clone();
        black.material.tint_linear = [0.0, 0.0, 0.0, 0.0];
        assert_eq!(
            render_glass_contribution(&white, &backdrop).unwrap(),
            render_glass_contribution(&black, &backdrop).unwrap(),
        );
    }

    #[test]
    fn analytic_shape_metric_respects_non_uniform_device_scale() {
        let sample = transformed_shape_sample(
            PackedGlassShapeKind::Circle,
            Rect::new(32.0, 32.0, 64.0, 64.0),
            [2.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 1.0],
            32.0,
            &[],
            [196.0, 32.0],
        )
        .unwrap();
        assert!((sample.distance - 4.0).abs() < 1e-9);
        assert!((sample.band - 64.0).abs() < 1e-9);
        assert!((sample.normal[0] - 1.0).abs() < 1e-9);
        assert!(sample.normal[1].abs() < 1e-9);
    }

    #[test]
    fn resolved_light_direction_controls_the_surface_highlight() {
        let backdrop = solid_backdrop(extent(), 0.2);
        let mut toward = program(1.0, 0.0);
        toward.material.tint_linear[3] = 0.0;
        toward.material.refractive_index = 1.0;
        toward.material.roughness = 0.0;
        toward.material.specular_strength = 1.0;
        toward.material.shadow_strength = 0.0;
        toward.material.dispersion = 0.0;
        toward.material.light = valle_draw::program::PackedGlassLight {
            direction: [-core::f32::consts::FRAC_1_SQRT_2; 2],
            elevation: 0.0,
            intensity: 1.0,
        };
        let mut away = toward.clone();
        away.material.light.direction = [core::f32::consts::FRAC_1_SQRT_2; 2];
        let toward = render_glass_contribution(&toward, &backdrop)
            .unwrap()
            .pixel(45, 45)
            .unwrap()
            .channels();
        let away = render_glass_contribution(&away, &backdrop)
            .unwrap()
            .pixel(45, 45)
            .unwrap()
            .channels();
        assert!(
            toward[..3].iter().sum::<f32>() > away[..3].iter().sum::<f32>(),
            "the surface facing the resolved light must receive more energy"
        );
    }

    #[test]
    fn ior_dispersion_executes_distinct_rgb_refraction() {
        let backdrop = checker_backdrop(extent());
        let mut none = program(1.0, 0.0);
        none.material.dispersion = 0.0;
        let mut dispersed = none.clone();
        dispersed.material.dispersion = 0.04;
        refresh_bounds(&mut dispersed, IDENTITY_MATRIX);
        assert_ne!(
            render_glass_contribution(&none, &backdrop).unwrap(),
            render_glass_contribution(&dispersed, &backdrop).unwrap()
        );
    }

    #[test]
    fn foreground_layout_metadata_cannot_repaint_the_glass_material() {
        let backdrop = checker_backdrop(extent());
        let baseline = program(1.0, 0.0);
        let baseline_image = render_glass_contribution(&baseline, &backdrop).unwrap();

        let mut protected = baseline;
        let surface = &mut protected.surfaces[0];
        surface.foreground_tone = valle_draw::program::glass::PackedGlassForegroundTone::Light;
        surface.foreground_protection = 1.0;
        surface.foreground_bounds = Some(Rect::new(40.0, 54.0, 48.0, 20.0));
        protected.validate().unwrap();
        let protected_image = render_glass_contribution(&protected, &backdrop).unwrap();

        assert_eq!(
            protected_image, baseline_image,
            "a layout rectangle is not glyph coverage and must never paint a band into glass"
        );
    }

    #[test]
    fn foreground_pass_masks_real_pixels_instead_of_synthesizing_content() {
        let mut owner = program(1.0, 0.0);
        let surface = &mut owner.surfaces[0];
        surface.foreground_tone = valle_draw::program::glass::PackedGlassForegroundTone::Light;
        surface.foreground_protection = 0.65;
        surface.foreground_bounds = Some(Rect::new(48.0, 60.0, 32.0, 8.0));
        let foreground = MotionGlassForegroundProgram::from_owner(&owner, "hero-lens").unwrap();
        let ordinary_pixels = ReferenceImage::solid(
            extent(),
            PremulRgba32::from_straight([0.9, 0.2, 0.1], 0.8).unwrap(),
        )
        .unwrap();
        let masked =
            apply_glass_foreground_transformed(&foreground, IDENTITY_MATRIX, &ordinary_pixels)
                .unwrap();
        assert_eq!(masked.pixel(8, 8).unwrap(), PremulRgba32::TRANSPARENT);
        assert_eq!(
            masked.pixel(64, 64).unwrap(),
            ordinary_pixels.pixel(64, 64).unwrap()
        );
    }

    #[test]
    fn contribution_is_premultiplied() {
        let backdrop = checker_backdrop(extent());
        let contribution = render_glass_contribution(&program(1.0, 0.0), &backdrop).unwrap();
        for y in 0..extent().height() {
            for x in 0..extent().width() {
                let pixel = contribution.pixel(x, y).unwrap();
                let channels = pixel.channels();
                assert!(channels[3] <= 1.0);
                if channels[3] == 0.0 {
                    assert_eq!(channels[..3], [0.0, 0.0, 0.0]);
                }
            }
        }
    }

    #[test]
    fn response_changes_optics_deterministically() {
        let backdrop = checker_backdrop(extent());
        let rest = render_glass_contribution(&program(1.0, 0.0), &backdrop).unwrap();
        let moving = render_glass_contribution(&program(1.0, 250.0), &backdrop).unwrap();
        assert_ne!(rest, moving);
        // Deterministic: same input twice is exact.
        let again = render_glass_contribution(&program(1.0, 250.0), &backdrop).unwrap();
        assert_eq!(moving, again);
    }

    #[test]
    fn field_program_fails_closed() {
        let backdrop = checker_backdrop(extent());
        let mut program = program(1.0, 0.0);
        program.owner_kind = GlassOwnerKind::Field;
        assert!(matches!(
            render_glass_contribution(&program, &backdrop),
            Err(GlassRenderError::NotIndependent)
        ));
    }

    #[test]
    fn foreign_kernel_fails_closed_before_pixel_work() {
        let backdrop = checker_backdrop(extent());
        let mut program = program(1.0, 0.0);
        program.kernel_id = "third-party-kernel".into();
        assert!(matches!(
            render_glass_contribution(&program, &backdrop),
            Err(GlassRenderError::KernelMismatch)
        ));
    }

    #[test]
    fn reference_rejects_bounds_that_clip_visible_output() {
        let backdrop = checker_backdrop(extent());
        let mut program = program(1.0, 0.0);
        program.backdrop.output_bounds = program.surfaces[0].rect;
        assert!(matches!(
            render_glass_contribution(&program, &backdrop),
            Err(GlassRenderError::BackdropCoverage)
        ));
    }

    #[test]
    fn composite_is_source_over() {
        let backdrop = checker_backdrop(extent());
        let contribution = render_glass_contribution(&program(1.0, 0.0), &backdrop).unwrap();
        let composite = composite_glass_contribution(&contribution, &backdrop).unwrap();
        // At the circle center, alpha stays 1 and color changed from the backdrop.
        let before = backdrop.pixel(64, 64).unwrap();
        let after = composite.pixel(64, 64).unwrap();
        assert_eq!(after.alpha(), 1.0);
        assert_ne!(before, after);
    }

    #[test]
    fn outer_opacity_zero_returns_to_backdrop() {
        // G1.7 wrapper contract: opacity=0 on the Glass contribution leaves the backdrop
        // pixel-identical; the wrapper never touches the backdrop itself.
        let backdrop = checker_backdrop(extent());
        let contribution = render_glass_contribution(&program(1.0, 0.0), &backdrop).unwrap();
        let faded = contribution.scale_coverage(0.0).unwrap();
        let composite = composite_glass_contribution(&faded, &backdrop).unwrap();
        assert_eq!(composite, backdrop);
    }

    #[test]
    fn mask_outside_returns_to_backdrop() {
        // G1.7 wrapper contract: a mask that does not intersect the Glass leaves the backdrop
        // pixel-identical.
        let backdrop = checker_backdrop(extent());
        let contribution = render_glass_contribution(&program(1.0, 0.0), &backdrop).unwrap();
        let masked = apply_rect_mask(&contribution, Rect::new(0.0, 0.0, 16.0, 16.0), 2.0);
        let composite = composite_glass_contribution(&masked, &backdrop).unwrap();
        assert_eq!(composite, backdrop);
        // A mask covering the Glass keeps the Glass visible.
        let covering = apply_rect_mask(&contribution, Rect::new(0.0, 0.0, 128.0, 128.0), 2.0);
        let composite = composite_glass_contribution(&covering, &backdrop).unwrap();
        assert_ne!(composite, backdrop);
    }

    #[test]
    fn current_backdrop_dynamics_have_no_history_ghost() {
        // G1.7 current-backdrop contract: the Glass output depends only on the current
        // backdrop frame — different backdrops give different results, the same backdrop
        // twice is exact, and no previous-frame state leaks in.
        let extent = extent();
        fn patch(image: &ReferenceImage, color: [f32; 3]) -> ReferenceImage {
            let mut pixels = image.pixels().to_vec();
            let width = image.extent().width() as usize;
            for y in 48..80 {
                for x in 48..80 {
                    pixels[y * width + x] = PremulRgba32::from_straight(color, 1.0).unwrap();
                }
            }
            ReferenceImage::new(image.extent(), pixels).unwrap()
        }
        let dark = patch(&checker_backdrop(extent), [0.9, 0.1, 0.1]);
        let bright = patch(&checker_backdrop(extent), [0.1, 0.9, 0.1]);
        let program = program(1.0, 0.0);
        let first = composite_glass_contribution(
            &render_glass_contribution(&program, &dark).unwrap(),
            &dark,
        )
        .unwrap();
        let second = composite_glass_contribution(
            &render_glass_contribution(&program, &bright).unwrap(),
            &bright,
        )
        .unwrap();
        assert_ne!(first, second);
        let again = composite_glass_contribution(
            &render_glass_contribution(&program, &dark).unwrap(),
            &dark,
        )
        .unwrap();
        assert_eq!(first, again);
    }

    fn field_program(merge_distance: f32, gap: f64) -> MotionGlassProgram {
        // Two 32px-radius circles whose centers are `gap` apart; the field merge is `mergeDistance`.
        let make = |id: &str, x: f64| PackedGlassSurface {
            surface_id: id.into(),
            shape: PackedGlassShapeKind::Circle,
            rect: Rect::new(x - 32.0, 32.0, 64.0, 64.0),
            local_to_owner: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            radius: 32.0,
            presence: 1.0,
            response: PackedGlassMotion::ZERO,
            path_points: vec![],
            foreground_tone: valle_draw::program::glass::PackedGlassForegroundTone::None,
            foreground_protection: 0.0,
            foreground_bounds: None,
            foreground_luma: None,
        };
        let merge = f64::from(merge_distance);
        let output = Rect::from_edges(
            64.0 - gap * 0.5 - 32.0 - merge,
            32.0 - merge,
            64.0 + gap * 0.5 + 32.0 + merge,
            96.0 + merge,
        );
        let sample = Rect::from_edges(
            output.left() - 16.0,
            output.top() - 16.0,
            output.right() + 16.0,
            output.bottom() + 16.0,
        );
        let mut program = MotionGlassProgram {
            owner_kind: GlassOwnerKind::Field,
            owner_id: "orbit".into(),
            surfaces: vec![
                make("left", 64.0 - gap * 0.5),
                make("right", 64.0 + gap * 0.5),
            ],
            field: Some(valle_draw::program::glass::PackedGlassField {
                field_id: "orbit".into(),
                merge_distance,
                member_ids: vec!["left".into(), "right".into()],
            }),
            material: PackedGlassMaterial {
                bevel_width: 8.0,
                thickness: 16.0,
                refractive_index: 1.2,
                roughness: 0.08,
                dispersion: 0.012,
                tint_linear: [0.3, 0.5, 0.9, 0.2],
                specular_strength: 0.5,
                shadow_strength: 0.1,
                foreground_gain: 0.7,
                light: valle_draw::program::PackedGlassLight::DEFAULT,
            },
            backdrop: BackdropUse {
                scope: valle_draw::program::BackdropScope::ScopeEntry("orbit".into()),
                sample_bounds: sample,
                output_bounds: output,
            },
            kernel_id: MOTION_GLASS_KERNEL_ID.into(),
            kernel_digest: motion_glass_kernel_digest(),
            schema_digest: schema_digest(),
        };
        refresh_bounds(&mut program, IDENTITY_MATRIX);
        program
    }

    #[test]
    fn field_far_members_do_not_bridge_and_close_members_do() {
        let backdrop = checker_backdrop(extent());
        // Gap 160px > merge 24: the midpoint between the two circles is transparent.
        let far = render_field_contribution(&field_program(24.0, 160.0), &backdrop).unwrap();
        assert_eq!(far.pixel(64, 64).unwrap().alpha(), 0.0);
        // Gap 12px < merge 24: the potential bridges and covers the midpoint.
        let close = render_field_contribution(&field_program(24.0, 12.0), &backdrop).unwrap();
        assert!(close.pixel(64, 64).unwrap().alpha() > 0.5);
    }

    #[test]
    fn field_single_member_matches_independent_optics_shape() {
        // A one-member Field must preserve the member's signed distance, including its center.
        let mut single = field_program(24.0, 0.0);
        single.surfaces = vec![single.surfaces[0].clone()];
        single.field = Some(valle_draw::program::glass::PackedGlassField {
            field_id: "orbit".into(),
            merge_distance: 24.0,
            member_ids: vec!["left".into()],
        });
        for (point, expected) in [
            ([64.0, 64.0], -32.0),
            ([80.0, 64.0], -16.0),
            ([95.0, 64.0], -1.0),
            ([96.0, 64.0], 0.0),
        ] {
            let sample = field_sample(&single, point, 24.0).unwrap();
            assert!(
                (sample.signed_distance - expected).abs() < 1.0e-6,
                "single-member Field distance drifted at {point:?}: {} != {expected}",
                sample.signed_distance,
            );
        }
    }

    #[test]
    fn field_raster_is_member_order_invariant() {
        let canonical = field_program(24.0, 12.0);
        let a = field_sample(&canonical, [64.0, 64.0], 24.0).unwrap();
        let mut reversed = field_program(24.0, 12.0);
        reversed.surfaces.reverse();
        let b = field_sample(&reversed, [64.0, 64.0], 24.0).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            render_field_contribution(&reversed, &checker_backdrop(extent())).unwrap_err(),
            GlassRenderError::InvalidProgram,
            "wire order remains canonical even though the field math is commutative",
        );
    }

    #[test]
    fn field_density_uses_the_local_owner_metric_under_projective_scale() {
        let program = field_program(24.0, 12.0);
        let owner_point = [64.0, 64.0];
        let identity =
            field_sample_transformed(&program, IDENTITY_MATRIX, owner_point, 24.0).unwrap();
        for transform in [
            [2.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.002, 0.0, 1.0],
        ] {
            let device_point = project(transform, owner_point).unwrap();
            let transformed =
                field_sample_transformed(&program, transform, device_point, 24.0).unwrap();
            assert!(
                (transformed.density - identity.density).abs() < 1e-6,
                "field owner-space density drifted under {transform:?}: {} vs {}",
                transformed.density,
                identity.density,
            );
        }
    }

    #[test]
    fn structurally_invalid_field_fails_before_pixel_work() {
        let mut invalid = field_program(24.0, 12.0);
        invalid.field.as_mut().unwrap().merge_distance = 0.0;
        assert_eq!(
            render_field_contribution(&invalid, &checker_backdrop(extent())).unwrap_err(),
            GlassRenderError::InvalidProgram,
        );
    }

    #[test]
    fn independent_program_rejected_by_field_raster() {
        let backdrop = checker_backdrop(extent());
        assert!(matches!(
            render_field_contribution(&program(1.0, 0.0), &backdrop),
            Err(GlassRenderError::NotField)
        ));
    }

    #[test]
    fn merge_split_has_no_single_frame_pop() {
        // G2.4: sweeping two members from far apart through the merge and back must produce a
        // continuous gap-center coverage series — no discontinuity at contact or break. The
        // step is steep (materialization derivative), so the assertion is convergence: halving
        // the sweep step must halve the observed per-frame alpha step. A discontinuity would
        // keep the step constant.
        let backdrop = checker_backdrop(extent());
        let max_step = |steps: usize| -> (f32, f64, f32, f32) {
            let mut previous: Option<f32> = None;
            let mut max_step = 0.0f32;
            let mut at_gap = 0.0;
            let mut from = 0.0;
            let mut to = 0.0;
            for step in 0..=steps {
                let phase = (step as f64 / steps as f64) * std::f64::consts::TAU;
                let gap = 80.0 + 80.0 * phase.cos();
                let contribution =
                    render_field_contribution(&field_program(24.0, gap), &backdrop).unwrap();
                let alpha = contribution.pixel(64, 64).unwrap().alpha();
                if let Some(previous) = previous {
                    let delta = (alpha - previous).abs();
                    if delta > max_step {
                        max_step = delta;
                        at_gap = gap;
                        from = previous;
                        to = alpha;
                    }
                }
                previous = Some(alpha);
            }
            (max_step, at_gap, from, to)
        };
        let coarse = max_step(160);
        let fine = max_step(640);
        assert!(
            fine.0 < coarse.0 * 0.5,
            "gap-center alpha did not converge ({coarse:?} -> {fine:?}); possible pop"
        );
    }

    #[test]
    fn field_random_seek_rebuilds_exact_pixels() {
        // G2.4/G2.8: any frame rebuilds exactly from its inputs (random seek) — the same gap
        // sampled twice, in different order, yields identical pixels and no hidden state.
        let backdrop = checker_backdrop(extent());
        let gaps = [4.0, 24.0, 60.0, 120.0, 160.0, 36.0];
        let frames = gaps
            .iter()
            .map(|gap| render_field_contribution(&field_program(24.0, *gap), &backdrop).unwrap())
            .collect::<Vec<_>>();
        for gap in &gaps {
            let rebuilt = render_field_contribution(&field_program(24.0, *gap), &backdrop).unwrap();
            let reference = frames[gaps.iter().position(|value| value == gap).unwrap()].clone();
            assert_eq!(rebuilt, reference);
        }
    }

    #[test]
    fn current_backdrop_changes_transmission_deterministically() {
        // The material samples the current backdrop, and the same backdrop twice is exact. There
        // is no hidden luma adaptation, history or readback policy.
        let extent = extent();
        let dark = solid_backdrop(extent, 0.05);
        let bright = solid_backdrop(extent, 0.90);
        let program = program(1.0, 0.0);
        let dark_glass = render_glass_contribution(&program, &dark).unwrap();
        let bright_glass = render_glass_contribution(&program, &bright).unwrap();
        let dark_center = dark_glass.pixel(64, 64).unwrap();
        let bright_center = bright_glass.pixel(64, 64).unwrap();
        assert_ne!(
            dark_center, bright_center,
            "transmission must sample the current backdrop"
        );
        let again = render_glass_contribution(&program, &dark).unwrap();
        assert_eq!(dark_glass, again);
    }

    #[test]
    fn caption_band_composites_after_glass_without_touching_it() {
        // Caption-terminal contract: a caption band drawn after the composite never affects
        // the Glass pixels or the backdrop outside the band — the glass output is identical
        // with and without the caption band.
        let backdrop = checker_backdrop(extent());
        let program = program(1.0, 0.0);
        let composite = composite_glass_contribution(
            &render_glass_contribution(&program, &backdrop).unwrap(),
            &backdrop,
        )
        .unwrap();
        // Caption band at the bottom (y >= 112), premul white with alpha 0.9.
        let mut with_band = composite.clone();
        let width = extent().width() as usize;
        let mut pixels = with_band.pixels().to_vec();
        for y in 112..128u32 {
            for x in 0..128u32 {
                pixels[y as usize * width + x as usize] =
                    PremulRgba32::from_premultiplied([0.9, 0.9, 0.9, 0.9]).unwrap();
            }
        }
        with_band = ReferenceImage::new(extent(), pixels).unwrap();
        // Glass center pixel is untouched by the band.
        assert_eq!(
            with_band.pixel(64, 64).unwrap(),
            composite.pixel(64, 64).unwrap()
        );
        // Backdrop above the band is untouched; inside the band the caption wins.
        assert_eq!(
            with_band.pixel(8, 8).unwrap(),
            composite.pixel(8, 8).unwrap()
        );
        assert!(with_band.pixel(64, 120).unwrap().channels()[3] > 0.89);
    }

    #[test]
    fn capsule_renders_stadium_geometry() {
        // G4.1: a capsule with radius >= half the smaller dimension is a stadium — the flat
        // side exists along the middle and the bounding-box corners stay outside.
        let mut program = program(1.0, 0.0);
        program.surfaces[0].shape = PackedGlassShapeKind::Capsule;
        program.surfaces[0].rect = Rect::new(24.0, 40.0, 80.0, 48.0);
        program.surfaces[0].radius = 24.0;
        refresh_bounds(&mut program, IDENTITY_MATRIX);
        let contribution =
            render_glass_contribution(&program, &checker_backdrop(extent())).unwrap();
        // Center and the left cap interior are covered.
        assert!(contribution.pixel(64, 64).unwrap().alpha() > 0.9);
        assert!(contribution.pixel(40, 64).unwrap().alpha() > 0.5);
        // The top-left bounding-box corner of a stadium is outside the material.
        assert!(contribution.pixel(26, 42).unwrap().alpha() < 0.3);
    }

    #[test]
    fn f16_kernel_matches_f32_reference_within_tolerance() {
        // G4.3 Native CPU F16 kernel: RGBA16F transport must stay inside the frozen
        // F16_REFERENCE_TOLERANCE against the RGBA32F reference.
        let backdrop = checker_backdrop(extent());
        let program = program(1.0, 0.0);
        let reference = render_glass_contribution(&program, &backdrop).unwrap();
        let f16 = render_glass_contribution_f16(&program, &backdrop).unwrap();
        for y in 0..extent().height() {
            for x in 0..extent().width() {
                let a = reference.pixel(x, y).unwrap().channels();
                let b = f16.pixel(x, y).unwrap().channels();
                for channel in 0..4 {
                    assert!(
                        crate::compositor::reference::F16_REFERENCE_TOLERANCE
                            .matches(b[channel], a[channel]),
                        "channel {channel} at ({x},{y}): f16={} f32={}",
                        b[channel],
                        a[channel]
                    );
                }
            }
        }
    }

    #[test]
    fn path_polygon_renders_closed_shape() {
        // G4.1 closed polygon: interior covered, outside corner outside the material.
        let mut program = program(1.0, 0.0);
        program.surfaces[0].shape = PackedGlassShapeKind::Path;
        program.surfaces[0].rect = Rect::new(40.0, 40.0, 80.0, 80.0);
        program.surfaces[0].radius = 0.0;
        program.surfaces[0].path_points =
            vec![[40.0, 40.0], [120.0, 40.0], [120.0, 120.0], [40.0, 120.0]];
        refresh_bounds(&mut program, IDENTITY_MATRIX);
        let contribution =
            render_glass_contribution(&program, &checker_backdrop(extent())).unwrap();
        assert!(contribution.pixel(80, 80).unwrap().alpha() > 0.5);
        assert!(contribution.pixel(36, 36).unwrap().alpha() < 0.3);
    }

    #[test]
    fn f16_round_trip_is_stable_and_saturates() {
        // Round-trip is idempotent; large values saturate to the f16 max.
        for value in [0.0f32, 0.1, 0.5, 1.0, 2.0, 100.0] {
            let once = f16_round_trip(value);
            let twice = f16_round_trip(once);
            assert_eq!(once, twice, "{value}");
            assert!((once - value).abs() <= value.abs() * 0.001 + 1e-4);
        }
        assert_eq!(f16_round_trip(1e6), 65504.0);
    }
}
