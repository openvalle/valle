//! Canonical Motion Glass SkSL uniform packing.
//!
//! The packed float stream is the only platform binding ABI. Native consumes it directly and the
//! Web host obtains the same bytes from this module compiled to Wasm; neither executor re-resolves
//! material intent or geometry.

use thiserror::Error;
use valle_draw::Rect;
use valle_draw::program::glass::{
    GlassOwnerKind, MAX_GLASS_MEMBERS, MAX_GLASS_PATH_POINTS, MotionGlassForegroundProgram,
    MotionGlassProgram, PackedGlassForegroundTone, PackedGlassShapeKind,
};

use super::bounds::{MotionGlassBoundsError, validate_motion_glass_backdrop_bounds};

pub const MOTION_GLASS_GPU_SURFACES: usize = 8;
pub const MOTION_GLASS_GPU_PATH_POINTS: usize = 24;
const GLOBAL_VECTORS: usize = 9;
const SURFACE_VECTOR_GROUPS: usize = 10;
const PATH_PAIR_VECTORS: usize = MOTION_GLASS_GPU_SURFACES * MOTION_GLASS_GPU_PATH_POINTS / 2;
pub const MOTION_GLASS_GPU_UNIFORM_FLOATS: usize =
    (GLOBAL_VECTORS + MOTION_GLASS_GPU_SURFACES * SURFACE_VECTOR_GROUPS + PATH_PAIR_VECTORS) * 4;
pub type MotionGlassGpuUniforms = [f32; MOTION_GLASS_GPU_UNIFORM_FLOATS];

const _: () = assert!(MAX_GLASS_MEMBERS == MOTION_GLASS_GPU_SURFACES);
const _: () = assert!(MAX_GLASS_PATH_POINTS == MOTION_GLASS_GPU_PATH_POINTS);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GlassGpuUniformError {
    #[error("Motion Glass program is not valid for the canonical kernel: {0}")]
    InvalidProgram(String),
    #[error(
        "Motion Glass owner transform is non-finite, singular, or crosses a projective horizon"
    )]
    InvalidOwnerTransform,
    #[error("Motion Glass surface transform is non-finite or singular")]
    InvalidSurfaceTransform,
    #[error("Motion Glass backdrop bounds do not cover the required {0}")]
    InsufficientBackdropBounds(&'static str),
    #[error("Motion Glass GPU uniform layout drifted")]
    LayoutDrift,
}

pub fn pack_motion_glass_gpu_uniforms(
    program: &MotionGlassProgram,
    owner_to_device: [f64; 9],
) -> Result<MotionGlassGpuUniforms, GlassGpuUniformError> {
    program
        .validate()
        .map_err(|error| GlassGpuUniformError::InvalidProgram(error.to_string()))?;
    let owner_inverse = validate_backdrop_bounds(program, owner_to_device)?;
    let merge = program
        .field
        .as_ref()
        .map_or(0.0, |field| f64::from(field.merge_distance));
    let mut uniforms = UniformWriter::new();
    push4(
        &mut uniforms,
        [
            0.0,
            program.owner_kind as u8 as f32,
            program.surfaces.len() as f32,
            merge as f32,
        ],
    );
    push4(
        &mut uniforms,
        [
            if program.surfaces.iter().any(|surface| {
                surface.foreground_protection > 0.0
                    && surface.foreground_bounds.is_some()
                    && surface.foreground_tone != PackedGlassForegroundTone::None
            }) {
                1.0
            } else {
                0.0
            },
            program.material.light.direction[0],
            program.material.light.direction[1],
            program.material.light.intensity,
        ],
    );
    push_matrix_rows(&mut uniforms, owner_inverse);
    push4(
        &mut uniforms,
        [
            program.material.bevel_width,
            program.material.thickness,
            program.material.refractive_index,
            program.material.roughness,
        ],
    );
    push4(
        &mut uniforms,
        [
            program.material.tint_linear[0],
            program.material.tint_linear[1],
            program.material.tint_linear[2],
            program.material.tint_linear[3],
        ],
    );
    push4(
        &mut uniforms,
        [
            program.material.dispersion,
            program.material.specular_strength,
            program.material.shadow_strength,
            program.material.foreground_gain,
        ],
    );
    push4(
        &mut uniforms,
        [program.material.light.elevation, 0.0, 0.0, 0.0],
    );
    push_surface_arrays(
        &mut uniforms,
        program.surfaces.iter().map(|surface| SurfaceUniformSource {
            shape: surface.shape,
            rect: surface.rect,
            local_to_owner: surface.local_to_owner,
            radius: surface.radius,
            presence: surface.presence,
            motion0: [
                surface.response.translation[0],
                surface.response.translation[1],
                surface.response.acceleration[0],
                surface.response.acceleration[1],
            ],
            motion1: [
                surface.response.angular,
                surface.response.scale[0],
                surface.response.scale[1],
                surface.response.shear,
            ],
            motion2: [
                surface.response.area,
                surface.response.pressure,
                surface.response.twist,
                surface.path_points.len() as f32,
            ],
            foreground_bounds: surface.foreground_bounds,
            foreground_meta: foreground_meta(
                surface.foreground_tone,
                surface.foreground_protection,
                surface.foreground_luma,
            ),
            path_points: &surface.path_points,
        }),
        owner_to_device,
    )?;
    finish(uniforms)
}

pub fn pack_motion_glass_foreground_gpu_uniforms(
    foreground: &MotionGlassForegroundProgram,
    owner_to_device: [f64; 9],
) -> Result<MotionGlassGpuUniforms, GlassGpuUniformError> {
    foreground
        .validate()
        .map_err(|error| GlassGpuUniformError::InvalidProgram(error.to_string()))?;
    let owner_inverse =
        matrix_inverse(owner_to_device).ok_or(GlassGpuUniformError::InvalidOwnerTransform)?;
    let mut uniforms = UniformWriter::new();
    push4(
        &mut uniforms,
        [1.0, GlassOwnerKind::Independent as u8 as f32, 1.0, 0.0],
    );
    push4(&mut uniforms, [0.0; 4]);
    push_matrix_rows(&mut uniforms, owner_inverse);
    for _ in 0..4 {
        push4(&mut uniforms, [0.0; 4]);
    }
    push_surface_arrays(
        &mut uniforms,
        std::iter::once(SurfaceUniformSource {
            shape: foreground.shape,
            rect: foreground.rect,
            local_to_owner: foreground.local_to_owner,
            radius: foreground.radius,
            presence: foreground.presence,
            motion0: [0.0; 4],
            motion1: [0.0; 4],
            motion2: [0.0, 0.0, 0.0, foreground.path_points.len() as f32],
            foreground_bounds: None,
            foreground_meta: [0.0; 4],
            path_points: &foreground.path_points,
        }),
        owner_to_device,
    )?;
    finish(uniforms)
}

#[derive(Clone, Copy)]
struct SurfaceUniformSource<'a> {
    shape: PackedGlassShapeKind,
    rect: Rect,
    local_to_owner: [f32; 9],
    radius: f32,
    presence: f32,
    motion0: [f32; 4],
    motion1: [f32; 4],
    motion2: [f32; 4],
    foreground_bounds: Option<Rect>,
    foreground_meta: [f32; 4],
    path_points: &'a [[f32; 2]],
}

fn push_surface_arrays<'a>(
    uniforms: &mut UniformWriter,
    sources: impl Iterator<Item = SurfaceUniformSource<'a>>,
    owner_to_device: [f64; 9],
) -> Result<(), GlassGpuUniformError> {
    let mut padded_sources = [None; MOTION_GLASS_GPU_SURFACES];
    let mut inverses = [[0.0; 9]; MOTION_GLASS_GPU_SURFACES];
    for (source_count, source) in sources.enumerate() {
        let Some(slot) = padded_sources.get_mut(source_count) else {
            return Err(GlassGpuUniformError::LayoutDrift);
        };
        let local_to_device = matrix_mul(owner_to_device, source.local_to_owner.map(f64::from));
        if !rect_has_no_horizon(source.rect, local_to_device) {
            return Err(GlassGpuUniformError::InvalidSurfaceTransform);
        }
        inverses[source_count] =
            matrix_inverse(local_to_device).ok_or(GlassGpuUniformError::InvalidSurfaceTransform)?;
        *slot = Some(source);
    }

    for source in padded_sources {
        push4(
            uniforms,
            source.map_or([0.0; 4], |source| {
                [
                    source.rect.x as f32,
                    source.rect.y as f32,
                    source.rect.width as f32,
                    source.rect.height as f32,
                ]
            }),
        );
    }
    for source in padded_sources {
        push4(
            uniforms,
            source.map_or([0.0; 4], |source| {
                [
                    source.shape as u8 as f32,
                    source.radius,
                    source.presence,
                    0.0,
                ]
            }),
        );
    }
    for row in 0..3 {
        for (index, source) in padded_sources.iter().enumerate() {
            let value = if source.is_some() {
                matrix_row(inverses[index], row)
            } else {
                [0.0; 4]
            };
            push4(uniforms, value);
        }
    }
    for source in padded_sources {
        push4(uniforms, source.map_or([0.0; 4], |source| source.motion0));
    }
    for source in padded_sources {
        push4(uniforms, source.map_or([0.0; 4], |source| source.motion1));
    }
    for source in padded_sources {
        push4(uniforms, source.map_or([0.0; 4], |source| source.motion2));
    }
    for source in padded_sources {
        push4(
            uniforms,
            source
                .and_then(|source| source.foreground_bounds)
                .map_or([0.0; 4], |bounds| {
                    [
                        bounds.x as f32,
                        bounds.y as f32,
                        bounds.width as f32,
                        bounds.height as f32,
                    ]
                }),
        );
    }
    for source in padded_sources {
        push4(
            uniforms,
            source.map_or([0.0; 4], |source| source.foreground_meta),
        );
    }
    for source in padded_sources {
        for pair in 0..MOTION_GLASS_GPU_PATH_POINTS / 2 {
            let value = source.map_or([0.0; 4], |source| {
                let first = source
                    .path_points
                    .get(pair * 2)
                    .copied()
                    .unwrap_or([0.0; 2]);
                let second = source
                    .path_points
                    .get(pair * 2 + 1)
                    .copied()
                    .unwrap_or([0.0; 2]);
                [first[0], first[1], second[0], second[1]]
            });
            push4(uniforms, value);
        }
    }
    Ok(())
}

fn foreground_meta(
    tone: PackedGlassForegroundTone,
    protection: f32,
    luma: Option<f32>,
) -> [f32; 4] {
    let tone_bias = match tone {
        PackedGlassForegroundTone::Light => 1.0,
        PackedGlassForegroundTone::Dark => -1.0,
        PackedGlassForegroundTone::Auto => luma.map_or(0.0, |value| value * 2.0 - 1.0),
        PackedGlassForegroundTone::None => 0.0,
    };
    [
        protection,
        tone_bias,
        if protection > 0.0 && tone != PackedGlassForegroundTone::None {
            1.0
        } else {
            0.0
        },
        0.0,
    ]
}

/// Re-derives the conservative output and sampling footprint at the executor boundary.
///
/// `MotionGlassProgram::validate` can prove structural owner-space containment without knowing
/// the draw transform. Device-pixel shadow and optical margins require that transform, so the
/// canonical packer is the first shared Native/Web boundary that can reject a program whose
/// bounds would clip visible output or sample outside the admitted backdrop ROI.
fn validate_backdrop_bounds(
    program: &MotionGlassProgram,
    owner_to_device: [f64; 9],
) -> Result<[f64; 9], GlassGpuUniformError> {
    let bounds = validate_motion_glass_backdrop_bounds(program, owner_to_device).map_err(
        |error| match error {
            MotionGlassBoundsError::EmptyOwner => {
                GlassGpuUniformError::InvalidProgram(error.to_string())
            }
            MotionGlassBoundsError::InvalidSurfaceTransform(_) => {
                GlassGpuUniformError::InvalidSurfaceTransform
            }
            MotionGlassBoundsError::InvalidOwnerTransform => {
                GlassGpuUniformError::InvalidOwnerTransform
            }
            MotionGlassBoundsError::InsufficientFieldSupport => {
                GlassGpuUniformError::InsufficientBackdropBounds("Field merge support")
            }
            MotionGlassBoundsError::InsufficientOutput => {
                GlassGpuUniformError::InsufficientBackdropBounds("visible output")
            }
            MotionGlassBoundsError::InsufficientSample => {
                GlassGpuUniformError::InsufficientBackdropBounds("optical sample footprint")
            }
        },
    )?;
    Ok(bounds.device_to_owner)
}

struct UniformWriter {
    values: MotionGlassGpuUniforms,
    cursor: usize,
    drifted: bool,
}

impl UniformWriter {
    fn new() -> Self {
        Self {
            values: [0.0; MOTION_GLASS_GPU_UNIFORM_FLOATS],
            cursor: 0,
            drifted: false,
        }
    }

    fn push4(&mut self, value: [f32; 4]) {
        let Some(target) = self.values.get_mut(self.cursor..self.cursor + 4) else {
            self.drifted = true;
            return;
        };
        target.copy_from_slice(&value);
        self.cursor += 4;
    }

    fn finish(self) -> Result<MotionGlassGpuUniforms, GlassGpuUniformError> {
        if !self.drifted
            && self.cursor == MOTION_GLASS_GPU_UNIFORM_FLOATS
            && self.values.iter().all(|value| value.is_finite())
        {
            Ok(self.values)
        } else {
            Err(GlassGpuUniformError::LayoutDrift)
        }
    }
}

fn finish(uniforms: UniformWriter) -> Result<MotionGlassGpuUniforms, GlassGpuUniformError> {
    uniforms.finish()
}

fn push4(output: &mut UniformWriter, value: [f32; 4]) {
    output.push4(value);
}

fn push_matrix_rows(output: &mut UniformWriter, matrix: [f64; 9]) {
    for row in 0..3 {
        push4(output, matrix_row(matrix, row));
    }
}

fn matrix_row(matrix: [f64; 9], row: usize) -> [f32; 4] {
    [
        matrix[row * 3] as f32,
        matrix[row * 3 + 1] as f32,
        matrix[row * 3 + 2] as f32,
        0.0,
    ]
}

fn rect_has_no_horizon(rect: Rect, transform: [f64; 9]) -> bool {
    let denominators = [
        [rect.left(), rect.top()],
        [rect.right(), rect.top()],
        [rect.right(), rect.bottom()],
        [rect.left(), rect.bottom()],
    ]
    .map(|point| transform[6] * point[0] + transform[7] * point[1] + transform[8]);
    let first = denominators[0];
    first.is_finite()
        && first.abs() > 1.0e-12
        && denominators.into_iter().all(|value| {
            value.is_finite()
                && value.abs() > 1.0e-12
                && value.is_sign_positive() == first.is_sign_positive()
        })
}

fn matrix_mul(left: [f64; 9], right: [f64; 9]) -> [f64; 9] {
    let mut output = [0.0; 9];
    for row in 0..3 {
        for column in 0..3 {
            output[row * 3 + column] = (0..3)
                .map(|index| left[row * 3 + index] * right[index * 3 + column])
                .sum();
        }
    }
    output
}

fn matrix_inverse(matrix: [f64; 9]) -> Option<[f64; 9]> {
    let [a, b, c, d, e, f, g, h, i] = matrix;
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
    (determinant.is_finite() && determinant.abs() > 1.0e-12)
        .then(|| cofactors.map(|value| value / determinant))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::glass::bounds::resolve_motion_glass_backdrop_bounds;
    use valle_draw::program::BackdropScope;
    use valle_draw::program::glass::{
        BackdropUse, MOTION_GLASS_KERNEL_ID, PackedGlassMaterial, PackedGlassMotion,
        PackedGlassSurface, motion_glass_kernel_digest, schema_digest,
    };

    fn program() -> MotionGlassProgram {
        MotionGlassProgram {
            owner_kind: GlassOwnerKind::Independent,
            owner_id: "surface".into(),
            surfaces: vec![PackedGlassSurface {
                surface_id: "surface".into(),
                shape: PackedGlassShapeKind::Circle,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                local_to_owner: [40.0, 0.0, 10.0, 0.0, 40.0, 5.0, 0.0, 0.0, 1.0],
                radius: 0.5,
                presence: 1.0,
                response: PackedGlassMotion::ZERO,
                path_points: vec![],
                foreground_tone: PackedGlassForegroundTone::None,
                foreground_protection: 0.0,
                foreground_bounds: None,
                foreground_luma: None,
            }],
            field: None,
            material: PackedGlassMaterial {
                bevel_width: 8.0,
                thickness: 16.0,
                refractive_index: 1.2,
                roughness: 0.08,
                dispersion: 0.012,
                tint_linear: [0.2, 0.3, 0.8, 0.2],
                specular_strength: 0.5,
                shadow_strength: 0.1,
                foreground_gain: 0.0,
                light: valle_draw::program::PackedGlassLight::DEFAULT,
            },
            backdrop: BackdropUse {
                scope: BackdropScope::Current,
                sample_bounds: Rect::new(-40.0, -45.0, 140.0, 140.0),
                output_bounds: Rect::new(-2.0, -7.0, 64.0, 64.0),
            },
            kernel_id: MOTION_GLASS_KERNEL_ID.into(),
            kernel_digest: motion_glass_kernel_digest(),
            schema_digest: schema_digest(),
        }
    }

    #[test]
    fn uniform_layout_is_fixed_and_finite() {
        let uniforms = pack_motion_glass_gpu_uniforms(
            &program(),
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        assert_eq!(uniforms.len(), MOTION_GLASS_GPU_UNIFORM_FLOATS);
        assert!(uniforms.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn owner_transform_crossing_a_projective_horizon_is_rejected() {
        let error = pack_motion_glass_gpu_uniforms(
            &program(),
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -0.05, 0.0, 1.0],
        )
        .unwrap_err();
        assert_eq!(error, GlassGpuUniformError::InvalidOwnerTransform);
    }

    #[test]
    fn field_merge_stays_in_owner_space_for_the_shader_local_metric() {
        let mut program = program();
        program.owner_kind = GlassOwnerKind::Field;
        program.owner_id = "field".into();
        program.field = Some(valle_draw::program::glass::PackedGlassField {
            field_id: "field".into(),
            merge_distance: 24.0,
            member_ids: vec!["surface".into()],
        });
        program.backdrop.scope = BackdropScope::ScopeEntry("field".into());
        program.backdrop.output_bounds = Rect::new(-14.0, -19.0, 88.0, 88.0);
        program.backdrop.sample_bounds = Rect::new(-100.0, -120.0, 300.0, 300.0);
        let uniforms =
            pack_motion_glass_gpu_uniforms(&program, [2.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 1.0])
                .unwrap();
        assert_eq!(uniforms[3], 24.0);
        assert_eq!(uniforms[4], 0.0, "no foreground protection is active");
    }

    #[test]
    fn foreground_protection_activity_is_packed_as_a_global_fast_path() {
        let mut program = program();
        program.surfaces[0].foreground_tone = PackedGlassForegroundTone::Light;
        program.surfaces[0].foreground_protection = 0.5;
        program.surfaces[0].foreground_bounds = Some(Rect::new(20.0, 15.0, 10.0, 5.0));
        let uniforms =
            pack_motion_glass_gpu_uniforms(&program, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
                .unwrap();
        assert_eq!(uniforms[4], 1.0);
    }

    #[test]
    fn independent_bounds_must_cover_shadow_and_optical_sampling() {
        let mut clipped_output = program();
        clipped_output.backdrop.output_bounds = Rect::new(10.0, 5.0, 40.0, 40.0);
        assert_eq!(
            pack_motion_glass_gpu_uniforms(
                &clipped_output,
                [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            )
            .unwrap_err(),
            GlassGpuUniformError::InsufficientBackdropBounds("visible output"),
        );

        let mut clipped_sample = program();
        let required = resolve_motion_glass_backdrop_bounds(
            &clipped_sample,
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        clipped_sample.backdrop.output_bounds = required.output_owner;
        clipped_sample.backdrop.sample_bounds = required.output_owner;
        clipped_sample.validate().unwrap();
        assert_eq!(
            pack_motion_glass_gpu_uniforms(
                &clipped_sample,
                [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            )
            .unwrap_err(),
            GlassGpuUniformError::InsufficientBackdropBounds("optical sample footprint"),
        );
    }

    #[test]
    fn field_bounds_must_cover_owner_space_merge_support() {
        let mut program = program();
        program.owner_kind = GlassOwnerKind::Field;
        program.owner_id = "field".into();
        program.field = Some(valle_draw::program::glass::PackedGlassField {
            field_id: "field".into(),
            merge_distance: 24.0,
            member_ids: vec!["surface".into()],
        });
        program.backdrop.scope = BackdropScope::ScopeEntry("field".into());
        assert_eq!(
            pack_motion_glass_gpu_uniforms(
                &program,
                [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            )
            .unwrap_err(),
            GlassGpuUniformError::InsufficientBackdropBounds("Field merge support"),
        );
    }
}
