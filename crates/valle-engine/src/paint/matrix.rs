//! Transforms of the plane: 3×3 matrices on points in px, y down. 2D transforms leave the last
//! row (0, 0, 1); a flattened 3D one may not, and divides by it. And of space: 4×4 matrices, z
//! toward the viewer, which flatten onto the plane.

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

    /// A turn of `degrees` clockwise, y down.
    pub fn rotate(degrees: f64) -> Matrix {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Matrix::affine([cos, sin, -sin, cos, 0.0, 0.0])
    }

    pub fn scale(k: f64) -> Matrix {
        Matrix::affine([k, 0.0, 0.0, k, 0.0, 0.0])
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
        let cof = |c, r| self.cofactor(c, r);
        let det = self.determinant();
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        Some(Matrix(std::array::from_fn(|c| {
            std::array::from_fn(|r| cof(r, c) / det)
        })))
    }

    fn cofactor(&self, c: usize, r: usize) -> f64 {
        let m = &self.0;
        let (c1, c2) = ((c + 1) % 3, (c + 2) % 3);
        let (r1, r2) = ((r + 1) % 3, (r + 2) % 3);
        m[c1][r1] * m[c2][r2] - m[c2][r1] * m[c1][r2]
    }

    pub fn determinant(&self) -> f64 {
        (0..3).map(|c| self.0[c][0] * self.cofactor(c, 0)).sum()
    }

    /// The smallest rectangle around `r` transformed: its corners', as `getBoundingClientRect()`
    /// has it, which takes one behind the viewer through it.
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

/// Columns: where x, y and z go, and the translation; each with its projective row last.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix4(pub [[f64; 4]; 4]);

impl Matrix4 {
    pub const IDENTITY: Matrix4 = Matrix4([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]);

    pub fn translate(x: f64, y: f64, z: f64) -> Matrix4 {
        let mut m = Matrix4::IDENTITY;
        m.0[3] = [x, y, z, 1.0];
        m
    }

    pub fn scale(x: f64, y: f64, z: f64) -> Matrix4 {
        let mut m = Matrix4::IDENTITY;
        (m.0[0][0], m.0[1][1], m.0[2][2]) = (x, y, z);
        m
    }

    /// A 2D transform, which keeps z.
    pub fn affine([a, b, c, d, e, f]: [f64; 6]) -> Matrix4 {
        Matrix4([
            [a, b, 0.0, 0.0],
            [c, d, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [e, f, 0.0, 1.0],
        ])
    }

    /// `degrees` about an axis, as CSS `rotate3d()` has it; about none for an axis of length 0.
    pub fn rotate(axis: [f64; 3], degrees: f64) -> Matrix4 {
        let length = axis.iter().map(|a| a * a).sum::<f64>().sqrt();
        if length == 0.0 || !length.is_finite() {
            return Matrix4::IDENTITY;
        }
        let [x, y, z] = axis.map(|a| a / length);
        let (sin, cos) = (degrees.to_radians() / 2.0).sin_cos();
        let (sc, sq) = (sin * cos, sin * sin);
        Matrix4([
            [
                1.0 - 2.0 * (y * y + z * z) * sq,
                2.0 * (x * y * sq + z * sc),
                2.0 * (x * z * sq - y * sc),
                0.0,
            ],
            [
                2.0 * (x * y * sq - z * sc),
                1.0 - 2.0 * (x * x + z * z) * sq,
                2.0 * (y * z * sq + x * sc),
                0.0,
            ],
            [
                2.0 * (x * z * sq + y * sc),
                2.0 * (y * z * sq - x * sc),
                1.0 - 2.0 * (x * x + y * y) * sq,
                0.0,
            ],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    /// A viewer `d` px in front of z = 0; at least 1 px, as CSS has it.
    pub fn perspective(d: f64) -> Matrix4 {
        let mut m = Matrix4::IDENTITY;
        m.0[2][3] = -1.0 / d.max(1.0);
        m
    }

    /// The product: `other` applied first, then `self`.
    pub fn times(self, other: Matrix4) -> Matrix4 {
        let (a, b) = (self.0, other.0);
        Matrix4(std::array::from_fn(|c| {
            std::array::from_fn(|r| (0..4).map(|k| a[k][r] * b[c][k]).sum())
        }))
    }

    /// A point in space before it is divided through by its projective coordinate.
    pub fn apply4(&self, p: [f64; 4]) -> [f64; 4] {
        let m = &self.0;
        std::array::from_fn(|r| (0..4).map(|c| m[c][r] * p[c]).sum())
    }

    /// Onto the plane z = 0, seen from the front: z in and z out dropped.
    pub fn flatten(&self) -> Matrix {
        let m = &self.0;
        Matrix([0, 1, 3].map(|c| [0, 1, 3].map(|r| m[c][r])))
    }

    pub fn determinant(&self) -> f64 {
        let m = &self.0;
        // Along the first column, each minor with the other three.
        (0..4)
            .map(|r| {
                let rows: Vec<usize> = (0..4).filter(|&k| k != r).collect();
                let minor = Matrix([1, 2, 3].map(|c| [0, 1, 2].map(|k| m[c][rows[k]])));
                let sign = if r % 2 == 0 { 1.0 } else { -1.0 };
                sign * m[0][r] * minor.determinant()
            })
            .sum()
    }

    /// Whether the plane z = 0 shows its back to the viewer: whether the z of its normal, which
    /// is the flattened determinant over the whole one, is below 0, as Chrome has it. A mirror
    /// in the plane does not show it.
    pub fn shows_back(&self) -> bool {
        self.flatten().determinant() * self.determinant() < -1e-12
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

    #[test]
    fn matrices_in_space_turn_and_flatten_in_perspective() {
        let near =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
        // About z as in the plane; about y, x goes away from the viewer.
        let quarter = Matrix4::rotate([0.0, 0.0, 1.0], 90.0).flatten();
        assert!(near(quarter.apply([1.0, 0.0]), [0.0, 1.0]));
        let turned = Matrix4::rotate([0.0, 2.0, 0.0], 90.0);
        assert!(turned.0[0][2] < -0.999);
        assert_eq!(turned.flatten().inverse(), None);
        // 100 px back from a viewer 200 px away: two thirds the size.
        let m = Matrix4::perspective(200.0).times(Matrix4::translate(0.0, 0.0, -100.0));
        assert!(near(m.flatten().apply([30.0, -60.0]), [20.0, -40.0]));
        assert_eq!(Matrix4::perspective(0.0), Matrix4::perspective(1.0));
        assert_eq!(Matrix4::rotate([0.0; 3], 30.0), Matrix4::IDENTITY);
        // Turned past its edge it shows its back; mirrored it does not, and from the side a
        // viewer may see the front of one turned past its edge from the front.
        assert!(Matrix4::rotate([0.0, 1.0, 0.0], 180.0).shows_back());
        assert!(Matrix4::rotate([1.0, 0.0, 0.0], 100.0).shows_back());
        assert!(!Matrix4::scale(-1.0, 1.0, 1.0).shows_back());
        let turned = Matrix4::rotate([0.0, 1.0, 0.0], 100.0);
        assert!(turned.shows_back());
        let aside = Matrix4::perspective(100.0)
            .times(Matrix4::translate(-190.0, 0.0, 0.0))
            .times(turned);
        assert!(!aside.shows_back());
    }
}
