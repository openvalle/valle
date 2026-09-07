use skia_safe::{
    BlendMode, ClipOp, Color4f, Image, ImageInfo, Matrix, Paint, Rect as SkRect, SamplingOptions,
    Surface, canvas::SrcRectConstraint,
};
use valle_draw::Rect;
use valle_engine::compositor::lower::{PlanEffect, RenderBindings, SurfaceSlotId};
use valle_engine::prepare::{
    DeviceRect, DeviceTransform, DynamicBindingKind, ExternalPlacement, ExternalSample,
    PreparedExternalBackdrop,
};

use super::{
    draw::DrawError,
    effect::{EffectRuntime, apply_filter_into, straight_color},
    surface::{ScratchSurfaces, working_color_space},
};

pub(crate) fn render_import(
    surfaces: &mut ScratchSurfaces<'_, '_>,
    output_slot: SurfaceSlotId,
    source: &Image,
    placement: ExternalPlacement,
    transform: DeviceTransform,
    bounds: DeviceRect,
    chroma: Option<&PlanEffect>,
    scalar: impl Fn(
        valle_engine::prepare::DynamicBindingId,
        DynamicBindingKind,
    ) -> Result<f32, DrawError>
    + Copy,
    effects: &EffectRuntime,
    bindings: &RenderBindings,
    info: &ImageInfo,
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    let transform = translated_device_transform(transform, target_origin)?;
    let bounds = DeviceRect::new(
        bounds.x - target_origin[0],
        bounds.y - target_origin[1],
        bounds.width,
        bounds.height,
    );
    surfaces
        .output_mut(output_slot)?
        .canvas()
        .clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    if bounds.is_empty() {
        return Ok(());
    }

    match placement.backdrop {
        None => {}
        Some(PreparedExternalBackdrop::Color {
            working_linear_rec2020_premul,
        }) => draw_color_backdrop(
            surfaces.output_mut(output_slot)?,
            working_linear_rec2020_premul,
            transform,
            bounds,
        )?,
        Some(PreparedExternalBackdrop::Blur {
            sigma_device_px,
            sample,
        }) => {
            render_sample_into(
                surfaces.surface_mut(0)?,
                source,
                sample,
                placement.clip_rect,
                placement.clip_rect,
                transform,
                bounds,
            )?;
            let cover = surfaces.surface_mut(0)?.image_snapshot();
            let sigma = scalar(sigma_device_px, DynamicBindingKind::DeviceLength)?;
            apply_filter_into(
                surfaces.surface_mut(1)?,
                &cover,
                &valle_draw::program::Filter::Blur {
                    sigma_x: sigma,
                    sigma_y: sigma,
                },
            )?;
            let blurred = surfaces.surface_mut(1)?.image_snapshot();
            clip_device_image_into(
                surfaces.output_mut(output_slot)?,
                &blurred,
                transform,
                placement.clip_rect,
                bounds,
            )?;
        }
    }

    if chroma.is_none() {
        draw_sample(
            surfaces.output_mut(output_slot)?,
            source,
            placement.sample,
            placement.content_rect,
            placement.clip_rect,
            transform,
            bounds,
            BlendMode::SrcOver,
        )?;
        return Ok(());
    }

    render_sample_into(
        surfaces.surface_mut(0)?,
        source,
        placement.sample,
        placement.content_rect,
        placement.clip_rect,
        transform,
        bounds,
    )?;
    let mut primary = surfaces.surface_mut(0)?.image_snapshot();
    if let Some(effect) = chroma {
        effects.apply_prepared_into(surfaces.surface_mut(1)?, &primary, effect, bindings, info)?;
        primary = surfaces.surface_mut(1)?.image_snapshot();
    }
    draw_image(
        surfaces.output_mut(output_slot)?,
        &primary,
        BlendMode::SrcOver,
    )
}

fn translated_device_transform(
    transform: DeviceTransform,
    origin: [i32; 2],
) -> Result<DeviceTransform, DrawError> {
    let mut matrix = transform.matrix();
    let x = f64::from(origin[0]);
    let y = f64::from(origin[1]);
    matrix[0] -= x * matrix[6];
    matrix[1] -= x * matrix[7];
    matrix[2] -= x * matrix[8];
    matrix[3] -= y * matrix[6];
    matrix[4] -= y * matrix[7];
    matrix[5] -= y * matrix[8];
    DeviceTransform::from_projective(matrix).map_err(|error| DrawError::Internal(error.to_string()))
}

fn draw_color_backdrop(
    surface: &mut Surface,
    color: [f32; 4],
    transform: DeviceTransform,
    bounds: DeviceRect,
) -> Result<(), DrawError> {
    let canvas = surface.canvas();
    canvas.save();
    canvas.clip_rect(device_rect(bounds), ClipOp::Intersect, false);
    canvas.concat(&matrix(transform.matrix()));
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    let space = working_color_space().map_err(|error| DrawError::Surface(error.to_string()))?;
    paint.set_color4f(
        straight_color(valle_draw::program::LinearColor {
            red: color[0],
            green: color[1],
            blue: color[2],
            alpha: color[3],
        }),
        &space,
    );
    canvas.draw_rect(SkRect::from_xywh(0.0, 0.0, 1.0, 1.0), &paint);
    canvas.restore();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_sample_into(
    surface: &mut Surface,
    image: &Image,
    sample: ExternalSample,
    destination: Rect,
    clip: Rect,
    transform: DeviceTransform,
    bounds: DeviceRect,
) -> Result<(), DrawError> {
    surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    draw_sample(
        surface,
        image,
        sample,
        destination,
        clip,
        transform,
        bounds,
        BlendMode::Src,
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_sample(
    surface: &mut Surface,
    image: &Image,
    sample: ExternalSample,
    destination: Rect,
    clip: Rect,
    transform: DeviceTransform,
    bounds: DeviceRect,
    blend_mode: BlendMode,
) -> Result<(), DrawError> {
    let ExternalSample::Texture {
        texture_from_content,
        input_sample_bounds,
        ..
    } = sample
    else {
        return Ok(());
    };
    let inverse = inverse3(texture_from_content)
        .ok_or_else(|| DrawError::Unsupported("non-invertible external sample".into()))?;
    let width = f64::from(image.width());
    let height = f64::from(image.height());
    let normalized_from_pixel = [1.0 / width, 0.0, 0.0, 0.0, 1.0 / height, 0.0, 0.0, 0.0, 1.0];
    let local_from_content = [
        destination.width,
        0.0,
        destination.x,
        0.0,
        destination.height,
        destination.y,
        0.0,
        0.0,
        1.0,
    ];
    let local_from_pixel = mul3(local_from_content, mul3(inverse, normalized_from_pixel));
    let src = SkRect::from_xywh(
        (input_sample_bounds.x * width) as f32,
        (input_sample_bounds.y * height) as f32,
        (input_sample_bounds.width * width) as f32,
        (input_sample_bounds.height * height) as f32,
    );

    let canvas = surface.canvas();
    canvas.save();
    canvas.clip_rect(device_rect(bounds), ClipOp::Intersect, false);
    canvas.concat(&matrix(transform.matrix()));
    canvas.clip_rect(sk_rect(clip), ClipOp::Intersect, true);
    canvas.clip_rect(sk_rect(destination), ClipOp::Intersect, true);
    canvas.concat(&matrix(local_from_pixel));
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_blend_mode(blend_mode);
    canvas.draw_image_rect_with_sampling_options(
        image,
        Some((&src, SrcRectConstraint::Strict)),
        src,
        SamplingOptions::new(skia_safe::FilterMode::Linear, skia_safe::MipmapMode::None),
        &paint,
    );
    canvas.restore();
    Ok(())
}

fn draw_image(surface: &mut Surface, image: &Image, mode: BlendMode) -> Result<(), DrawError> {
    let mut paint = Paint::default();
    paint.set_blend_mode(mode);
    surface.canvas().draw_image(image, (0.0, 0.0), Some(&paint));
    Ok(())
}

fn clip_device_image_into(
    surface: &mut Surface,
    image: &Image,
    transform: DeviceTransform,
    clip: Rect,
    bounds: DeviceRect,
) -> Result<(), DrawError> {
    let canvas = surface.canvas();
    canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    canvas.save();
    canvas.clip_rect(device_rect(bounds), ClipOp::Intersect, false);
    canvas.concat(&matrix(transform.matrix()));
    canvas.clip_rect(sk_rect(clip), ClipOp::Intersect, true);
    canvas.reset_matrix();
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    canvas.draw_image(image, (0.0, 0.0), Some(&paint));
    canvas.restore();
    Ok(())
}

fn sk_rect(rect: Rect) -> SkRect {
    SkRect::from_xywh(
        rect.x as f32,
        rect.y as f32,
        rect.width as f32,
        rect.height as f32,
    )
}

fn device_rect(rect: DeviceRect) -> SkRect {
    SkRect::from_xywh(
        rect.x as f32,
        rect.y as f32,
        rect.width as f32,
        rect.height as f32,
    )
}

fn matrix(value: [f64; 9]) -> Matrix {
    Matrix::new_all(
        value[0] as f32,
        value[1] as f32,
        value[2] as f32,
        value[3] as f32,
        value[4] as f32,
        value[5] as f32,
        value[6] as f32,
        value[7] as f32,
        value[8] as f32,
    )
}

fn mul3(left: [f64; 9], right: [f64; 9]) -> [f64; 9] {
    let mut result = [0.0; 9];
    for row in 0..3 {
        for column in 0..3 {
            result[row * 3 + column] = (0..3)
                .map(|index| left[row * 3 + index] * right[index * 3 + column])
                .sum();
        }
    }
    result
}

fn inverse3(value: [f64; 9]) -> Option<[f64; 9]> {
    let [a, b, c, d, e, f, g, h, i] = value;
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
    if !determinant.is_finite() || determinant.abs() <= 1.0e-15 {
        return None;
    }
    Some(cofactors.map(|value| value / determinant))
}
