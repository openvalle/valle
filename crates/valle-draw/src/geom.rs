//! Shared two-dimensional geometry without domain-specific concepts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }

    pub const UP: Vec2 = Vec2::new(0.0, -1.0);
    pub const DOWN: Vec2 = Vec2::new(0.0, 1.0);
    pub const LEFT: Vec2 = Vec2::new(-1.0, 0.0);
    pub const RIGHT: Vec2 = Vec2::new(1.0, 0.0);
}

/// Axis-aligned rectangle in canvas coordinates; y points down and marks the top edge.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    pub fn from_edges(left: f64, top: f64, right: f64, bottom: f64) -> Self {
        Rect::new(left, top, right - left, bottom - top)
    }

    pub fn left(&self) -> f64 {
        self.x
    }
    pub fn right(&self) -> f64 {
        self.x + self.width
    }
    pub fn top(&self) -> f64 {
        self.y
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }
    pub fn center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// Inset each edge, allowing negative values for expansion. Clamp dimensions to zero when
    /// margins consume all available space.
    pub fn inset(&self, left: f64, top: f64, right: f64, bottom: f64) -> Rect {
        Rect::new(
            self.x + left,
            self.y + top,
            (self.width - left - right).max(0.0),
            (self.height - top - bottom).max(0.0),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inset_clamps_instead_of_going_negative() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        assert_eq!(
            r.inset(10.0, 5.0, 10.0, 5.0),
            Rect::new(10.0, 5.0, 80.0, 40.0)
        );
        // Excessive margins produce zero area rather than negative dimensions.
        let squeezed = r.inset(80.0, 0.0, 80.0, 0.0);
        assert_eq!(squeezed.width, 0.0);
        assert!(squeezed.is_empty());
    }

    #[test]
    fn edges_and_center_agree() {
        let r = Rect::from_edges(10.0, 20.0, 110.0, 70.0);
        assert_eq!((r.width, r.height), (100.0, 50.0));
        assert_eq!(r.right(), 110.0);
        assert_eq!(r.bottom(), 70.0);
        assert_eq!(r.center(), Point::new(60.0, 45.0));
    }
}
