//! Full-resolution Gaussian passes avoid Skia's sigma-dependent downsampling grid.
use super::{blur, draw::DrawError, effect::straight_color};
use skia_safe::{
    BlendMode, ImageFilter, RuntimeEffect, color_filters, image_filters,
    runtime_effect::RuntimeShaderBuilder,
};
use valle_draw::program::Filter;

const PAIRS: usize = 48;

fn effect() -> Result<RuntimeEffect, DrawError> {
    thread_local! {
        static EFFECT: Result<RuntimeEffect, String> = {
            let mut source = String::from("uniform shader image; uniform float2 axis; uniform float center;");
            for i in 1..=PAIRS { source.push_str(&format!("uniform float2 tap{i};")); }
            source.push_str("half4 main(float2 xy) { float4 sum = float4(image.eval(xy)) * center;");
            for i in 1..=PAIRS {
                source.push_str(&format!("if (tap{i}.y > 0) {{ float2 delta = axis * tap{i}.x; sum += (float4(image.eval(xy + delta)) + float4(image.eval(xy - delta))) * tap{i}.y; }}"));
            }
            source.push_str("return half4(sum); }");
            RuntimeEffect::make_for_shader(source, None)
        };
    }
    EFFECT.with(|effect| {
        effect.clone().map_err(|message| DrawError::ShaderCompile {
            uri: "builtin://stable-gaussian".into(),
            message,
        })
    })
}

fn axis(
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
    let mut builder = RuntimeShaderBuilder::new(effect()?);
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
    for pair in 0..PAIRS {
        let a = pair * 2 + 1;
        let first = weights.get(radius + a).copied().unwrap_or(0.0);
        let second = weights.get(radius + a + 1).copied().unwrap_or(0.0);
        let weight = first + second;
        let offset = a as f32 + if weight > 0.0 { second / weight } else { 0.0 };
        set(&mut builder, &format!("tap{}", pair + 1), &[offset, weight])?;
    }
    image_filters::runtime_shader_with_options(&builder, radius as f32, "image", input, false)
        .ok_or_else(|| DrawError::Unsupported("stable Gaussian image filter".into()))
        .map(Some)
}

pub(super) fn supports(filter: &Filter) -> bool {
    blur::supports(filter)
}

pub(super) fn filter(filter: &Filter) -> Result<Option<ImageFilter>, DrawError> {
    let (sx, sy) = match filter {
        Filter::Blur { sigma_x, sigma_y }
        | Filter::DropShadow {
            sigma_x, sigma_y, ..
        } => (*sigma_x, *sigma_y),
        _ => return Err(DrawError::Unsupported("stable Gaussian filter kind".into())),
    };
    let blurred = axis(sy, false, axis(sx, true, None)?)?;
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
