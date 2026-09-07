//! Shared geometry, drawing commands, text measurement interfaces, location protocols, and
//! deterministic math. Keep domain-specific concepts outside this crate; animation helpers map
//! caller-supplied progress to geometry.

pub mod anim;
pub mod camera;
pub mod color;
pub mod draw;
/// Shared filter content-bounds calculation for native and web execution.
pub mod filter_bounds;
pub mod geom;
pub mod locate;
pub mod math;
pub mod measure;
pub mod program;
pub mod requirements;
pub mod space;
pub mod text;

pub use anim::{ArcLength, flatten_curve};
pub use camera::Camera;
pub use color::Rgba;
pub use draw::{
    Cap, DrawCmd, DrawList, Join, LineKind, Paint, PathBuilder, PathRef, PathSink, PathVerb, Span,
    Stroke,
};
pub use geom::{Point, Rect, Vec2};
pub use locate::{Hit, Locatable, LocateError};
pub use measure::{ApproxTextMeasure, ResolvedTextStyle, TextMeasure, TextMetrics};
pub use space::{Local, Mapping, Screen, Space, SpaceDir, SpaceHit, SpacePoint, SpaceRect, World};
pub use text::{FontStyle, FontWeight, HAlign, PlacedText, TextStyle, VAlign};
