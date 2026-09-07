//! Content-location protocol using string targets and domain-neutral geometry. Hits use the content
//! layer's own canvas coordinates; consumers map them into composition space.

use std::fmt;

use serde::Serialize;

use crate::color::Rgba;
use crate::geom::{Point, Rect, Vec2};

/// Locate content through validated string paths.
pub trait Locatable {
    /// Return final geometry at progress 1 without rendering.
    fn locate(&self, target: &str) -> Result<Hit, LocateError> {
        self.locate_at(target, 1.0)
    }

    /// Return geometry at progress p. Geometric motion follows progress; visibility-only effects
    /// preserve final geometry. Location does not imply visibility.
    fn locate_at(&self, target: &str, p: f64) -> Result<Hit, LocateError>;

    /// List valid target names for discovery and error messages.
    fn locatable(&self) -> Vec<String>;
}

/// Location result with a suggested anchor and outward direction for domain-independent
/// annotations.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    /// Bounds in the content layer's canvas coordinates.
    pub rect: Rect,
    /// Suggested annotation anchor.
    pub anchor: Point,
    /// Suggested unit direction pointing away from the geometry.
    pub outward: Vec2,
    /// Primary element color for matching annotations.
    pub color: Option<Rgba>,
    /// Original numeric value for labels.
    pub value: Option<f64>,
}

/// Location error including available target names.
#[derive(Debug, Clone, PartialEq)]
pub struct LocateError {
    pub target: String,
    pub message: String,
    pub candidates: Vec<String>,
}

impl fmt::Display for LocateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot locate `{}`: {}", self.target, self.message)?;
        if !self.candidates.is_empty() {
            write!(f, "; available targets: {}", self.candidates.join(", "))?;
        }
        Ok(())
    }
}

impl std::error::Error for LocateError {}
