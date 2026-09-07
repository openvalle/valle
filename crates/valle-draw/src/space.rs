//! Typed Local, World, and Screen coordinate spaces. Layout and content location produce Local
//! coordinates; placement maps to World; the camera maps to Screen. Mapping types enforce
//! conversion order. Directions use only the linear transform and are renormalized, so translation
//! cannot alter annotation direction.

use core::marker::PhantomData;

use crate::color::Rgba;
use crate::geom::{Point, Rect, Vec2};
use crate::locate::Hit;
use crate::program::recording::Affine;

/// Sealed coordinate-space markers forming the supported protocol.
pub trait Space: sealed::Sealed + Copy + core::fmt::Debug {
    /// Space name for diagnostics.
    const NAME: &'static str;
}

/// Coordinates in the content layer's own canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local;
/// Composition canvas coordinates after placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct World;
/// Coordinates after the scene camera transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Screen;

impl Space for Local {
    const NAME: &'static str = "local";
}
impl Space for World {
    const NAME: &'static str = "world";
}
impl Space for Screen {
    const NAME: &'static str = "screen";
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Local {}
    impl Sealed for super::World {}
    impl Sealed for super::Screen {}
}

/// Point tagged with its coordinate space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpacePoint<S: Space> {
    pub point: Point,
    space: PhantomData<S>,
}

impl<S: Space> SpacePoint<S> {
    pub const fn new(point: Point) -> Self {
        SpacePoint {
            point,
            space: PhantomData,
        }
    }
    pub const fn xy(x: f64, y: f64) -> Self {
        SpacePoint::new(Point::new(x, y))
    }
}

/// Axis-aligned rectangle tagged with its coordinate space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpaceRect<S: Space> {
    pub rect: Rect,
    space: PhantomData<S>,
}

impl<S: Space> SpaceRect<S> {
    pub const fn new(rect: Rect) -> Self {
        SpaceRect {
            rect,
            space: PhantomData,
        }
    }

    /// Convert node-local bounds into the target space through an affine transform, using the
    /// shared axis-aligned bounding-box calculation.
    pub fn of_node(node_box: Rect, node_to_space: Affine) -> Self {
        SpaceRect::new(bbox(node_to_space, node_box))
    }

    pub fn center(self) -> SpacePoint<S> {
        SpacePoint::xy(
            self.rect.x + self.rect.width / 2.0,
            self.rect.y + self.rect.height / 2.0,
        )
    }

    /// Intersect rectangles in the same space; return None when no positive-area intersection
    /// remains.
    pub fn intersect(self, other: SpaceRect<S>) -> Option<SpaceRect<S>> {
        let (a, b) = (self.rect, other.rect);
        let x0 = a.x.max(b.x);
        let y0 = a.y.max(b.y);
        let x1 = (a.x + a.width).min(b.x + b.width);
        let y1 = (a.y + a.height).min(b.y + b.height);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(SpaceRect::new(Rect::new(x0, y0, x1 - x0, y1 - y0)))
    }
}

/// Unit direction tagged with its coordinate space; transforms exclude translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpaceDir<S: Space> {
    pub dir: Vec2,
    space: PhantomData<S>,
}

impl<S: Space> SpaceDir<S> {
    /// Normalize a direction; zero vectors return None.
    pub fn new(dir: Vec2) -> Option<Self> {
        let len = crate::math::sqrt(dir.x * dir.x + dir.y * dir.y);
        if !len.is_finite() || len <= f64::EPSILON {
            return None;
        }
        Some(SpaceDir {
            dir: Vec2::new(dir.x / len, dir.y / len),
            space: PhantomData,
        })
    }
}

/// Typed location result with the same fields as Hit, supporting conversion back to the untyped
/// protocol.
#[derive(Debug, Clone, PartialEq)]
pub struct SpaceHit<S: Space> {
    pub rect: SpaceRect<S>,
    pub anchor: SpacePoint<S>,
    pub outward: SpaceDir<S>,
    pub color: Option<Rgba>,
    pub value: Option<f64>,
}

impl SpaceHit<Local> {
    /// Convert a content-location Hit into Local coordinates.
    pub fn of_hit(hit: &Hit) -> Option<Self> {
        Some(SpaceHit {
            rect: SpaceRect::new(hit.rect),
            anchor: SpacePoint::new(hit.anchor),
            outward: SpaceDir::new(hit.outward)?,
            color: hit.color,
            value: hit.value,
        })
    }
}

impl<S: Space> SpaceHit<S> {
    /// Convert into the untyped Hit representation.
    pub fn into_hit(self) -> Hit {
        Hit {
            rect: self.rect.rect,
            anchor: self.anchor.point,
            outward: self.outward.dir,
            color: self.color,
            value: self.value,
        }
    }

    /// Map bounds and anchors as positions, and outward as a direction.
    pub fn map<B: Space>(self, m: &Mapping<S, B>) -> Option<SpaceHit<B>> {
        Some(SpaceHit {
            rect: m.rect(self.rect),
            anchor: m.point(self.anchor),
            outward: m.dir(self.outward)?,
            color: self.color,
            value: self.value,
        })
    }
}

/// Typed coordinate conversion constructed from the affine matrix for its source and destination
/// spaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mapping<A: Space, B: Space> {
    affine: Affine,
    from: PhantomData<A>,
    to: PhantomData<B>,
}

impl<A: Space, B: Space> Mapping<A, B> {
    pub const fn new(affine: Affine) -> Self {
        Mapping {
            affine,
            from: PhantomData,
            to: PhantomData,
        }
    }

    /// Identity coordinate conversion.
    pub const fn identity() -> Self {
        Mapping::new(Affine::IDENTITY)
    }

    pub const fn affine(self) -> Affine {
        self.affine
    }

    pub fn point(self, p: SpacePoint<A>) -> SpacePoint<B> {
        SpacePoint::new(self.affine.apply(p.point))
    }

    /// Map a rectangle to the axis-aligned bounds of its transformed corners, including rotations.
    pub fn rect(self, r: SpaceRect<A>) -> SpaceRect<B> {
        SpaceRect::new(bbox(self.affine, r.rect))
    }

    /// Transform a direction with the linear matrix only and renormalize; degenerate results return
    /// None.
    pub fn dir(self, d: SpaceDir<A>) -> Option<SpaceDir<B>> {
        let m = self.affine.0;
        SpaceDir::new(Vec2::new(
            m[0] * d.dir.x + m[2] * d.dir.y,
            m[1] * d.dir.x + m[3] * d.dir.y,
        ))
    }

    /// Compose mappings with matching intermediate spaces; incompatible order is rejected by the
    /// type system.
    pub fn then<C: Space>(self, next: Mapping<B, C>) -> Mapping<A, C> {
        Mapping::new(self.affine.then(next.affine))
    }
}

/// Shared transformed-corner bounding-box calculation.
fn bbox(affine: Affine, r: Rect) -> Rect {
    let corners = [
        affine.apply(Point::new(r.x, r.y)),
        affine.apply(Point::new(r.x + r.width, r.y)),
        affine.apply(Point::new(r.x, r.y + r.height)),
        affine.apply(Point::new(r.x + r.width, r.y + r.height)),
    ];
    let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
    let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for c in corners {
        x0 = x0.min(c.x);
        y0 = y0.min(c.y);
        x1 = x1.max(c.x);
        y1 = y1.max(c.y);
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_maps_to_the_bounding_box_of_the_transformed_corners() {
        let m: Mapping<Local, World> = Mapping::new(Affine::translate(10.0, 5.0));
        let r = SpaceRect::<Local>::new(Rect::new(0.0, 0.0, 20.0, 10.0));
        assert_eq!(m.rect(r).rect, Rect::new(10.0, 5.0, 20.0, 10.0));

        // A quarter turn exchanges bounding-box width and height.
        let rot: Mapping<Local, World> = Mapping::new(Affine::rotate(90.0));
        let got = rot
            .rect(SpaceRect::<Local>::new(Rect::new(0.0, 0.0, 20.0, 10.0)))
            .rect;
        assert!((got.width - 10.0).abs() < 1e-9, "got {got:?}");
        assert!((got.height - 20.0).abs() < 1e-9, "got {got:?}");
    }

    /// Direction transforms must ignore translation.
    #[test]
    fn directions_ignore_translation_and_stay_unit() {
        let m: Mapping<Local, World> = Mapping::new(Affine::translate(100.0, -40.0));
        let up = SpaceDir::<Local>::new(Vec2::new(0.0, -1.0)).expect("unit");
        let out = m.dir(up).expect("still a direction");
        assert_eq!(
            out.dir,
            Vec2::new(0.0, -1.0),
            "translation must preserve direction"
        );

        // Scaling preserves the unit-vector invariant after normalization.
        let s: Mapping<Local, World> = Mapping::new(Affine::scale(3.0, 3.0));
        let out = s.dir(up).expect("still a direction");
        assert!(
            (out.dir.y + 1.0).abs() < 1e-12,
            "normalization must yield a unit vector: {:?}",
            out.dir
        );
    }

    #[test]
    fn zero_length_directions_have_no_answer() {
        assert!(SpaceDir::<Local>::new(Vec2::new(0.0, 0.0)).is_none());
        // A collapsed transform cannot produce a meaningful direction.
        let degenerate: Mapping<Local, World> = Mapping::new(Affine::scale(0.0, 0.0));
        let up = SpaceDir::<Local>::new(Vec2::new(0.0, -1.0)).expect("unit");
        assert!(degenerate.dir(up).is_none());
    }

    #[test]
    fn intersection_is_fail_closed_when_empty() {
        let a = SpaceRect::<World>::new(Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = SpaceRect::<World>::new(Rect::new(5.0, 5.0, 10.0, 10.0));
        assert_eq!(
            a.intersect(b).expect("overlap").rect,
            Rect::new(5.0, 5.0, 5.0, 5.0)
        );
        let far = SpaceRect::<World>::new(Rect::new(100.0, 100.0, 1.0, 1.0));
        assert!(
            a.intersect(far).is_none(),
            "disjoint rectangles must return None, not a zero-area rectangle"
        );
    }

    /// Composed mappings match direct Local-to-Screen conversion.
    #[test]
    fn mappings_compose_through_the_middle_space() {
        let l2w: Mapping<Local, World> = Mapping::new(Affine::translate(10.0, 0.0));
        let w2s: Mapping<World, Screen> = Mapping::new(Affine::scale(2.0, 2.0));
        let l2s = l2w.then(w2s);
        let p = SpacePoint::<Local>::xy(1.0, 1.0);
        assert_eq!(l2s.point(p).point, Point::new(22.0, 2.0));
    }

    /// Typed and untyped hits preserve the same data when round-tripped.
    #[test]
    fn space_hit_round_trips_the_existing_protocol_shape() {
        let hit = Hit {
            rect: Rect::new(1.0, 2.0, 3.0, 4.0),
            anchor: Point::new(2.5, 2.0),
            outward: Vec2::new(0.0, -1.0),
            color: Some(Rgba::rgb(255, 0, 0)),
            value: Some(42.0),
        };
        let typed = SpaceHit::of_hit(&hit).expect("has a direction");
        assert_eq!(typed.clone().into_hit(), hit);

        // Translation moves bounds and anchors without changing outward direction.
        let m: Mapping<Local, World> = Mapping::new(Affine::translate(100.0, 100.0));
        let moved = typed.map(&m).expect("mapped");
        assert_eq!(moved.rect.rect, Rect::new(101.0, 102.0, 3.0, 4.0));
        assert_eq!(moved.anchor.point, Point::new(102.5, 102.0));
        assert_eq!(moved.outward.dir, Vec2::new(0.0, -1.0));
    }
}
