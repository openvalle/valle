//! Production Motion Glass binding for Skia.
//!
//! Geometry and material policy stay in `valle-engine`; this module only admits the canonical
//! SkSL, binds one ROI-local working image, and executes the packed uniform ABI.

use skia_safe::{
    BlendMode, Color4f, Data, FilterMode, Image, Matrix, MipmapMode, Paint, Rect as SkRect,
    RuntimeEffect, SamplingOptions, Surface, TileMode, runtime_effect::ChildPtr,
};
use valle_engine::prepare::DeviceRect;

use super::draw::DrawError;

pub(super) fn admit_motion_glass_shader() -> Result<RuntimeEffect, DrawError> {
    const URI: &str = "builtin://motion-glass";
    let effect =
        RuntimeEffect::make_for_shader(valle_draw::program::glass::MOTION_GLASS_SKSL, None)
            .map_err(|message| DrawError::ShaderCompile {
                uri: URI.into(),
                message,
            })?;
    if effect.uniform_size()
        != valle_engine::compositor::glass::MOTION_GLASS_GPU_UNIFORM_FLOATS
            * core::mem::size_of::<f32>()
        || effect.children().len() != 1
        || effect.children()[0].name() != "content"
    {
        return Err(DrawError::ShaderAbi { uri: URI.into() });
    }
    Ok(effect)
}

pub(super) fn render_motion_glass_into(
    target: &mut Surface,
    effect: &RuntimeEffect,
    input: &Image,
    input_roi: DeviceRect,
    program: &valle_draw::program::MotionGlassProgram,
    owner_to_device: [f64; 9],
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    let uniforms =
        valle_engine::compositor::glass::pack_motion_glass_gpu_uniforms(program, owner_to_device)
            .map_err(|error| DrawError::Unsupported(error.to_string()))?;
    render_motion_glass_shader_into(target, effect, input, input_roi, &uniforms, target_origin)
}

pub(super) fn render_motion_glass_foreground_into(
    target: &mut Surface,
    effect: &RuntimeEffect,
    input: &Image,
    input_roi: DeviceRect,
    program: &valle_draw::program::MotionGlassForegroundProgram,
    owner_to_device: [f64; 9],
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    let uniforms = valle_engine::compositor::glass::pack_motion_glass_foreground_gpu_uniforms(
        program,
        owner_to_device,
    )
    .map_err(|error| DrawError::Unsupported(error.to_string()))?;
    render_motion_glass_shader_into(target, effect, input, input_roi, &uniforms, target_origin)
}

fn render_motion_glass_shader_into(
    target: &mut Surface,
    effect: &RuntimeEffect,
    input: &Image,
    input_roi: DeviceRect,
    uniforms: &[f32],
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    if input_roi.is_empty() {
        return Err(DrawError::Internal(
            "Motion Glass input ROI is empty".into(),
        ));
    }
    if uniforms.len() != valle_engine::compositor::glass::MOTION_GLASS_GPU_UNIFORM_FLOATS {
        return Err(DrawError::Internal(
            "Motion Glass uniform float count drifted".into(),
        ));
    }
    let mut bytes = [0_u8;
        valle_engine::compositor::glass::MOTION_GLASS_GPU_UNIFORM_FLOATS
            * core::mem::size_of::<f32>()];
    for (target, value) in bytes
        .chunks_exact_mut(core::mem::size_of::<f32>())
        .zip(uniforms)
    {
        // RuntimeEffect consumes host-native scalar memory, not a serialized little-endian wire.
        target.copy_from_slice(&value.to_ne_bytes());
    }
    if bytes.len() != effect.uniform_size() {
        return Err(DrawError::Internal(
            "Motion Glass RuntimeEffect uniform size drifted".into(),
        ));
    }
    let child_matrix = Matrix::translate((input_roi.x as f32, input_roi.y as f32));
    let child = input
        .to_shader(
            (TileMode::Clamp, TileMode::Clamp),
            SamplingOptions::new(FilterMode::Linear, MipmapMode::None),
            Some(&child_matrix),
        )
        .ok_or_else(|| DrawError::Unsupported("Motion Glass input shader".into()))?;
    let shader = effect
        .make_shader(Data::new_copy(&bytes), &[ChildPtr::Shader(child)], None)
        .ok_or_else(|| DrawError::Unsupported("Motion Glass RuntimeEffect instantiation".into()))?;
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    paint.set_shader(shader);
    target.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    let bounds = SkRect::from_xywh(
        target_origin[0] as f32,
        target_origin[1] as f32,
        target.width() as f32,
        target.height() as f32,
    );
    let canvas = target.canvas();
    canvas.save();
    canvas.translate((-target_origin[0] as f32, -target_origin[1] as f32));
    canvas.draw_rect(bounds, &paint);
    canvas.restore();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skia_safe::image::CachingHint;
    use skia_safe::{AlphaType, ColorType, IRect, ImageInfo};
    use valle_draw::Rect;
    use valle_draw::program::BackdropScope;
    use valle_draw::program::glass::{
        BackdropUse, GlassOwnerKind, MOTION_GLASS_KERNEL_ID, MotionGlassForegroundProgram,
        MotionGlassProgram, PackedGlassField, PackedGlassForegroundTone, PackedGlassMaterial,
        PackedGlassMotion, PackedGlassShapeKind, PackedGlassSurface, motion_glass_kernel_digest,
        schema_digest,
    };
    use valle_engine::compositor::glass::{
        apply_glass_foreground_transformed, render_field_contribution_transformed,
        render_glass_contribution_transformed, resolve_motion_glass_backdrop_bounds,
    };
    use valle_engine::compositor::reference::{PremulRgba32, ReferenceImage};
    use valle_engine::resource::Extent2d;

    use super::super::surface::{raster_surface, working_color_space, working_info};

    fn material() -> PackedGlassMaterial {
        PackedGlassMaterial {
            bevel_width: 9.0,
            thickness: 18.0,
            refractive_index: 1.22,
            roughness: 0.12,
            dispersion: 0.015,
            tint_linear: [0.18, 0.42, 0.78, 0.24],
            specular_strength: 0.55,
            shadow_strength: 0.12,
            foreground_gain: 0.8,
            light: valle_draw::program::PackedGlassLight {
                direction: [0.6, -0.8],
                elevation: 0.4,
                intensity: 0.9,
            },
        }
    }

    fn sample_program() -> MotionGlassProgram {
        MotionGlassProgram {
            owner_kind: GlassOwnerKind::Independent,
            owner_id: "hero-lens".into(),
            surfaces: vec![PackedGlassSurface {
                surface_id: "hero-lens".into(),
                shape: PackedGlassShapeKind::Circle,
                rect: Rect::new(32.0, 32.0, 64.0, 64.0),
                local_to_owner: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                radius: 32.0,
                presence: 1.0,
                response: PackedGlassMotion::ZERO,
                path_points: vec![],
                foreground_tone: PackedGlassForegroundTone::None,
                foreground_protection: 0.0,
                foreground_bounds: None,
                foreground_luma: None,
            }],
            field: None,
            material: material(),
            backdrop: BackdropUse {
                scope: BackdropScope::Current,
                // Non-origin, asymmetric bounds prove that the production child matrix maps the
                // ROI-local image back into global device coordinates.
                sample_bounds: Rect::new(11.0, 7.0, 106.0, 104.0),
                output_bounds: Rect::new(32.0, 32.0, 64.0, 64.0),
            },
            kernel_id: MOTION_GLASS_KERNEL_ID.into(),
            kernel_digest: motion_glass_kernel_digest(),
            schema_digest: schema_digest(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn field_surface(
        id: &str,
        rect: Rect,
        response: PackedGlassMotion,
        tone: PackedGlassForegroundTone,
        protection: f32,
        foreground_bounds: Rect,
        foreground_luma: Option<f32>,
    ) -> PackedGlassSurface {
        PackedGlassSurface {
            surface_id: id.into(),
            shape: PackedGlassShapeKind::Circle,
            rect,
            local_to_owner: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            radius: rect.width as f32 * 0.5,
            presence: 0.92,
            response,
            path_points: vec![],
            foreground_tone: tone,
            foreground_protection: protection,
            foreground_bounds: Some(foreground_bounds),
            foreground_luma,
        }
    }

    fn field_program() -> MotionGlassProgram {
        MotionGlassProgram {
            owner_kind: GlassOwnerKind::Field,
            owner_id: "pair".into(),
            surfaces: vec![
                field_surface(
                    "left",
                    Rect::new(24.0, 30.0, 40.0, 40.0),
                    PackedGlassMotion {
                        translation: [18.0, -7.0],
                        acceleration: [120.0, -80.0],
                        angular: 0.2,
                        scale: [0.1, -0.05],
                        shear: 0.08,
                        area: 0.12,
                        pressure: 0.18,
                        twist: -0.1,
                    },
                    PackedGlassForegroundTone::Light,
                    0.65,
                    Rect::new(35.0, 46.0, 17.0, 9.0),
                    None,
                ),
                field_surface(
                    "right",
                    Rect::new(66.0, 34.0, 40.0, 40.0),
                    PackedGlassMotion {
                        translation: [-11.0, 9.0],
                        acceleration: [-45.0, 70.0],
                        angular: -0.14,
                        scale: [-0.04, 0.09],
                        shear: -0.05,
                        area: -0.08,
                        pressure: 0.11,
                        twist: 0.16,
                    },
                    PackedGlassForegroundTone::Auto,
                    0.5,
                    Rect::new(77.0, 49.0, 17.0, 10.0),
                    Some(0.25),
                ),
            ],
            field: Some(PackedGlassField {
                field_id: "pair".into(),
                merge_distance: 18.0,
                member_ids: vec!["left".into(), "right".into()],
            }),
            material: material(),
            backdrop: BackdropUse {
                scope: BackdropScope::ScopeEntry("pair".into()),
                sample_bounds: Rect::new(1.0, 2.0, 128.0, 116.0),
                output_bounds: Rect::new(10.0, 10.0, 110.0, 95.0),
            },
            kernel_id: MOTION_GLASS_KERNEL_ID.into(),
            kernel_digest: motion_glass_kernel_digest(),
            schema_digest: schema_digest(),
        }
    }

    fn path_program() -> MotionGlassProgram {
        let mut program = sample_program();
        program.owner_id = "path-lens".into();
        program.surfaces[0] = PackedGlassSurface {
            surface_id: "path-lens".into(),
            shape: PackedGlassShapeKind::Path,
            rect: Rect::new(24.0, 16.0, 70.0, 74.0),
            local_to_owner: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            radius: 0.0,
            presence: 0.84,
            response: PackedGlassMotion {
                translation: [13.0, -5.0],
                acceleration: [90.0, -55.0],
                angular: 0.16,
                scale: [0.08, -0.04],
                shear: 0.06,
                area: 0.10,
                pressure: 0.14,
                twist: -0.08,
            },
            path_points: vec![
                [34.0, 20.0],
                [79.0, 17.0],
                [92.0, 52.0],
                [72.0, 86.0],
                [29.0, 72.0],
            ],
            foreground_tone: PackedGlassForegroundTone::None,
            foreground_protection: 0.0,
            foreground_bounds: None,
            foreground_luma: None,
        };
        program.backdrop.output_bounds = program.surfaces[0].rect;
        program
    }

    fn draw_checker(surface: &mut Surface, extent: Extent2d) {
        let space = working_color_space().unwrap();
        let cell_width = 13.0f32;
        let cell_height = 11.0f32;
        let mut y = 0.0f32;
        let mut row = 0;
        while y < extent.height() as f32 {
            let mut x = 0.0f32;
            let mut column = 0;
            while x < extent.width() as f32 {
                let checker = if (row + column) % 2 == 0 { 0.04 } else { 0.26 };
                let value = checker + row as f32 * 0.011 + column as f32 * 0.007;
                let mut paint = Paint::default();
                paint.set_blend_mode(BlendMode::Src);
                paint.set_color4f(
                    Color4f::new(value, value * 0.82 + 0.03, value * 1.15, 1.0),
                    &space,
                );
                surface
                    .canvas()
                    .draw_rect(SkRect::from_xywh(x, y, cell_width, cell_height), &paint);
                x += cell_width;
                column += 1;
            }
            y += cell_height;
            row += 1;
        }
    }

    fn read_image(image: &Image, extent: Extent2d) -> ReferenceImage {
        let info = ImageInfo::new(
            (extent.width() as i32, extent.height() as i32),
            ColorType::RGBAF32,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let mut channels = vec![0.0f32; (extent.width() * extent.height() * 4) as usize];
        assert!(image.read_pixels(
            &info,
            &mut channels,
            (extent.width() * 16) as usize,
            (0, 0),
            CachingHint::Disallow,
        ));
        let pixels = channels
            .chunks_exact(4)
            .map(|chunk| {
                PremulRgba32::from_premultiplied([chunk[0], chunk[1], chunk[2], chunk[3]]).unwrap()
            })
            .collect::<Vec<_>>();
        ReferenceImage::new(extent, pixels).unwrap()
    }

    fn project(matrix: [f64; 9], point: [f64; 2]) -> [f64; 2] {
        let denominator = matrix[6] * point[0] + matrix[7] * point[1] + matrix[8];
        [
            (matrix[0] * point[0] + matrix[1] * point[1] + matrix[2]) / denominator,
            (matrix[3] * point[0] + matrix[4] * point[1] + matrix[5]) / denominator,
        ]
    }

    fn projected_roi(rect: Rect, matrix: [f64; 9], extent: Extent2d) -> DeviceRect {
        let points = [
            project(matrix, [rect.left(), rect.top()]),
            project(matrix, [rect.right(), rect.top()]),
            project(matrix, [rect.right(), rect.bottom()]),
            project(matrix, [rect.left(), rect.bottom()]),
        ];
        let left = points
            .iter()
            .map(|point| point[0])
            .fold(f64::INFINITY, f64::min)
            .floor()
            .max(0.0) as i32;
        let top = points
            .iter()
            .map(|point| point[1])
            .fold(f64::INFINITY, f64::min)
            .floor()
            .max(0.0) as i32;
        let right = points
            .iter()
            .map(|point| point[0])
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            .min(f64::from(extent.width())) as i32;
        let bottom = points
            .iter()
            .map(|point| point[1])
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            .min(f64::from(extent.height())) as i32;
        DeviceRect::new(
            left,
            top,
            u32::try_from(right - left).unwrap(),
            u32::try_from(bottom - top).unwrap(),
        )
    }

    fn assert_production_parity(
        program: &MotionGlassProgram,
        owner_to_device: [f64; 9],
        field: bool,
    ) {
        let mut program = program.clone();
        let bounds = resolve_motion_glass_backdrop_bounds(&program, owner_to_device).unwrap();
        program.backdrop.output_bounds = bounds.output_owner;
        program.backdrop.sample_bounds = bounds.sample_owner;
        program.validate().unwrap();
        let extent = Extent2d::new(192, 144).unwrap();
        let info = working_info(extent).unwrap();
        let mut backdrop_surface = raster_surface(&info).unwrap();
        draw_checker(&mut backdrop_surface, extent);
        let backdrop_image = backdrop_surface.image_snapshot();
        let backdrop = read_image(&backdrop_image, extent);
        let reference = if field {
            render_field_contribution_transformed(&program, owner_to_device, &backdrop).unwrap()
        } else {
            render_glass_contribution_transformed(&program, owner_to_device, &backdrop).unwrap()
        };

        let input_roi = projected_roi(program.backdrop.sample_bounds, owner_to_device, extent);
        let output_roi = projected_roi(program.backdrop.output_bounds, owner_to_device, extent);
        assert!(!input_roi.is_empty() && !output_roi.is_empty());
        // The physical footprint may legitimately reach a canvas edge even when the material
        // output is offset. Keep the cropped-output assertion without requiring a non-zero input
        // origin.
        assert_ne!([output_roi.x, output_roi.y], [0, 0]);
        let input = backdrop_surface
            .image_snapshot_with_bounds(IRect::from_xywh(
                input_roi.x,
                input_roi.y,
                i32::try_from(input_roi.width).unwrap(),
                i32::try_from(input_roi.height).unwrap(),
            ))
            .unwrap();
        let output_extent = Extent2d::new(output_roi.width, output_roi.height).unwrap();
        let output_info = working_info(output_extent).unwrap();
        let mut output_surface = raster_surface(&output_info).unwrap();
        let effect = admit_motion_glass_shader().unwrap();
        render_motion_glass_into(
            &mut output_surface,
            &effect,
            &input,
            input_roi,
            &program,
            owner_to_device,
            [output_roi.x, output_roi.y],
        )
        .unwrap();
        let native = read_image(&output_surface.image_snapshot(), output_extent);

        let mut worst = 0.0f32;
        let mut first_mismatch = None;
        let mut mismatch_count = 0usize;
        for local_y in 0..output_roi.height {
            for local_x in 0..output_roi.width {
                let x = u32::try_from(output_roi.x).unwrap() + local_x;
                let y = u32::try_from(output_roi.y).unwrap() + local_y;
                let expected = reference.pixel(x, y).unwrap().channels();
                let actual = native.pixel(local_x, local_y).unwrap().channels();
                for channel in 0..4 {
                    let tolerance = 0.003 * (1.0 + expected[channel].abs());
                    let diff = (actual[channel] - expected[channel]).abs();
                    worst = worst.max(diff);
                    if diff > tolerance {
                        mismatch_count += 1;
                        first_mismatch.get_or_insert((
                            x,
                            y,
                            channel,
                            actual[channel],
                            expected[channel],
                            diff,
                            tolerance,
                        ));
                    }
                }
            }
        }
        assert!(
            first_mismatch.is_none(),
            "production SkSL parity found {mismatch_count} channel mismatches; first={first_mismatch:?}, worst={worst}"
        );
        assert!(
            worst <= 0.012,
            "production SkSL parity exceeded F16/SkSL budget: {worst}"
        );
    }

    #[test]
    fn production_sksl_matches_reference_on_native_skia_raster_surface() {
        assert_production_parity(
            &sample_program(),
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            false,
        );
    }

    #[test]
    fn production_sksl_field_matches_reference_under_projective_owner_transform() {
        assert_production_parity(
            &field_program(),
            [1.1, 0.15, 8.0, 0.05, 0.9, 10.0, 0.0012, 0.0005, 1.0],
            true,
        );
    }

    #[test]
    fn production_sksl_path_matches_reference_under_projective_owner_transform() {
        assert_production_parity(
            &path_program(),
            [0.96, -0.08, 10.0, 0.06, 0.92, 6.0, 0.0007, -0.0005, 1.0],
            false,
        );
    }

    #[test]
    fn production_sksl_foreground_matches_reference_on_a_cropped_projective_roi() {
        let foreground =
            MotionGlassForegroundProgram::from_owner(&sample_program(), "hero-lens").unwrap();
        let owner_to_device = [1.08, 0.12, 7.0, -0.04, 0.94, 9.0, 0.0008, -0.0003, 1.0];
        let extent = Extent2d::new(192, 144).unwrap();
        let info = working_info(extent).unwrap();
        let mut input_surface = raster_surface(&info).unwrap();
        draw_checker(&mut input_surface, extent);
        let full_input = read_image(&input_surface.image_snapshot(), extent);
        let reference =
            apply_glass_foreground_transformed(&foreground, owner_to_device, &full_input).unwrap();

        let output_roi = projected_roi(foreground.rect, owner_to_device, extent);
        assert!(!output_roi.is_empty());
        assert_ne!([output_roi.x, output_roi.y], [0, 0]);
        let input = input_surface
            .image_snapshot_with_bounds(IRect::from_xywh(
                output_roi.x,
                output_roi.y,
                i32::try_from(output_roi.width).unwrap(),
                i32::try_from(output_roi.height).unwrap(),
            ))
            .unwrap();
        let output_extent = Extent2d::new(output_roi.width, output_roi.height).unwrap();
        let mut output_surface = raster_surface(&working_info(output_extent).unwrap()).unwrap();
        let effect = admit_motion_glass_shader().unwrap();
        render_motion_glass_foreground_into(
            &mut output_surface,
            &effect,
            &input,
            output_roi,
            &foreground,
            owner_to_device,
            [output_roi.x, output_roi.y],
        )
        .unwrap();
        let native = read_image(&output_surface.image_snapshot(), output_extent);

        let mut worst = 0.0f32;
        for local_y in 0..output_roi.height {
            for local_x in 0..output_roi.width {
                let x = u32::try_from(output_roi.x).unwrap() + local_x;
                let y = u32::try_from(output_roi.y).unwrap() + local_y;
                let expected = reference.pixel(x, y).unwrap().channels();
                let actual = native.pixel(local_x, local_y).unwrap().channels();
                for channel in 0..4 {
                    worst = worst.max((actual[channel] - expected[channel]).abs());
                }
            }
        }
        assert!(worst <= 0.003, "foreground SkSL parity drifted by {worst}");
    }
}
