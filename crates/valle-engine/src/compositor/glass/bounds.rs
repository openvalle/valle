//! Authoritative Motion Glass output and backdrop-sampling bounds.
//!
//! Shape geometry is stored in owner space, while shadow and optical sampling margins are device
//! pixels. This module is the single place that crosses those spaces. Prepare writes the result
//! into the typed program; Native/Web admission independently re-derives it before execution.

use thiserror::Error;
use valle_draw::Rect;
use valle_draw::program::glass::MotionGlassProgram;

use super::pack::{glass_independent_output_margin, motion_glass_sample_margin};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionGlassBackdropBounds {
    /// Geometry plus Field support, before device-space independent shadow expansion.
    pub field_support_owner: Rect,
    /// Conservative owner-space bounds for every visible material/shadow pixel.
    pub output_owner: Rect,
    /// Conservative owner-space bounds for every backdrop texture sample.
    pub sample_owner: Rect,
    pub(crate) device_to_owner: [f64; 9],
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MotionGlassBoundsError {
    #[error("Motion Glass owner has no surfaces")]
    EmptyOwner,
    #[error("Motion Glass surface '{0}' has an invalid owner transform")]
    InvalidSurfaceTransform(String),
    #[error("Motion Glass owner transform is singular or crosses a projective horizon")]
    InvalidOwnerTransform,
    #[error("Motion Glass output bounds do not cover the Field merge support")]
    InsufficientFieldSupport,
    #[error("Motion Glass output bounds do not cover every visible material pixel")]
    InsufficientOutput,
    #[error("Motion Glass sample bounds do not cover the optical texture footprint")]
    InsufficientSample,
}

pub fn validate_motion_glass_backdrop_bounds(
    program: &MotionGlassProgram,
    owner_to_device: [f64; 9],
) -> Result<MotionGlassBackdropBounds, MotionGlassBoundsError> {
    transformed_rect_bounds(program.backdrop.sample_bounds, owner_to_device)
        .ok_or(MotionGlassBoundsError::InvalidOwnerTransform)?;
    let required = resolve_motion_glass_backdrop_bounds(program, owner_to_device)?;
    if program.field.is_some()
        && !rect_contains(program.backdrop.output_bounds, required.field_support_owner)
    {
        return Err(MotionGlassBoundsError::InsufficientFieldSupport);
    }
    if !rect_contains(program.backdrop.output_bounds, required.output_owner) {
        return Err(MotionGlassBoundsError::InsufficientOutput);
    }
    if !rect_contains(program.backdrop.sample_bounds, required.sample_owner) {
        return Err(MotionGlassBoundsError::InsufficientSample);
    }
    Ok(required)
}

pub fn resolve_motion_glass_backdrop_bounds(
    program: &MotionGlassProgram,
    owner_to_device: [f64; 9],
) -> Result<MotionGlassBackdropBounds, MotionGlassBoundsError> {
    let device_to_owner =
        matrix_inverse(owner_to_device).ok_or(MotionGlassBoundsError::InvalidOwnerTransform)?;
    let mut geometry_owner = None;
    for surface in &program.surfaces {
        let bounds = transformed_rect_bounds(surface.rect, surface.local_to_owner.map(f64::from))
            .ok_or_else(|| {
            MotionGlassBoundsError::InvalidSurfaceTransform(surface.surface_id.clone())
        })?;
        geometry_owner = Some(match geometry_owner {
            Some(current) => union_rect(current, bounds),
            None => bounds,
        });
    }
    let geometry_owner = geometry_owner.ok_or(MotionGlassBoundsError::EmptyOwner)?;
    let field_support_owner = program.field.as_ref().map_or(geometry_owner, |field| {
        outset_rect(geometry_owner, f64::from(field.merge_distance))
    });

    let mut output_device = transformed_rect_bounds(field_support_owner, owner_to_device)
        .ok_or(MotionGlassBoundsError::InvalidOwnerTransform)?;
    if program.field.is_none() {
        output_device = outset_rect(
            output_device,
            glass_independent_output_margin(&program.material),
        );
    }
    let projected_output_owner = transformed_rect_bounds(output_device, device_to_owner)
        .ok_or(MotionGlassBoundsError::InvalidOwnerTransform)?;
    let output_owner = union_rect(field_support_owner, projected_output_owner);

    let sample_device = outset_rect(
        output_device,
        motion_glass_sample_margin(
            &program.material,
            &program.surfaces,
            program.field.is_some(),
        ),
    );
    let projected_sample_owner = transformed_rect_bounds(sample_device, device_to_owner)
        .ok_or(MotionGlassBoundsError::InvalidOwnerTransform)?;
    let sample_owner = union_rect(output_owner, projected_sample_owner);

    Ok(MotionGlassBackdropBounds {
        field_support_owner,
        output_owner,
        sample_owner,
        device_to_owner,
    })
}

fn outset_rect(rect: Rect, margin: f64) -> Rect {
    Rect::from_edges(
        rect.left() - margin,
        rect.top() - margin,
        rect.right() + margin,
        rect.bottom() + margin,
    )
}

fn union_rect(left: Rect, right: Rect) -> Rect {
    Rect::from_edges(
        left.left().min(right.left()),
        left.top().min(right.top()),
        left.right().max(right.right()),
        left.bottom().max(right.bottom()),
    )
}

fn rect_contains(outer: Rect, inner: Rect) -> bool {
    let scale = [
        outer.left(),
        outer.top(),
        outer.right(),
        outer.bottom(),
        inner.left(),
        inner.top(),
        inner.right(),
        inner.bottom(),
    ]
    .into_iter()
    .map(f64::abs)
    .fold(1.0, f64::max);
    let tolerance = scale * 1.0e-9;
    outer.left() <= inner.left() + tolerance
        && outer.top() <= inner.top() + tolerance
        && outer.right() + tolerance >= inner.right()
        && outer.bottom() + tolerance >= inner.bottom()
}

fn transformed_rect_bounds(rect: Rect, transform: [f64; 9]) -> Option<Rect> {
    if rect.is_empty() || !rect_has_no_horizon(rect, transform) {
        return None;
    }
    let points = [
        project(transform, [rect.left(), rect.top()])?,
        project(transform, [rect.right(), rect.top()])?,
        project(transform, [rect.right(), rect.bottom()])?,
        project(transform, [rect.left(), rect.bottom()])?,
    ];
    Some(Rect::from_edges(
        points
            .iter()
            .map(|point| point[0])
            .fold(f64::INFINITY, f64::min),
        points
            .iter()
            .map(|point| point[1])
            .fold(f64::INFINITY, f64::min),
        points
            .iter()
            .map(|point| point[0])
            .fold(f64::NEG_INFINITY, f64::max),
        points
            .iter()
            .map(|point| point[1])
            .fold(f64::NEG_INFINITY, f64::max),
    ))
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

fn project(matrix: [f64; 9], point: [f64; 2]) -> Option<[f64; 2]> {
    let denominator = matrix[6] * point[0] + matrix[7] * point[1] + matrix[8];
    if !denominator.is_finite() || denominator.abs() <= 1.0e-12 {
        return None;
    }
    let output = [
        (matrix[0] * point[0] + matrix[1] * point[1] + matrix[2]) / denominator,
        (matrix[3] * point[0] + matrix[4] * point[1] + matrix[5]) / denominator,
    ];
    output.into_iter().all(f64::is_finite).then_some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projective_device_margin_round_trips_to_a_conservative_owner_rect() {
        let owner = Rect::new(20.0, 10.0, 180.0, 90.0);
        let owner_to_device = [1.2, 0.1, 8.0, -0.05, 0.9, 12.0, 0.001, -0.0004, 1.0];
        let device_to_owner = matrix_inverse(owner_to_device).unwrap();
        let device = transformed_rect_bounds(owner, owner_to_device).unwrap();
        let expanded_device = outset_rect(device, 64.0);
        let expanded_owner = transformed_rect_bounds(expanded_device, device_to_owner).unwrap();
        let projected_again = transformed_rect_bounds(expanded_owner, owner_to_device).unwrap();
        let epsilon = 1e-8;
        assert!(projected_again.left() <= expanded_device.left() + epsilon);
        assert!(projected_again.top() <= expanded_device.top() + epsilon);
        assert!(projected_again.right() + epsilon >= expanded_device.right());
        assert!(projected_again.bottom() + epsilon >= expanded_device.bottom());
    }
}
