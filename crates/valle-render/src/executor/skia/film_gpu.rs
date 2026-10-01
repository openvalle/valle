//! Skia/Ganesh F16 film grain with the CPU reference's seeded noise lattice.

use std::cell::RefCell;

use skia_safe::{
    AlphaType, BlendMode, ColorType, Data, FilterMode, Image, ImageInfo, MipmapMode, Paint,
    RuntimeEffect, SamplingOptions, Surface, TileMode, images, runtime_effect::ChildPtr,
};
use valle_engine::compositor::film::{FilmGrainParams, hash_noise_bits};

use super::draw::DrawError;

const FILM_GRAIN: &str = include_str!("../../../../valle-draw/assets/shaders/filmgrain.sksl");

thread_local! {
    static EFFECT: RefCell<Option<RuntimeEffect>> = const { RefCell::new(None) };
}

pub(super) fn apply_film_grain_into(
    target: &mut Surface,
    source: &Image,
    source_size: [i32; 2],
    input_offset: [i32; 2],
    output_origin: [i32; 2],
    params: FilmGrainParams,
) -> Result<(), DrawError> {
    if !params.amount.is_finite()
        || !(0.0..=1.0).contains(&params.amount)
        || !params.size.is_finite()
        || params.size < 1.0
    {
        return Err(DrawError::Unsupported(
            "invalid GPU film grain parameters".into(),
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
            "invalid GPU film grain extent".into(),
        ));
    }
    let (lattice, lattice_origin) =
        make_lattice(dimensions.width, dimensions.height, output_origin, params)?;
    if std::env::var_os("VALLE_TRACE_F16_STAGES").is_some() {
        eprintln!(
            "[valle f16] gpu-film-grain {}x{} cpu-lattice={}x{} (no source readback)",
            dimensions.width,
            dimensions.height,
            lattice.width(),
            lattice.height()
        );
    }
    EFFECT.with(|cell| {
        let mut shader = cell.borrow_mut();
        if shader.is_none() {
            *shader = Some(RuntimeEffect::make_for_shader(FILM_GRAIN, None).map_err(
                |message| DrawError::ShaderCompile {
                    uri: "builtin://gpu-film-grain".into(),
                    message,
                },
            )?);
        }
        draw(
            target,
            source,
            &lattice,
            shader.as_ref().expect("GPU film grain shader admitted"),
            [
                source_size[0],
                source_size[1],
                input_offset[0],
                input_offset[1],
                output_origin[0],
                output_origin[1],
                lattice_origin[0],
                lattice_origin[1],
            ],
            [params.amount, params.size],
        )
    })
}

fn make_lattice(
    width: i32,
    height: i32,
    output_origin: [i32; 2],
    params: FilmGrainParams,
) -> Result<(Image, [i32; 2]), DrawError> {
    let size = f64::from(params.size);
    let first_x = (f64::from(output_origin[0]) / size).floor() as i64;
    let first_y = (f64::from(output_origin[1]) / size).floor() as i64;
    let last_x =
        ((i64::from(output_origin[0]) + i64::from(width) - 1) as f64 / size).floor() as i64 + 1;
    let last_y =
        ((i64::from(output_origin[1]) + i64::from(height) - 1) as f64 / size).floor() as i64 + 1;
    let lattice_width = usize::try_from(last_x - first_x + 1)
        .map_err(|_| DrawError::Unsupported("invalid GPU film grain lattice".into()))?;
    let lattice_height = usize::try_from(last_y - first_y + 1)
        .map_err(|_| DrawError::Unsupported("invalid GPU film grain lattice".into()))?;
    let count = lattice_width
        .checked_mul(lattice_height)
        .filter(|count| *count <= 33_600_000)
        .ok_or_else(|| DrawError::Unsupported("GPU film grain lattice is too large".into()))?;
    let mut bytes = Vec::with_capacity(count * 4);
    for y in first_y..=last_y {
        for x in first_x..=last_x {
            let bits = hash_noise_bits(x, y, params.seed).to_le_bytes();
            bytes.extend_from_slice(&[bits[1], bits[2], bits[3], 255]);
        }
    }
    let info = ImageInfo::new(
        (
            i32::try_from(lattice_width)
                .map_err(|_| DrawError::Unsupported("invalid GPU film grain lattice".into()))?,
            i32::try_from(lattice_height)
                .map_err(|_| DrawError::Unsupported("invalid GPU film grain lattice".into()))?,
        ),
        ColorType::RGBA8888,
        AlphaType::Opaque,
        None,
    );
    let image = images::raster_from_data(&info, Data::new_copy(&bytes), lattice_width * 4)
        .ok_or_else(|| DrawError::Surface("GPU film grain lattice allocation failed".into()))?;
    Ok((
        image,
        [
            i32::try_from(first_x)
                .map_err(|_| DrawError::Unsupported("invalid GPU film grain origin".into()))?,
            i32::try_from(first_y)
                .map_err(|_| DrawError::Unsupported("invalid GPU film grain origin".into()))?,
        ],
    ))
}

fn draw(
    target: &mut Surface,
    source: &Image,
    lattice: &Image,
    effect: &RuntimeEffect,
    integer_uniforms: [i32; 8],
    float_uniforms: [f32; 2],
) -> Result<(), DrawError> {
    let mut bytes = Vec::with_capacity((integer_uniforms.len() + float_uniforms.len()) * 4);
    for value in integer_uniforms {
        bytes.extend_from_slice(&(value as f32).to_ne_bytes());
    }
    for value in float_uniforms {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    if bytes.len() != effect.uniform_size() || effect.children().len() != 2 {
        return Err(DrawError::Internal(
            "GPU film grain shader ABI mismatch".into(),
        ));
    }
    let source_shader = source
        .to_raw_shader(
            (TileMode::Clamp, TileMode::Clamp),
            SamplingOptions::new(FilterMode::Nearest, MipmapMode::None),
            None,
        )
        .ok_or_else(|| DrawError::Unsupported("GPU film grain source shader".into()))?;
    let lattice_shader = lattice
        .to_raw_shader(
            (TileMode::Clamp, TileMode::Clamp),
            SamplingOptions::new(FilterMode::Nearest, MipmapMode::None),
            None,
        )
        .ok_or_else(|| DrawError::Unsupported("GPU film grain lattice shader".into()))?;
    let shader = effect
        .make_shader(
            Data::new_copy(&bytes),
            &[
                ChildPtr::Shader(source_shader),
                ChildPtr::Shader(lattice_shader),
            ],
            None,
        )
        .ok_or_else(|| DrawError::Unsupported("GPU film grain shader instantiation".into()))?;
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    paint.set_shader(shader);
    target.canvas().draw_paint(&paint);
    Ok(())
}
