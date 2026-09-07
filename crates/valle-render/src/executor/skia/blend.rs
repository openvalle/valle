use skia_safe::{Image, RuntimeEffect, Surface};
use valle_draw::program::BlendMode as DrawBlend;

use super::{draw::DrawError, effect::render_runtime_into};

const BLEND_SOURCE: &str = include_str!("../../../../valle-draw/assets/shaders/creativeblend.sksl");

#[derive(Debug)]
pub(crate) struct BlendRuntime {
    effect: RuntimeEffect,
}

impl BlendRuntime {
    pub(crate) fn admit() -> Result<Self, DrawError> {
        let effect = RuntimeEffect::make_for_shader(BLEND_SOURCE, None).map_err(|message| {
            DrawError::ShaderCompile {
                uri: "builtin://creative-blend".into(),
                message,
            }
        })?;
        Ok(Self { effect })
    }

    pub(crate) fn effective_source_into(
        &self,
        output: &mut Surface,
        source: &Image,
        destination: &Image,
        mode: DrawBlend,
    ) -> Result<(), DrawError> {
        render_runtime_into(
            output,
            &self.effect,
            &[source, destination],
            &[draw_code(mode), 1.0, 0.0],
        )
    }

    pub(crate) fn composite_into(
        &self,
        output: &mut Surface,
        source: &Image,
        destination: &Image,
        mode: DrawBlend,
        opacity: f32,
    ) -> Result<(), DrawError> {
        render_runtime_into(
            output,
            &self.effect,
            &[source, destination],
            &[draw_code(mode), opacity, 1.0],
        )
    }
}

const fn draw_code(mode: DrawBlend) -> f32 {
    match mode {
        DrawBlend::Normal => 0.0,
        DrawBlend::Multiply => 1.0,
        DrawBlend::Screen => 2.0,
        DrawBlend::Overlay => 3.0,
        DrawBlend::Darken => 4.0,
        DrawBlend::Lighten => 5.0,
        DrawBlend::ColorDodge => 6.0,
        DrawBlend::ColorBurn => 7.0,
        DrawBlend::LinearBurn => 8.0,
        DrawBlend::HardLight => 9.0,
        DrawBlend::SoftLight => 10.0,
        DrawBlend::Difference => 11.0,
        DrawBlend::Exclusion => 12.0,
        DrawBlend::Hue => 13.0,
        DrawBlend::Saturation => 14.0,
        DrawBlend::Color => 15.0,
        DrawBlend::Luminosity => 16.0,
    }
}

#[cfg(test)]
mod tests {
    use skia_safe::{
        AlphaType, BlendMode, Color4f, ColorType, ImageInfo, Paint, image::CachingHint,
    };
    use valle_engine::{
        compositor::reference::{PremulRgba32, ReferenceBlendMode, blend_over},
        resource::Extent2d,
    };

    use super::*;
    use crate::executor::skia::surface::{raster_surface, working_color_space, working_info};

    #[test]
    fn admitted_kernel_matches_reference_linear_burn_effective_composite() {
        let extent = Extent2d::new(1, 1).unwrap();
        let info = working_info(extent).unwrap();
        let source_value = PremulRgba32::from_premultiplied([0.35, 0.12, 0.08, 0.7]).unwrap();
        let destination_value = PremulRgba32::from_premultiplied([0.18, 0.32, 0.09, 0.8]).unwrap();
        let source = solid(source_value, &info);
        let destination = solid(destination_value, &info);
        let opacity = 0.65;
        let mut output = raster_surface(&info).unwrap();
        BlendRuntime::admit()
            .unwrap()
            .composite_into(
                &mut output,
                &source,
                &destination,
                DrawBlend::LinearBurn,
                opacity,
            )
            .unwrap();
        let actual = output.image_snapshot();
        let expected = blend_over(
            source_value.scale_coverage(opacity).unwrap(),
            destination_value,
            ReferenceBlendMode::LinearBurn,
        )
        .unwrap();
        let actual = read(&actual);
        assert!(
            actual.approx_eq(expected, 0.004),
            "actual={actual:?} expected={expected:?}"
        );
    }

    fn solid(value: PremulRgba32, info: &ImageInfo) -> Image {
        let mut surface = raster_surface(info).unwrap();
        let channels = value.channels();
        let alpha = channels[3];
        let straight = if alpha == 0.0 {
            Color4f::new(0.0, 0.0, 0.0, 0.0)
        } else {
            Color4f::new(
                channels[0] / alpha,
                channels[1] / alpha,
                channels[2] / alpha,
                alpha,
            )
        };
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::Src);
        let space = working_color_space().unwrap();
        paint.set_color4f(straight, &space);
        surface.canvas().draw_paint(&paint);
        surface.image_snapshot()
    }

    fn read(image: &Image) -> PremulRgba32 {
        let info = ImageInfo::new(
            (1, 1),
            ColorType::RGBAF32,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let mut channels = [0.0_f32; 4];
        assert!(image.read_pixels(&info, &mut channels, 16, (0, 0), CachingHint::Disallow,));
        PremulRgba32::from_premultiplied(channels).unwrap()
    }
}
