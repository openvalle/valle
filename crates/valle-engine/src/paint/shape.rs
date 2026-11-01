//! Clip paths: a box's basic shape, or one of its boxes, as what masks all the box paints.

use std::f32::consts::SQRT_2;

use super::{Item, Radii, corner, fit};
use crate::layout::Rect;
use crate::path::{Path, Segment};
use crate::style::math::Calc;
use crate::style::position::Offset;
use crate::style::shape::{Radius, Shape};

/// What a clip path shows all of.
pub(super) const SHOWN: [f32; 4] = [1.0; 4];

/// `shape` in the box `r`, or without one the box with its corners rounded by `radii`, as the
/// item that masks what the box paints; `None` where it shows nothing.
pub(super) fn mask(shape: Option<&Shape>, r: Rect, radii: Radii) -> Option<Item> {
    let rect = |rect: Rect, radii| {
        (rect.width > 0.0 && rect.height > 0.0).then_some(Item::Rect {
            rect,
            radii,
            color: SHOWN,
        })
    };
    let filled = |path, even_odd: &bool| Item::Path {
        path,
        even_odd: *even_odd,
        color: SHOWN,
    };
    // An ellipse is a rectangle with its corners all of it.
    let ellipse = |c: [f32; 2], [rx, ry]: [f32; 2]| {
        let e = Rect {
            x: c[0] - rx,
            y: c[1] - ry,
            width: 2.0 * rx,
            height: 2.0 * ry,
        };
        rect(e, [[rx, ry]; 4])
    };
    match shape {
        None => rect(r, radii),
        Some(Shape::Inset { sides, radii }) => {
            let (inside, radii) = inset(sides, radii, r);
            rect(inside, radii)
        }
        Some(Shape::Circle { radius, at }) => {
            let c = center(at, r);
            ellipse(c, [circle(radius, c, r); 2])
        }
        Some(Shape::Ellipse { radii, at }) => {
            let c = center(at, r);
            ellipse(c, self::radii(radii, c, r))
        }
        Some(Shape::Polygon { even_odd, points }) => {
            let at = |[x, y]: &[Calc; 2]| [r.x + x.resolve(r.width), r.y + y.resolve(r.height)];
            let segments = (points.iter().enumerate())
                .map(|(i, p)| match i {
                    0 => Segment::Move(at(p)),
                    _ => Segment::Line(at(p)),
                })
                .collect();
            Some(filled(Path(segments), even_odd))
        }
        Some(Shape::Path { even_odd, path }) => Some(filled(path.moved([r.x, r.y]), even_odd)),
    }
}

/// An inset's rectangle in the box `r`, less `sides` from its top, right, bottom and left, and
/// its corners' radii fitting it.
pub(super) fn inset(sides: &[Calc; 4], radii: &[[Calc; 2]; 4], r: Rect) -> (Rect, Radii) {
    let size = [r.width, r.height];
    // Top and bottom of the height, right and left of the width.
    let [t, right, b, l] = std::array::from_fn(|k| sides[k].resolve(size[(k + 1) % 2]));
    let inside = Rect {
        x: r.x + l,
        y: r.y + t,
        width: r.width - l - right,
        height: r.height - t - b,
    };
    // Percentages of the box, as in Chrome, fitting the rectangle.
    let radii = radii
        .each_ref()
        .map(|[x, y]| corner([x.resolve(size[0]), y.resolve(size[1])]));
    (inside, fit(radii, inside))
}

/// A circle's radius around `c` in the box `r`: a length a percentage of the box's diagonal
/// over √2.
pub(super) fn circle(radius: &Radius, c: [f32; 2], r: Rect) -> f32 {
    let sides = sides(c, r).into_iter().flatten();
    match radius {
        Radius::ClosestSide => sides.fold(f32::INFINITY, f32::min),
        Radius::FarthestSide => sides.fold(0.0, f32::max),
        Radius::Len(l) => l.resolve(r.width.hypot(r.height) / SQRT_2),
    }
}

/// An ellipse's radii around `c` in the box `r`: lengths percentages of its width and height.
pub(super) fn radii(radii: &[Radius; 2], c: [f32; 2], r: Rect) -> [f32; 2] {
    let sides = sides(c, r);
    let size = [r.width, r.height];
    std::array::from_fn(|k| match &radii[k] {
        Radius::ClosestSide => sides[k][0].min(sides[k][1]),
        Radius::FarthestSide => sides[k][0].max(sides[k][1]),
        Radius::Len(l) => l.resolve(size[k]),
    })
}

/// A position in the box `r`.
pub(super) fn center(at: &[Offset; 2], r: Rect) -> [f32; 2] {
    [r.x + at[0].resolve(r.width), r.y + at[1].resolve(r.height)]
}

/// How far `c` is from the box's left and right sides, and from its top and bottom.
fn sides(c: [f32; 2], r: Rect) -> [[f32; 2]; 2] {
    [
        [c[0] - r.x, r.x + r.width - c[0]].map(f32::abs),
        [c[1] - r.y, r.y + r.height - c[1]].map(f32::abs),
    ]
}
