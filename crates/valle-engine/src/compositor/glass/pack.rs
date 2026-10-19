//! G1.3/G1.4 backend-neutral packing: one independent Glass device shape → `MotionGlassProgram`.
//!
//! The packer consumes only typed inputs — the device shape and the resolved material base — and
//! emits a validated `MotionGlassProgram` with `BackdropScope::Current`. No executor-side
//! interpretation of Artifact intent, no per-backend policy, and no shader source crosses this
//! boundary.

use thiserror::Error;
use valle_draw::Rect;
use valle_draw::program::glass::{
    BackdropUse, GlassOwnerKind, MOTION_GLASS_KERNEL_ID, MotionGlassProgram, PackedGlassField,
    PackedGlassSurface, motion_glass_kernel_digest, schema_digest,
};

use super::material::{
    MAX_REFRACTION_BEVEL_FRACTION, ResolvedGlassMaterialBase, diffusion_radius, pack_material_base,
};
use super::track::{GlassDeviceShape, GlassSurfaceSpec};

/// `BackdropScope::Current` — G1 programs read only the current backdrop. Canonical definition
/// lives in the ABI owner (`valle-draw`).
pub use valle_draw::program::glass::BACKDROP_SCOPE_CURRENT;

/// Complete independent-surface output outset in device pixels. Coverage is a one-pixel C2 edge
/// centered on the analytic boundary, so it remains visible for half a pixel outside it.
pub const GLASS_INDEPENDENT_OUTPUT_MARGIN: f64 = 0.5;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum PackError {
    #[error("device shape has a non-finite presence")]
    NonFinitePresence,
    #[error("Glass program validation failed: {0}")]
    InvalidProgram(#[from] valle_draw::program::glass::MotionGlassError),
    #[error("GlassField has no members")]
    FieldEmpty,
    #[error("GlassField exceeds {0} members")]
    FieldTooLarge(usize),
    #[error("GlassField member ids must be unique and sorted, got duplicate at {0}")]
    FieldDuplicateMember(String),
    #[error("merge distance {0} is outside [1.0, 128.0]")]
    MergeOutOfRange(f64),
}

/// Everything needed to pack one independent Glass surface.
pub struct IndependentGlassPackage<'a> {
    pub spec: &'a GlassSurfaceSpec,
    pub shape: &'a GlassDeviceShape,
    pub material: ResolvedGlassMaterialBase,
    /// G4.1 closed-polygon path points (device pixels); empty for analytic shapes.
    pub path_points: Vec<[f32; 2]>,
}

/// One member's device shape inside a field package.
pub struct FieldMemberPackage<'a> {
    pub spec: &'a GlassSurfaceSpec,
    pub shape: &'a GlassDeviceShape,
    pub path_points: &'a [[f32; 2]],
}

/// G2.2 field package: all members share one material and one merge distance; the field packs
/// exactly one owner program.
pub struct FieldGlassPackage<'a> {
    pub field_id: &'a str,
    pub members: &'a [FieldMemberPackage<'a>],
    pub material: ResolvedGlassMaterialBase,
    pub merge_distance: f64,
}

/// Packs the device shape into a validated independent `MotionGlassProgram`.
///
/// - `presence` uses the same materialization math via the resolved material base;
/// - sample bounds outset the output bounds by the material's optical margins.
pub fn pack_independent_glass(
    package: &IndependentGlassPackage<'_>,
) -> Result<MotionGlassProgram, PackError> {
    let program = assemble_independent_glass(package)?;
    program.validate()?;
    Ok(program)
}

/// Prepare-only first phase. The device shape resolves the material, but a projective surface
/// cannot be represented faithfully as an identity-space analytic shape. Product prepare
/// therefore assembles the program first, replaces geometry with the authoritative owner-local
/// homography, and validates exactly once before lowering.
pub(crate) fn assemble_independent_glass(
    package: &IndependentGlassPackage<'_>,
) -> Result<MotionGlassProgram, PackError> {
    let shape = package.shape;
    if !shape.presence.is_finite() {
        return Err(PackError::NonFinitePresence);
    }
    let material = pack_material_base(&package.material);
    let surface = PackedGlassSurface {
        surface_id: package.spec.surface_id.clone(),
        shape: package.spec.shape,
        rect: shape.rect,
        local_to_owner: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        radius: shape.radius,
        presence: shape.presence,
        path_points: package.path_points.clone(),
    };

    let output_bounds = outset(&shape.rect, GLASS_INDEPENDENT_OUTPUT_MARGIN);
    let sample_bounds = outset(&output_bounds, motion_glass_sample_margin(&material));
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

/// Conservative texture footprint in device pixels.
///
/// The refraction displacement is bounded to a fraction of the material bevel. The five-tap
/// diffusion footprint reaches 1.3 blur radii along each axis beyond the refracted coordinate,
/// counted as two radii; one bilinear support pixel is added after both terms. Every optical
/// length therefore scales with the resolved device-space material instead of changing
/// appearance at a fixed resolution threshold.
pub fn motion_glass_sample_margin(
    material: &valle_draw::program::glass::PackedGlassMaterial,
) -> f64 {
    let bevel_width = f64::from(material.bevel_width).max(0.0);
    let thickness = f64::from(material.thickness).max(0.0);
    let displacement = bevel_width * MAX_REFRACTION_BEVEL_FRACTION;
    let diffusion = diffusion_radius(f64::from(material.roughness), thickness) * 2.0;
    displacement + diffusion + 1.0
}

/// Packs one GlassField into a single validated owner `MotionGlassProgram`.
///
/// - one field = one owner program (ScopeEntry semantics);
/// - members share the field material;
/// - member ids are sorted and unique;
/// - sample bounds are the union of member optical margins, output bounds the union of
///   member rects;
/// - the render kernel performs the potential-weighted spatial mix at each pixel.
pub fn pack_field_glass(package: &FieldGlassPackage<'_>) -> Result<MotionGlassProgram, PackError> {
    let program = assemble_field_glass(package)?;
    program.validate()?;
    Ok(program)
}

/// Prepare-only material assembly; see [`assemble_independent_glass`].
pub(crate) fn assemble_field_glass(
    package: &FieldGlassPackage<'_>,
) -> Result<MotionGlassProgram, PackError> {
    if package.members.is_empty() {
        return Err(PackError::FieldEmpty);
    }
    if package.members.len() > valle_draw::program::glass::MAX_GLASS_MEMBERS {
        return Err(PackError::FieldTooLarge(package.members.len()));
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
        if let Some(previous) = member_ids.last()
            && previous == &member.spec.surface_id
        {
            return Err(PackError::FieldDuplicateMember(
                member.spec.surface_id.clone(),
            ));
        }
        member_ids.push(member.spec.surface_id.clone());

        let shape = member.shape;
        if !shape.presence.is_finite() {
            return Err(PackError::NonFinitePresence);
        }
        output_bounds = Some(match output_bounds {
            Some(current) => union(&current, &shape.rect),
            None => shape.rect,
        });
        surfaces.push(PackedGlassSurface {
            surface_id: member.spec.surface_id.clone(),
            shape: member.spec.shape,
            rect: shape.rect,
            local_to_owner: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            radius: shape.radius,
            presence: shape.presence,
            path_points: member.path_points.to_vec(),
        });
    }

    let material = pack_material_base(&package.material);
    // `potential_w` has support for one effective merge radius around each member. That support
    // is visible material output, not merely a texture sampling footprint.
    let output_bounds = outset(&output_bounds.expect("non-empty"), package.merge_distance);
    let sample_bounds = outset(&output_bounds, motion_glass_sample_margin(&material));
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

    use super::super::resolve_material_base;

    fn shape() -> GlassDeviceShape {
        GlassDeviceShape {
            rect: Rect::new(100.0, 100.0, 80.0, 80.0),
            radius: 40.0,
            presence: 1.0,
        }
    }

    fn package<'a>(
        spec: &'a GlassSurfaceSpec,
        shape: &'a GlassDeviceShape,
    ) -> IndependentGlassPackage<'a> {
        IndependentGlassPackage {
            spec,
            shape,
            material: resolve_material_base(0.82, 0.52, [0.0; 4], 1.0).unwrap(),
            path_points: vec![],
        }
    }

    fn field_package<'a>(members: &'a [FieldMemberPackage<'a>]) -> FieldGlassPackage<'a> {
        FieldGlassPackage {
            field_id: "orbit",
            members,
            material: resolve_material_base(0.82, 0.52, [0.0; 4], 1.0).unwrap(),
            merge_distance: 24.0,
        }
    }

    fn member<'a>(
        spec: &'a GlassSurfaceSpec,
        shape: &'a GlassDeviceShape,
    ) -> FieldMemberPackage<'a> {
        FieldMemberPackage {
            spec,
            shape,
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
        let (left, right) = (shape(), shape());
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
        let mut left_shape = shape();
        left_shape.rect = Rect::new(10.0, 10.0, 40.0, 40.0);
        left_shape.radius = 20.0;
        let mut right_shape = shape();
        right_shape.rect = Rect::new(200.0, 120.0, 40.0, 40.0);
        right_shape.radius = 20.0;
        let left_spec = circle_spec("left");
        let right_spec = circle_spec("right");
        let members = vec![
            member(&left_spec, &left_shape),
            member(&right_spec, &right_shape),
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
        let (left, right) = (shape(), shape());
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
        let shape = shape();
        let left_spec = circle_spec("left");
        let members = vec![member(&left_spec, &shape), member(&left_spec, &shape)];
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
        let shape = shape();
        let left_spec = circle_spec("left");
        let members = vec![member(&left_spec, &shape)];
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
    fn surface_packs_a_valid_current_program() {
        let spec = spec();
        let shape = shape();
        let program = pack_independent_glass(&package(&spec, &shape)).unwrap();
        program.validate().unwrap();
        assert_eq!(program.owner_kind, GlassOwnerKind::Independent);
        assert_eq!(program.surfaces.len(), 1);
        assert_eq!(program.backdrop.scope, BACKDROP_SCOPE_CURRENT);
        // sample bounds contain output bounds
        assert!(program.backdrop.sample_bounds.left() <= program.backdrop.output_bounds.left());
        assert!(program.backdrop.sample_bounds.right() >= program.backdrop.output_bounds.right());
    }

    #[test]
    fn independent_output_keeps_the_antialiased_edge() {
        let spec = spec();
        let shape = shape();
        let program = pack_independent_glass(&package(&spec, &shape)).unwrap();
        let rect = shape.rect;
        let output = program.backdrop.output_bounds;
        assert!((output.left() - (rect.left() - 0.5)).abs() < 1.0e-9);
        assert!((output.top() - (rect.top() - 0.5)).abs() < 1.0e-9);
        assert!((output.right() - (rect.right() + 0.5)).abs() < 1.0e-9);
        assert!((output.bottom() - (rect.bottom() + 0.5)).abs() < 1.0e-9);
    }

    #[test]
    fn sample_margin_covers_the_refraction_clamp_and_filter_footprint() {
        let spec = spec();
        let shape = shape();
        let program = pack_independent_glass(&package(&spec, &shape)).unwrap();
        let margin = motion_glass_sample_margin(&program.material);
        let displacement = f64::from(program.material.bevel_width) * MAX_REFRACTION_BEVEL_FRACTION;
        let filter_footprint = diffusion_radius(
            f64::from(program.material.roughness),
            f64::from(program.material.thickness),
        ) * 2.0;
        let expected = displacement + filter_footprint + 1.0;
        assert!((margin - expected).abs() < 1.0e-6, "{margin} != {expected}");
    }

    #[test]
    fn non_finite_presence_fails_closed() {
        let spec = spec();
        let mut shape = shape();
        shape.presence = f32::NAN;
        assert!(matches!(
            pack_independent_glass(&package(&spec, &shape)),
            Err(PackError::NonFinitePresence)
        ));
    }
}
