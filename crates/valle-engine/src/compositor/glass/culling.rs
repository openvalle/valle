//! G2.6 field bounds, ROI, and support culling.
//!
//! Each field member contributes only inside its own compact potential support (the shape
//! outset by `mergeDistance`). `member_cull_bounds` yields those device-space bounds; the
//! field sample bounds are their union. Culling is a bounds-level optimization: disabling it
//! (evaluating every member everywhere) must not change pixels.

use thiserror::Error;
use valle_draw::Rect;
use valle_draw::program::glass::{GlassOwnerKind, MotionGlassProgram};

use super::render::field_sample;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CullError {
    #[error("field culling requires a field program")]
    NotField,
    #[error("field culling requires a merge payload")]
    MissingMerge,
}

/// Device-space potential support bounds of one member: its box outset by `mergeDistance`
/// (the compact C2 support of W).
pub fn member_cull_bounds(program: &MotionGlassProgram, index: usize) -> Result<Rect, CullError> {
    if program.owner_kind != GlassOwnerKind::Field {
        return Err(CullError::NotField);
    }
    let field = program.field.as_ref().ok_or(CullError::MissingMerge)?;
    let surface = program.surfaces.get(index).ok_or(CullError::MissingMerge)?;
    let merge = f64::from(field.merge_distance);
    Ok(Rect::from_edges(
        surface.rect.left() - merge,
        surface.rect.top() - merge,
        surface.rect.right() + merge,
        surface.rect.bottom() + merge,
    ))
}

/// Per-member cull bounds for the whole field, in member order.
pub fn field_cull_bounds(program: &MotionGlassProgram) -> Result<Vec<Rect>, CullError> {
    (0..program.surfaces.len())
        .map(|index| member_cull_bounds(program, index))
        .collect()
}

/// True when the point lies inside any member's potential support (the tile-culling predicate
/// used to skip a pixel's member loop).
pub fn in_any_support(program: &MotionGlassProgram, p: [f64; 2]) -> Result<bool, CullError> {
    let bounds = field_cull_bounds(program)?;
    Ok(bounds.iter().any(|rect| {
        p[0] >= rect.left() && p[0] <= rect.right() && p[1] >= rect.top() && p[1] <= rect.bottom()
    }))
}

/// G2.6 culled field sample: potential, coverage, and normal only from members whose support
/// contains the point (identical to evaluating every member, since W is zero outside support).
pub fn field_sample_culled(
    program: &MotionGlassProgram,
    p: [f64; 2],
) -> Option<(f64, f64, [f64; 2])> {
    let field = program.field.as_ref()?;
    if !in_any_support(program, p).ok()? {
        return None;
    }
    let sample = field_sample(program, p, f64::from(field.merge_distance))?;
    Some((sample.density, sample.coverage, sample.normal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::glass::render::field_sample;
    use valle_draw::program::glass::{
        BackdropUse, GlassOwnerKind, MOTION_GLASS_KERNEL_ID, PackedGlassField, PackedGlassMaterial,
        PackedGlassMotion, PackedGlassSurface, motion_glass_kernel_digest, schema_digest,
    };

    fn field_program() -> MotionGlassProgram {
        let make = |id: &str, x: f64| PackedGlassSurface {
            surface_id: id.into(),
            shape: valle_draw::program::glass::PackedGlassShapeKind::Circle,
            rect: Rect::new(x - 32.0, 32.0, 64.0, 64.0),
            local_to_owner: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            radius: 32.0,
            presence: 1.0,
            response: PackedGlassMotion::ZERO,
            path_points: vec![],
            foreground_tone: valle_draw::program::glass::PackedGlassForegroundTone::None,
            foreground_protection: 0.0,
            foreground_bounds: None,
            foreground_luma: None,
        };
        MotionGlassProgram {
            owner_kind: GlassOwnerKind::Field,
            owner_id: "orbit".into(),
            surfaces: vec![make("left", 40.0), make("right", 220.0)],
            field: Some(PackedGlassField {
                field_id: "orbit".into(),
                merge_distance: 24.0,
                member_ids: vec!["left".into(), "right".into()],
            }),
            material: PackedGlassMaterial {
                bevel_width: 8.0,
                thickness: 16.0,
                refractive_index: 1.2,
                roughness: 0.08,
                dispersion: 0.012,
                tint_linear: [0.3, 0.5, 0.9, 0.2],
                specular_strength: 0.5,
                shadow_strength: 0.1,
                foreground_gain: 0.7,
                light: valle_draw::program::PackedGlassLight::DEFAULT,
            },
            backdrop: BackdropUse {
                scope: valle_draw::program::BackdropScope::ScopeEntry("field".into()),
                sample_bounds: Rect::new(0.0, 0.0, 256.0, 128.0),
                output_bounds: Rect::new(0.0, 0.0, 256.0, 128.0),
            },
            kernel_id: MOTION_GLASS_KERNEL_ID.into(),
            kernel_digest: motion_glass_kernel_digest(),
            schema_digest: schema_digest(),
        }
    }

    #[test]
    fn cull_bounds_outset_members_by_merge_distance_and_union_covers_sample() {
        let program = field_program();
        let bounds = field_cull_bounds(&program).unwrap();
        assert_eq!(bounds.len(), 2);
        assert_eq!(
            bounds[0],
            Rect::from_edges(40.0 - 32.0 - 24.0, 8.0, 40.0 + 32.0 + 24.0, 120.0)
        );
        assert_eq!(
            bounds[1],
            Rect::from_edges(220.0 - 32.0 - 24.0, 8.0, 220.0 + 32.0 + 24.0, 120.0)
        );
        // The sample bounds must be contained by the union of cull bounds.
        let sample = program.backdrop.sample_bounds;
        let union = bounds.iter().fold(bounds[0], |acc, rect| {
            Rect::from_edges(
                acc.left().min(rect.left()),
                acc.top().min(rect.top()),
                acc.right().max(rect.right()),
                acc.bottom().max(rect.bottom()),
            )
        });
        assert!(sample.left() >= union.left() - 1.0);
        assert!(sample.right() <= union.right() + 1.0);
    }

    #[test]
    fn culled_sample_matches_unculled_sample_everywhere() {
        let program = field_program();
        for y in 0..128u32 {
            for x in 0..256u32 {
                let p = [f64::from(x) + 0.5, f64::from(y) + 0.5];
                let unculled = field_sample(&program, p, 24.0);
                let culled = field_sample_culled(&program, p);
                match (unculled, culled) {
                    (None, None) => {}
                    (Some(sample), Some((density, coverage, _))) => {
                        assert!(
                            (sample.density - density).abs() < 1e-9,
                            "density mismatch at {p:?}: {} vs {density}",
                            sample.density
                        );
                        assert!(
                            (sample.coverage - coverage).abs() < 1e-9,
                            "coverage mismatch at {p:?}: {} vs {coverage}",
                            sample.coverage
                        );
                    }
                    other => panic!("culled mismatch at {p:?}: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn in_any_support_matches_potential_support() {
        let program = field_program();
        // Midpoint far from both members: no support.
        assert!(!in_any_support(&program, [128.0, 64.0]).unwrap());
        // Inside the left member: supported.
        assert!(in_any_support(&program, [40.0, 64.0]).unwrap());
    }

    #[test]
    fn independent_program_rejected_by_culling() {
        let mut program = field_program();
        program.owner_kind = GlassOwnerKind::Independent;
        program.field = None;
        assert!(matches!(
            field_cull_bounds(&program),
            Err(CullError::NotField)
        ));
    }
}
