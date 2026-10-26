//! Deferred Scene3D DOF. Color/depth are uploaded once; the result stays on the same Ganesh
//! device as composition. The raw sRGB F16 boundary matches the CPU raster's quantization.

use super::{
    draw::DrawError, scene3d_region::Region, surface::SurfaceFrame, target::SkiaObjectError,
};
use skia_safe::{
    AlphaType, BlendMode, ColorSpace, ColorType, Data, FilterMode, Image, ImageInfo, Matrix,
    MipmapMode, Paint, RuntimeEffect, SamplingOptions, TileMode, images, runtime_effect::ChildPtr,
};
use std::cell::RefCell;
use valle_engine::resource::Extent2d;

const SOURCE: &str = include_str!("../../../../valle-draw/assets/shaders/scene3ddof.sksl");
thread_local! { static EFFECT: RefCell<Option<RuntimeEffect>> = const { RefCell::new(None) }; }

#[derive(Clone)]
pub(crate) struct Scene3dDofInput {
    extent: Extent2d,
    color: Image,
    depth: Image,
    input_region: Region,
    uniforms: [f32; 13],
}

impl Scene3dDofInput {
    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn from_gpu(
        extent: Extent2d,
        color: Image,
        depth: Image,
        dof: valle_engine::motion::scene3d::DepthOfFieldState,
        far: f32,
    ) -> Self {
        let (width, height) = (extent.width(), extent.height());
        Self {
            extent,
            color,
            depth,
            input_region: Region::full(width as usize, height as usize),
            uniforms: [
                width as f32,
                height as f32,
                dof.focus_distance,
                dof.max_blur_radius,
                0.0,
                0.0,
                width as f32,
                height as f32,
                0.0,
                0.0,
                width as f32,
                height as f32,
                far,
            ],
        }
    }
    pub(crate) fn new(
        extent: Extent2d,
        premul_rgba16f: &[u16],
        camera_depth: &[f32],
        focus_distance: f32,
        max_blur_radius: f32,
        far: f32,
    ) -> Result<Self, SkiaObjectError> {
        if !focus_distance.is_finite()
            || focus_distance <= 0.0
            || !max_blur_radius.is_finite()
            || !(0.0..=16.0).contains(&max_blur_radius)
            || !far.is_finite()
            || far <= 0.0
        {
            return Err(SkiaObjectError::InvalidPixelPayload);
        }
        let (width, height) = (extent.width(), extent.height());
        let pixels = (width as usize)
            .checked_mul(height as usize)
            .ok_or(SkiaObjectError::InvalidExtent)?;
        if premul_rgba16f.len()
            != pixels
                .checked_mul(4)
                .ok_or(SkiaObjectError::InvalidExtent)?
            || camera_depth.len() != pixels
        {
            return Err(SkiaObjectError::InvalidPixelPayload);
        }
        let mut visible = None;
        let mut near = None;
        for (y, (depth_row, color_row)) in camera_depth
            .chunks_exact(width as usize)
            .zip(premul_rgba16f.chunks_exact(width as usize * 4))
            .enumerate()
        {
            let (mut first, mut last) = (u32::MAX, 0);
            let (mut near_first, mut near_last) = (u32::MAX, 0);
            let pixels = depth_row.iter().zip(color_row.as_chunks::<4>().0);
            for (x, (&z, color)) in pixels.enumerate() {
                if !z.is_finite() || z <= 0.0 {
                    return Err(SkiaObjectError::InvalidPixelPayload);
                }
                if color[3] != 0 {
                    let x = x as u32;
                    first = first.min(x);
                    last = x;
                    if z != f32::MAX && z < focus_distance {
                        near_first = near_first.min(x);
                        near_last = x;
                    }
                }
            }
            if first != u32::MAX {
                grow(&mut visible, first, y as u32);
                grow(&mut visible, last, y as u32);
            }
            if near_first != u32::MAX {
                grow(&mut near, near_first, y as u32);
                grow(&mut near, near_last, y as u32);
            }
        }
        // Output pixels may be one blur radius outside the visible source, and
        // their gathers reach another radius. Keep both halos and the original
        // layer coordinates, including depth in otherwise transparent pixels.
        let input_region = visible.map_or(Region::full(1, 1), |[x0, y0, x1, y1]| {
            Region {
                x0: x0 as usize,
                y0: y0 as usize,
                x1: x1 as usize + 1,
                y1: y1 as usize + 1,
            }
            .expand(
                2 * max_blur_radius.max(1.0).ceil() as usize,
                width as usize,
                height as usize,
            )
        });
        let info = ImageInfo::new(
            (input_region.width() as i32, input_region.height() as i32),
            ColorType::RGBAF16,
            AlphaType::Premul,
            None,
        );
        let color = images::raster_from_data(
            &info,
            input_region.color_data(premul_rgba16f, width as usize),
            input_region.width() * 8,
        )
        .ok_or(SkiaObjectError::InvalidPixelPayload)?;
        let info = ImageInfo::new(
            (input_region.width() as i32, input_region.height() as i32),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            None,
        );
        // Keep the original f32 bits, including the uncovered sentinel. The shader
        // resolves that sentinel to `far`, so no full-frame byte staging Vec is needed.
        let depth = images::raster_from_data(
            &info,
            input_region.depth_data(camera_depth, width as usize),
            input_region.width() * 4,
        )
        .ok_or(SkiaObjectError::InvalidPixelPayload)?;
        let expand = |bounds: Option<[u32; 4]>| -> [f32; 4] {
            let Some([x0, y0, x1, y1]) = bounds else {
                return [0.0; 4];
            };
            let margin = max_blur_radius.ceil() as u32;
            [
                x0.saturating_sub(margin),
                y0.saturating_sub(margin),
                x1.saturating_add(margin).saturating_add(1).min(width),
                y1.saturating_add(margin).saturating_add(1).min(height),
            ]
            .map(|v| v as f32)
        };
        let [x0, y0, x1, y1] = expand(visible);
        let [nx0, ny0, nx1, ny1] = expand(near);
        Ok(Self {
            extent,
            color,
            depth,
            input_region,
            uniforms: [
                width as f32,
                height as f32,
                focus_distance,
                max_blur_radius,
                x0,
                y0,
                x1,
                y1,
                nx0,
                ny0,
                nx1,
                ny1,
                far,
            ],
        })
    }

    pub(crate) fn resident_bytes(&self) -> u64 {
        self.input_region.pixels() as u64 * 12
    }
}

fn grow(bounds: &mut Option<[u32; 4]>, x: u32, y: u32) {
    *bounds = Some(match *bounds {
        Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        None => [x, y, x, y],
    });
}

pub(super) fn render(
    input: &Scene3dDofInput,
    surfaces: &mut SurfaceFrame<'_>,
) -> Result<Image, DrawError> {
    if surfaces.backend_kind() == super::SkiaBackendKind::Raster {
        return Err(DrawError::Internal(
            "deferred Scene3D DOF requires a GPU surface".into(),
        ));
    }
    let mut scratch = surfaces.scratch(&[input.extent, input.extent])?;
    EFFECT.with(|cell| {
        let mut effect = cell.borrow_mut();
        if effect.is_none() {
            *effect = Some(
                RuntimeEffect::make_for_shader(SOURCE, None).map_err(|message| {
                    DrawError::ShaderCompile {
                        uri: "builtin://scene3d-dof".into(),
                        message,
                    }
                })?,
            );
        }
        let effect = effect.as_ref().unwrap();
        let bytes = input
            .uniforms
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect::<Vec<_>>();
        if effect.uniform_size() != bytes.len() || effect.children().len() != 2 {
            return Err(DrawError::Internal(
                "Scene3D DOF shader ABI mismatch".into(),
            ));
        }
        let origin =
            Matrix::translate((input.input_region.x0 as f32, input.input_region.y0 as f32));
        let children = [&input.color, &input.depth].map(|image| {
            image
                .to_raw_shader(
                    (TileMode::Decal, TileMode::Decal),
                    SamplingOptions::new(FilterMode::Nearest, MipmapMode::None),
                    Some(&origin),
                )
                .map(ChildPtr::Shader)
                .ok_or_else(|| DrawError::Surface("Scene3D DOF input shader".into()))
        });
        let [color, depth] = children;
        let shader = effect
            .make_shader(Data::new_copy(&bytes), &[color?, depth?], None)
            .ok_or_else(|| DrawError::Surface("Scene3D DOF shader instantiation".into()))?;
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::Src);
        paint.set_shader(shader);
        let raw = scratch.surface_mut(0)?;
        raw.canvas().draw_paint(&paint);
        // The raw shader wrote premultiplied sRGB numbers into F16 storage, matching CPU DOF.
        // Retag that snapshot before the usual sRGB -> working Rec.2020 conversion, on GPU.
        let encoded = raw
            .image_snapshot()
            .reinterpret_color_space(ColorSpace::new_srgb())
            .ok_or_else(|| DrawError::Surface("Scene3D DOF output color space".into()))?;
        let output = scratch.surface_mut(1)?;
        paint.set_shader(None);
        output
            .canvas()
            .draw_image(&encoded, (0.0, 0.0), Some(&paint));
        if std::env::var_os("VALLE_TRACE_F16_STAGES").is_some() {
            eprintln!(
                "[valle f16] gpu-scene3d-dof {}x{} input={}x{} (no intermediate readback)",
                input.extent.width(),
                input.extent.height(),
                input.input_region.width(),
                input.input_region.height()
            );
        }
        Ok(output.image_snapshot())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_upload_owns_exact_little_endian_bits_including_clear_depth() {
        let mut depth = [f32::from_bits(1), 1.0, 1234.5, f32::MAX];
        let expected: Vec<_> = depth.iter().flat_map(|v| v.to_le_bytes()).collect();
        let data = Region::full(depth.len(), 1).depth_data(&depth, depth.len());
        depth.fill(0.0);
        assert_eq!(data.as_bytes(), expected);
        assert!(Region::full(0, 1).depth_data(&[], 0).is_empty());
    }

    #[test]
    fn row_bounds_match_per_pixel_reference_and_validate_depth() {
        for (width, height) in [(1, 1), (1, 17), (31, 1), (37, 23)] {
            let extent = Extent2d::new(width, height).unwrap();
            let count = (width * height) as usize;
            for seed in 0..8 {
                let mut colors = vec![0_u16; count * 4];
                let mut depths = vec![f32::MAX; count];
                let mut visible = None;
                let mut near = None;
                for (index, depth) in depths.iter_mut().enumerate() {
                    let tag = (index * 31 + seed * 7) % 13;
                    *depth = [f32::MAX, 0.1, 2.0, 4.0, 6.0][(index + seed) % 5];
                    if tag < 7 {
                        colors[index * 4 + 3] = 0x3c00;
                        let (x, y) = (index as u32 % width, index as u32 / width);
                        grow(&mut visible, x, y);
                        if *depth != f32::MAX && *depth < 4.0 {
                            grow(&mut near, x, y);
                        }
                    }
                }
                let input =
                    Scene3dDofInput::new(extent, &colors, &depths, 4.0, 1.25, 20.0).unwrap();
                let expand = |bounds: Option<[u32; 4]>| {
                    bounds.map_or([0.0; 4], |[x0, y0, x1, y1]| {
                        [
                            x0.saturating_sub(2),
                            y0.saturating_sub(2),
                            x1.saturating_add(3).min(width),
                            y1.saturating_add(3).min(height),
                        ]
                        .map(|v| v as f32)
                    })
                };
                assert_eq!(&input.uniforms[4..8], &expand(visible));
                assert_eq!(&input.uniforms[8..12], &expand(near));
                assert_eq!(input.uniforms[12], 20.0);
            }
        }
        for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(
                Scene3dDofInput::new(
                    Extent2d::new(1, 1).unwrap(),
                    &[0; 4],
                    &[invalid],
                    4.0,
                    1.0,
                    20.0
                )
                .is_err()
            );
        }
    }

    #[test]
    fn depth_gather_shader_compiles_without_gpu_and_matches_the_host_abi() {
        let effect = RuntimeEffect::make_for_shader(SOURCE, None).unwrap();
        assert_eq!(effect.uniform_size(), 13 * std::mem::size_of::<f32>());
        assert_eq!(effect.children().len(), 2);
    }
}
