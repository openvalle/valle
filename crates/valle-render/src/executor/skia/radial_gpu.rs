//! Skia/Ganesh F16 radial blur over a GPU-resident source image.

use std::cell::RefCell;

use skia_safe::{
    BlendMode, Data, FilterMode, Image, MipmapMode, Paint, RuntimeEffect, SamplingOptions, Surface,
    TileMode, runtime_effect::ChildPtr,
};
use valle_engine::compositor::radial::{RadialBlurParams, radial_blur_plan};

use super::draw::DrawError;

const RADIAL_BLUR: &str = include_str!("../../../../valle-draw/assets/shaders/radialblur.sksl");

thread_local! {
    static EFFECT: RefCell<Option<RuntimeEffect>> = const { RefCell::new(None) };
}

pub(super) fn apply_radial_blur_into(
    target: &mut Surface,
    source: &Image,
    source_size: [i32; 2],
    input_offset: [i32; 2],
    output_origin: [i32; 2],
    params: RadialBlurParams,
) -> Result<(), DrawError> {
    if !params.center.iter().all(|value| value.is_finite())
        || !params.amount.is_finite()
        || params.amount < 0.0
    {
        return Err(DrawError::Unsupported(
            "invalid GPU radial blur parameters".into(),
        ));
    }
    let dimensions = target.image_info().dimensions();
    if source_size[0] <= 0
        || source_size[1] <= 0
        || source_size[0] > source.width()
        || source_size[1] > source.height()
        || i64::from(source_size[0]) * i64::from(source_size[1]) > 16_777_216
        || dimensions.width <= 0
        || dimensions.height <= 0
        || i64::from(dimensions.width) * i64::from(dimensions.height) > 16_777_216
    {
        return Err(DrawError::Unsupported(
            "invalid GPU radial blur extent".into(),
        ));
    }
    if std::env::var_os("VALLE_TRACE_F16_STAGES").is_some() {
        eprintln!(
            "[valle f16] gpu-radial-blur {}x{} (no intermediate readback)",
            dimensions.width, dimensions.height
        );
    }
    EFFECT.with(|cell| {
        let mut shader = cell.borrow_mut();
        if shader.is_none() {
            *shader = Some(RuntimeEffect::make_for_shader(RADIAL_BLUR, None).map_err(
                |message| DrawError::ShaderCompile {
                    uri: "builtin://gpu-radial-blur".into(),
                    message,
                },
            )?);
        }
        let initial_origin = [
            output_origin[0].checked_add(input_offset[0]),
            output_origin[1].checked_add(input_offset[1]),
        ];
        let [Some(mut x), Some(mut y)] = initial_origin else {
            return Err(DrawError::Unsupported("invalid radial blur origin".into()));
        };
        let plan = radial_blur_plan(
            params.amount,
            [x, y],
            source_size.map(|s| s as u32),
            output_origin,
            [dimensions.width as u32, dimensions.height as u32],
        )
        .map_err(|e| DrawError::Unsupported(e.to_string()))?;
        let mut image = source.clone();
        let mut size = source_size;
        for (index, pass) in plan.iter().enumerate() {
            let uniforms = [
                source_size[0] as f32,
                source_size[1] as f32,
                (i64::from(initial_origin[0].unwrap()) - i64::from(pass.origin[0])) as f32,
                (i64::from(initial_origin[1].unwrap()) - i64::from(pass.origin[1])) as f32,
                if index + 1 == plan.len() { 1.0 } else { 0.0 },
                size[0] as f32,
                size[1] as f32,
                (i64::from(x) - i64::from(pass.origin[0])) as f32,
                (i64::from(y) - i64::from(pass.origin[1])) as f32,
                pass.origin[0] as f32,
                pass.origin[1] as f32,
                params.center[0],
                params.center[1],
                params.amount,
                pass.fraction,
            ];
            let effect = shader.as_ref().expect("GPU radial blur shader admitted");
            if index + 1 == plan.len() {
                draw(target, &image, source, effect, uniforms, BlendMode::Src)?;
            } else {
                let info = target
                    .image_info()
                    .with_dimensions((pass.size[0] as i32, pass.size[1] as i32));
                let mut intermediate = target.new_surface(&info).ok_or_else(|| {
                    DrawError::Surface("radial blur intermediate allocation failed".into())
                })?;
                draw(
                    &mut intermediate,
                    &image,
                    source,
                    effect,
                    uniforms,
                    BlendMode::Src,
                )?;
                image = intermediate.image_snapshot();
            }
            [x, y] = pass.origin;
            size = pass.size.map(|s| s as i32);
        }
        Ok(())
    })
}

fn draw(
    target: &mut Surface,
    source: &Image,
    original: &Image,
    effect: &RuntimeEffect,
    uniforms: [f32; 15],
    blend: BlendMode,
) -> Result<(), DrawError> {
    let mut bytes = Vec::with_capacity(uniforms.len() * 4);
    for value in uniforms {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    if bytes.len() != effect.uniform_size() || effect.children().len() != 2 {
        return Err(DrawError::Internal(
            "GPU radial blur shader ABI mismatch".into(),
        ));
    }
    let source_shader = source
        .to_raw_shader(
            (TileMode::Clamp, TileMode::Clamp),
            SamplingOptions::new(FilterMode::Nearest, MipmapMode::None),
            None,
        )
        .ok_or_else(|| DrawError::Unsupported("GPU radial blur source shader".into()))?;
    let original_shader = original
        .to_raw_shader(
            (TileMode::Clamp, TileMode::Clamp),
            SamplingOptions::new(FilterMode::Nearest, MipmapMode::None),
            None,
        )
        .ok_or_else(|| DrawError::Unsupported("GPU radial blur original shader".into()))?;
    let shader = effect
        .make_shader(
            Data::new_copy(&bytes),
            &[
                ChildPtr::Shader(source_shader),
                ChildPtr::Shader(original_shader),
            ],
            None,
        )
        .ok_or_else(|| DrawError::Unsupported("GPU radial blur shader instantiation".into()))?;
    let mut paint = Paint::default();
    paint.set_blend_mode(blend);
    paint.set_shader(shader);
    target.canvas().draw_paint(&paint);
    Ok(())
}
