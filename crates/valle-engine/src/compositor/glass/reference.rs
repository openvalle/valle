//! Scalar geometry formulas shared by the reference kernel.

/// C2 materialization `M(p)=p³(10-15p+6p²)` on `[0, 1]`.
pub fn materialization(p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    let p2 = p * p;
    let p3 = p2 * p;
    p3 * (10.0 - 15.0 * p + 6.0 * p2)
}

pub fn signed_circle_distance(point: [f64; 2], center: [f64; 2], radius: f64) -> f64 {
    let dx = point[0] - center[0];
    let dy = point[1] - center[1];
    (dx * dx + dy * dy).sqrt() - radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialization_is_c2_at_endpoints() {
        assert!((materialization(0.0) - 0.0).abs() < 1e-12);
        assert!((materialization(1.0) - 1.0).abs() < 1e-12);
        let d0 = (materialization(1e-6) - materialization(0.0)) / 1e-6;
        let d1 = (materialization(1.0) - materialization(1.0 - 1e-6)) / 1e-6;
        assert!(d0.abs() < 1e-6, "M'(0)≈0 got {d0}");
        assert!(d1.abs() < 1e-6, "M'(1)≈0 got {d1}");
        let mid = materialization(0.5);
        assert!((mid - 0.5).abs() < 1e-12);
    }
}
