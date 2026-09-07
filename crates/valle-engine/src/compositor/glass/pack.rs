//! G1.3/G1.4 backend-neutral packing: one independent Glass device track → `MotionGlassProgram`.
//!
//! The packer consumes only typed inputs — device shapes (identity + kinematics), resolved
//! material base, character, settle, and the force history — and emits a validated
//! `MotionGlassProgram` with `BackdropScope::Current`. No executor-side interpretation of
//! Artifact intent, no per-backend policy, and no shader source crosses this boundary.

use thiserror::Error;
use valle_draw::Rect;
use valle_draw::program::glass::{
    BackdropUse, GlassOwnerKind, MOTION_GLASS_KERNEL_ID, MotionGlassProgram, PackedGlassField,
    PackedGlassForegroundTone, PackedGlassSurface, motion_glass_kernel_digest, schema_digest,
};

use super::material::{
    MAX_REFRACTION_BEVEL_FRACTION, ResolvedGlassMaterialBase, diffusion_radius, pack_material_base,
};
use super::response::{CharacterKernel, packed_current_response, response_force};
use super::track::{GlassDeviceShape, GlassSurfaceSpec};

/// `BackdropScope::Current` — G1 programs read only the current backdrop. Canonical definition
/// lives in the ABI owner (`valle-draw`).
pub use valle_draw::program::glass::BACKDROP_SCOPE_CURRENT;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum PackError {
    #[error("independent Glass package has no device samples")]
    Empty,
    #[error("device shapes mix surface ids")]
    SurfaceMismatch,
    #[error("settle {0}s is outside [0.08, 1.20]")]
    SettleOutOfRange(f64),
    #[error("device shape at {0} has non-finite presence/intensity")]
    NonFiniteValue(usize),
    #[error("Glass program validation failed: {0}")]
    InvalidProgram(#[from] valle_draw::program::glass::MotionGlassError),
    #[error("GlassField has no members")]
    FieldEmpty,
    #[error("GlassField exceeds {0} members")]
    FieldTooLarge(usize),
    #[error("GlassField member ids must be unique and sorted, got duplicate at {0}")]
    FieldDuplicateMember(String),
    #[error("GlassField member '{0}' has no device samples")]
    FieldMemberEmpty(String),
    #[error("GlassField member shapes mix surface ids in '{0}'")]
    FieldMemberMismatch(String),
    #[error("merge distance {0} is outside [1.0, 128.0]")]
    MergeOutOfRange(f64),
}

/// Everything needed to pack one independent Glass surface.
pub struct IndependentGlassPackage<'a> {
    pub spec: &'a GlassSurfaceSpec,
    pub shapes: &'a [GlassDeviceShape],
    pub material: ResolvedGlassMaterialBase,
    pub character: CharacterKernel,
    pub settle: f64,
    /// G4.1 closed-polygon path points (device pixels); empty for analytic shapes.
    pub path_points: Vec<[f32; 2]>,
}

impl IndependentGlassPackage<'_> {
    /// G4.2 static elimination: a surface at rest (zero kinematics, zero drive) produces a
    /// zero force and therefore a zero current response — the caller may sample the track
    /// once instead of a `[t-settle, t]` history.
    pub fn requires_track_history(&self) -> bool {
        self.shapes.iter().any(|shape| {
            let kinematics = shape.kinematics;
            kinematics.linear_velocity != [0.0, 0.0]
                || kinematics.linear_acceleration != [0.0, 0.0]
                || kinematics.angular_velocity != 0.0
                || kinematics.scale_rate != [0.0, 0.0]
                || kinematics.shear_rate != 0.0
                || kinematics.area_rate != 0.0
                || shape.drive != [0.0; 4]
        })
    }
}

/// One member's device track inside a field package.
pub struct FieldMemberPackage<'a> {
    pub spec: &'a GlassSurfaceSpec,
    pub shapes: &'a [GlassDeviceShape],
    pub path_points: &'a [[f32; 2]],
}

/// G2.2 field package: all members share one material, one character, one settle, and one
/// merge distance; the field packs exactly one owner program.
pub struct FieldGlassPackage<'a> {
    pub field_id: &'a str,
    pub members: &'a [FieldMemberPackage<'a>],
    pub material: ResolvedGlassMaterialBase,
    pub character: CharacterKernel,
    pub settle: f64,
    pub merge_distance: f64,
}

/// Packs the last device sample into a validated independent `MotionGlassProgram`.
///
/// - support is exactly `settle` seconds;
/// - the force history is `response_force(kinematics, drive, intensity)` and is integrated on the
///   fixed causal quadrature grid;
/// - crossing an epoch zeroes the current response (kernel reset);
/// - `presence` uses the same materialization math via the resolved material base;
/// - sample bounds outset the output bounds by the material's optical margins.
pub fn pack_independent_glass(
    package: &IndependentGlassPackage<'_>,
) -> Result<MotionGlassProgram, PackError> {
    let program = assemble_independent_glass(package)?;
    program.validate()?;
    Ok(program)
}

/// Prepare-only first phase. Device-qualified track samples resolve motion and material, but a
/// projective surface cannot be represented faithfully as an identity-space analytic shape.
/// Product prepare therefore assembles response first, replaces geometry with the authoritative
/// owner-local homography, and validates exactly once before lowering.
pub(crate) fn assemble_independent_glass(
    package: &IndependentGlassPackage<'_>,
) -> Result<MotionGlassProgram, PackError> {
    if package.shapes.is_empty() {
        return Err(PackError::Empty);
    }
    if !(0.08..=1.20).contains(&package.settle) {
        return Err(PackError::SettleOutOfRange(package.settle));
    }
    if package
        .shapes
        .iter()
        .any(|shape| shape.key.surface_id != package.spec.surface_id)
    {
        return Err(PackError::SurfaceMismatch);
    }
    let last = package.shapes.last().expect("non-empty");
    if !last.presence.is_finite() || !last.intensity.is_finite() {
        return Err(PackError::NonFiniteValue(package.shapes.len() - 1));
    }

    // Time-indexed force history relative to the last sample, ascending by time. Before the
    // first sample the surface was at rest, so motion that just started produces no
    // pre-response.
    let last_time = last.time.composition().as_f64();
    let history = package
        .shapes
        .iter()
        .map(|shape| {
            (
                shape.time.composition().as_f64() - last_time,
                response_force(
                    shape.kinematics,
                    [
                        f64::from(shape.drive[0]),
                        f64::from(shape.drive[1]),
                        f64::from(shape.drive[2]),
                        f64::from(shape.drive[3]),
                    ],
                    f64::from(shape.intensity),
                ),
            )
        })
        .collect::<Vec<_>>();
    let epoch_reset = package.shapes.len() >= 2
        && package.shapes[package.shapes.len() - 1].key.epoch
            != package.shapes[package.shapes.len() - 2].key.epoch;
    let response =
        packed_current_response(package.character, package.settle, &history, epoch_reset);

    let material = pack_material_base(&package.material);
    let surface = PackedGlassSurface {
        surface_id: package.spec.surface_id.clone(),
        shape: package.spec.shape,
        rect: last.rect,
        local_to_owner: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        radius: last.radius,
        presence: last.presence,
        response,
        path_points: package.path_points.clone(),
        foreground_tone: PackedGlassForegroundTone::None,
        foreground_protection: 0.0,
        foreground_bounds: None,
        foreground_luma: None,
    };

    let output_bounds = outset(&last.rect, glass_independent_output_margin(&material));
    let sample_bounds = outset(
        &output_bounds,
        motion_glass_sample_margin(&material, std::slice::from_ref(&surface), false),
    );
    Ok(MotionGlassProgram {
        owner_kind: GlassOwnerKind::Independent,
        owner_id: package.spec.surface_id.clone(),
        surfaces: vec![surface],
        field: None,
        material,
        backdrop: BackdropUse {
            scope: BACKDROP_SCOPE_CURRENT,
            sample_bounds,
            output_bounds,
        },
        kernel_id: MOTION_GLASS_KERNEL_ID.into(),
        kernel_digest: motion_glass_kernel_digest(),
        schema_digest: schema_digest(),
    })
}

fn outset(rect: &Rect, margin: f64) -> Rect {
    Rect::from_edges(
        rect.left() - margin,
        rect.top() - margin,
        rect.right() + margin,
        rect.bottom() + margin,
    )
}

fn union(left: &Rect, right: &Rect) -> Rect {
    Rect::from_edges(
        left.left().min(right.left()),
        left.top().min(right.top()),
        left.right().max(right.right()),
        left.bottom().max(right.bottom()),
    )
}

/// Maximum independent outer-shadow extent in device pixels. The production shader and the
/// reference kernel use this exact radius, so output culling cannot clip a visible shadow.
pub fn glass_shadow_radius(material: &valle_draw::program::glass::PackedGlassMaterial) -> f64 {
    if material.shadow_strength > 0.0 && material.bevel_width > 0.0 {
        f64::from(material.bevel_width) * 0.60
    } else {
        0.0
    }
}

/// Complete independent-surface output outset in device pixels. Coverage is a one-pixel C2 edge
/// centered on the analytic boundary, so it remains visible for half a pixel even when the outer
/// shadow is disabled or shorter than the antialiasing fringe.
pub fn glass_independent_output_margin(
    material: &valle_draw::program::glass::PackedGlassMaterial,
) -> f64 {
    glass_shadow_radius(material).max(0.5)
}

/// Conservative texture footprint in device pixels for the current packed response.
///
/// The displacement expression is bounded channel-by-channel to a fraction of the material bevel.
/// The 5x5 binomial footprint reaches two blur radii beyond the refracted coordinate; one bilinear
/// support pixel is added after both terms. Every optical length therefore scales with the resolved
/// device-space material instead of changing appearance at a fixed resolution threshold.
pub fn motion_glass_sample_margin(
    material: &valle_draw::program::glass::PackedGlassMaterial,
    _surfaces: &[PackedGlassSurface],
    _field: bool,
) -> f64 {
    let bevel_width = f64::from(material.bevel_width).max(0.0);
    let thickness = f64::from(material.thickness).max(0.0);
    let displacement = bevel_width * MAX_REFRACTION_BEVEL_FRACTION;
    // The 5x5 binomial kernel reaches two radii away from its center sample.
    let diffusion = diffusion_radius(f64::from(material.roughness), thickness) * 2.0;
    displacement + diffusion + 1.0
}

/// Packs one GlassField into a single validated owner `MotionGlassProgram`.
///
/// - one field = one owner program (ScopeEntry semantics);
/// - members share the field material/character/settle;
/// - member ids are sorted and unique;
/// - sample bounds are the union of member optical margins, output bounds the union of
///   member rects;
/// - every member retains its own signed current response; the render kernel performs the
///   potential-weighted spatial mix and relative-strain calculation at each pixel.
pub fn pack_field_glass(package: &FieldGlassPackage<'_>) -> Result<MotionGlassProgram, PackError> {
    let program = assemble_field_glass(package)?;
    program.validate()?;
    Ok(program)
}

/// Prepare-only response/material assembly; see [`assemble_independent_glass`].
pub(crate) fn assemble_field_glass(
    package: &FieldGlassPackage<'_>,
) -> Result<MotionGlassProgram, PackError> {
    if package.members.is_empty() {
        return Err(PackError::FieldEmpty);
    }
    if package.members.len() > valle_draw::program::glass::MAX_GLASS_MEMBERS {
        return Err(PackError::FieldTooLarge(package.members.len()));
    }
    if !(0.08..=1.20).contains(&package.settle) {
        return Err(PackError::SettleOutOfRange(package.settle));
    }
    if !(1.0..=128.0).contains(&package.merge_distance) || !package.merge_distance.is_finite() {
        return Err(PackError::MergeOutOfRange(package.merge_distance));
    }

    let mut ordered = package.members.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.spec.surface_id.cmp(&right.spec.surface_id));
    let mut member_ids = Vec::with_capacity(ordered.len());
    let mut surfaces = Vec::with_capacity(ordered.len());
    let mut output_bounds: Option<Rect> = None;

    for member in &ordered {
        if member.shapes.is_empty() {
            return Err(PackError::FieldMemberEmpty(member.spec.surface_id.clone()));
        }
        if member
            .shapes
            .iter()
            .any(|shape| shape.key.surface_id != member.spec.surface_id)
        {
            return Err(PackError::FieldMemberMismatch(
                member.spec.surface_id.clone(),
            ));
        }
        if let Some(previous) = member_ids.last()
            && previous == &member.spec.surface_id
        {
            return Err(PackError::FieldDuplicateMember(
                member.spec.surface_id.clone(),
            ));
        }
        member_ids.push(member.spec.surface_id.clone());

        let last = member.shapes.last().expect("non-empty");
        if !last.presence.is_finite() || !last.intensity.is_finite() {
            return Err(PackError::NonFiniteValue(member.shapes.len() - 1));
        }
        let last_time = last.time.composition().as_f64();
        let history = member
            .shapes
            .iter()
            .map(|shape| {
                (
                    shape.time.composition().as_f64() - last_time,
                    response_force(
                        shape.kinematics,
                        [
                            f64::from(shape.drive[0]),
                            f64::from(shape.drive[1]),
                            f64::from(shape.drive[2]),
                            f64::from(shape.drive[3]),
                        ],
                        f64::from(shape.intensity),
                    ),
                )
            })
            .collect::<Vec<_>>();
        let epoch_reset = member.shapes.len() >= 2
            && member.shapes[member.shapes.len() - 1].key.epoch
                != member.shapes[member.shapes.len() - 2].key.epoch;
        let response =
            packed_current_response(package.character, package.settle, &history, epoch_reset);
        output_bounds = Some(match output_bounds {
            Some(current) => union(&current, &last.rect),
            None => last.rect,
        });
        surfaces.push(PackedGlassSurface {
            surface_id: member.spec.surface_id.clone(),
            shape: member.spec.shape,
            rect: last.rect,
            local_to_owner: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            radius: last.radius,
            presence: last.presence,
            response,
            path_points: member.path_points.to_vec(),
            foreground_tone: PackedGlassForegroundTone::None,
            foreground_protection: 0.0,
            foreground_bounds: None,
            foreground_luma: None,
        });
    }

    let material = pack_material_base(&package.material);
    // `potential_w` has support for one effective merge radius around each member. That support
    // is visible material/shadow output, not merely a texture sampling footprint.
    let output_bounds = outset(&output_bounds.expect("non-empty"), package.merge_distance);
    let sample_bounds = outset(
        &output_bounds,
        motion_glass_sample_margin(&material, &surfaces, true),
    );
    Ok(MotionGlassProgram {
        owner_kind: GlassOwnerKind::Field,
        owner_id: package.field_id.into(),
        surfaces,
        field: Some(PackedGlassField {
            field_id: package.field_id.into(),
            merge_distance: package.merge_distance as f32,
            member_ids,
        }),
        material,
        backdrop: BackdropUse {
            scope: valle_draw::program::BackdropScope::ScopeEntry(package.field_id.into()),
            sample_bounds,
            output_bounds,
        },
        kernel_id: MOTION_GLASS_KERNEL_ID.into(),
        kernel_digest: motion_glass_kernel_digest(),
        schema_digest: schema_digest(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use valle_draw::program::glass::PackedGlassMotion;
    use valle_timeline::internal::{
        DiscontinuityIndex, FrameKey, FrameRate, MotionInstanceId, SampleTime, TimeMapSegment,
        TrackEpoch,
    };

    use super::super::track::GlassSurfaceKey;
    use super::super::{GlassKinematics, resolve_material_base};

    fn shape(frame: i64, epoch: u32, velocity: [f64; 2]) -> GlassDeviceShape {
        GlassDeviceShape {
            key: GlassSurfaceKey {
                surface_id: "lens".into(),
                instance: MotionInstanceId::new("clip-a").unwrap(),
                epoch: TrackEpoch::new(
                    1,
                    MotionInstanceId::new("clip-a").unwrap(),
                    TimeMapSegment::new(0),
                    0,
                    DiscontinuityIndex::new(epoch),
                ),
            },
            time: SampleTime::from_frame(FrameKey::new(frame), FrameRate::new(30, 1).unwrap())
                .unwrap(),
            rect: Rect::new(100.0, 100.0, 80.0, 80.0),
            matrix: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            radius: 40.0,
            presence: 1.0,
            intensity: 0.6,
            drive: [0.0; 4],
            kinematics: GlassKinematics {
                linear_velocity: velocity,
                ..GlassKinematics::ZERO
            },
        }
    }

    fn shape_named(id: &str, frame: i64, epoch: u32, velocity: [f64; 2]) -> GlassDeviceShape {
        let mut shape = shape(frame, epoch, velocity);
        shape.key.surface_id = id.into();
        shape
    }

    fn package<'a>(
        spec: &'a GlassSurfaceSpec,
        shapes: &'a [GlassDeviceShape],
    ) -> IndependentGlassPackage<'a> {
        IndependentGlassPackage {
            spec,
            shapes,
            material: resolve_material_base(0.82, 0.52, [0.0; 4], 1.0, 0.65).unwrap(),
            character: CharacterKernel::Fluid,
            settle: 0.32,
            path_points: vec![],
        }
    }

    fn field_package<'a>(members: &'a [FieldMemberPackage<'a>]) -> FieldGlassPackage<'a> {
        FieldGlassPackage {
            field_id: "orbit",
            members,
            material: resolve_material_base(0.82, 0.52, [0.0; 4], 1.0, 0.65).unwrap(),
            character: CharacterKernel::Fluid,
            settle: 0.32,
            merge_distance: 24.0,
        }
    }

    fn member<'a>(
        spec: &'a GlassSurfaceSpec,
        shapes: &'a [GlassDeviceShape],
    ) -> FieldMemberPackage<'a> {
        FieldMemberPackage {
            spec,
            shapes,
            path_points: &[],
        }
    }

    fn circle_spec(id: &str) -> GlassSurfaceSpec {
        GlassSurfaceSpec {
            surface_id: id.into(),
            shape: valle_draw::program::glass::PackedGlassShapeKind::Circle,
            radius: 40.0,
        }
    }

    #[test]
    fn field_packs_single_owner_program_with_sorted_members() {
        let left = vec![
            shape_named("left", 0, 0, [0.0, 0.0]),
            shape_named("left", 1, 0, [0.0, 0.0]),
        ];
        let right = vec![
            shape_named("right", 0, 0, [0.0, 0.0]),
            shape_named("right", 1, 0, [0.0, 0.0]),
        ];
        let right_spec = circle_spec("right");
        let left_spec = circle_spec("left");
        let members = vec![member(&right_spec, &right), member(&left_spec, &left)];
        let program = pack_field_glass(&field_package(&members)).unwrap();
        program.validate().unwrap();
        assert_eq!(program.owner_kind, GlassOwnerKind::Field);
        assert_eq!(program.owner_id, "orbit");
        assert_eq!(program.surfaces.len(), 2);
        assert_eq!(program.surfaces[0].surface_id, "left");
        assert_eq!(program.surfaces[1].surface_id, "right");
        let field = program.field.as_ref().unwrap();
        assert_eq!(field.field_id, "orbit");
        assert_eq!(
            field.member_ids,
            vec!["left".to_string(), "right".to_string()]
        );
    }

    #[test]
    fn field_merged_bounds_contain_all_member_rects() {
        let mut left_shape = shape_named("left", 0, 0, [0.0, 0.0]);
        left_shape.rect = Rect::new(10.0, 10.0, 40.0, 40.0);
        left_shape.radius = 20.0;
        let mut right_shape = shape_named("right", 1, 0, [0.0, 0.0]);
        right_shape.rect = Rect::new(200.0, 120.0, 40.0, 40.0);
        right_shape.radius = 20.0;
        let left_spec = circle_spec("left");
        let right_spec = circle_spec("right");
        let left_shapes = vec![left_shape];
        let right_shapes = vec![right_shape];
        let members = vec![
            member(&left_spec, &left_shapes),
            member(&right_spec, &right_shapes),
        ];
        let program = pack_field_glass(&field_package(&members)).unwrap();
        let output = program.backdrop.output_bounds;
        let sample = program.backdrop.sample_bounds;
        assert!(output.left() <= 10.0 && output.top() <= 10.0);
        assert!(output.right() >= 240.0 && output.bottom() >= 160.0);
        assert!(sample.left() <= output.left() && sample.right() >= output.right());
    }

    #[test]
    fn field_member_order_does_not_change_packing() {
        let left = vec![
            shape_named("left", 0, 0, [0.0, 0.0]),
            shape_named("left", 1, 0, [0.0, 0.0]),
        ];
        let right = vec![
            shape_named("right", 0, 0, [0.0, 0.0]),
            shape_named("right", 1, 0, [0.0, 0.0]),
        ];
        let right_spec = circle_spec("right");
        let left_spec = circle_spec("left");
        let mut first = vec![member(&right_spec, &right), member(&left_spec, &left)];
        let a = pack_field_glass(&field_package(&first)).unwrap();
        first.reverse();
        let b = pack_field_glass(&field_package(&first)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn field_duplicate_member_fails_closed() {
        let shapes = vec![shape_named("left", 0, 0, [0.0, 0.0])];
        let left_spec = circle_spec("left");
        let members = vec![member(&left_spec, &shapes), member(&left_spec, &shapes)];
        assert!(matches!(
            pack_field_glass(&field_package(&members)),
            Err(PackError::FieldDuplicateMember(_))
        ));
    }

    #[test]
    fn empty_field_fails_closed() {
        assert!(matches!(
            pack_field_glass(&field_package(&[])),
            Err(PackError::FieldEmpty)
        ));
    }

    #[test]
    fn merge_out_of_range_fails_closed() {
        let shapes = vec![shape(0, 0, [0.0, 0.0])];
        let left_spec = circle_spec("left");
        let members = vec![member(&left_spec, &shapes)];
        let mut package = field_package(&members);
        package.merge_distance = 0.0;
        assert!(matches!(
            pack_field_glass(&package),
            Err(PackError::MergeOutOfRange(_))
        ));
        package.merge_distance = 200.0;
        assert!(matches!(
            pack_field_glass(&package),
            Err(PackError::MergeOutOfRange(_))
        ));
    }

    fn spec() -> GlassSurfaceSpec {
        GlassSurfaceSpec {
            surface_id: "lens".into(),
            shape: valle_draw::program::glass::PackedGlassShapeKind::Circle,
            radius: 50.0,
        }
    }

    #[test]
    fn moving_surface_packs_a_valid_current_program() {
        let spec = spec();
        // A long constant-motion track so the response reaches steady state.
        let shapes = (0..=12)
            .map(|frame| shape(i64::from(frame), 0, [3000.0, 0.0]))
            .collect::<Vec<_>>();
        let program = pack_independent_glass(&package(&spec, &shapes)).unwrap();
        program.validate().unwrap();
        assert_eq!(program.owner_kind, GlassOwnerKind::Independent);
        assert_eq!(program.surfaces.len(), 1);
        assert_eq!(program.backdrop.scope, BACKDROP_SCOPE_CURRENT);
        assert!(program.surfaces[0].response.translation[0] > 0.0);
        // sample bounds contain output bounds
        assert!(program.backdrop.sample_bounds.left() <= program.backdrop.output_bounds.left());
        assert!(program.backdrop.sample_bounds.right() >= program.backdrop.output_bounds.right());
        // The typed program is validated here; only the enclosing DrawProgram is packetized.
        program.validate().unwrap();
    }

    #[test]
    fn independent_output_contains_the_complete_outer_shadow() {
        let spec = spec();
        let shapes = vec![shape(0, 0, [0.0, 0.0])];
        let program = pack_independent_glass(&package(&spec, &shapes)).unwrap();
        let radius = glass_independent_output_margin(&program.material);
        let rect = shapes[0].rect;
        let output = program.backdrop.output_bounds;
        assert!((output.left() - (rect.left() - radius)).abs() < 1.0e-9);
        assert!((output.top() - (rect.top() - radius)).abs() < 1.0e-9);
        assert!((output.right() - (rect.right() + radius)).abs() < 1.0e-9);
        assert!((output.bottom() - (rect.bottom() + radius)).abs() < 1.0e-9);
    }

    #[test]
    fn extreme_motion_sample_margin_covers_shader_clamp_and_filter_footprint() {
        let spec = spec();
        let shapes = vec![shape(0, 0, [0.0, 0.0])];
        let mut program = pack_independent_glass(&package(&spec, &shapes)).unwrap();
        program.surfaces[0].response.acceleration = [f32::MAX, f32::MAX];
        let margin = motion_glass_sample_margin(&program.material, &program.surfaces, false);
        let displacement = f64::from(program.material.bevel_width) * MAX_REFRACTION_BEVEL_FRACTION;
        let filter_footprint = diffusion_radius(
            f64::from(program.material.roughness),
            f64::from(program.material.thickness),
        ) * 2.0;
        let expected = displacement + filter_footprint + 1.0;
        assert!((margin - expected).abs() < 1.0e-6, "{margin} != {expected}");
    }

    #[test]
    fn independent_output_keeps_antialiasing_when_shadow_is_disabled() {
        let spec = spec();
        let shapes = vec![shape(0, 0, [0.0, 0.0])];
        let mut program = assemble_independent_glass(&package(&spec, &shapes)).unwrap();
        program.material.shadow_strength = 0.0;
        let rect = program.surfaces[0].rect;
        let margin = glass_independent_output_margin(&program.material);
        program.backdrop.output_bounds = outset(&rect, margin);
        program.backdrop.sample_bounds = outset(
            &program.backdrop.output_bounds,
            motion_glass_sample_margin(&program.material, &program.surfaces, false),
        );
        assert_eq!(margin, 0.5);
        assert!((program.backdrop.output_bounds.left() - (rect.left() - 0.5)).abs() < 1.0e-9);
        program.validate().unwrap();
    }

    #[test]
    fn rest_surface_packs_zero_response() {
        let spec = spec();
        let shapes = (0..=12)
            .map(|frame| shape(i64::from(frame), 0, [0.0, 0.0]))
            .collect::<Vec<_>>();
        let program = pack_independent_glass(&package(&spec, &shapes)).unwrap();
        assert_eq!(program.surfaces[0].response, PackedGlassMotion::ZERO);
    }

    #[test]
    fn static_surface_needs_no_track_history() {
        // G4.2: rest kinematics + zero drive => requires_track_history is false and the
        // packed response is zero, so the caller can sample one point instead of a history.
        let spec = spec();
        let shapes = (0..=12)
            .map(|frame| shape(i64::from(frame), 0, [0.0, 0.0]))
            .collect::<Vec<_>>();
        let package = package(&spec, &shapes);
        assert!(!package.requires_track_history());
        let program = pack_independent_glass(&package).unwrap();
        assert_eq!(program.surfaces[0].response, PackedGlassMotion::ZERO);
    }

    #[test]
    fn moving_or_driven_surface_requires_track_history() {
        let spec = spec();
        let moving = (0..=12)
            .map(|frame| shape(i64::from(frame), 0, [3000.0, 0.0]))
            .collect::<Vec<_>>();
        assert!(package(&spec, &moving).requires_track_history());
        let mut driven = (0..=12)
            .map(|frame| shape(i64::from(frame), 0, [0.0, 0.0]))
            .collect::<Vec<_>>();
        for shape in driven.iter_mut() {
            shape.drive = [5.0, 0.0, 0.0, 0.0];
        }
        assert!(package(&spec, &driven).requires_track_history());
    }

    #[test]
    fn epoch_crossing_zeroes_the_current_response() {
        let spec = spec();
        let mut shapes = (0..=12)
            .map(|frame| shape(i64::from(frame), 0, [3000.0, 0.0]))
            .collect::<Vec<_>>();
        let last = shapes.len() - 1;
        shapes[last] = shape(12, 1, [3000.0, 0.0]);
        let program = pack_independent_glass(&package(&spec, &shapes)).unwrap();
        assert_eq!(program.surfaces[0].response, PackedGlassMotion::ZERO);
    }

    #[test]
    fn empty_package_fails_closed() {
        let spec = spec();
        assert!(matches!(
            pack_independent_glass(&package(&spec, &[])),
            Err(PackError::Empty)
        ));
    }

    #[test]
    fn settle_out_of_range_fails_closed() {
        let spec = spec();
        let shapes = vec![shape(0, 0, [0.0, 0.0])];
        let mut package = package(&spec, &shapes);
        package.settle = 2.0;
        assert!(matches!(
            pack_independent_glass(&package),
            Err(PackError::SettleOutOfRange(_))
        ));
    }

    #[test]
    fn drive_translation_is_part_of_the_typed_force() {
        let spec = spec();
        let mut shapes = (0..=12)
            .map(|frame| shape(i64::from(frame), 0, [0.0, 0.0]))
            .collect::<Vec<_>>();
        for shape in shapes.iter_mut() {
            shape.drive = [5.0, 0.0, 0.0, 0.0];
        }
        let program = pack_independent_glass(&package(&spec, &shapes)).unwrap();
        assert!(program.surfaces[0].response.translation[0] > 0.0);
    }
}
