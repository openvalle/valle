//! Transforms of the plane: 3×3 matrices on points in px, y down. 2D transforms leave the last
//! row (0, 0, 1); a flattened 3D one may not, and divides by it.

use crate::layout::Rect;

/// Columns: where x and y go, and the translation; each with its projective row last.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix(pub [[f64; 3]; 3]);

impl Matrix {
    pub const IDENTITY: Matrix = Matrix([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    /// CSS `matrix(a, b, c, d, e, f)`: x to (a, b), y to (c, d), moved by (e, f).
    pub fn affine([a, b, c, d, e, f]: [f64; 6]) -> Matrix {
        Matrix([[a, b, 0.0], [c, d, 0.0], [e, f, 1.0]])
    }

    pub fn translate(x: f64, y: f64) -> Matrix {
        Matrix::affine([1.0, 0.0, 0.0, 1.0, x, y])
    }

    /// The product: `other` applied first, then `self`.
    pub fn times(self, other: Matrix) -> Matrix {
        let (a, b) = (self.0, other.0);
        Matrix(std::array::from_fn(|c| {
            std::array::from_fn(|r| (0..3).map(|k| a[k][r] * b[c][k]).sum())
        }))
    }

    pub fn apply(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        let m = &self.0;
        let (x, y) = (f64::from(x), f64::from(y));
        let h = [0, 1, 2].map(|r| m[0][r] * x + m[1][r] * y + m[2][r]);
        [(h[0] / h[2]) as f32, (h[1] / h[2]) as f32]
    }

    /// `None` when it flattens the plane, which leaves nothing to see.
    pub fn inverse(&self) -> Option<Matrix> {
        let m = &self.0;
        // The cofactors, transposed: the adjugate.
        let cof = |c: usize, r: usize| {
            let (c1, c2) = ((c + 1) % 3, (c + 2) % 3);
            let (r1, r2) = ((r + 1) % 3, (r + 2) % 3);
            m[c1][r1] * m[c2][r2] - m[c2][r1] * m[c1][r2]
        };
        let det = (0..3).map(|c| m[c][0] * cof(c, 0)).sum::<f64>();
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        Some(Matrix(std::array::from_fn(|c| {
            std::array::from_fn(|r| cof(r, c) / det)
        })))
    }

    /// The smallest rectangle around `r` transformed.
    pub fn bounds(&self, r: Rect) -> Rect {
        let corners = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]
            .map(|[u, v]| self.apply([r.x + u * r.width, r.y + v * r.height]));
        let lo = [0, 1].map(|k| corners.iter().map(|p| p[k]).fold(f32::INFINITY, f32::min));
        let hi = [0, 1].map(|k| (corners.iter().map(|p| p[k])).fold(f32::NEG_INFINITY, f32::max));
        Rect {
            x: lo[0],
            y: lo[1],
            width: hi[0] - lo[0],
            height: hi[1] - lo[1],
        }
    }

    /// How much it scales a radius: the geometric mean of what it makes of a unit step along x
    /// and one along y, as Skia scales a blur.
    pub fn radius_scale(&self) -> f64 {
        let m = &self.0;
        (m[0][0].hypot(m[0][1]) * m[1][0].hypot(m[1][1])).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices_compose_invert_and_bound() {
        let rotate = Matrix::affine([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);
        let m = Matrix::translate(10.0, 20.0).times(rotate);
        // A quarter turn clockwise, y down, then moved.
        assert_eq!(m.apply([1.0, 0.0]), [10.0, 21.0]);
        let back = m.inverse().unwrap();
        assert_eq!(back.apply([10.0, 21.0]), [1.0, 0.0]);
        assert_eq!(m.times(back), Matrix::IDENTITY);
        let r = Rect {
            x: 0.0,
            y: 0.0,
            width: 4.0,
            height: 2.0,
        };
        let b = m.bounds(r);
        assert_eq!([b.x, b.y, b.width, b.height], [8.0, 20.0, 2.0, 4.0]);
        assert_eq!(
            Matrix::affine([2.0, 0.0, 0.0, 8.0, 0.0, 0.0]).radius_scale(),
            4.0
        );
        assert_eq!(
            Matrix::affine([1.0, 2.0, 2.0, 4.0, 0.0, 0.0]).inverse(),
            None
        );
    }
}
