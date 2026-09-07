use super::DERIVATIVE_STEP_SECONDS;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassKinematics {
    pub linear_velocity: [f64; 2],
    pub linear_acceleration: [f64; 2],
    pub angular_velocity: f64,
    pub scale_rate: [f64; 2],
    pub shear_rate: f64,
    pub area_rate: f64,
    pub discontinuity: bool,
}

impl GlassKinematics {
    pub const ZERO: Self = Self {
        linear_velocity: [0.0, 0.0],
        linear_acceleration: [0.0, 0.0],
        angular_velocity: 0.0,
        scale_rate: [0.0, 0.0],
        shear_rate: 0.0,
        area_rate: 0.0,
        discontinuity: false,
    };
}

/// Backward-only derivative. `earlier` is at `t - dt` with `dt > 0`.
pub fn backward_derivative(
    current: [f64; 2],
    earlier: [f64; 2],
    dt: f64,
    same_epoch: bool,
) -> [f64; 2] {
    if !same_epoch || dt <= 0.0 {
        return [0.0, 0.0];
    }
    [
        (current[0] - earlier[0]) / dt,
        (current[1] - earlier[1]) / dt,
    ]
}

pub fn kinematics_from_centers(
    current: [f64; 2],
    previous: [f64; 2],
    earlier: [f64; 2],
    same_epoch: bool,
) -> GlassKinematics {
    let dt = DERIVATIVE_STEP_SECONDS;
    let velocity = backward_derivative(current, previous, dt, same_epoch);
    let prev_velocity = backward_derivative(previous, earlier, dt, same_epoch);
    GlassKinematics {
        linear_velocity: velocity,
        linear_acceleration: backward_derivative(velocity, prev_velocity, dt, same_epoch),
        ..GlassKinematics::ZERO
    }
}

/// Device-space pose features extracted from a row-major homography that maps the Glass node's
/// normalized layout box to device pixels. `scale` is the length of the two affine columns,
/// `rotation` the angle of the x axis, and `shear` the cosine of the angle between axes
/// (0 = orthogonal axes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DevicePose {
    pub center: [f64; 2],
    pub scale: [f64; 2],
    pub rotation: f64,
    pub shear: f64,
}

impl DevicePose {
    /// Extracts the pose from a homography, or `None` when the affine part degenerates
    /// (zero-length column) or the center projection lands on the horizon.
    pub fn from_matrix(matrix: [f64; 9]) -> Option<Self> {
        if !matrix.iter().all(|value| value.is_finite()) {
            return None;
        }
        let project = |x: f64, y: f64| -> Option<[f64; 2]> {
            let w = matrix[6] * x + matrix[7] * y + matrix[8];
            if !w.is_finite() || w.abs() < super::GLASS_EPSILON {
                return None;
            }
            Some([
                (matrix[0] * x + matrix[1] * y + matrix[2]) / w,
                (matrix[3] * x + matrix[4] * y + matrix[5]) / w,
            ])
        };
        let center = project(0.5, 0.5)?;
        let column_0 = [matrix[0], matrix[3]];
        let column_1 = [matrix[1], matrix[4]];
        let scale_0 = (column_0[0] * column_0[0] + column_0[1] * column_0[1]).sqrt();
        let scale_1 = (column_1[0] * column_1[0] + column_1[1] * column_1[1]).sqrt();
        if scale_0 < super::GLASS_EPSILON || scale_1 < super::GLASS_EPSILON {
            return None;
        }
        let shear = (column_0[0] * column_1[0] + column_0[1] * column_1[1]) / (scale_0 * scale_1);
        Some(Self {
            center,
            scale: [scale_0, scale_1],
            rotation: column_0[1].atan2(column_0[0]),
            shear: shear.clamp(-1.0, 1.0),
        })
    }

    /// Signed shortest angle difference `current - previous` in radians, in `(-pi, pi]`.
    pub fn angle_delta(previous: f64, current: f64) -> f64 {
        let mut delta = current - previous;
        while delta > std::f64::consts::PI {
            delta -= std::f64::consts::TAU;
        }
        while delta <= -std::f64::consts::PI {
            delta += std::f64::consts::TAU;
        }
        delta
    }
}

/// Instantaneous rates between two consecutive device poses. Returns zero rates when the sample
/// pair crosses an epoch (finite differences are forbidden across discontinuities).
pub fn kinematics_between(
    previous: DevicePose,
    current: DevicePose,
    dt: f64,
    same_epoch: bool,
) -> GlassKinematics {
    if !same_epoch || !dt.is_finite() || dt <= 0.0 {
        return GlassKinematics {
            discontinuity: !same_epoch,
            ..GlassKinematics::ZERO
        };
    }
    let inverse = 1.0 / dt;
    GlassKinematics {
        linear_velocity: [
            (current.center[0] - previous.center[0]) * inverse,
            (current.center[1] - previous.center[1]) * inverse,
        ],
        linear_acceleration: [0.0, 0.0],
        angular_velocity: DevicePose::angle_delta(previous.rotation, current.rotation) * inverse,
        scale_rate: [
            (current.scale[0] / previous.scale[0]).ln() * inverse,
            (current.scale[1] / previous.scale[1]).ln() * inverse,
        ],
        shear_rate: (current.shear - previous.shear) * inverse,
        area_rate: (((current.scale[0] * current.scale[1])
            / (previous.scale[0] * previous.scale[1]))
            .ln())
            * inverse,
        discontinuity: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_change_does_not_form_a_velocity() {
        let v = backward_derivative([10.0, 0.0], [0.0, 0.0], DERIVATIVE_STEP_SECONDS, false);
        assert_eq!(v, [0.0, 0.0]);
    }

    #[test]
    fn constant_motion_has_zero_acceleration() {
        let dt = DERIVATIVE_STEP_SECONDS;
        let now = [2.0 * dt, 0.0];
        let prev = [dt, 0.0];
        let earlier = [0.0, 0.0];
        let kin = kinematics_from_centers(now, prev, earlier, true);
        assert!((kin.linear_velocity[0] - 1.0).abs() < 1e-9);
        assert!(kin.linear_acceleration[0].abs() < 1e-9);
    }
}
