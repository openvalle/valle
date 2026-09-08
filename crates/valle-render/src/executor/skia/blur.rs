//! Direct separable convolution for raster float surfaces. Skia's specialized CPU blur
//! handles A8/8888 only; its float fallback evaluates a general shader for every tap.

use skia_safe::{
    AlphaType, BlendMode, Canvas, ColorType, Data, FilterMode, IRect, Image, ImageInfo, MipmapMode,
    Paint, SamplingOptions, color_filters, image::CachingHint, images,
};
use valle_draw::program::{FILTER_GAUSSIAN_SUPPORT_SIGMAS, Filter};

use super::{draw::DrawError, effect::straight_color};

// Very wide kernels are cheaper on Skia's downsampled path. Bound direct convolution's
// work and temporary extent rather than replacing that path with an unbounded tap loop.
const MAX_DIRECT_RADIUS: usize = 96;

/// Draws at integer device coordinates, respecting the existing canvas clip. Returns false
/// for GPU/non-float images or wide kernels, leaving those to the existing Skia filter.
pub(super) fn draw(
    canvas: &Canvas,
    input: &Image,
    source: IRect,
    origin: [i32; 2],
    filter: &Filter,
) -> Result<bool, DrawError> {
    let (sigma_x, sigma_y, shadow) = match filter {
        Filter::Blur { sigma_x, sigma_y } => (*sigma_x, *sigma_y, None),
        Filter::DropShadow {
            sigma_x,
            sigma_y,
            offset,
            color,
        } => (*sigma_x, *sigma_y, Some((*offset, *color))),
        _ => return Ok(false),
    };
    if !matches!(input.color_type(), ColorType::RGBAF16 | ColorType::RGBAF32)
        || canvas.peek_pixels().is_none()
        || input.peek_pixels().is_none()
        || !canvas.local_to_device_as_3x3().is_identity()
        || source.is_empty()
    {
        return Ok(false);
    }
    let Some(horizontal) = kernel(sigma_x) else {
        return Ok(false);
    };
    let Some(vertical) = kernel(sigma_y) else {
        return Ok(false);
    };
    let rx = horizontal.len() / 2;
    let ry = vertical.len() / 2;
    let width = source.width() as usize;
    let height = source.height() as usize;
    let output_width = width + rx * 2;
    let output_height = height + ry * 2;
    let info = ImageInfo::new(
        (source.width(), source.height()),
        ColorType::RGBAF32,
        AlphaType::Premul,
        input.color_space(),
    );
    let mut pixels = vec![0.0_f32; width * height * 4];
    if !input.read_pixels(
        &info,
        &mut pixels,
        width * 16,
        (source.left, source.top),
        CachingHint::Disallow,
    ) {
        return Err(DrawError::Surface("float blur input read failed".into()));
    }
    let blurred = if shadow.is_some() {
        let alpha = pixels
            .chunks_exact(4)
            .map(|pixel| pixel[3])
            .collect::<Vec<_>>();
        let coverage = convolve(&alpha, width, height, 1, &horizontal, &vertical);
        coverage
            .into_iter()
            .flat_map(|a| [0.0, 0.0, 0.0, a])
            .collect::<Vec<_>>()
    } else {
        convolve(&pixels, width, height, 4, &horizontal, &vertical)
    };
    let bytes = blurred
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect::<Vec<_>>();
    let output_info = info.with_dimensions((output_width as i32, output_height as i32));
    let image = images::raster_from_data(&output_info, Data::new_copy(&bytes), output_width * 16)
        .ok_or_else(|| DrawError::Surface("float blur output allocation failed".into()))?;
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    let mut position = [origin[0] as f32 - rx as f32, origin[1] as f32 - ry as f32];
    if let Some((offset, color)) = shadow {
        position[0] += offset[0];
        position[1] += offset[1];
        // Match Skia DropShadow's existing color interpretation. Only the alpha convolution
        // changes; tinting, fractional translation and source-over stay with Skia.
        paint.set_color_filter(color_filters::blend(
            straight_color(color).to_color(),
            BlendMode::SrcIn,
        ));
    }
    canvas.draw_image_with_sampling_options(
        &image,
        (position[0], position[1]),
        SamplingOptions::new(FilterMode::Linear, MipmapMode::None),
        Some(&paint),
    );
    if shadow.is_some() {
        paint.set_color_filter(None);
        paint.set_blend_mode(BlendMode::SrcOver);
        let destination = skia_safe::Rect::from_xywh(
            origin[0] as f32,
            origin[1] as f32,
            width as f32,
            height as f32,
        );
        canvas.draw_image_rect_with_sampling_options(
            input,
            Some((&source.into(), skia_safe::canvas::SrcRectConstraint::Strict)),
            destination,
            SamplingOptions::default(),
            &paint,
        );
    }
    Ok(true)
}

fn kernel(sigma: f32) -> Option<Vec<f32>> {
    if !sigma.is_finite() || sigma < 0.0 {
        return None;
    }
    if sigma <= 0.03 {
        return Some(vec![1.0]);
    }
    let radius = (sigma * FILTER_GAUSSIAN_SUPPORT_SIGMAS).ceil() as usize;
    if radius > MAX_DIRECT_RADIUS {
        return None;
    }
    let denominator = 2.0 * f64::from(sigma).powi(2);
    let weights = (-(radius as i32)..=radius as i32)
        .map(|x| (-(f64::from(x).powi(2)) / denominator).exp())
        .collect::<Vec<_>>();
    let total: f64 = weights.iter().sum();
    Some(
        weights
            .into_iter()
            .map(|weight| (weight / total) as f32)
            .collect(),
    )
}

// Full convolution with transparent extension. Loop over each tap and contiguous row spans
// so the compiler can vectorize channels/pixels, with no edge tests in the inner loop.
fn convolve(
    source: &[f32],
    width: usize,
    height: usize,
    channels: usize,
    x: &[f32],
    y: &[f32],
) -> Vec<f32> {
    let output_width = width + x.len() - 1;
    let input_stride = width * channels;
    let stride = output_width * channels;
    let mut horizontal = vec![0.0; stride * height];
    for (input, output) in source
        .chunks_exact(input_stride)
        .zip(horizontal.chunks_exact_mut(stride))
    {
        for (tap, weight) in x.iter().enumerate() {
            accumulate(
                &mut output[tap * channels..tap * channels + input_stride],
                input,
                *weight,
            );
        }
    }
    let mut output = vec![0.0; stride * (height + y.len() - 1)];
    for (row, input) in horizontal.chunks_exact(stride).enumerate() {
        for (tap, weight) in y.iter().enumerate() {
            accumulate(
                &mut output[(row + tap) * stride..(row + tap + 1) * stride],
                input,
                *weight,
            );
        }
    }
    output
}

fn accumulate(output: &mut [f32], input: &[f32], weight: f32) {
    for (output, input) in output.iter_mut().zip(input) {
        *output += *input * weight;
    }
}

#[cfg(test)]
mod tests {
    use super::super::{effect::image_filter, surface::working_color_space};
    use super::*;
    use skia_safe::{Color4f, Rect, Surface, surfaces};
    use valle_draw::program::LinearColor;

    #[test]
    fn separable_float_blur_matches_direct_2d_gaussian_with_transparent_edges() {
        let (width, height, channels) = (7, 5, 4);
        let source = (0..width * height)
            .flat_map(|i| {
                let a = (i % 5) as f32 / 4.0;
                [-0.1 * a, 3.0 * a, (i % 3) as f32 * a, a]
            })
            .collect::<Vec<_>>();
        for (sx, sy) in [(0.0_f32, 0.0_f32), (0.25, 1.3), (2.4, 0.0), (1.2, 3.1)] {
            let x = kernel(sx).unwrap();
            let y = kernel(sy).unwrap();
            let output = convolve(&source, width, height, channels, &x, &y);
            let ow = width + x.len() - 1;
            let oh = height + y.len() - 1;
            // Independent two-dimensional definition, including signed/wide-gamut channels.
            let weight = |d: i32, sigma: f32| {
                if sigma == 0.0 {
                    f64::from(d == 0)
                } else {
                    (-(d as f64).powi(2) / (2.0 * (sigma as f64).powi(2))).exp()
                }
            };
            let rx = (x.len() / 2) as i32;
            let ry = (y.len() / 2) as i32;
            let normalization: f64 = (-ry..=ry)
                .flat_map(|dy| (-rx..=rx).map(move |dx| weight(dx, sx) * weight(dy, sy)))
                .sum();
            for oy in 0..oh {
                for ox in 0..ow {
                    for c in 0..channels {
                        let mut expected = 0.0;
                        for iy in 0..height {
                            for ix in 0..width {
                                let dx = ox as i32 - rx - ix as i32;
                                let dy = oy as i32 - ry - iy as i32;
                                if dx.abs() <= rx && dy.abs() <= ry {
                                    expected += source[(iy * width + ix) * channels + c] as f64
                                        * weight(dx, sx)
                                        * weight(dy, sy)
                                        / normalization;
                                }
                            }
                        }
                        assert!(
                            (output[(oy * ow + ox) * channels + c] as f64 - expected).abs() < 2e-6
                        );
                    }
                }
            }
        }
    }

    fn input_image() -> Image {
        let info = ImageInfo::new(
            (23, 17),
            ColorType::RGBAF16,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let mut surface = surfaces::raster(&info, None, None).unwrap();
        surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        let mut paint = Paint::default();
        paint.set_color4f(Color4f::new(0.7, 0.2, 0.5, 0.65), None);
        surface
            .canvas()
            .draw_rect(Rect::from_xywh(2.0, 3.0, 16.0, 11.0), &paint);
        surface.image_snapshot()
    }

    fn read_float(surface: &mut Surface) -> Vec<f32> {
        let image = surface.image_snapshot();
        let info = image.image_info().with_color_type(ColorType::RGBAF32);
        let mut pixels = vec![0.0; (image.width() * image.height() * 4) as usize];
        assert!(image.read_pixels(
            &info,
            &mut pixels,
            image.width() as usize * 16,
            (0, 0),
            CachingHint::Disallow
        ));
        pixels
    }

    #[test]
    fn raster_blur_and_shadow_preserve_clip_roi_and_fractional_shadow_offsets() {
        let input = input_image();
        let source = IRect::from_xywh(1, 2, 19, 13);
        for origin in [[6, 7], [-3, 4]] {
            for filter in [
                Filter::Blur {
                    sigma_x: 0.0,
                    sigma_y: 0.0,
                },
                Filter::Blur {
                    sigma_x: 1.3,
                    sigma_y: 0.7,
                },
                Filter::Blur {
                    sigma_x: 0.0,
                    sigma_y: 2.0,
                },
                Filter::DropShadow {
                    sigma_x: 2.2,
                    sigma_y: 1.3,
                    offset: [-2.5, 3.25],
                    color: LinearColor::new(0.2, 0.05, 0.1, 0.5),
                },
                Filter::DropShadow {
                    sigma_x: 16.0,
                    sigma_y: 16.0,
                    offset: [0.0, 12.0],
                    color: LinearColor::new(0.0, 0.0, 0.0, 0.5),
                },
            ] {
                let info = input.image_info().with_dimensions((40, 36));
                let mut actual = surfaces::raster(&info, None, None).unwrap();
                let mut expected = surfaces::raster(&info, None, None).unwrap();
                for surface in [&mut actual, &mut expected] {
                    surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
                    surface.canvas().clip_rect(
                        Rect::from_xywh(2.0, 1.0, 35.0, 32.0),
                        skia_safe::ClipOp::Intersect,
                        false,
                    );
                }
                assert!(draw(actual.canvas(), &input, source, origin, &filter).unwrap());
                let mut paint = Paint::default();
                paint.set_blend_mode(BlendMode::Src);
                paint.set_image_filter(image_filter(&filter).unwrap());
                expected.canvas().draw_image_rect_with_sampling_options(
                    &input,
                    Some((&source.into(), skia_safe::canvas::SrcRectConstraint::Strict)),
                    Rect::from_xywh(
                        origin[0] as f32,
                        origin[1] as f32,
                        source.width() as f32,
                        source.height() as f32,
                    ),
                    SamplingOptions::default(),
                    &paint,
                );
                let actual = read_float(&mut actual);
                let expected = read_float(&mut expected);
                let delta = actual
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0_f32, f32::max);
                // Skia downsamples wide float kernels; compare that approximation separately
                // from the exact Gaussian test above. F16 storage also rounds intermediates.
                assert!(
                    delta < 0.008,
                    "{filter:?}, origin={origin:?}, delta={delta}"
                );
            }
        }
    }

    #[test]
    fn wide_kernels_and_nonidentity_canvas_keep_the_skia_path() {
        assert!(kernel(33.0).is_none());
        let input = input_image();
        let mut output = surfaces::raster(&input.image_info(), None, None).unwrap();
        output.canvas().scale((2.0, 2.0));
        assert!(
            !draw(
                output.canvas(),
                &input,
                IRect::from_size(input.dimensions()),
                [0, 0],
                &Filter::Blur {
                    sigma_x: 1.0,
                    sigma_y: 1.0
                }
            )
            .unwrap()
        );
    }
}
