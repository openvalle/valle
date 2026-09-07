use thiserror::Error;
use valle_draw::math::{cos, sin, sqrt};

use crate::prepare::PreparedTransitionKernel;

use super::{
    BlendError, PixelError, PremulRgba32, ReferenceBlendMode, ReferenceImage, blend_over,
    validate_unit,
};

const CIRCLE_COVERAGE_SAMPLES_PER_AXIS: u32 = 4;
const LINEAR_BLUR_SAMPLES_PER_AXIS: u32 = 6;

/// Executes one compositor-owned transition between two local premultiplied sources and places
/// that result over their common immutable backdrop exactly once.
///
/// Layer opacity is coverage, so it scales both premultiplied RGB and alpha before any transition
/// sampling. Every kernel has exact endpoints: `progress == 0` is the opacity-scaled `from`
/// source, and `progress == 1` is the opacity-scaled `to` source.
pub fn composite_transition(
    backdrop: &ReferenceImage,
    from: &ReferenceImage,
    to: &ReferenceImage,
    kernel: PreparedTransitionKernel,
    progress: f32,
    from_opacity: f32,
    to_opacity: f32,
) -> Result<ReferenceImage, ReferenceTransitionError> {
    kernel
        .validate_wire()
        .map_err(|_| ReferenceTransitionError::ExtensionImplementationMismatch)?;
    validate_unit(progress, "transition progress")?;
    validate_unit(from_opacity, "from opacity")?;
    validate_unit(to_opacity, "to opacity")?;
    backdrop.ensure_same_extent(from)?;
    from.ensure_same_extent(to)?;

    let from = from.scale_coverage(from_opacity)?;
    let to = to.scale_coverage(to_opacity)?;
    let local = transition_local(&from, &to, kernel, f64::from(progress))?;
    local.source_over(backdrop).map_err(Into::into)
}

fn transition_local(
    from: &ReferenceImage,
    to: &ReferenceImage,
    kernel: PreparedTransitionKernel,
    progress: f64,
) -> Result<ReferenceImage, ReferenceTransitionError> {
    if progress == 0.0 {
        return Ok(from.clone());
    }
    if progress == 1.0 {
        return Ok(to.clone());
    }

    let extent = from.extent();
    let mut pixels = Vec::with_capacity(from.pixels().len());
    for y in 0..extent.height() {
        for x in 0..extent.width() {
            pixels.push(transition_pixel(from, to, kernel, progress, x, y)?);
        }
    }
    ReferenceImage::new(extent, pixels).map_err(Into::into)
}

fn transition_pixel(
    from: &ReferenceImage,
    to: &ReferenceImage,
    kernel: PreparedTransitionKernel,
    progress: f64,
    x: u32,
    y: u32,
) -> Result<PremulRgba32, ReferenceTransitionError> {
    let width = f64::from(from.extent().width());
    let height = f64::from(from.extent().height());
    let uv = [(f64::from(x) + 0.5) / width, (f64::from(y) + 0.5) / height];
    let direct_from = pixel(from, x, y);
    let direct_to = pixel(to, x, y);

    match kernel {
        PreparedTransitionKernel::Fade | PreparedTransitionKernel::ExtensionCrossFade { .. } => {
            mix_pixel(direct_from, direct_to, progress)
        }
        PreparedTransitionKernel::WipeLeft => {
            let coverage = (progress * width - f64::from(x)).clamp(0.0, 1.0);
            mix_pixel(direct_from, direct_to, coverage)
        }
        PreparedTransitionKernel::WipeRight => {
            let front = (1.0 - progress) * width;
            let coverage = (f64::from(x) + 1.0 - front).clamp(0.0, 1.0);
            mix_pixel(direct_from, direct_to, coverage)
        }
        PreparedTransitionKernel::CircleOpen => {
            let center = [width * 0.5, height * 0.5];
            let radius = progress * 0.5 * sqrt(width * width + height * height);
            let mut inside = 0_u32;
            for sample_y in 0..CIRCLE_COVERAGE_SAMPLES_PER_AXIS {
                for sample_x in 0..CIRCLE_COVERAGE_SAMPLES_PER_AXIS {
                    let sample = [
                        f64::from(x)
                            + (f64::from(sample_x) + 0.5)
                                / f64::from(CIRCLE_COVERAGE_SAMPLES_PER_AXIS),
                        f64::from(y)
                            + (f64::from(sample_y) + 0.5)
                                / f64::from(CIRCLE_COVERAGE_SAMPLES_PER_AXIS),
                    ];
                    let delta = [sample[0] - center[0], sample[1] - center[1]];
                    if sqrt(delta[0] * delta[0] + delta[1] * delta[1]) <= radius {
                        inside += 1;
                    }
                }
            }
            let samples = CIRCLE_COVERAGE_SAMPLES_PER_AXIS * CIRCLE_COVERAGE_SAMPLES_PER_AXIS;
            mix_pixel(
                direct_from,
                direct_to,
                f64::from(inside) / f64::from(samples),
            )
        }
        PreparedTransitionKernel::SimpleZoom => {
            let zoom = 1.0 + 0.5 * (1.0 - progress);
            let to_uv = [(uv[0] - 0.5) / zoom + 0.5, (uv[1] - 0.5) / zoom + 0.5];
            mix_pixel(
                direct_from,
                sample_normalized(to, to_uv)?,
                smoothstep(0.0, 1.0, progress),
            )
        }
        PreparedTransitionKernel::CrossWarp => {
            let amount = smoothstep(0.0, 1.0, progress * 2.0 + uv[0] - 1.0);
            let from_uv = scale_about_center(uv, 1.0 - amount);
            let to_uv = scale_about_center(uv, amount);
            mix_pixel(
                sample_normalized(from, from_uv)?,
                sample_normalized(to, to_uv)?,
                amount,
            )
        }
        PreparedTransitionKernel::LinearBlur => {
            let displacement = 0.1 * progress.min(1.0 - progress);
            let blurred_from = linear_blur_sample(from, uv, displacement)?;
            let blurred_to = linear_blur_sample(to, uv, displacement)?;
            mix_pixel(blurred_from, blurred_to, progress)
        }
        PreparedTransitionKernel::DirectionalWarp => {
            // L1-normalized (-1, 1) direction. Moving the feathered front outside the complete
            // projection domain makes both endpoints exact without a near-end discontinuity.
            let projection = -0.5 * (uv[0] - 0.5) + 0.5 * (uv[1] - 0.5);
            let edge = 0.08;
            let front = -0.5 - edge + progress * (1.0 + 2.0 * edge);
            let amount = 1.0 - smoothstep(front - edge, front + edge, projection);
            let from_uv = scale_about_center(uv, 1.0 - amount);
            let to_uv = scale_about_center(uv, amount);
            mix_pixel(
                sample_normalized(from, from_uv)?,
                sample_normalized(to, to_uv)?,
                amount,
            )
        }
        PreparedTransitionKernel::DreamyZoom => {
            let from_uv = scale_about_center(uv, 1.0 + progress * 0.5);
            let to_uv = scale_about_center(uv, 1.5 - progress * 0.5);
            mix_pixel(
                sample_normalized(from, from_uv)?,
                sample_normalized(to, to_uv)?,
                smoothstep(0.0, 1.0, progress),
            )
        }
        PreparedTransitionKernel::Ripple => {
            let direction = [uv[0] - 0.5, uv[1] - 0.5];
            let distance = sqrt(direction[0] * direction[0] + direction[1] * direction[1]);
            let envelope = sin(std::f64::consts::PI * progress);
            let wave = sin(distance * 100.0 - progress * 50.0) * envelope / 30.0;
            let from_uv = [uv[0] + direction[0] * wave, uv[1] + direction[1] * wave];
            mix_pixel(
                sample_normalized(from, from_uv)?,
                direct_to,
                smoothstep(0.0, 1.0, progress),
            )
        }
        PreparedTransitionKernel::FlyEye => {
            let inverse = 1.0 - progress;
            let displacement = [0.04 * cos(50.0 * uv[0]), 0.04 * sin(50.0 * uv[1])];
            let displaced_to = sample_normalized(
                to,
                [
                    uv[0] + inverse * displacement[0],
                    uv[1] + inverse * displacement[1],
                ],
            )?;
            let separated_from = fly_eye_from(from, uv, displacement, progress)?;
            mix_pixel(separated_from, displaced_to, progress)
        }
        PreparedTransitionKernel::MultiplyBlend => {
            let bridge = blend_over(direct_to, direct_from, ReferenceBlendMode::Multiply)?;
            if progress < 0.5 {
                mix_pixel(direct_from, bridge, progress * 2.0)
            } else {
                mix_pixel(bridge, direct_to, (progress - 0.5) * 2.0)
            }
        }
        PreparedTransitionKernel::Perlin => {
            let noise = value_noise([uv[0] * 5.0, uv[1] * 5.0]);
            let amount = smoothstep(noise - 0.1, noise + 0.1, progress);
            mix_pixel(direct_from, direct_to, amount)
        }
    }
}

fn linear_blur_sample(
    image: &ReferenceImage,
    uv: [f64; 2],
    displacement: f64,
) -> Result<PremulRgba32, ReferenceTransitionError> {
    let mut channels = [0.0_f64; 4];
    for sample_y in 0..LINEAR_BLUR_SAMPLES_PER_AXIS {
        for sample_x in 0..LINEAR_BLUR_SAMPLES_PER_AXIS {
            let offset = [
                (f64::from(sample_x) + 0.5) / f64::from(LINEAR_BLUR_SAMPLES_PER_AXIS) - 0.5,
                (f64::from(sample_y) + 0.5) / f64::from(LINEAR_BLUR_SAMPLES_PER_AXIS) - 0.5,
            ];
            let sample = sample_normalized(
                image,
                [
                    uv[0] + displacement * offset[0],
                    uv[1] + displacement * offset[1],
                ],
            )?;
            for (sum, channel) in channels.iter_mut().zip(sample.channels()) {
                *sum += f64::from(channel);
            }
        }
    }
    let count = f64::from(LINEAR_BLUR_SAMPLES_PER_AXIS * LINEAR_BLUR_SAMPLES_PER_AXIS);
    PremulRgba32::from_premultiplied(channels.map(|channel| (channel / count) as f32))
        .map_err(Into::into)
}

fn fly_eye_from(
    image: &ReferenceImage,
    uv: [f64; 2],
    displacement: [f64; 2],
    progress: f64,
) -> Result<PremulRgba32, ReferenceTransitionError> {
    let scales = [0.7, 1.0, 1.3];
    let [red, green, blue] = scales.map(|scale| {
        sample_normalized(
            image,
            [
                uv[0] + progress * displacement[0] * scale,
                uv[1] + progress * displacement[1] * scale,
            ],
        )
    });
    let samples = [red?, green?, blue?];
    let alpha = samples.iter().map(|sample| sample.alpha()).sum::<f32>() / 3.0;
    let straight = [
        samples[0].straight_rgb()?[0],
        samples[1].straight_rgb()?[1],
        samples[2].straight_rgb()?[2],
    ];
    PremulRgba32::from_straight(straight, alpha).map_err(Into::into)
}

fn sample_normalized(
    image: &ReferenceImage,
    uv: [f64; 2],
) -> Result<PremulRgba32, ReferenceTransitionError> {
    let width = i64::from(image.extent().width());
    let height = i64::from(image.extent().height());
    let coordinate = [
        uv[0].clamp(0.0, 1.0) * width as f64 - 0.5,
        uv[1].clamp(0.0, 1.0) * height as f64 - 0.5,
    ];
    let floor = [coordinate[0].floor(), coordinate[1].floor()];
    let amount = [coordinate[0] - floor[0], coordinate[1] - floor[1]];
    let x0 = (floor[0] as i64).clamp(0, width - 1) as u32;
    let y0 = (floor[1] as i64).clamp(0, height - 1) as u32;
    let x1 = (floor[0] as i64 + 1).clamp(0, width - 1) as u32;
    let y1 = (floor[1] as i64 + 1).clamp(0, height - 1) as u32;
    let top = mix_pixel(pixel(image, x0, y0), pixel(image, x1, y0), amount[0])?;
    let bottom = mix_pixel(pixel(image, x0, y1), pixel(image, x1, y1), amount[0])?;
    mix_pixel(top, bottom, amount[1])
}

fn mix_pixel(
    from: PremulRgba32,
    to: PremulRgba32,
    amount: f64,
) -> Result<PremulRgba32, ReferenceTransitionError> {
    if amount <= 0.0 {
        return Ok(from);
    }
    if amount >= 1.0 {
        return Ok(to);
    }
    let from = from.channels();
    let to = to.channels();
    PremulRgba32::from_premultiplied(std::array::from_fn(|index| {
        (f64::from(from[index]) * (1.0 - amount) + f64::from(to[index]) * amount) as f32
    }))
    .map_err(Into::into)
}

fn pixel(image: &ReferenceImage, x: u32, y: u32) -> PremulRgba32 {
    image
        .pixel(x, y)
        .expect("reference transition coordinates remain inside the image")
}

fn scale_about_center(uv: [f64; 2], scale: f64) -> [f64; 2] {
    [(uv[0] - 0.5) * scale + 0.5, (uv[1] - 0.5) * scale + 0.5]
}

fn smoothstep(start: f64, end: f64, value: f64) -> f64 {
    let amount = ((value - start) / (end - start)).clamp(0.0, 1.0);
    amount * amount * (3.0 - 2.0 * amount)
}

fn value_noise(point: [f64; 2]) -> f64 {
    let cell = [point[0].floor(), point[1].floor()];
    let fraction = [point[0] - cell[0], point[1] - cell[1]];
    let a = hash21(cell);
    let b = hash21([cell[0] + 1.0, cell[1]]);
    let c = hash21([cell[0], cell[1] + 1.0]);
    let d = hash21([cell[0] + 1.0, cell[1] + 1.0]);
    let curve = fraction.map(|value| value * value * (3.0 - 2.0 * value));
    let top = a * (1.0 - curve[0]) + b * curve[0];
    let bottom = c * (1.0 - curve[0]) + d * curve[0];
    top * (1.0 - curve[1]) + bottom * curve[1]
}

fn hash21(point: [f64; 2]) -> f64 {
    let mut point = [fract(point[0] * 123.34), fract(point[1] * 456.21)];
    let dot = point[0] * (point[0] + 45.32) + point[1] * (point[1] + 45.32);
    point[0] += dot;
    point[1] += dot;
    fract(point[0] * point[1])
}

fn fract(value: f64) -> f64 {
    value - value.floor()
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ReferenceTransitionError {
    #[error("extension transition implementation digest is not executable")]
    ExtensionImplementationMismatch,
    #[error(transparent)]
    Pixel(#[from] PixelError),
    #[error(transparent)]
    Blend(#[from] BlendError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::Extent2d;

    const KERNELS: [PreparedTransitionKernel; 13] = [
        PreparedTransitionKernel::Fade,
        PreparedTransitionKernel::WipeLeft,
        PreparedTransitionKernel::WipeRight,
        PreparedTransitionKernel::CircleOpen,
        PreparedTransitionKernel::SimpleZoom,
        PreparedTransitionKernel::CrossWarp,
        PreparedTransitionKernel::LinearBlur,
        PreparedTransitionKernel::DirectionalWarp,
        PreparedTransitionKernel::DreamyZoom,
        PreparedTransitionKernel::Ripple,
        PreparedTransitionKernel::FlyEye,
        PreparedTransitionKernel::MultiplyBlend,
        PreparedTransitionKernel::Perlin,
    ];

    fn extent(width: u32, height: u32) -> Extent2d {
        Extent2d::new(width, height).unwrap()
    }

    fn solid(extent: Extent2d, rgb: [f32; 3], alpha: f32) -> ReferenceImage {
        ReferenceImage::solid(extent, PremulRgba32::from_straight(rgb, alpha).unwrap()).unwrap()
    }

    #[test]
    fn extension_cross_fade_rejects_an_unbound_implementation_digest() {
        let extent = extent(1, 1);
        let transparent = ReferenceImage::transparent(extent).unwrap();
        let kernel = PreparedTransitionKernel::ExtensionCrossFade {
            implementation_sha256: [2; 32],
            past_frames: 0,
            future_frames: 0,
        };
        assert_eq!(
            composite_transition(
                &transparent,
                &transparent,
                &transparent,
                kernel,
                0.0,
                1.0,
                1.0,
            ),
            Err(ReferenceTransitionError::ExtensionImplementationMismatch)
        );
    }

    #[test]
    fn every_kernel_has_exact_opacity_scaled_endpoints_over_one_backdrop() {
        let extent = extent(5, 3);
        let backdrop = solid(extent, [0.0, 0.4, 0.0], 0.5);
        let from = solid(extent, [1.0, 0.0, 0.0], 0.75);
        let to = solid(extent, [0.0, 0.0, 1.0], 0.25);
        let expected_from = from
            .scale_coverage(0.4)
            .unwrap()
            .source_over(&backdrop)
            .unwrap();
        let expected_to = to
            .scale_coverage(0.8)
            .unwrap()
            .source_over(&backdrop)
            .unwrap();

        for kernel in KERNELS {
            assert_eq!(
                composite_transition(&backdrop, &from, &to, kernel, 0.0, 0.4, 0.8).unwrap(),
                expected_from,
                "{kernel:?} from endpoint"
            );
            assert_eq!(
                composite_transition(&backdrop, &from, &to, kernel, 1.0, 0.4, 0.8).unwrap(),
                expected_to,
                "{kernel:?} to endpoint"
            );
        }
    }

    #[test]
    fn fade_interpolates_local_premul_then_places_it_over_backdrop_once() {
        let extent = extent(1, 1);
        let backdrop = solid(extent, [0.0, 1.0, 0.0], 0.5);
        let from = solid(extent, [1.0, 0.0, 0.0], 1.0);
        let to = solid(extent, [0.0, 0.0, 1.0], 1.0);
        let local = PremulRgba32::from_premultiplied([0.25, 0.0, 0.25, 0.5]).unwrap();
        let expected = local.source_over(pixel(&backdrop, 0, 0)).unwrap();

        let actual = composite_transition(
            &backdrop,
            &from,
            &to,
            PreparedTransitionKernel::Fade,
            0.5,
            0.5,
            0.5,
        )
        .unwrap();

        assert!(pixel(&actual, 0, 0).approx_eq(expected, 1.0e-7));
    }

    #[test]
    fn geometric_wipes_use_subpixel_coverage_and_mirror() {
        let extent = extent(4, 1);
        let transparent = ReferenceImage::transparent(extent).unwrap();
        let from = solid(extent, [1.0, 0.0, 0.0], 1.0);
        let to = solid(extent, [0.0, 0.0, 1.0], 1.0);
        let left = composite_transition(
            &transparent,
            &from,
            &to,
            PreparedTransitionKernel::WipeLeft,
            0.375,
            1.0,
            1.0,
        )
        .unwrap();
        let right = composite_transition(
            &transparent,
            &from,
            &to,
            PreparedTransitionKernel::WipeRight,
            0.375,
            1.0,
            1.0,
        )
        .unwrap();

        assert_eq!(pixel(&left, 0, 0), pixel(&to, 0, 0));
        assert!(pixel(&left, 1, 0).approx_eq(
            PremulRgba32::from_straight([0.5, 0.0, 0.5], 1.0).unwrap(),
            1.0e-7
        ));
        for x in 0..4 {
            assert_eq!(pixel(&left, x, 0), pixel(&right, 3 - x, 0));
        }
    }

    #[test]
    fn circle_open_is_circular_in_device_pixels_on_a_nonsquare_frame() {
        let extent = extent(8, 4);
        let transparent = ReferenceImage::transparent(extent).unwrap();
        let from = solid(extent, [1.0, 0.0, 0.0], 1.0);
        let to = solid(extent, [0.0, 0.0, 1.0], 1.0);
        let result = composite_transition(
            &transparent,
            &from,
            &to,
            PreparedTransitionKernel::CircleOpen,
            0.35,
            1.0,
            1.0,
        )
        .unwrap();

        assert_eq!(pixel(&result, 4, 0), pixel(&result, 2, 2));
    }

    #[test]
    fn transparent_sources_never_gain_coverage_in_motion_kernels() {
        let extent = extent(7, 5);
        let transparent = ReferenceImage::transparent(extent).unwrap();
        for kernel in KERNELS {
            assert_eq!(
                composite_transition(
                    &transparent,
                    &transparent,
                    &transparent,
                    kernel,
                    0.37,
                    0.8,
                    0.6,
                )
                .unwrap(),
                transparent,
                "{kernel:?}"
            );
        }
    }

    #[test]
    fn every_kernel_executes_semitransparent_spatial_inputs_at_mid_progress() {
        let extent = extent(7, 5);
        let backdrop = solid(extent, [0.1, 0.2, 0.3], 0.6);
        let mut from = Vec::new();
        let mut to = Vec::new();
        for y in 0..extent.height() {
            for x in 0..extent.width() {
                from.push(
                    PremulRgba32::from_straight(
                        [f32::from(x as u16) / 6.0, 0.1, f32::from(y as u16) / 4.0],
                        0.35,
                    )
                    .unwrap(),
                );
                to.push(
                    PremulRgba32::from_straight(
                        [0.2, f32::from(y as u16) / 4.0, f32::from(x as u16) / 6.0],
                        0.7,
                    )
                    .unwrap(),
                );
            }
        }
        let from = ReferenceImage::new(extent, from).unwrap();
        let to = ReferenceImage::new(extent, to).unwrap();

        for kernel in KERNELS {
            let result =
                composite_transition(&backdrop, &from, &to, kernel, 0.43, 0.8, 0.65).unwrap();
            assert_eq!(result.extent(), extent, "{kernel:?}");
            assert!(
                result
                    .pixels()
                    .iter()
                    .all(|pixel| (0.0..=1.0).contains(&pixel.alpha())),
                "{kernel:?}"
            );
        }
    }
}
