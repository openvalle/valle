//! Skia/Ganesh F16 lens distortion over a GPU-resident source image.

use std::cell::RefCell;

use skia_safe::{
    BlendMode, Data, FilterMode, Image, MipmapMode, Paint, RuntimeEffect, SamplingOptions, Surface,
    TileMode, runtime_effect::ChildPtr,
};
use valle_engine::compositor::lens::LensDistortionParams;

use super::draw::DrawError;

const LENS_DISTORTION: &str = r#"
uniform shader source;
uniform float2 sourceSize;
uniform float2 inputOffset;
uniform float2 outputOrigin;
uniform float4 frame;
uniform float k1;
uniform float k2;

float4 at(int2 pixel) {
    if (pixel.x < 0 || pixel.y < 0 || float(pixel.x) >= sourceSize.x || float(pixel.y) >= sourceSize.y) {
        return float4(0.0);
    }
    return float4(source.eval(float2(pixel) + 0.5));
}

float4 bilinearDecal(float2 pixel) {
    float2 base = floor(pixel);
    float2 fraction = pixel - base;
    int2 origin = int2(base);
    float4 sum = at(origin) * ((1.0 - fraction.x) * (1.0 - fraction.y));
    sum += at(origin + int2(1, 0)) * (fraction.x * (1.0 - fraction.y));
    sum += at(origin + int2(0, 1)) * ((1.0 - fraction.x) * fraction.y);
    sum += at(origin + int2(1, 1)) * (fraction.x * fraction.y);
    return sum;
}

half4 main(float2 xy) {
    float2 pixel = floor(xy);
    float2 sourcePixel = pixel - inputOffset;
    if (k1 == 0.0 && k2 == 0.0) return half4(at(int2(sourcePixel)));
    float2 devicePosition = outputOrigin + pixel + 0.5;
    if (devicePosition.x < frame.x || devicePosition.x >= frame.x + frame.z ||
        devicePosition.y < frame.y || devicePosition.y >= frame.y + frame.w) {
        return half4(0.0);
    }
    float2 center = frame.xy + frame.zw * 0.5;
    float2 delta = devicePosition - center;
    float radiusSquared = dot(frame.zw, frame.zw) * 0.25;
    float r2 = min(dot(delta, delta) / radiusSquared, 1.0);
    float scale = 1.0 + k1 * r2 + k2 * r2 * r2;
    float2 inputOrigin = outputOrigin + inputOffset;
    float2 samplePixel = center + delta * scale - inputOrigin - 0.5;
    return half4(bilinearDecal(samplePixel));
}
"#;

thread_local! {
    static EFFECT: RefCell<Option<RuntimeEffect>> = const { RefCell::new(None) };
}

pub(super) fn apply_lens_distortion_into(
    target: &mut Surface,
    source: &Image,
    source_size: [i32; 2],
    input_offset: [i32; 2],
    output_origin: [i32; 2],
    params: LensDistortionParams,
) -> Result<(), DrawError> {
    if !params.k1.is_finite()
        || !params.k2.is_finite()
        || params.k1.abs() > 0.5
        || params.k2.abs() > 0.5
        || !params
            .frame
            .iter()
            .all(|value| value.is_finite() && value.abs() <= 10_000_000.0)
        || params.frame[2] <= 0.0
        || params.frame[3] <= 0.0
    {
        return Err(DrawError::Unsupported(
            "invalid GPU lens distortion parameters".into(),
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
            "invalid GPU lens distortion extent".into(),
        ));
    }
    if std::env::var_os("VALLE_TRACE_F16_STAGES").is_some() {
        eprintln!(
            "[valle f16] gpu-lens-distortion {}x{} (no intermediate readback)",
            dimensions.width, dimensions.height
        );
    }
    EFFECT.with(|cell| {
        let mut shader = cell.borrow_mut();
        if shader.is_none() {
            *shader = Some(
                RuntimeEffect::make_for_shader(LENS_DISTORTION, None).map_err(|message| {
                    DrawError::ShaderCompile {
                        uri: "builtin://gpu-lens-distortion".into(),
                        message,
                    }
                })?,
            );
        }
        draw(
            target,
            source,
            shader
                .as_ref()
                .expect("GPU lens distortion shader admitted"),
            [
                source_size[0] as f32,
                source_size[1] as f32,
                input_offset[0] as f32,
                input_offset[1] as f32,
                output_origin[0] as f32,
                output_origin[1] as f32,
                params.frame[0],
                params.frame[1],
                params.frame[2],
                params.frame[3],
                params.k1,
                params.k2,
            ],
        )
    })
}

fn draw(
    target: &mut Surface,
    source: &Image,
    effect: &RuntimeEffect,
    uniforms: [f32; 12],
) -> Result<(), DrawError> {
    let mut bytes = Vec::with_capacity(uniforms.len() * 4);
    for value in uniforms {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    if bytes.len() != effect.uniform_size() || effect.children().len() != 1 {
        return Err(DrawError::Internal(
            "GPU lens distortion shader ABI mismatch".into(),
        ));
    }
    let source_shader = source
        .to_raw_shader(
            (TileMode::Clamp, TileMode::Clamp),
            SamplingOptions::new(FilterMode::Nearest, MipmapMode::None),
            None,
        )
        .ok_or_else(|| DrawError::Unsupported("GPU lens distortion source shader".into()))?;
    let shader = effect
        .make_shader(
            Data::new_copy(&bytes),
            &[ChildPtr::Shader(source_shader)],
            None,
        )
        .ok_or_else(|| DrawError::Unsupported("GPU lens distortion shader instantiation".into()))?;
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    paint.set_shader(shader);
    target.canvas().draw_paint(&paint);
    Ok(())
}
