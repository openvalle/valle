//! G1.2 device-space Glass surface qualification.
//!
//! Turns one Glass surface at the current frame into its device shape: device-space geometry
//! after the Timeline/camera transform. Every frame is qualified on its own, so the result
//! depends only on that frame's layout and transform.

use thiserror::Error;
use valle_draw::Rect;
use valle_draw::program::glass::PackedGlassShapeKind;

use crate::prepare::DeviceTransform;

/// Static shape description resolved from the Glass Artifact. `radius` is in the normalized
/// box units that `DeviceTransform` maps to device pixels; the transform's mean column norm
/// converts it to the device radius.
#[derive(Debug, Clone, PartialEq)]
pub struct GlassSurfaceSpec {
    pub surface_id: String,
    pub shape: PackedGlassShapeKind,
    pub radius: f32,
}

/// One device-qualified Glass surface.
#[derive(Debug, Clone, PartialEq)]
pub struct GlassDeviceShape {
    /// Axis-aligned device bounds of the Glass box (projected normalized corners).
    pub rect: Rect,
    /// Device-space radius (layout radius × mean column norm).
    pub radius: f32,
    pub presence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DeviceShapeError {
    #[error("device transform is not finite")]
    NonFiniteTransform,
    #[error("device transform is degenerate")]
    DegenerateTransform,
}

/// Qualifies one Glass surface with `presence` under `transform`, which maps the surface's
/// normalized box to device pixels.
pub fn qualify_device_shape(
    spec: &GlassSurfaceSpec,
    presence: f64,
    transform: DeviceTransform,
) -> Result<GlassDeviceShape, DeviceShapeError> {
    let matrix = transform.matrix();
    if !matrix.iter().all(|value| value.is_finite()) {
        return Err(DeviceShapeError::NonFiniteTransform);
    }
    let scale = column_scale(matrix).ok_or(DeviceShapeError::DegenerateTransform)?;
    let rect = device_rect(matrix).ok_or(DeviceShapeError::DegenerateTransform)?;
    let mean_scale = 0.5 * (scale[0] + scale[1]);
    Ok(GlassDeviceShape {
        rect,
        radius: (spec.radius as f64 * mean_scale) as f32,
        presence: presence as f32,
    })
}

/// The lengths of the two affine columns of `matrix`, or `None` when one degenerates or the
/// box's center projects onto the horizon.
fn column_scale(matrix: [f64; 9]) -> Option<[f64; 2]> {
    let w = matrix[6] * 0.5 + matrix[7] * 0.5 + matrix[8];
    if !w.is_finite() || w.abs() < super::GLASS_EPSILON {
        return None;
    }
    let scale = [
        (matrix[0] * matrix[0] + matrix[3] * matrix[3]).sqrt(),
        (matrix[1] * matrix[1] + matrix[4] * matrix[4]).sqrt(),
    ];
    (scale[0] >= super::GLASS_EPSILON && scale[1] >= super::GLASS_EPSILON).then_some(scale)
}

/// Axis-aligned device bounds of the normalized Glass box under `matrix`.
fn device_rect(matrix: [f64; 9]) -> Option<Rect> {
    let mut xs = [0.0; 4];
    let mut ys = [0.0; 4];
    let corners = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
    for (index, corner) in corners.iter().enumerate() {
        let [x, y] = *corner;
        let w = matrix[6] * x + matrix[7] * y + matrix[8];
        if !w.is_finite() || w.abs() < super::GLASS_EPSILON {
            return None;
        }
        xs[index] = (matrix[0] * x + matrix[1] * y + matrix[2]) / w;
        ys[index] = (matrix[3] * x + matrix[4] * y + matrix[5]) / w;
    }
    let left = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let right = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let top = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let bottom = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !left.is_finite() || !right.is_finite() || !top.is_finite() || !bottom.is_finite() {
        return None;
    }
    Some(Rect::new(left, top, right - left, bottom - top))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::glass::GLASS_EPSILON;

    fn spec() -> GlassSurfaceSpec {
        GlassSurfaceSpec {
            surface_id: "lens".into(),
            shape: PackedGlassShapeKind::Circle,
            radius: 50.0,
        }
    }

    #[test]
    fn device_radius_follows_the_transform_scale() {
        let shape = qualify_device_shape(
            &spec(),
            1.0,
            DeviceTransform::from_affine([2.0, 0.0, 10.0, 0.0, 2.0, 20.0]),
        )
        .unwrap();
        assert!((shape.radius as f64 - 100.0).abs() < 1e-3);
        assert_eq!(shape.rect, Rect::new(10.0, 20.0, 2.0, 2.0));
        assert_eq!(shape.presence, 1.0);
    }

    #[test]
    fn degenerate_transform_fails_closed() {
        assert_eq!(
            qualify_device_shape(
                &spec(),
                1.0,
                DeviceTransform::from_affine([0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            ),
            Err(DeviceShapeError::DegenerateTransform)
        );
    }

    #[test]
    fn epsilon_constant_is_sane() {
        assert!(GLASS_EPSILON > 0.0);
    }

    #[test]
    fn perspective_transform_qualifies_device_bounds() {
        // G4.1.5: a projective homography (perspective camera) projects the box corners into
        // device bounds.
        let flat = qualify_device_shape(
            &spec(),
            1.0,
            DeviceTransform::from_affine([1.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
        )
        .unwrap();
        let perspective = qualify_device_shape(
            &spec(),
            1.0,
            DeviceTransform::from_projective([1.2, 0.0, 30.0, 0.0, 1.2, 20.0, 0.001, 0.0, 1.0])
                .unwrap(),
        )
        .unwrap();
        // The perspective box projects to device bounds around (30, 20)+scaled extent.
        assert!(perspective.rect.left() > flat.rect.left());
        assert!(perspective.rect.top() > flat.rect.top());
    }
}
