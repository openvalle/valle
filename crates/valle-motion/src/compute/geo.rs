//! Prepare-time geographic projection and canvas fitting using deterministic math. Parameterize
//! projections from input bounds without bundled geographic boundary data. Project into an
//! upward-positive plane, then fit uniformly and flip y into canvas coordinates.

use valle_draw::math;

/// Supported projection kinds, defaulting to Albers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectionKind {
    /// Albers equal-area conic projection.
    Albers,
    /// Spherical Mercator projection.
    Mercator,
}

/// Parameterized longitude/latitude projection into an upward-positive plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projector {
    Albers {
        /// n = (sin φ₁ + sin φ₂) / 2
        n: f64,
        /// C = cos²φ₁ + 2·n·sin φ₁
        c: f64,
        /// ρ₀ = √(C − 2·n·sin φ₀) / n
        rho0: f64,
        /// Central meridian in radians.
        lam0: f64,
    },
    Mercator,
}

impl Projector {
    /// Parameterize from the coordinate bounds, falling back from degenerate Albers configurations
    /// to Mercator.
    pub fn fit_kind(kind: ProjectionKind, lon: (f64, f64), lat: (f64, f64)) -> Projector {
        match kind {
            ProjectionKind::Mercator => Projector::Mercator,
            ProjectionKind::Albers => {
                let span = lat.1 - lat.0;
                // Choose standard parallels at one-sixth and five-sixths of the latitude span.
                let p1 = (lat.0 + span / 6.0).to_radians();
                let p2 = (lat.1 - span / 6.0).to_radians();
                let phi0 = ((lat.0 + lat.1) / 2.0).to_radians();
                let lam0 = ((lon.0 + lon.1) / 2.0).to_radians();
                let n = (math::sin(p1) + math::sin(p2)) / 2.0;
                // Near-zero conic n is degenerate; use Mercator to avoid division by zero.
                if n.abs() < 1e-9 {
                    return Projector::Mercator;
                }
                let cos_p1 = math::cos(p1);
                let c = cos_p1 * cos_p1 + 2.0 * n * math::sin(p1);
                let rho0 = math::sqrt((c - 2.0 * n * math::sin(phi0)).max(0.0)) / n;
                Projector::Albers { n, c, rho0, lam0 }
            }
        }
    }

    /// Project longitude and latitude in degrees into planar coordinates.
    pub fn project(&self, lon: f64, lat: f64) -> (f64, f64) {
        match *self {
            Projector::Mercator => {
                // Clamp polar latitude to avoid Mercator divergence.
                let lat = lat.clamp(-85.0, 85.0);
                let y = math::ln(math::tan(
                    std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0,
                ));
                (lon.to_radians(), y)
            }
            Projector::Albers { n, c, rho0, lam0 } => {
                let rho = math::sqrt((c - 2.0 * n * math::sin(lat.to_radians())).max(0.0)) / n;
                let theta = n * (lon.to_radians() - lam0);
                {
                    let (sin, cos) = math::sin_cos(theta);
                    (rho * sin, rho0 - rho * cos)
                }
            }
        }
    }
}

/// Uniform plane-to-canvas mapping including a y-axis flip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    s: f64,
    tx: f64,
    ty: f64,
    max_y: f64,
}

impl Fit {
    /// Fit planar bounds into the canvas, with padding relative to its shorter dimension.
    pub fn contain(
        (min_x, min_y): (f64, f64),
        (max_x, max_y): (f64, f64),
        width: f64,
        height: f64,
        pad: f64,
    ) -> Fit {
        let m = width.min(height) * pad;
        let (dw, dh) = ((max_x - min_x).max(1e-12), (max_y - min_y).max(1e-12));
        let s = ((width - 2.0 * m) / dw)
            .min((height - 2.0 * m) / dh)
            .max(0.0);
        // Center the unused space on each axis.
        let tx = (width - dw * s) / 2.0 - min_x * s;
        let ty = (height - dh * s) / 2.0;
        Fit { s, tx, ty, max_y }
    }

    /// Map an upward-positive plane point to downward-positive canvas coordinates.
    pub fn apply(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (x * self.s + self.tx, (self.max_y - y) * self.s + self.ty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercator_keeps_north_up_and_lon_order() {
        let p = Projector::Mercator;
        let (x1, y1) = p.project(100.0, 20.0);
        let (x2, y2) = p.project(110.0, 40.0);
        assert!(x2 > x1, "x increases eastward");
        assert!(y2 > y1, "y increases northward in upward coordinates");
    }

    #[test]
    fn albers_is_symmetric_about_central_meridian() {
        let p = Projector::fit_kind(ProjectionKind::Albers, (100.0, 110.0), (20.0, 40.0));
        let (xl, yl) = p.project(100.0, 30.0);
        let (xr, yr) = p.project(110.0, 30.0);
        assert!(
            (xl + xr).abs() < 1e-9,
            "symmetry around the central meridian: {xl} vs {xr}"
        );
        assert!((yl - yr).abs() < 1e-9, "equal latitude gives equal height");
        // Albers meridians converge, reducing horizontal span at higher latitude.
        let (xh, _) = p.project(110.0, 40.0);
        assert!(
            xh.abs() < xr.abs(),
            "higher latitudes converge horizontally: {xh} vs {xr}"
        );
    }

    #[test]
    fn albers_degenerates_to_mercator_when_symmetric_about_equator() {
        let p = Projector::fit_kind(ProjectionKind::Albers, (0.0, 10.0), (-30.0, 30.0));
        assert_eq!(
            p,
            Projector::Mercator,
            "n near zero falls back to Mercator without NaN"
        );
    }

    #[test]
    fn fit_contains_bbox_with_padding_and_flips_y() {
        let f = Fit::contain((0.0, 0.0), (10.0, 5.0), 1000.0, 500.0, 0.04);
        let tl = f.apply((0.0, 5.0));
        let br = f.apply((10.0, 0.0));
        // The highest planar point maps to the top of the canvas.
        assert!(tl.1 < br.1);
        for p in [tl, br] {
            assert!(p.0 >= 19.9 && p.0 <= 980.1, "{p:?}");
            assert!(p.1 >= 19.9 && p.1 <= 480.1, "{p:?}");
        }
        // Both dimensions use the same scale.
        assert!(((br.0 - tl.0) / 10.0 - (br.1 - tl.1) / 5.0).abs() < 1e-9);
    }
}
