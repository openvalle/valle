//! The style layer. A variant's inline styles parse once into declared values; each frame sets
//! the declarations bound to expressions and computes every node top down, as CSS computes
//! styles: text properties inherit, `em` resolves against the font size and `vw` against the
//! canvas, which is the viewport, and `display` blockifies in flex and grid containers. Layout
//! reads a Taffy style; paint reads the rest.
//!
//! What the engine does not take is an error, never ignored. Properties Valle will never take say
//! why and what to write instead.

pub mod background;
mod cache;
pub mod color;
mod computed;
pub(crate) mod css;
mod declaration;
pub mod filter;
mod font;
pub mod gradient;
mod grid;
pub mod math;
pub mod offset;
pub mod position;
mod property;
pub mod shape;
mod sheet;
pub mod tailwind;
pub mod transform;
mod units;

pub use computed::{
    Blend, BorderStyle, Cap, Fit, Instance, Join, Paint, PaintPart, Shaded, Shadow, Source, Stroke,
    Style,
};
pub use font::{
    Collapse, Decoration, DecorationStyle, Family, Feature, Font, FontStyle, LineHeight,
    OverflowWrap, TextAlign, TextShadow, TextTransform, VARIANTS, Variant, Variation, WordBreak,
};
pub use property::properties;
pub use sheet::Sheet;
