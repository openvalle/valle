use serde::{Deserialize, Serialize};

use crate::{Point, Rect};

/// CSS-order affine transform: `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Affine2d(pub [f64; 6]);

impl Affine2d {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub const fn translate(dx: f64, dy: f64) -> Self {
        Self([1.0, 0.0, 0.0, 1.0, dx, dy])
    }

    pub const fn scale(sx: f64, sy: f64) -> Self {
        Self([sx, 0.0, 0.0, sy, 0.0, 0.0])
    }

    pub fn rotate(degrees: f64) -> Self {
        let (sin, cos) = crate::math::sin_cos(degrees.to_radians());
        Self([cos, sin, -sin, cos, 0.0, 0.0])
    }

    /// Apply `self`, then `next`.
    pub fn then(self, next: Self) -> Self {
        let (a, b) = (self.0, next.0);
        Self([
            a[0] * b[0] + a[1] * b[2],
            a[0] * b[1] + a[1] * b[3],
            a[2] * b[0] + a[3] * b[2],
            a[2] * b[1] + a[3] * b[3],
            a[4] * b[0] + a[5] * b[2] + b[4],
            a[4] * b[1] + a[5] * b[3] + b[5],
        ])
    }

    pub fn inverse(self) -> Option<Self> {
        let [a, b, c, d, e, f] = self.0;
        let determinant = a * d - b * c;
        if !determinant.is_finite() || determinant == 0.0 {
            return None;
        }
        let inverse = 1.0 / determinant;
        let output = Self([
            d * inverse,
            -b * inverse,
            -c * inverse,
            a * inverse,
            (c * f - d * e) * inverse,
            (b * e - a * f) * inverse,
        ]);
        output
            .0
            .iter()
            .all(|value| value.is_finite())
            .then_some(output)
    }
}

impl Default for Affine2d {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Row-major projective 3x3 transform in the same order used by Skia and CanvasKit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Transform2d(pub [f64; 9]);

impl Transform2d {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    pub const fn from_affine(transform: Affine2d) -> Self {
        let [a, b, c, d, e, f] = transform.0;
        Self([a, c, e, b, d, f, 0.0, 0.0, 1.0])
    }

    pub fn then(self, next: Self) -> Self {
        Self(mul3(next.0, self.0))
    }

    pub fn try_apply(self, point: Point) -> Option<Point> {
        let matrix = self.0;
        let w = matrix[6] * point.x + matrix[7] * point.y + matrix[8];
        if !w.is_finite() || w.abs() <= f64::EPSILON {
            return None;
        }
        let mapped = Point::new(
            (matrix[0] * point.x + matrix[1] * point.y + matrix[2]) / w,
            (matrix[3] * point.x + matrix[4] * point.y + matrix[5]) / w,
        );
        (mapped.x.is_finite() && mapped.y.is_finite()).then_some(mapped)
    }

    pub fn inverse(self) -> Option<Self> {
        let [a, b, c, d, e, f, g, h, i] = self.0;
        let determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
        if !determinant.is_finite() || determinant == 0.0 {
            return None;
        }
        let inverse = 1.0 / determinant;
        let matrix = [
            (e * i - f * h) * inverse,
            (c * h - b * i) * inverse,
            (b * f - c * e) * inverse,
            (f * g - d * i) * inverse,
            (a * i - c * g) * inverse,
            (c * d - a * f) * inverse,
            (d * h - e * g) * inverse,
            (b * g - a * h) * inverse,
            (a * e - b * d) * inverse,
        ];
        matrix
            .iter()
            .all(|value| value.is_finite())
            .then_some(Self(matrix))
    }

    /// CSS `perspective(d) rotateY(y) rotateX(x)` around `(cx, cy)`.
    pub fn layer(
        cx: f64,
        cy: f64,
        rotate_y_degrees: f64,
        rotate_x_degrees: f64,
        perspective: f64,
    ) -> Option<Self> {
        if !cx.is_finite()
            || !cy.is_finite()
            || !rotate_y_degrees.is_finite()
            || !rotate_x_degrees.is_finite()
            || !perspective.is_finite()
            || perspective <= 0.0
        {
            return None;
        }
        if rotate_y_degrees.abs() < 1e-6 && rotate_x_degrees.abs() < 1e-6 {
            return None;
        }
        let (sin_y, cos_y) = crate::math::sin_cos(rotate_y_degrees.to_radians());
        let (sin_x, cos_x) = crate::math::sin_cos(rotate_x_degrees.to_radians());
        let rotate = [
            cos_y,
            sin_y * sin_x,
            0.0,
            0.0,
            cos_x,
            0.0,
            sin_y / perspective,
            -cos_y * sin_x / perspective,
            1.0,
        ];
        let to_origin = [1.0, 0.0, -cx, 0.0, 1.0, -cy, 0.0, 0.0, 1.0];
        let from_origin = [1.0, 0.0, cx, 0.0, 1.0, cy, 0.0, 0.0, 1.0];
        let matrix = mul3(from_origin, mul3(rotate, to_origin));
        matrix
            .iter()
            .all(|value| value.is_finite())
            .then_some(Self(matrix))
    }

    pub fn map_rect(source: Rect, destination: [Point; 4]) -> Option<Self> {
        if !source.width.is_finite()
            || !source.height.is_finite()
            || source.width.abs() < 1e-9
            || source.height.abs() < 1e-9
        {
            return None;
        }
        let source = [
            Point::new(source.x, source.y),
            Point::new(source.x + source.width, source.y),
            Point::new(source.x + source.width, source.y + source.height),
            Point::new(source.x, source.y + source.height),
        ];
        Self::map_quad(source, destination)
    }

    pub fn map_quad(source: [Point; 4], destination: [Point; 4]) -> Option<Self> {
        if !quad_is_usable(source) || !quad_is_usable(destination) {
            return None;
        }
        let mut coefficients = [[0.0; 8]; 8];
        let mut values = [0.0; 8];
        for index in 0..4 {
            let Point { x, y } = source[index];
            let Point { x: u, y: v } = destination[index];
            let row = index * 2;
            coefficients[row] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y];
            values[row] = u;
            coefficients[row + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y];
            values[row + 1] = v;
        }
        let solved = solve8(coefficients, values)?;
        let transform = Self([
            solved[0], solved[1], solved[2], solved[3], solved[4], solved[5], solved[6], solved[7],
            1.0,
        ]);
        for index in 0..4 {
            let mapped = transform.try_apply(source[index])?;
            if (mapped.x - destination[index].x).abs() > 1e-6
                || (mapped.y - destination[index].y).abs() > 1e-6
            {
                return None;
            }
        }
        Some(transform)
    }

    /// Conservative finite bounds. A rectangle crossing the projective vanishing line is invalid.
    pub fn map_bounds(self, rect: Rect) -> Option<Rect> {
        if rect.is_empty() {
            return Some(rect);
        }
        let corners = [
            Point::new(rect.left(), rect.top()),
            Point::new(rect.right(), rect.top()),
            Point::new(rect.right(), rect.bottom()),
            Point::new(rect.left(), rect.bottom()),
        ];
        let denominators =
            corners.map(|point| self.0[6] * point.x + self.0[7] * point.y + self.0[8]);
        if denominators
            .iter()
            .any(|value| !value.is_finite() || value.abs() <= f64::EPSILON)
            || denominators
                .iter()
                .skip(1)
                .any(|value| value.signum() != denominators[0].signum())
        {
            return None;
        }
        let mapped = corners.map(|point| self.try_apply(point));
        let [Some(a), Some(b), Some(c), Some(d)] = mapped else {
            return None;
        };
        let left = a.x.min(b.x).min(c.x).min(d.x);
        let top = a.y.min(b.y).min(c.y).min(d.y);
        let right = a.x.max(b.x).max(c.x).max(d.x);
        let bottom = a.y.max(b.y).max(c.y).max(d.y);
        Some(Rect::from_edges(left, top, right, bottom))
    }
}

impl Default for Transform2d {
    fn default() -> Self {
        Self::IDENTITY
    }
}

fn quad_is_usable(points: [Point; 4]) -> bool {
    if points
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return false;
    }
    let signed_area =
        |a: Point, b: Point, c: Point| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let first = signed_area(points[0], points[1], points[2]);
    let second = signed_area(points[0], points[2], points[3]);
    first.is_finite()
        && second.is_finite()
        && first.abs() > 1e-8
        && second.abs() > 1e-8
        && first.signum() == second.signum()
}

fn solve8(mut coefficients: [[f64; 8]; 8], mut values: [f64; 8]) -> Option<[f64; 8]> {
    for column in 0..8 {
        let mut pivot = column;
        let mut best = coefficients[column][column].abs();
        for row in (column + 1)..8 {
            let candidate = coefficients[row][column].abs();
            if candidate > best {
                best = candidate;
                pivot = row;
            }
        }
        if best < 1e-12 {
            return None;
        }
        if pivot != column {
            coefficients.swap(column, pivot);
            values.swap(column, pivot);
        }
        let diagonal = coefficients[column][column];
        for row in (column + 1)..8 {
            let factor = coefficients[row][column] / diagonal;
            if !factor.is_finite() {
                return None;
            }
            for index in column..8 {
                coefficients[row][index] -= factor * coefficients[column][index];
            }
            values[row] -= factor * values[column];
        }
    }
    let mut output = [0.0; 8];
    for row in (0..8).rev() {
        let mut accumulated = values[row];
        for index in (row + 1)..8 {
            accumulated -= coefficients[row][index] * output[index];
        }
        output[row] = accumulated / coefficients[row][row];
        if !output[row].is_finite() {
            return None;
        }
    }
    Some(output)
}

fn mul3(left: [f64; 9], right: [f64; 9]) -> [f64; 9] {
    let mut output = [0.0; 9];
    for row in 0..3 {
        for column in 0..3 {
            output[row * 3 + column] = left[row * 3] * right[column]
                + left[row * 3 + 1] * right[3 + column]
                + left[row * 3 + 2] * right[6 + column];
        }
    }
    output
}
