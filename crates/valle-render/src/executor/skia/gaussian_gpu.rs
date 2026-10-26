//! Stable Gaussian blur with fixed-grid, prefiltered pyramids for wide kernels.
use super::{blur, draw::DrawError, effect::straight_color};
use skia_safe::{
    BlendMode, Data, FilterMode, ImageFilter, Matrix, MipmapMode, RuntimeEffect, SamplingOptions,
    color_filters, image_filters, runtime_effect::RuntimeShaderBuilder,
};
use std::{cell::RefCell, collections::BTreeMap};
use valle_draw::program::{Filter, MAX_FILTER_SIGMA};

const DIRECT_SIGMA: f32 = 16.0;

fn effect(pairs: usize) -> Result<RuntimeEffect, DrawError> {
    fn compile(pairs: usize) -> Result<RuntimeEffect, String> {
        let mut source =
            String::from("uniform shader image; uniform float2 axis; uniform float center;");
        for i in 1..=pairs {
            source.push_str(&format!("uniform float2 tap{i};"));
        }
        source.push_str("half4 main(float2 xy) { float4 sum = float4(image.eval(xy)) * center;");
        for i in 1..=pairs {
            source.push_str(&format!("if (tap{i}.y > 0) {{ float2 delta = axis * tap{i}.x; sum += (float4(image.eval(xy + delta)) + float4(image.eval(xy - delta))) * tap{i}.y; }}"));
        }
        source.push_str("return half4(sum); }");
        RuntimeEffect::make_for_shader(source, None)
    }
    thread_local! {
        static EFFECTS: RefCell<BTreeMap<usize, Result<RuntimeEffect, String>>> = const { RefCell::new(BTreeMap::new()) };
    }
    EFFECTS.with(|effects| {
        effects
            .borrow_mut()
            .entry(pairs)
            .or_insert_with(|| compile(pairs))
            .clone()
            .map_err(|message| DrawError::ShaderCompile {
                uri: "builtin://stable-gaussian".into(),
                message,
            })
    })
}

fn direct_axis(
    sigma: f32,
    horizontal: bool,
    input: Option<ImageFilter>,
) -> Result<Option<ImageFilter>, DrawError> {
    let weights = blur::kernel(sigma)
        .ok_or_else(|| DrawError::Unsupported("Gaussian kernel exceeds direct support".into()))?;
    let radius = weights.len() / 2;
    if radius == 0 {
        return Ok(input);
    }
    let pairs = match radius {
        0..=6 => 3,
        7..=24 => 12,
        25..=48 => 24,
        _ => 48,
    };
    let mut builder = RuntimeShaderBuilder::new(effect(pairs)?);
    let set = |builder: &mut RuntimeShaderBuilder, name: &str, values: &[f32]| {
        builder
            .set_uniform_float(name, values)
            .map_err(|error| DrawError::Unsupported(format!("Gaussian uniform {name}: {error}")))
    };
    set(
        &mut builder,
        "axis",
        if horizontal { &[1.0, 0.0] } else { &[0.0, 1.0] },
    )?;
    set(&mut builder, "center", &[weights[radius]])?;
    for pair in 0..pairs {
        let a = pair * 2 + 1;
        let first = weights.get(radius + a).copied().unwrap_or(0.0);
        let second = weights.get(radius + a + 1).copied().unwrap_or(0.0);
        let weight = first + second;
        let offset = a as f32 + if weight > 0.0 { second / weight } else { 0.0 };
        set(&mut builder, &format!("tap{}", pair + 1), &[offset, weight])?;
    }
    image_filters::runtime_shader_with_options(&builder, radius as f32, "image", input, true)
        .ok_or_else(|| DrawError::Unsupported("stable Gaussian image filter".into()))
        .map(Some)
}

pub(super) fn supports(filter: &Filter) -> bool {
    match filter {
        Filter::Blur { sigma_x, sigma_y }
        | Filter::DropShadow {
            sigma_x, sigma_y, ..
        } => [*sigma_x, *sigma_y]
            .into_iter()
            .all(|sigma| sigma.is_finite() && (0.0..=MAX_FILTER_SIGMA).contains(&sigma)),
        _ => false,
    }
}

fn scaled_axis(
    sigma: f32,
    horizontal: bool,
    scale: f32,
    mut input: Option<ImageFilter>,
    output_scale: f32,
) -> Result<Option<ImageFilter>, DrawError> {
    let matrix = |value| {
        Matrix::scale(if horizontal {
            (value, 1.0)
        } else {
            (1.0, value)
        })
    };
    let sampling = SamplingOptions::new(FilterMode::Linear, MipmapMode::None);
    let mut level = 1.0;
    while level < scale {
        // Filter each fixed 2:1 step before decimation. Besides suppressing aliasing, this
        // prevents Skia from collapsing the pyramid into a single sparse resampling.
        let smoothed = direct_axis(2.0, horizontal, input)?;
        input = Some(
            image_filters::matrix_transform(&matrix(0.5), sampling, smoothed)
                .ok_or_else(|| DrawError::Unsupported("Gaussian downsample".into()))?,
        );
        level *= 2.0;
    }
    // Account for the prefilter and bilinear decimation variance in original pixels.
    let variance = (scale * scale - 1.0) * (4.0 + 0.25) / 3.0;
    let local_sigma = (sigma * sigma - variance).max(0.0).sqrt() / scale;
    let blurred = direct_axis(local_sigma, horizontal, input)?;
    if scale == output_scale {
        return Ok(blurred);
    }
    image_filters::matrix_transform(&matrix(scale / output_scale), sampling, blurred)
        .ok_or_else(|| DrawError::Unsupported("Gaussian upsample".into()))
        .map(Some)
}

fn axis(
    sigma: f32,
    horizontal: bool,
    input: Option<ImageFilter>,
) -> Result<(Option<ImageFilter>, f32), DrawError> {
    if sigma <= DIRECT_SIGMA {
        return Ok((direct_axis(sigma, horizontal, input)?, 1.0));
    }
    // Grid spacing is always a power of two, anchored in filter coordinates. Sigma only
    // changes the kernel and the continuous blend; it never slides a sampling grid.
    let mut scale = 1.0;
    while sigma / scale > 2.0 * DIRECT_SIGMA {
        scale *= 2.0;
    }
    let weight = sigma / (DIRECT_SIGMA * scale) - 1.0;
    // Keep the horizontal result on its finest active grid until the vertical pass finishes.
    // The two separable linear operations commute with horizontal resampling; delaying it
    // avoids processing full-width intermediate images in every vertical pyramid branch.
    let output_scale = if horizontal {
        if weight == 1.0 { 2.0 * scale } else { scale }
    } else {
        1.0
    };
    let coarse = scaled_axis(sigma, horizontal, 2.0 * scale, input.clone(), output_scale)?;
    if weight == 1.0 {
        return Ok((coarse, output_scale));
    }
    let fine = scaled_axis(sigma, horizontal, scale, input, output_scale)?;
    thread_local! {
        static MIX: Result<RuntimeEffect, String> = RuntimeEffect::make_for_blender(
            "uniform float weight; half4 main(half4 src, half4 dst) { return half4(mix(float4(dst), float4(src), weight)); }", None);
    }
    let blender = MIX.with(|effect| {
        effect
            .as_ref()
            .map_err(|message| DrawError::ShaderCompile {
                uri: "builtin://stable-gaussian-mix".into(),
                message: message.clone(),
            })?
            .make_blender(Data::new_copy(&weight.to_ne_bytes()), None)
            .ok_or_else(|| DrawError::Unsupported("Gaussian level blend".into()))
    })?;
    let mixed = image_filters::blend(blender, fine, coarse, None)
        .ok_or_else(|| DrawError::Unsupported("Gaussian level blend filter".into()))
        .map(Some)?;
    Ok((mixed, output_scale))
}

pub(super) fn filter(filter: &Filter) -> Result<Option<ImageFilter>, DrawError> {
    if !supports(filter) {
        return Err(DrawError::Unsupported(
            "invalid stable Gaussian filter".into(),
        ));
    }
    let (sx, sy) = match filter {
        Filter::Blur { sigma_x, sigma_y }
        | Filter::DropShadow {
            sigma_x, sigma_y, ..
        } => (*sigma_x, *sigma_y),
        _ => return Err(DrawError::Unsupported("stable Gaussian filter kind".into())),
    };
    let (horizontal, scale) = axis(sx, true, None)?;
    let (mut blurred, _) = axis(sy, false, horizontal)?;
    if scale != 1.0 {
        blurred = Some(
            image_filters::matrix_transform(
                &Matrix::scale((scale, 1.0)),
                SamplingOptions::new(FilterMode::Linear, MipmapMode::None),
                blurred,
            )
            .ok_or_else(|| DrawError::Unsupported("Gaussian final horizontal upsample".into()))?,
        );
    }
    if let Filter::DropShadow { offset, color, .. } = filter {
        let tint =
            color_filters::blend_with_color_space(straight_color(*color), None, BlendMode::SrcIn)
                .ok_or_else(|| DrawError::Unsupported("shadow tint".into()))?;
        let tinted = image_filters::color_filter(tint, blurred, None);
        let shifted = image_filters::offset((offset[0], offset[1]), tinted, None);
        return image_filters::merge([shifted, None], None)
            .ok_or_else(|| DrawError::Unsupported("shadow merge".into()))
            .map(Some);
    }
    Ok(blurred)
}
