//! G1.2 device-space Glass surface qualification.
//!
//! Turns composition-local track samples into device shapes: stable identity
//! (`GlassSurfaceKey`), device-space geometry after the Timeline/camera transform, and the
//! causal kinematics derived only from past samples. Crossing an epoch forbids finite
//! differences — the response kernel resets instead of forming a velocity.
//!
//! Determinism contract: results depend only on the samples and transforms, never on input
//! order, worker count, or evaluation order. Samples are ordered by canonical `SampleTime`
//! internally and duplicate times fail closed.

use thiserror::Error;
use valle_draw::Rect;
use valle_motion::glass::GlassSurfaceTrackSample;
use valle_timeline::internal::{MotionInstanceId, SampleTime, TrackEpoch};

use crate::prepare::DeviceTransform;

use super::kinematics::{DevicePose, GlassKinematics, kinematics_between};
use valle_draw::program::glass::PackedGlassShapeKind;

/// Upper bound from the G0 ABI manifest (`abi.toml` `max_surfaces`).
pub const MAX_GLASS_DEVICE_SAMPLES: usize = 64;

/// Durable identity of one Glass surface track in a device plan. Two Timeline instances of the
/// same Artifact produce different keys and never share tracks or caches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlassSurfaceKey {
    pub surface_id: String,
    pub instance: MotionInstanceId,
    pub epoch: TrackEpoch,
}

/// Static shape description resolved from the Glass Artifact. `radius` is in the normalized
/// box units that `DeviceTransform` maps to device pixels; the transform's mean column norm
/// converts it to the device radius.
#[derive(Debug, Clone, PartialEq)]
pub struct GlassSurfaceSpec {
    pub surface_id: String,
    pub shape: PackedGlassShapeKind,
    pub radius: f32,
}

/// One device-qualified sample: identity, device geometry, and backward kinematics.
#[derive(Debug, Clone, PartialEq)]
pub struct GlassDeviceShape {
    pub key: GlassSurfaceKey,
    pub time: SampleTime,
    /// Axis-aligned device bounds of the Glass box (projected normalized corners).
    pub rect: Rect,
    /// Homography mapping the Glass box to device pixels; kept for analytic optics and bounds.
    pub matrix: [f64; 9],
    /// Device-space radius (layout radius × mean column norm).
    pub radius: f32,
    pub presence: f32,
    pub intensity: f32,
    /// `[translation.x, translation.y, pressure, twist]` authored drive at this sample.
    pub drive: [f32; 4],
    /// Causal kinematics derived only from `[t-2, t]` samples of the same epoch.
    pub kinematics: GlassKinematics,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DeviceTrackError {
    #[error("device Glass track has no samples")]
    Empty,
    #[error("device Glass track exceeds {0} samples")]
    TooManySamples(usize),
    #[error("device Glass track has duplicate SampleTime entries")]
    DuplicateTime,
    #[error("device Glass track mixes Motion instances")]
    InstanceMismatch,
    #[error("device transform at sample {0} is not finite")]
    NonFiniteTransform(usize),
    #[error("device transform at sample {0} is degenerate")]
    DegenerateTransform(usize),
}

/// Qualifies one Glass surface's track into device space.
///
/// `entries` may arrive in any order; they are sorted by canonical `SampleTime`. The first
/// sample carries zero kinematics; later samples derive rates from the immediately preceding
/// same-epoch sample only.
pub fn qualify_device_track(
    spec: &GlassSurfaceSpec,
    entries: &[(GlassSurfaceTrackSample, DeviceTransform)],
) -> Result<Vec<GlassDeviceShape>, DeviceTrackError> {
    if entries.is_empty() {
        return Err(DeviceTrackError::Empty);
    }
    if entries.len() > MAX_GLASS_DEVICE_SAMPLES {
        return Err(DeviceTrackError::TooManySamples(entries.len()));
    }
    let instance = &entries[0].0.instance;
    if entries
        .iter()
        .any(|(sample, _)| sample.instance != *instance)
    {
        return Err(DeviceTrackError::InstanceMismatch);
    }

    // Deterministic order by canonical time; duplicate times fail closed.
    let mut ordered = entries.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(sample, _)| sample.time);
    for pair in ordered.windows(2) {
        if pair[0].0.time == pair[1].0.time {
            return Err(DeviceTrackError::DuplicateTime);
        }
    }

    let mut shapes = Vec::with_capacity(ordered.len());
    let mut previous_pose: Option<(DevicePose, GlassKinematics)> = None;
    let mut previous_epoch: Option<TrackEpoch> = None;
    for (index, (sample, transform)) in ordered.iter().enumerate() {
        let matrix = transform.matrix();
        if !matrix.iter().all(|value| value.is_finite()) {
            return Err(DeviceTrackError::NonFiniteTransform(index));
        }
        let pose =
            DevicePose::from_matrix(matrix).ok_or(DeviceTrackError::DegenerateTransform(index))?;
        let rect = device_rect(matrix).ok_or(DeviceTrackError::DegenerateTransform(index))?;
        let mean_scale = 0.5 * (pose.scale[0] + pose.scale[1]);
        let dt = if index == 0 {
            0.0
        } else {
            let delta = sample
                .time
                .composition()
                .checked_sub(ordered[index - 1].0.time.composition())
                .map_err(|_| DeviceTrackError::DuplicateTime)?;
            delta.as_f64()
        };
        let same_epoch = previous_epoch
            .as_ref()
            .map_or(true, |epoch| epoch == &sample.epoch);
        let mut kinematics = if index == 0 {
            GlassKinematics::ZERO
        } else {
            let (pose_prev, _) = previous_pose.as_ref().expect("previous pose");
            kinematics_between(*pose_prev, pose, dt, same_epoch)
        };
        // Second-order backward acceleration when both finite-difference steps stay in one epoch.
        if index >= 2 && same_epoch {
            if let Some((_, previous_kinematics)) = previous_pose.as_ref() {
                if !previous_kinematics.discontinuity {
                    let delta = sample
                        .time
                        .composition()
                        .checked_sub(ordered[index - 1].0.time.composition())
                        .map_err(|_| DeviceTrackError::DuplicateTime)?;
                    kinematics.linear_acceleration = [
                        (kinematics.linear_velocity[0] - previous_kinematics.linear_velocity[0])
                            / delta.as_f64(),
                        (kinematics.linear_velocity[1] - previous_kinematics.linear_velocity[1])
                            / delta.as_f64(),
                    ];
                }
            }
        }
        previous_pose = Some((pose, kinematics));
        previous_epoch = Some(sample.epoch.clone());
        shapes.push(GlassDeviceShape {
            key: GlassSurfaceKey {
                surface_id: spec.surface_id.clone(),
                instance: sample.instance.clone(),
                epoch: sample.epoch.clone(),
            },
            time: sample.time,
            rect,
            matrix,
            radius: (spec.radius as f64 * mean_scale) as f32,
            presence: sample.local.presence as f32,
            intensity: sample.local.intensity as f32,
            drive: [
                sample.local.drive_translation.x as f32,
                sample.local.drive_translation.y as f32,
                sample.local.drive_pressure as f32,
                sample.local.drive_twist as f32,
            ],
            kinematics,
        });
    }
    Ok(shapes)
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
    use valle_timeline::FrameRate;
    use valle_timeline::internal::{DiscontinuityIndex, FrameKey, TimeMapSegment};

    fn sample(frame: i64, epoch: u32) -> GlassSurfaceTrackSample {
        GlassSurfaceTrackSample {
            surface_id: valle_motion::glass::GlassSurfaceId::new("lens").unwrap(),
            instance: MotionInstanceId::new("clip-a").unwrap(),
            epoch: TrackEpoch::new(
                1,
                MotionInstanceId::new("clip-a").unwrap(),
                TimeMapSegment::new(0),
                0,
                DiscontinuityIndex::new(epoch),
            ),
            time: SampleTime::from_frame(FrameKey::new(frame), FrameRate::new(30, 1).unwrap())
                .unwrap(),
            local: valle_motion::glass::GlassLocalShape {
                rect: Rect::new(0.0, 0.0, 100.0, 100.0),
                presence: 1.0,
                intensity: 0.6,
                drive_translation: valle_draw::Point::new(0.0, 0.0),
                drive_pressure: 0.0,
                drive_twist: 0.0,
            },
        }
    }

    fn translate(tx: f64, ty: f64) -> DeviceTransform {
        DeviceTransform::from_affine([1.0, 0.0, tx, 0.0, 1.0, ty])
    }

    fn spec() -> GlassSurfaceSpec {
        GlassSurfaceSpec {
            surface_id: "lens".into(),
            shape: PackedGlassShapeKind::Circle,
            radius: 50.0,
        }
    }

    #[test]
    fn constant_translation_has_analytic_velocity_and_zero_acceleration() {
        // 100 px per frame at 30 fps => 3000 px/s.
        let entries = vec![
            (sample(0, 0), translate(0.0, 0.0)),
            (sample(1, 0), translate(100.0, 0.0)),
            (sample(2, 0), translate(200.0, 0.0)),
        ];
        let shapes = qualify_device_track(&spec(), &entries).unwrap();
        assert!((shapes[1].kinematics.linear_velocity[0] - 3000.0).abs() < 1e-6);
        assert!((shapes[2].kinematics.linear_velocity[0] - 3000.0).abs() < 1e-6);
        assert!(shapes[2].kinematics.linear_acceleration[0].abs() < 1e-6);
        assert!(!shapes[2].kinematics.discontinuity);
    }

    #[test]
    fn camera_only_motion_still_drives_the_surface() {
        // Local shape is identical; the device transform alone changes.
        let entries = vec![
            (sample(0, 0), translate(0.0, 0.0)),
            (sample(1, 0), translate(30.0, 0.0)),
        ];
        let shapes = qualify_device_track(&spec(), &entries).unwrap();
        assert!((shapes[1].kinematics.linear_velocity[0] - 900.0).abs() < 1e-6);
    }

    #[test]
    fn epoch_crossing_forbids_finite_differences() {
        let entries = vec![
            (sample(0, 0), translate(0.0, 0.0)),
            (sample(1, 1), translate(500.0, 0.0)),
        ];
        let shapes = qualify_device_track(&spec(), &entries).unwrap();
        assert_eq!(shapes[1].kinematics.linear_velocity, [0.0, 0.0]);
        assert!(shapes[1].kinematics.discontinuity);
    }

    #[test]
    fn rotation_rate_matches_analytic_angle() {
        // 90 degrees over one second: angular velocity = pi/2.
        let rot = |angle: f64| {
            let (sin, cos) = angle.sin_cos();
            DeviceTransform::from_affine([cos, -sin, 0.0, sin, cos, 0.0])
        };
        let entries = vec![
            (sample(0, 0), rot(0.0)),
            (sample(30, 0), rot(std::f64::consts::FRAC_PI_2)),
        ];
        let shapes = qualify_device_track(&spec(), &entries).unwrap();
        assert!((shapes[1].kinematics.angular_velocity - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    }

    #[test]
    fn scale_rate_matches_analytic_log_ratio() {
        let entries = vec![
            (sample(0, 0), translate(0.0, 0.0)),
            (
                sample(30, 0),
                DeviceTransform::from_affine([2.0, 0.0, 0.0, 0.0, 2.0, 0.0]),
            ),
        ];
        let shapes = qualify_device_track(&spec(), &entries).unwrap();
        assert!((shapes[1].kinematics.scale_rate[0] - std::f64::consts::LN_2).abs() < 1e-9);
        assert!((shapes[1].kinematics.area_rate - std::f64::consts::LN_2 * 2.0).abs() < 1e-9);
        assert!((shapes[1].radius as f64 - 100.0).abs() < 1e-3);
    }

    #[test]
    fn input_order_does_not_change_results() {
        let mut entries = vec![
            (sample(0, 0), translate(0.0, 0.0)),
            (sample(1, 0), translate(100.0, 0.0)),
            (sample(2, 0), translate(200.0, 0.0)),
        ];
        let forward = qualify_device_track(&spec(), &entries).unwrap();
        entries.reverse();
        let reversed = qualify_device_track(&spec(), &entries).unwrap();
        assert_eq!(forward, reversed);
    }

    #[test]
    fn duplicate_time_fails_closed() {
        let entries = vec![
            (sample(1, 0), translate(0.0, 0.0)),
            (sample(1, 0), translate(100.0, 0.0)),
        ];
        assert!(matches!(
            qualify_device_track(&spec(), &entries),
            Err(DeviceTrackError::DuplicateTime)
        ));
    }

    #[test]
    fn degenerate_transform_fails_closed() {
        let entries = vec![(sample(0, 0), translate(0.0, 0.0))];
        let bad = (
            sample(1, 0),
            DeviceTransform::from_affine([0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        );
        let mut with_bad = entries.clone();
        with_bad.push(bad);
        assert!(matches!(
            qualify_device_track(&spec(), &with_bad),
            Err(DeviceTrackError::DegenerateTransform(_))
        ));
    }

    #[test]
    fn epsilon_constant_is_sane() {
        assert!(GLASS_EPSILON > 0.0);
    }

    #[test]
    fn perspective_transform_qualifies_device_bounds_and_kinematics() {
        // G4.1.5: a projective homography (perspective camera) projects the box corners into
        // device bounds and still drives the kinematics through the pose features.
        let perspective =
            DeviceTransform::from_projective([1.2, 0.0, 30.0, 0.0, 1.2, 20.0, 0.001, 0.0, 1.0])
                .unwrap();
        let entries = vec![
            (
                sample(0, 0),
                DeviceTransform::from_affine([1.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
            ),
            (sample(1, 0), perspective),
        ];
        let shapes = qualify_device_track(&spec(), &entries).unwrap();
        // The perspective box projects to device bounds around (30, 20)+scaled extent.
        assert!(shapes[1].rect.left() > shapes[0].rect.left());
        assert!(shapes[1].rect.top() > shapes[0].rect.top());
        // Camera-only movement (the perspective shift) drives velocity.
        assert!(shapes[1].kinematics.linear_velocity[0] > 0.0);
        assert!(!shapes[1].kinematics.discontinuity);
    }
}
