//! P010 planes are left-aligned 10-bit codes in R16Unorm textures. Skia's 8-bit Rec.601/709
//! matrices use different normalization and offsets. Convert their raw codes before clamping;
//! retain the input transfer/primaries for the existing working-color import.
use std::cell::RefCell;

use skia_safe::{
    BlendMode, ColorSpace, ColorType, Data, Image, ImageInfo, Paint, RuntimeEffect,
    SamplingOptions, TileMode, gpu, runtime_effect::ChildPtr,
};
use valle_media::YuvMatrix;

use super::surface::SurfaceError;

const SOURCE: &str = r#"
uniform shader planes;
uniform float4 parameters;
half4 main(float2 xy) {
    float3 codes = float3(planes.eval(xy).rgb) * (65535.0 / 64.0);
    float y = (codes.x - parameters.x) / parameters.y;
    float2 chroma = (codes.yz - float2(512.0)) / parameters.z;
    float kr = parameters.w;
    float kb = kr > 0.25 ? 0.114 : 0.0722;
    float kg = 1.0 - kr - kb;
    float3 rgb = float3(
        y + 2.0 * (1.0 - kr) * chroma.y,
        y - 2.0 * kb * (1.0 - kb) / kg * chroma.x
          - 2.0 * kr * (1.0 - kr) / kg * chroma.y,
        y + 2.0 * (1.0 - kb) * chroma.x);
    return half4(clamp(rgb, 0.0, 1.0), 1.0);
}
"#;

pub(super) fn p010_image(
    context: &mut gpu::DirectContext,
    planes: &Image,
    color_space: ColorSpace,
    matrix: YuvMatrix,
    full_range: bool,
) -> Result<Image, SurfaceError> {
    if std::env::var_os("VALLE_TRACE_F16_STAGES").is_some() {
        eprintln!(
            "[valle f16] gpu-p010 {}x{} matrix={matrix:?} full-range={full_range}",
            planes.width(),
            planes.height()
        );
    }
    thread_local! {
        static EFFECT: RefCell<Option<Result<RuntimeEffect, String>>> = const { RefCell::new(None) };
    }
    let effect = EFFECT.with(|effect| {
        effect
            .borrow_mut()
            .get_or_insert_with(|| RuntimeEffect::make_for_shader(SOURCE, None))
            .clone()
            .map_err(|_| SurfaceError::InvalidDecodedGpuFrame)
    })?;
    let parameters: [f32; 4] = [
        if full_range { 0.0 } else { 64.0 },
        if full_range { 1023.0 } else { 876.0 },
        if full_range { 1023.0 } else { 896.0 },
        match matrix {
            YuvMatrix::Bt601 => 0.299,
            YuvMatrix::Bt709 => 0.2126,
        },
    ];
    let bytes: Vec<u8> = parameters.into_iter().flat_map(f32::to_ne_bytes).collect();
    let child = planes
        .to_raw_shader(
            (TileMode::Clamp, TileMode::Clamp),
            SamplingOptions::default(),
            None,
        )
        .ok_or(SurfaceError::InvalidDecodedGpuFrame)?;
    let shader = effect
        .make_shader(Data::new_copy(&bytes), &[ChildPtr::Shader(child)], None)
        .ok_or(SurfaceError::InvalidDecodedGpuFrame)?;
    let info = ImageInfo::new(
        planes.dimensions(),
        ColorType::RGBAF16,
        skia_safe::AlphaType::Opaque,
        color_space,
    );
    // This additional F16 import remains inside the existing 32-byte/pixel visual allowance.
    // Its snapshot and decoded plane textures live through the existing frame-completion fence.
    let mut surface = gpu::surfaces::render_target(
        context,
        gpu::Budgeted::Yes,
        &info,
        None::<usize>,
        Some(gpu::SurfaceOrigin::TopLeft),
        None,
        Some(false),
        Some(false),
    )
    .ok_or(SurfaceError::Allocation)?;
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    paint.set_shader(shader);
    surface.canvas().draw_paint(&paint);
    Ok(surface.image_snapshot())
}
