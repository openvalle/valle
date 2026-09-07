use valle_draw::math::{pow, sqrt};

use super::{PixelError, PremulRgba32};

pub(crate) const EFFECT_GAUSSIAN_SUPPORT_SIGMAS: f64 = 4.0;

const REC2020_KR: f32 = 0.2627;
const REC2020_KG: f32 = 0.6780;
const REC2020_KB: f32 = 0.0593;
const KEY_RADIUS_MIN: f32 = 0.02;
const KEY_RADIUS_MAX: f32 = 0.30;
const KEY_TRANSITION_BAND: f32 = 0.12;
const SHADOW_PROTECT_MAX_Y: f32 = 0.18;
const COLOR_GRADE_MIDDLE_GRAY: f64 = 0.18;
const COLOR_GRADE_TEMPERATURE_STOPS: f64 = 0.25;
const VIGNETTE_CORNER_DISTANCE: f64 = core::f64::consts::FRAC_1_SQRT_2;

/// Coverage retained by one key at one source sample. Transparent samples are neutral so the
/// device-space feather kernel never invents erosion at the source's geometric boundary.
pub(crate) fn chroma_key_coverage(
    pixel: PremulRgba32,
    key_working_linear_rec2020: [f32; 3],
    intensity: f32,
    shadow: f32,
) -> Result<f32, PixelError> {
    if pixel.alpha() == 0.0 {
        return Ok(1.0);
    }
    let rgb = pixel.straight_rgb()?;
    let pixel_chroma = rec2020_chroma(rgb);
    let key_chroma = rec2020_chroma(key_working_linear_rec2020);
    let delta = [
        pixel_chroma[0] - key_chroma[0],
        pixel_chroma[1] - key_chroma[1],
    ];
    let distance = sqrt(f64::from(delta[0] * delta[0] + delta[1] * delta[1])) as f32;
    let radius = KEY_RADIUS_MIN + (KEY_RADIUS_MAX - KEY_RADIUS_MIN) * intensity;
    let color_coverage = smoothstep(radius, radius + KEY_TRANSITION_BAND, distance);
    let shadow_threshold = shadow * SHADOW_PROTECT_MAX_Y;
    let shadow_coverage = if shadow_threshold > 0.0 {
        1.0 - smoothstep(0.0, shadow_threshold, rec2020_luma(rgb))
    } else {
        0.0
    };
    Ok(color_coverage.max(shadow_coverage).clamp(0.0, 1.0))
}

/// Removes only the component pointing toward the key chroma. Luma, alpha, HDR and negative
/// working-light values remain representable; no display-referred clamp occurs inside the effect.
pub(crate) fn chroma_key_despill(
    pixel: PremulRgba32,
    key_working_linear_rec2020: [f32; 3],
    edge_clean: f32,
    removed_coverage: f32,
) -> Result<PremulRgba32, PixelError> {
    if pixel.alpha() == 0.0 || edge_clean == 0.0 || removed_coverage == 0.0 {
        return Ok(pixel);
    }
    let rgb = pixel.straight_rgb()?;
    let luma = rec2020_luma(rgb);
    let mut chroma = rec2020_chroma(rgb);
    let key_chroma = rec2020_chroma(key_working_linear_rec2020);
    let key_length = sqrt(f64::from(
        key_chroma[0] * key_chroma[0] + key_chroma[1] * key_chroma[1],
    )) as f32;
    if key_length == 0.0 {
        return Ok(pixel);
    }
    let direction = [key_chroma[0] / key_length, key_chroma[1] / key_length];
    let projection = (chroma[0] * direction[0] + chroma[1] * direction[1]).max(0.0);
    let amount = edge_clean * removed_coverage.clamp(0.0, 1.0);
    chroma[0] -= direction[0] * projection * amount;
    chroma[1] -= direction[1] * projection * amount;

    let red = luma + 2.0 * (1.0 - REC2020_KR) * chroma[1];
    let blue = luma + 2.0 * (1.0 - REC2020_KB) * chroma[0];
    let green = (luma - REC2020_KR * red - REC2020_KB * blue) / REC2020_KG;
    PremulRgba32::from_straight([red, green, blue], pixel.alpha())
}

pub(crate) const fn rec2020_luma(rgb: [f32; 3]) -> f32 {
    REC2020_KR * rgb[0] + REC2020_KG * rgb[1] + REC2020_KB * rgb[2]
}

/// Five-scalar grade in straight Working Linear Rec.2020. The transfer deliberately preserves
/// scene-referred negative and HDR values and never changes coverage alpha.
#[allow(clippy::too_many_arguments)]
pub(crate) fn color_grade_pixel(
    pixel: PremulRgba32,
    unit_position: [f64; 2],
    brightness: f32,
    contrast: f32,
    saturation: f32,
    temperature: f32,
    vignette: f32,
) -> Result<PremulRgba32, PixelError> {
    if pixel.alpha() == 0.0 {
        return Ok(pixel);
    }
    let source = pixel.straight_rgb()?;
    let mut rgb = [
        f64::from(source[0]),
        f64::from(source[1]),
        f64::from(source[2]),
    ];

    let exposure = pow(2.0, f64::from(brightness));
    for channel in &mut rgb {
        *channel *= exposure;
    }

    let contrast_factor = 1.0 + f64::from(contrast);
    for channel in &mut rgb {
        *channel = (*channel - COLOR_GRADE_MIDDLE_GRAY) * contrast_factor + COLOR_GRADE_MIDDLE_GRAY;
    }

    let luminance = f64::from(REC2020_KR) * rgb[0]
        + f64::from(REC2020_KG) * rgb[1]
        + f64::from(REC2020_KB) * rgb[2];
    let saturation_factor = 1.0 + f64::from(saturation);
    for channel in &mut rgb {
        *channel = luminance + (*channel - luminance) * saturation_factor;
    }

    let warm_gain = pow(2.0, f64::from(temperature) * COLOR_GRADE_TEMPERATURE_STOPS);
    rgb[0] *= warm_gain;
    rgb[2] /= warm_gain;

    if vignette != 0.0 {
        let dx = unit_position[0] - 0.5;
        let dy = unit_position[1] - 0.5;
        let distance = (sqrt(dx * dx + dy * dy) / VIGNETTE_CORNER_DISTANCE).clamp(0.0, 1.0);
        let gain = 1.0 - f64::from(vignette) * distance * distance;
        for channel in &mut rgb {
            *channel *= gain;
        }
    }

    PremulRgba32::from_straight([rgb[0] as f32, rgb[1] as f32, rgb[2] as f32], pixel.alpha())
}

/// Device-space circular spotlight. Center is root-canvas normalized, radius/feather use the root
/// short side, and only premultiplied RGB is dimmed; coverage alpha is invariant.
pub(crate) fn spotlight_pixel(
    pixel: PremulRgba32,
    device_position: [f64; 2],
    root_size: [f64; 2],
    center: [f32; 2],
    radius: f32,
    feather: f32,
    intensity: f32,
) -> Result<PremulRgba32, PixelError> {
    if pixel.alpha() == 0.0 || intensity == 0.0 {
        return Ok(pixel);
    }
    let short_side = root_size[0].min(root_size[1]);
    let dx = (device_position[0] - f64::from(center[0]) * root_size[0]) / short_side;
    let dy = (device_position[1] - f64::from(center[1]) * root_size[1]) / short_side;
    let distance = sqrt(dx * dx + dy * dy) as f32;
    let light = if feather == 0.0 {
        if distance <= radius { 1.0 } else { 0.0 }
    } else {
        1.0 - smoothstep(radius, radius + feather, distance)
    };
    let gain = 1.0 - intensity * (1.0 - light);
    let [red, green, blue, alpha] = pixel.channels();
    PremulRgba32::from_premultiplied([red * gain, green * gain, blue * gain, alpha])
}

fn rec2020_chroma(rgb: [f32; 3]) -> [f32; 2] {
    let luma = rec2020_luma(rgb);
    [
        (rgb[2] - luma) / (2.0 * (1.0 - REC2020_KB)),
        (rgb[0] - luma) / (2.0 * (1.0 - REC2020_KR)),
    ]
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let amount = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    amount * amount * (3.0 - 2.0 * amount)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight(rgb: [f32; 3]) -> PremulRgba32 {
        PremulRgba32::from_straight(rgb, 1.0).unwrap()
    }

    #[test]
    fn exact_key_far_subject_and_transparency_have_closed_coverages() {
        let key = [0.0, 1.0, 0.0];
        assert_eq!(
            chroma_key_coverage(straight(key), key, 0.5, 0.0).unwrap(),
            0.0
        );
        assert_eq!(
            chroma_key_coverage(straight([1.0, 0.0, 0.0]), key, 0.5, 0.0).unwrap(),
            1.0
        );
        assert_eq!(
            chroma_key_coverage(PremulRgba32::TRANSPARENT, key, 0.5, 1.0).unwrap(),
            1.0
        );
    }

    #[test]
    fn shadow_protection_and_despill_preserve_alpha_and_working_luma() {
        let key = [0.0, 1.0, 0.0];
        let dark_key = straight([0.0, 0.05, 0.0]);
        assert!(chroma_key_coverage(dark_key, key, 1.0, 1.0).unwrap() > 0.9);

        let spill = PremulRgba32::from_straight([0.2, 0.8, 0.1], 0.5).unwrap();
        let cleaned = chroma_key_despill(spill, key, 1.0, 0.5).unwrap();
        assert_eq!(cleaned.alpha(), spill.alpha());
        assert!(
            (rec2020_luma(cleaned.straight_rgb().unwrap())
                - rec2020_luma(spill.straight_rgb().unwrap()))
            .abs()
                < 1.0e-6
        );
        assert!(cleaned.straight_rgb().unwrap()[1] < spill.straight_rgb().unwrap()[1]);
    }

    #[test]
    fn color_grade_uses_exposure_rec2020_luma_and_preserves_extended_range() {
        let source = PremulRgba32::from_straight([-0.25, 0.5, 2.0], 0.5).unwrap();
        let identity = color_grade_pixel(source, [0.5, 0.5], 0.0, 0.0, 0.0, 0.0, 0.0).unwrap();
        assert_eq!(identity, source);

        let doubled = color_grade_pixel(source, [0.5, 0.5], 1.0, 0.0, 0.0, 0.0, 0.0).unwrap();
        assert_eq!(doubled.alpha(), source.alpha());
        assert_eq!(doubled.straight_rgb().unwrap(), [-0.5, 1.0, 4.0]);

        let gray = color_grade_pixel(source, [0.5, 0.5], 0.0, 0.0, -1.0, 0.0, 0.0)
            .unwrap()
            .straight_rgb()
            .unwrap();
        assert!((gray[0] - gray[1]).abs() < 1.0e-6);
        assert!((gray[1] - gray[2]).abs() < 1.0e-6);
        assert!((gray[0] - rec2020_luma(source.straight_rgb().unwrap())).abs() < 1.0e-6);
    }

    #[test]
    fn color_grade_vignette_and_spotlight_are_root_positioned_rgb_only_gains() {
        let source = PremulRgba32::from_straight([0.4, 0.2, 0.1], 0.5).unwrap();
        let center = color_grade_pixel(source, [0.5, 0.5], 0.0, 0.0, 0.0, 0.0, 1.0).unwrap();
        let corner = color_grade_pixel(source, [0.0, 0.0], 0.0, 0.0, 0.0, 0.0, 1.0).unwrap();
        assert_eq!(center, source);
        assert_eq!(corner, PremulRgba32::from_straight([0.0; 3], 0.5).unwrap());

        let lit = spotlight_pixel(
            source,
            [50.0, 25.0],
            [100.0, 50.0],
            [0.5, 0.5],
            0.1,
            0.0,
            0.75,
        )
        .unwrap();
        let dimmed = spotlight_pixel(
            source,
            [0.5, 0.5],
            [100.0, 50.0],
            [0.5, 0.5],
            0.1,
            0.0,
            0.75,
        )
        .unwrap();
        assert_eq!(lit, source);
        assert_eq!(dimmed.alpha(), source.alpha());
        assert!(dimmed.approx_eq(
            PremulRgba32::from_straight([0.1, 0.05, 0.025], 0.5).unwrap(),
            1.0e-6
        ));
    }
}
