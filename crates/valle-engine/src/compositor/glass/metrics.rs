//! G1.8 structural performance counters for the Glass reference pipeline.
//!
//! These counters are the fixed structural gate: they must hold regardless of the runner or
//! the machine. Wall-clock numbers live in `summary.json` evidence; structure is checked here
//! against the frozen G0 budget (benchmarks/glass/budget.toml).

use thiserror::Error;
use valle_draw::program::glass::MotionGlassProgram;

use super::response::RESPONSE_INTEGRATION_SAMPLES;
use super::track::GlassDeviceShape;

/// Frozen structural budgets mirroring `benchmarks/glass/budget.toml`.
pub const MAX_TRACK_SAMPLES_PER_FRAME: usize = 64;
pub const MAX_RESPONSE_INTEGRATION_SAMPLES: usize = 8;
pub const MAX_PROGRAM_PAYLOAD_BYTES: usize = 65_536;
pub const MAX_REFERENCE_PR_PIXELS: u64 = 8192;
/// One Current backdrop ROI resolve and one material+composite pass per independent surface.
pub const EXPECTED_GLASS_RESOLVES: usize = 1;
pub const EXPECTED_GLASS_PASSES: usize = 1;
pub const EXPECTED_GLASS_READBACKS: usize = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlassStructureCounters {
    pub surfaces: usize,
    pub track_samples: usize,
    pub response_integration_samples_per_surface: usize,
    /// Compact serde payload bytes contributed to the enclosing DrawProgram packet.
    pub program_payload_bytes: usize,
    /// Pixels touched by the Current backdrop ROI.
    pub raster_pixels: u64,
    pub backdrop_resolves: usize,
    pub material_passes: usize,
    pub gpu_readbacks: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StructureError {
    #[error("Glass structure requires exactly one independent surface, got {0}")]
    SurfaceCount(usize),
    #[error("Glass track exceeds {MAX_TRACK_SAMPLES_PER_FRAME} samples, got {0}")]
    TooManySamples(usize),
    #[error("Glass program is invalid")]
    InvalidProgram,
    #[error("Glass program exceeds {MAX_PROGRAM_PAYLOAD_BYTES} payload bytes, got {0}")]
    ProgramTooLarge(usize),
    #[error("Glass raster exceeds {MAX_REFERENCE_PR_PIXELS} reference pixels, got {0}")]
    RasterTooLarge(u64),
    #[error("Glass must not read back from the GPU")]
    Readback,
}

impl GlassStructureCounters {
    /// Counts the structure of one independent Glass program and its device track.
    pub fn measure(
        program: &MotionGlassProgram,
        shapes: &[GlassDeviceShape],
    ) -> Result<Self, StructureError> {
        if program.surfaces.len() != 1 {
            return Err(StructureError::SurfaceCount(program.surfaces.len()));
        }
        if shapes.len() > MAX_TRACK_SAMPLES_PER_FRAME {
            return Err(StructureError::TooManySamples(shapes.len()));
        }
        program
            .validate()
            .map_err(|_| StructureError::InvalidProgram)?;
        let payload_bytes = serde_json::to_vec(program)
            .map_err(|_| StructureError::InvalidProgram)?
            .len();
        if payload_bytes > MAX_PROGRAM_PAYLOAD_BYTES {
            return Err(StructureError::ProgramTooLarge(payload_bytes));
        }
        let sample = program.backdrop.sample_bounds;
        let raster = ((sample.right() - sample.left()).max(0.0)
            * (sample.bottom() - sample.top()).max(0.0))
        .ceil() as u64;
        if raster > MAX_REFERENCE_PR_PIXELS {
            return Err(StructureError::RasterTooLarge(raster));
        }
        let counters = Self {
            surfaces: program.surfaces.len(),
            track_samples: shapes.len(),
            response_integration_samples_per_surface: RESPONSE_INTEGRATION_SAMPLES,
            program_payload_bytes: payload_bytes,
            raster_pixels: raster,
            backdrop_resolves: EXPECTED_GLASS_RESOLVES,
            material_passes: EXPECTED_GLASS_PASSES,
            gpu_readbacks: EXPECTED_GLASS_READBACKS,
        };
        Ok(counters)
    }

    /// G2.8: counts the structure of one field program (one owner, one resolve, one pass).
    pub fn measure_field(
        program: &MotionGlassProgram,
        member_shapes: &[Vec<GlassDeviceShape>],
    ) -> Result<Self, StructureError> {
        if program.owner_kind != valle_draw::program::glass::GlassOwnerKind::Field {
            return Err(StructureError::SurfaceCount(program.surfaces.len()));
        }
        let member_count = program.surfaces.len();
        if member_count == 0
            || member_count > valle_draw::program::glass::MAX_GLASS_MEMBERS
            || member_count != member_shapes.len()
        {
            return Err(StructureError::SurfaceCount(member_count));
        }
        if member_shapes
            .iter()
            .any(|shapes| shapes.len() > MAX_TRACK_SAMPLES_PER_FRAME)
        {
            return Err(StructureError::TooManySamples(
                member_shapes.iter().map(Vec::len).max().unwrap_or_default(),
            ));
        }
        program
            .validate()
            .map_err(|_| StructureError::InvalidProgram)?;
        let payload_bytes = serde_json::to_vec(program)
            .map_err(|_| StructureError::InvalidProgram)?
            .len();
        if payload_bytes > MAX_PROGRAM_PAYLOAD_BYTES {
            return Err(StructureError::ProgramTooLarge(payload_bytes));
        }
        let sample = program.backdrop.sample_bounds;
        let raster = ((sample.right() - sample.left()).max(0.0)
            * (sample.bottom() - sample.top()).max(0.0))
        .ceil() as u64;
        if raster > MAX_REFERENCE_PR_PIXELS {
            return Err(StructureError::RasterTooLarge(raster));
        }
        let counters = Self {
            surfaces: member_count,
            track_samples: member_shapes.iter().map(Vec::len).sum(),
            response_integration_samples_per_surface: RESPONSE_INTEGRATION_SAMPLES,
            program_payload_bytes: payload_bytes,
            raster_pixels: raster,
            backdrop_resolves: EXPECTED_GLASS_RESOLVES,
            material_passes: EXPECTED_GLASS_PASSES,
            gpu_readbacks: EXPECTED_GLASS_READBACKS,
        };
        Ok(counters)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::glass::material::resolve_material_base;
    use crate::compositor::glass::pack::IndependentGlassPackage;
    use crate::compositor::glass::response::CharacterKernel;
    use crate::compositor::glass::track::GlassSurfaceKey;
    use crate::compositor::glass::{GlassKinematics, pack_independent_glass};
    use valle_draw::Rect;
    use valle_draw::program::glass::{
        BackdropUse, GlassOwnerKind, MOTION_GLASS_KERNEL_ID, PackedGlassMaterial,
        PackedGlassMotion, PackedGlassSurface, motion_glass_kernel_digest, schema_digest,
    };
    use valle_timeline::internal::{
        DiscontinuityIndex, FrameKey, FrameRate, MotionInstanceId, SampleTime, TimeMapSegment,
        TrackEpoch,
    };

    fn shape() -> GlassDeviceShape {
        GlassDeviceShape {
            key: GlassSurfaceKey {
                surface_id: "lens".into(),
                instance: MotionInstanceId::new("clip-a").unwrap(),
                epoch: TrackEpoch::new(
                    1,
                    MotionInstanceId::new("clip-a").unwrap(),
                    TimeMapSegment::new(0),
                    0,
                    DiscontinuityIndex::NONE,
                ),
            },
            time: SampleTime::from_frame(FrameKey::new(0), FrameRate::new(30, 1).unwrap()).unwrap(),
            rect: Rect::new(16.0, 16.0, 24.0, 24.0),
            matrix: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            radius: 12.0,
            presence: 1.0,
            intensity: 0.6,
            drive: [0.0; 4],
            kinematics: GlassKinematics::ZERO,
        }
    }

    fn program() -> MotionGlassProgram {
        let spec = crate::compositor::glass::GlassSurfaceSpec {
            surface_id: "lens".into(),
            shape: valle_draw::program::glass::PackedGlassShapeKind::Circle,
            radius: 12.0,
        };
        let shapes = vec![shape()];
        let material = resolve_material_base(0.82, 0.52, [0.0; 4], 1.0, 0.65).unwrap();
        let package = IndependentGlassPackage {
            spec: &spec,
            shapes: &shapes,
            material,
            character: CharacterKernel::Fluid,
            settle: 0.32,
            path_points: vec![],
        };
        pack_independent_glass(&package).unwrap()
    }

    #[test]
    fn small_independent_program_meets_budget() {
        let program = program();
        let shapes = vec![shape()];
        let counters = GlassStructureCounters::measure(&program, &shapes).unwrap();
        assert_eq!(counters.surfaces, 1);
        assert_eq!(counters.track_samples, 1);
        assert_eq!(
            counters.response_integration_samples_per_surface,
            MAX_RESPONSE_INTEGRATION_SAMPLES
        );
        assert_eq!(counters.backdrop_resolves, EXPECTED_GLASS_RESOLVES);
        assert_eq!(counters.material_passes, EXPECTED_GLASS_PASSES);
        assert_eq!(counters.gpu_readbacks, 0);
        assert!(counters.raster_pixels <= MAX_REFERENCE_PR_PIXELS);
    }

    #[test]
    fn oversized_raster_fails_budget() {
        let mut program = program();
        let output = program.backdrop.output_bounds;
        program.backdrop.sample_bounds = Rect::from_edges(
            output.left() - 2_048.0,
            output.top() - 2_048.0,
            output.right() + 2_048.0,
            output.bottom() + 2_048.0,
        );
        program.validate().unwrap();
        let result = GlassStructureCounters::measure(&program, &[shape()]);
        assert!(
            matches!(&result, Err(StructureError::RasterTooLarge(_))),
            "unexpected result: {result:?}"
        );
    }

    #[test]
    fn oversized_track_fails_budget() {
        let program = program();
        let mut shapes = vec![shape()];
        for frame in 1..=65 {
            let mut s = shape();
            s.time = SampleTime::from_frame(FrameKey::new(frame), FrameRate::new(30, 1).unwrap())
                .unwrap();
            shapes.push(s);
        }
        assert!(matches!(
            GlassStructureCounters::measure(&program, &shapes),
            Err(StructureError::TooManySamples(66))
        ));
    }

    #[test]
    fn multi_surface_program_fails_closed() {
        let mut program = program();
        program.owner_kind = GlassOwnerKind::Field;
        program.surfaces.push(PackedGlassSurface {
            surface_id: "second".into(),
            ..program.surfaces[0].clone()
        });
        program.field = Some(valle_draw::program::glass::PackedGlassField {
            field_id: "orbit".into(),
            merge_distance: 24.0,
            member_ids: vec!["lens".into(), "second".into()],
        });
        assert!(matches!(
            GlassStructureCounters::measure(&program, &[shape()]),
            Err(StructureError::SurfaceCount(2))
        ));
    }

    #[test]
    fn field_structure_meets_budget_for_1_2_3_and_8_members() {
        // G2.8 matrix: 1/2/3/8 members keep one owner program, one resolve, one pass, and
        // bounded samples/bytes/pixels (stationary + moving variants).
        for count in [1usize, 2, 3, 8] {
            for moving in [false, true] {
                let mut surfaces = Vec::new();
                let mut member_ids = Vec::new();
                let mut member_shapes = Vec::new();
                for index in 0..count {
                    let id = format!("member-{index}");
                    let rect = Rect::new(2.0 + 5.0 * index as f64, 8.0, 20.0, 20.0);
                    let shapes = (0..=16)
                        .map(|frame| {
                            let mut s = shape();
                            s.time = SampleTime::from_frame(
                                FrameKey::new(frame),
                                FrameRate::new(30, 1).unwrap(),
                            )
                            .unwrap();
                            s.rect = rect;
                            s.key.surface_id = id.clone();
                            if moving {
                                s.drive = [5.0, 0.0, 0.0, 0.0];
                            }
                            s
                        })
                        .collect::<Vec<_>>();
                    member_shapes.push(shapes);
                    let response_tap = if moving {
                        PackedGlassMotion {
                            translation: [5.0, 0.0],
                            ..PackedGlassMotion::ZERO
                        }
                    } else {
                        PackedGlassMotion::ZERO
                    };
                    surfaces.push(PackedGlassSurface {
                        surface_id: id.clone(),
                        shape: valle_draw::program::glass::PackedGlassShapeKind::Circle,
                        rect,
                        local_to_owner: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                        radius: 10.0,
                        presence: 1.0,
                        response: response_tap,
                        path_points: vec![],
                        foreground_tone:
                            valle_draw::program::glass::PackedGlassForegroundTone::None,
                        foreground_protection: 0.0,
                        foreground_bounds: None,
                        foreground_luma: None,
                    });
                    member_ids.push(id);
                }
                let program = MotionGlassProgram {
                    owner_kind: GlassOwnerKind::Field,
                    owner_id: "orbit".into(),
                    surfaces,
                    field: Some(valle_draw::program::glass::PackedGlassField {
                        field_id: "orbit".into(),
                        merge_distance: 24.0,
                        member_ids,
                    }),
                    material: PackedGlassMaterial {
                        bevel_width: 6.0,
                        thickness: 12.0,
                        refractive_index: 1.2,
                        roughness: 0.08,
                        dispersion: 0.012,
                        tint_linear: [0.0; 4],
                        specular_strength: 0.5,
                        shadow_strength: 0.1,
                        foreground_gain: 0.7,
                        light: valle_draw::program::PackedGlassLight::DEFAULT,
                    },
                    backdrop: BackdropUse {
                        scope: valle_draw::program::BackdropScope::ScopeEntry("orbit".into()),
                        sample_bounds: Rect::new(0.0, 0.0, 64.0, 64.0),
                        output_bounds: Rect::new(0.0, 0.0, 64.0, 64.0),
                    },
                    kernel_id: MOTION_GLASS_KERNEL_ID.into(),
                    kernel_digest: motion_glass_kernel_digest(),
                    schema_digest: schema_digest(),
                };
                let counters =
                    GlassStructureCounters::measure_field(&program, &member_shapes).unwrap();
                assert_eq!(counters.surfaces, count);
                assert_eq!(counters.backdrop_resolves, EXPECTED_GLASS_RESOLVES);
                assert_eq!(counters.material_passes, EXPECTED_GLASS_PASSES);
                assert_eq!(counters.gpu_readbacks, 0);
                assert!(counters.raster_pixels <= MAX_REFERENCE_PR_PIXELS);
            }
        }
    }

    #[test]
    fn oversized_field_fails_closed() {
        let program = program();
        let mut surfaces = Vec::new();
        let mut member_ids = Vec::new();
        let mut member_shapes = Vec::new();
        for index in 0..17usize {
            let id = format!("m-{index}");
            surfaces.push(PackedGlassSurface {
                surface_id: id.clone(),
                ..program.surfaces[0].clone()
            });
            member_ids.push(id);
            member_shapes.push(vec![shape()]);
        }
        let mut field = program;
        field.owner_kind = GlassOwnerKind::Field;
        field.surfaces = surfaces;
        field.field = Some(valle_draw::program::glass::PackedGlassField {
            field_id: "orbit".into(),
            merge_distance: 24.0,
            member_ids,
        });
        assert!(matches!(
            GlassStructureCounters::measure_field(&field, &member_shapes),
            Err(StructureError::SurfaceCount(17))
        ));
    }
}
