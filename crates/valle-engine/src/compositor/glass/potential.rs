//! Distance-preserving compact-support field potential `W(u)`.

use super::GLASS_EPSILON;

pub fn potential_w(u: f64) -> f64 {
    if u <= 0.0 {
        // Never saturate the member interior. For one member, rho = 1 - sdf / merge and the
        // inverse-gradient pseudo-distance below recovers the original signed distance exactly.
        1.0 - u
    } else if u < 1.0 {
        // Quintic tail with W(0)=1, W'(0)=-1, W''(0)=0 and W(1)=W'(1)=W''(1)=0.
        1.0 - u - 4.0 * u.powi(3) + 7.0 * u.powi(4) - 3.0 * u.powi(5)
    } else {
        0.0
    }
}

/// Converts the `rho=1` isocontour to device-space signed distance. One density unit is normalized
/// by `merge_distance`, so `1 / merge_distance` is the canonical slope when symmetry makes the
/// measured spatial gradient vanish. This preserves the exact distance for a single member.
pub fn field_pseudo_distance(rho: f64, gradient_length: f64, merge_distance: f64) -> f64 {
    let merge_distance = merge_distance.max(1.0);
    let normalized_slope = 1.0 / merge_distance;
    (1.0 - rho) / gradient_length.max(normalized_slope).max(GLASS_EPSILON)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn potential_matches_design_anchors() {
        assert!((potential_w(0.0) - 1.0).abs() < 1e-12);
        assert!((potential_w(0.5) - 0.34375).abs() < 1e-12);
        assert!((potential_w(1.0)).abs() < 1e-12);
        assert!((potential_w(-1.0) - 2.0).abs() < 1e-12);
        let w_prime_0 = (potential_w(1e-6) - potential_w(-1e-6)) / 2e-6;
        assert!(
            (w_prime_0 + 1.0).abs() < 1e-4,
            "W'(0) should be -1, got {w_prime_0}"
        );
        let left = (potential_w(-1.0 + 1e-6) - potential_w(-1.0)) / 1e-6;
        let right = (potential_w(1.0) - potential_w(1.0 - 1e-6)) / 1e-6;
        assert!((left + 1.0).abs() < 2e-3);
        assert!(right.abs() < 2e-3);
    }

    #[test]
    fn zero_gradient_critical_points_use_the_normalized_field_scale() {
        let outside = field_pseudo_distance(0.99, 0.0, 24.0);
        let boundary = field_pseudo_distance(1.0, 0.0, 24.0);
        let inside = field_pseudo_distance(1.01, 0.0, 24.0);
        assert!(outside > boundary && boundary > inside);
        assert_eq!(boundary, 0.0);
        assert!((outside - 0.24).abs() < 1e-12);
        assert!((inside + 0.24).abs() < 1e-12);
    }

    #[test]
    fn single_member_interior_recovers_signed_distance() {
        let merge = 24.0;
        for distance in [-20.0, -4.0, 0.0, 4.0] {
            let rho = potential_w(distance / merge);
            let derivative = if distance <= 0.0 {
                1.0 / merge
            } else {
                let epsilon = 1.0e-5;
                (potential_w((distance - epsilon) / merge)
                    - potential_w((distance + epsilon) / merge))
                    / (2.0 * epsilon)
            };
            let recovered = field_pseudo_distance(rho, derivative.abs(), merge);
            assert!(
                (recovered - distance).abs() < 0.5,
                "{distance} -> {recovered}"
            );
        }
    }
}
