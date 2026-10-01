//! Skia/Ganesh F16 bloom and glow pyramids. Each stage writes a half-float surface so the
//! CPU reference and GPU path have the same quantization boundaries.

use std::cell::RefCell;

use skia_safe::{
    BlendMode, Color4f, Data, FilterMode, IRect, Image, MipmapMode, Paint, Rect, RuntimeEffect,
    SamplingOptions, Surface, TileMode, runtime_effect::ChildPtr,
};
use valle_engine::compositor::bloom::{
    BloomParams, GlowParams, PyramidGraph, PyramidPassKind, PyramidRegion,
};

use super::draw::DrawError;

const THRESHOLD: &str = r#"
uniform shader source;
uniform float threshold;
uniform float knee;
half4 main(float2 xy) {
    float4 pixel = float4(source.eval(xy));
    if (pixel.a <= 0.0) return half4(0.0, 0.0, 0.0, 1.0);
    float luminance = dot(pixel.rgb, float3(0.2627, 0.6780, 0.0593)) / pixel.a;
    float weight;
    if (knee == 0.0) {
        weight = luminance >= threshold ? 1.0 : 0.0;
    } else {
        float t = clamp((luminance - threshold + knee) / (2.0 * knee), 0.0, 1.0);
        weight = t * t * (3.0 - 2.0 * t);
    }
    return half4(pixel.rgb * weight, 1.0);
}
"#;

const COVERAGE: &str = r#"
uniform shader source;
half4 main(float2 xy) {
    float alpha = float(source.eval(xy).a);
    return half4(alpha, alpha, alpha, 1.0);
}
"#;

const DOWNSAMPLE: &str = r#"
uniform shader source;
uniform float2 sourceSize;
float3 at(float2 xy) {
    if (any(lessThan(xy, float2(0.0))) || any(greaterThanEqual(xy, sourceSize))) return float3(0.0);
    return float3(source.eval(xy + 0.5).rgb);
}
half4 main(float2 xy) {
    float2 center = floor(xy) * 2.0 + 1.0;
    float3 sum = at(center) * 4.0;
    sum += at(center + float2(-1.0,  0.0)) * 2.0;
    sum += at(center + float2( 1.0,  0.0)) * 2.0;
    sum += at(center + float2( 0.0, -1.0)) * 2.0;
    sum += at(center + float2( 0.0,  1.0)) * 2.0;
    sum += at(center + float2(-1.0, -1.0));
    sum += at(center + float2( 1.0, -1.0));
    sum += at(center + float2(-1.0,  1.0));
    sum += at(center + float2( 1.0,  1.0));
    sum += at(center + float2(-2.0,  0.0));
    sum += at(center + float2( 2.0,  0.0));
    sum += at(center + float2( 0.0, -2.0));
    sum += at(center + float2( 0.0,  2.0));
    return half4(sum / 20.0, 1.0);
}
"#;

const BLUR: &str = r#"
uniform shader source;
uniform float2 sourceSize;
uniform float spread;
float rounded(float value) {
    return sign(value) * floor(abs(value) + 0.5);
}
float3 at(float2 xy) {
    float2 sample = float2(rounded(xy.x), rounded(xy.y));
    if (any(lessThan(sample, float2(0.0))) || any(greaterThanEqual(sample, sourceSize))) return float3(0.0);
    return float3(source.eval(sample + 0.5).rgb);
}
half4 main(float2 xy) {
    float2 center = floor(xy);
    float3 sum = at(center + float2(-spread, -spread));
    sum += at(center + float2(0.0, -spread)) * 2.0;
    sum += at(center + float2(spread, -spread));
    sum += at(center + float2(-spread, 0.0)) * 2.0;
    sum += at(center) * 4.0;
    sum += at(center + float2(spread, 0.0)) * 2.0;
    sum += at(center + float2(-spread, spread));
    sum += at(center + float2(0.0, spread)) * 2.0;
    sum += at(center + float2(spread, spread));
    return half4(sum / 16.0, 1.0);
}
"#;

const UPSAMPLE: &str = r#"
uniform shader coarse;
uniform shader fine;
uniform float2 coarseSize;
uniform float2 outputSize;
uniform float spread;
uniform float localWeight;
float3 at(float2 xy) {
    if (any(lessThan(xy, float2(0.0))) || any(greaterThanEqual(xy, coarseSize))) return float3(0.0);
    return float3(coarse.eval(xy + 0.5).rgb);
}
float3 bilinear(float2 xy) {
    float2 base = floor(xy);
    float2 fraction = xy - base;
    float3 top = at(base) * (1.0 - fraction.x) + at(base + float2(1.0, 0.0)) * fraction.x;
    float3 bottom = at(base + float2(0.0, 1.0)) * (1.0 - fraction.x)
        + at(base + float2(1.0, 1.0)) * fraction.x;
    return top * (1.0 - fraction.y) + bottom * fraction.y;
}
half4 main(float2 xy) {
    float2 pixel = floor(xy);
    float2 center = (pixel + 0.5) * coarseSize / outputSize - 0.5;
    float tap = 0.5 * spread;
    float3 sum = bilinear(center + float2(-tap, -tap));
    sum += bilinear(center + float2(0.0, -tap)) * 2.0;
    sum += bilinear(center + float2(tap, -tap));
    sum += bilinear(center + float2(-tap, 0.0)) * 2.0;
    sum += bilinear(center) * 4.0;
    sum += bilinear(center + float2(tap, 0.0)) * 2.0;
    sum += bilinear(center + float2(-tap, tap));
    sum += bilinear(center + float2(0.0, tap)) * 2.0;
    sum += bilinear(center + float2(tap, tap));
    float3 far = sum / 16.0;
    float3 local = localWeight > 0.0 ? float3(fine.eval(xy).rgb) : float3(0.0);
    return half4(localWeight * local + (1.0 - localWeight) * far, 1.0);
}
"#;

const COMPOSITE: &str = r#"
uniform shader base;
uniform shader halo;
uniform float intensity;
half4 main(float2 xy) {
    float4 source = float4(base.eval(xy));
    float3 bloom = float3(halo.eval(xy).rgb);
    float3 color = clamp(source.rgb + intensity * bloom, -65504.0, 65504.0);
    float alpha = max(source.a, min(max(max(bloom.r, bloom.g), bloom.b), 1.0));
    return half4(color, alpha);
}
"#;

const GLOW_COMPOSITE: &str = r#"
uniform shader base;
uniform shader halo;
uniform float4 color;
uniform float intensity;
half4 main(float2 xy) {
    float4 source = float4(base.eval(xy));
    float coverage = float(halo.eval(xy).r);
    float gain = coverage * intensity * 12.0;
    float peak = max(max(color.r, color.g), color.b);
    float colorGain = peak > 0.0 ? min(gain, 1.0 / peak) : gain;
    float haloAlpha = min(gain * color.a, 1.0);
    float3 rgb = clamp(source.rgb + colorGain * color.rgb * (1.0 - source.a), -65504.0, 65504.0);
    float alpha = source.a + haloAlpha * (1.0 - source.a);
    return half4(rgb, alpha);
}
"#;

struct Effects {
    threshold: RuntimeEffect,
    coverage: RuntimeEffect,
    downsample: RuntimeEffect,
    blur: RuntimeEffect,
    upsample: RuntimeEffect,
    composite: RuntimeEffect,
    glow_composite: RuntimeEffect,
}

impl Effects {
    fn compile() -> Result<Self, DrawError> {
        let compile = |name, source| {
            RuntimeEffect::make_for_shader(source, None).map_err(|message| {
                DrawError::ShaderCompile {
                    uri: format!("builtin://gpu-pyramid/{name}"),
                    message,
                }
            })
        };
        Ok(Self {
            threshold: compile("threshold", THRESHOLD)?,
            coverage: compile("coverage", COVERAGE)?,
            downsample: compile("downsample", DOWNSAMPLE)?,
            blur: compile("blur", BLUR)?,
            upsample: compile("upsample", UPSAMPLE)?,
            composite: compile("composite", COMPOSITE)?,
            glow_composite: compile("glow-composite", GLOW_COMPOSITE)?,
        })
    }
}

thread_local! {
    static EFFECTS: RefCell<Option<Effects>> = const { RefCell::new(None) };
}

pub(super) fn apply_bloom_into(
    target: &mut Surface,
    source: &Image,
    source_roi: IRect,
    offset: [i32; 2],
    params: BloomParams,
) -> Result<(), DrawError> {
    if !params.threshold.is_finite()
        || !(0.0..=1.0).contains(&params.threshold)
        || !params.knee.is_finite()
        || !(0.0..=1.0).contains(&params.knee)
        || !params.intensity.is_finite()
        || !(0.0..=4.0).contains(&params.intensity)
        || !params.radius.is_finite()
        || params.radius < 0.0
    {
        return Err(DrawError::Unsupported(
            "invalid GPU bloom parameters".into(),
        ));
    }
    let dimensions = target.image_info().dimensions();
    let (width, height) = (dimensions.width, dimensions.height);
    if width <= 0 || height <= 0 || i64::from(width) * i64::from(height) > 16_777_216 {
        return Err(DrawError::Unsupported("invalid GPU bloom extent".into()));
    }
    if std::env::var_os("VALLE_TRACE_F16_STAGES").is_some() {
        eprintln!("[valle f16] gpu-bloom {width}x{height} (no intermediate readback)");
    }
    EFFECTS.with(|cell| {
        let mut shaders = cell.borrow_mut();
        if shaders.is_none() {
            *shaders = Some(Effects::compile()?);
        }
        render_region(
            target,
            source_roi,
            offset,
            params.radius,
            params.radius,
            |target, offset| {
                render_bloom(
                    target,
                    source,
                    source_roi,
                    offset,
                    params,
                    shaders.as_ref().expect("GPU pyramid shaders admitted"),
                )
            },
        )
    })
}

pub(super) fn apply_glow_into(
    target: &mut Surface,
    source: &Image,
    source_roi: IRect,
    offset: [i32; 2],
    params: GlowParams,
) -> Result<(), DrawError> {
    if !params.intensity.is_finite()
        || !(0.0..=4.0).contains(&params.intensity)
        || !params.radius.is_finite()
        || params.radius < 0.0
        || params.color.iter().any(|channel| !channel.is_finite())
        || !(0.0..=1.0).contains(&params.color[3])
    {
        return Err(DrawError::Unsupported("invalid GPU glow parameters".into()));
    }
    let dimensions = target.image_info().dimensions();
    let (width, height) = (dimensions.width, dimensions.height);
    if width <= 0 || height <= 0 || i64::from(width) * i64::from(height) > 16_777_216 {
        return Err(DrawError::Unsupported("invalid GPU glow extent".into()));
    }
    if std::env::var_os("VALLE_TRACE_F16_STAGES").is_some() {
        eprintln!("[valle f16] gpu-glow {width}x{height} (no intermediate readback)");
    }
    EFFECTS.with(|cell| {
        let mut shaders = cell.borrow_mut();
        if shaders.is_none() {
            *shaders = Some(Effects::compile()?);
        }
        render_region(
            target,
            source_roi,
            offset,
            params.radius,
            params.radius * 2.0,
            |target, offset| {
                render_glow(
                    target,
                    source,
                    source_roi,
                    offset,
                    params,
                    shaders.as_ref().expect("GPU pyramid shaders admitted"),
                )
            },
        )
    })
}

fn render_region(
    target: &mut Surface,
    source_roi: IRect,
    offset: [i32; 2],
    radius: f32,
    spread_radius: f32,
    render: impl FnOnce(&mut Surface, [i32; 2]) -> Result<(), DrawError>,
) -> Result<(), DrawError> {
    let info = target.image_info();
    let region = PyramidRegion::new(
        [source_roi.width() as u32, source_roi.height() as u32],
        [info.width() as u32, info.height() as u32],
        offset,
        radius,
        spread_radius,
    )
    .map_err(|error| DrawError::Surface(error.to_string()))?;
    if region.origin == [0, 0] && region.size == [info.width() as u32, info.height() as u32] {
        return render(target, offset);
    }
    let mut working = target
        .new_surface(&info.with_dimensions((region.size[0] as i32, region.size[1] as i32)))
        .ok_or_else(|| DrawError::Surface("GPU pyramid working region allocation failed".into()))?;
    render(&mut working, region.input_offset)?;
    target.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    target.canvas().draw_image(
        &working.image_snapshot(),
        (region.origin[0] as f32, region.origin[1] as f32),
        Some(&paint),
    );
    Ok(())
}

fn render_bloom(
    target: &mut Surface,
    source: &Image,
    source_roi: IRect,
    offset: [i32; 2],
    params: BloomParams,
    effects: &Effects,
) -> Result<(), DrawError> {
    let info = target.image_info();
    let (width, height) = (info.width(), info.height());
    let base = prepare_base(target, source, source_roi, offset)?;
    if params.radius == 0.0 || params.intensity == 0.0 {
        copy_base(target, &base);
        return Ok(());
    }
    let graph = PyramidGraph::new(width as u32, height as u32, params.radius, params.radius)
        .map_err(|error| DrawError::Unsupported(error.to_string()))?;
    let bright = pass(
        target,
        &effects.threshold,
        &[&base],
        &[params.threshold, params.knee],
        width,
        height,
    )?;
    let halo = render_pyramid(target, bright, &graph, effects)?;
    draw(
        target,
        &effects.composite,
        &[&base, &halo],
        &[params.intensity],
    )
}

fn render_glow(
    target: &mut Surface,
    source: &Image,
    source_roi: IRect,
    offset: [i32; 2],
    params: GlowParams,
    effects: &Effects,
) -> Result<(), DrawError> {
    let info = target.image_info();
    let (width, height) = (info.width(), info.height());
    let base = prepare_base(target, source, source_roi, offset)?;
    if params.radius == 0.0 || params.intensity == 0.0 || params.color[3] == 0.0 {
        copy_base(target, &base);
        return Ok(());
    }
    let graph = PyramidGraph::new(
        width as u32,
        height as u32,
        params.radius,
        params.radius * 2.0,
    )
    .map_err(|error| DrawError::Unsupported(error.to_string()))?;
    let coverage = pass(target, &effects.coverage, &[&base], &[], width, height)?;
    let halo = render_pyramid(target, coverage, &graph, effects)?;
    draw(
        target,
        &effects.glow_composite,
        &[&base, &halo],
        &[
            params.color[0],
            params.color[1],
            params.color[2],
            params.color[3],
            params.intensity,
        ],
    )
}

fn prepare_base(
    target: &mut Surface,
    source: &Image,
    source_roi: IRect,
    offset: [i32; 2],
) -> Result<Image, DrawError> {
    let info = target.image_info();
    Ok(
        if offset == [0, 0]
            && source_roi == IRect::from_size(info.dimensions())
            && source.dimensions() == info.dimensions()
        {
            source.clone()
        } else {
            let mut base = target
                .new_surface(&info)
                .ok_or_else(|| DrawError::Surface("GPU pyramid base allocation failed".into()))?;
            base.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
            let mut paint = Paint::default();
            paint.set_blend_mode(BlendMode::Src);
            base.canvas().draw_image_rect_with_sampling_options(
                source,
                Some((
                    &source_roi.into(),
                    skia_safe::canvas::SrcRectConstraint::Strict,
                )),
                Rect::from_xywh(
                    offset[0] as f32,
                    offset[1] as f32,
                    source_roi.width() as f32,
                    source_roi.height() as f32,
                ),
                SamplingOptions::default(),
                &paint,
            );
            base.image_snapshot()
        },
    )
}

fn copy_base(target: &mut Surface, base: &Image) {
    target.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    target.canvas().draw_image(base, (0.0, 0.0), Some(&paint));
}

fn render_pyramid(
    target: &mut Surface,
    initial: Image,
    graph: &PyramidGraph,
    effects: &Effects,
) -> Result<Image, DrawError> {
    let mut images = vec![None; graph.resource_count()];
    images[0] = Some(initial);
    for stage in graph.passes() {
        let image = match stage.kind() {
            PyramidPassKind::Downsample { input } => {
                let source = pyramid_image(&images, input)?;
                pass(
                    target,
                    &effects.downsample,
                    &[source],
                    &[source.width() as f32, source.height() as f32],
                    stage.width() as i32,
                    stage.height() as i32,
                )?
            }
            PyramidPassKind::Blur { input } => {
                let source = pyramid_image(&images, input)?;
                pass(
                    target,
                    &effects.blur,
                    &[source],
                    &[
                        source.width() as f32,
                        source.height() as f32,
                        graph.spread() as f32,
                    ],
                    stage.width() as i32,
                    stage.height() as i32,
                )?
            }
            PyramidPassKind::Upsample {
                coarse,
                fine,
                local_weight,
            } => {
                let coarse = pyramid_image(&images, coarse)?;
                let fine = fine
                    .map(|id| pyramid_image(&images, id))
                    .transpose()?
                    .unwrap_or(coarse);
                pass(
                    target,
                    &effects.upsample,
                    &[coarse, fine],
                    &[
                        coarse.width() as f32,
                        coarse.height() as f32,
                        stage.width() as f32,
                        stage.height() as f32,
                        graph.spread() as f32,
                        local_weight as f32,
                    ],
                    stage.width() as i32,
                    stage.height() as i32,
                )?
            }
        };
        images[stage.output()] = Some(image);
        for &resource in stage.retire_after() {
            images[resource] = None;
        }
    }
    images[graph.output()]
        .take()
        .ok_or_else(|| DrawError::Internal("GPU pyramid halo is missing".into()))
}

fn pyramid_image(images: &[Option<Image>], id: usize) -> Result<&Image, DrawError> {
    images
        .get(id)
        .and_then(Option::as_ref)
        .ok_or_else(|| DrawError::Internal(format!("GPU pyramid resource {id} is missing")))
}

fn pass(
    template: &mut Surface,
    effect: &RuntimeEffect,
    inputs: &[&Image],
    uniforms: &[f32],
    width: i32,
    height: i32,
) -> Result<Image, DrawError> {
    let info = template.image_info().with_dimensions((width, height));
    let mut surface = template
        .new_surface(&info)
        .ok_or_else(|| DrawError::Surface("GPU pyramid allocation failed".into()))?;
    draw(&mut surface, effect, inputs, uniforms)?;
    let image = surface.image_snapshot();
    debug_assert!(image.is_texture_backed(), "GPU pyramid stage left Metal");
    Ok(image)
}

fn draw(
    target: &mut Surface,
    effect: &RuntimeEffect,
    inputs: &[&Image],
    uniforms: &[f32],
) -> Result<(), DrawError> {
    let mut bytes = Vec::with_capacity(uniforms.len() * 4);
    for value in uniforms {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    if bytes.len() != effect.uniform_size() || inputs.len() != effect.children().len() {
        return Err(DrawError::Internal(
            "GPU pyramid shader ABI mismatch".into(),
        ));
    }
    let children = inputs
        .iter()
        .map(|image| {
            image
                .to_raw_shader(
                    (TileMode::Clamp, TileMode::Clamp),
                    SamplingOptions::new(FilterMode::Nearest, MipmapMode::None),
                    None,
                )
                .map(ChildPtr::Shader)
                .ok_or_else(|| DrawError::Unsupported("GPU pyramid source shader".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let shader = effect
        .make_shader(Data::new_copy(&bytes), &children, None)
        .ok_or_else(|| DrawError::Unsupported("GPU pyramid shader instantiation".into()))?;
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    paint.set_shader(shader);
    target.canvas().draw_paint(&paint);
    Ok(())
}
