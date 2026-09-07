//! Intrinsic text measurement during preparation. Use the same Takumi nodes, fonts, viewport, zero
//! CSS clock, and layout engine as [`crate::build_tree`] so measured dimensions match rendering.
//! Callers must provide fonts and reject an empty font environment before measuring.

use std::rc::Rc;
use std::str::FromStr;

use takumi_core::geometry::{AvailableSpace, Size};
use takumi_core::layout::node::Node;
use takumi_core::layout::tree::{LayoutTree as TakumiLayoutTree, RenderNode};
use takumi_core::resources::font::Fonts;
use takumi_core::style::{ComputedStyle, SizingContext, Style, TailwindValues};
use takumi_core::viewport::Viewport;

use crate::LayoutOptions;

/// Text measurement request, corresponding to `measureText(text, options)`.
#[derive(Debug, Clone, Default)]
pub struct TextMeasure<'a> {
    pub text: &'a str,
    /// Tailwind classes from the same catalog used by Scene nodes.
    pub class_names: &'a [String],
    /// Inline CSS declarations parsed as Scene styles.
    pub style: &'a str,
    /// Available width in pixels. `None` measures unconstrained intrinsic width; a value constrains
    /// wrapping and the resulting height.
    pub max_width: Option<f64>,
}

/// Measured dimensions in CSS pixels, using layout-box coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasuredBox {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MeasureError {
    /// Invalid Tailwind classes or CSS declarations.
    BadStyle {
        declarations: String,
        reason: String,
    },
    /// Non-finite or non-positive `maxWidth`.
    BadConstraint { max_width: f64 },
    /// Missing root layout box or non-finite dimensions.
    NoBox,
}

impl core::fmt::Display for MeasureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MeasureError::BadStyle {
                declarations,
                reason,
            } => write!(f, "cannot parse `{declarations}`: {reason}"),
            MeasureError::BadConstraint { max_width } => {
                write!(f, "maxWidth must be finite and positive, got {max_width}")
            }
            MeasureError::NoBox => write!(f, "layout produced no finite box for the measured node"),
        }
    }
}

impl std::error::Error for MeasureError {}

/// Measure text through the same layout path as [`crate::build_tree`].
pub fn measure_text(
    request: &TextMeasure<'_>,
    fonts: &Fonts,
    viewport: Viewport,
) -> Result<MeasuredBox, MeasureError> {
    if let Some(max_width) = request.max_width
        && (!max_width.is_finite() || max_width <= 0.0)
    {
        return Err(MeasureError::BadConstraint { max_width });
    }

    let mut node = Node::text(request.text.to_owned());
    if !request.class_names.is_empty() {
        let joined = request.class_names.join(" ");
        let tw = TailwindValues::from_str(&joined)
            .unwrap_or_else(|_| unreachable!("Takumi TailwindValues::from_str is infallible"));
        node = node.with_tw(tw).with_class_name(joined);
    }
    if !request.style.trim().is_empty() {
        node = node.with_style(parse(request.style)?);
    }

    let context = takumi_core::context::RenderContext::builder()
        .fonts(fonts.snapshot())
        .sizing(SizingContext::builder().viewport(viewport).build())
        .images(Rc::new(Default::default()))
        .stylesheet(Default::default())
        .time_ms(LayoutOptions::TIME_MS)
        .draw_debug_border(false)
        .style(Box::new(ComputedStyle::default()))
        .build();
    let root = RenderNode::from_node(&context, node);
    let mut tree = TakumiLayoutTree::from_render_node(&root);
    // Express intrinsic constraints through `AvailableSpace`, without changing node styles.
    //
    // A definite viewport width would make the block root fill the viewport rather than measure its
    // content.
    //
    // Use `MaxContent` for intrinsic width or `Definite(w)` for constrained wrapping. Both preserve
    // the original styles.
    //
    // Keep the viewport in the sizing context for relative units; available space is a separate
    // constraint.
    let available = Size {
        width: match request.max_width {
            Some(max_width) => AvailableSpace::Definite(max_width as f32),
            None => AvailableSpace::MaxContent,
        },
        height: AvailableSpace::MaxContent,
    };
    tree.compute_layout(available);
    let results = tree.into_results();
    let layout = results
        .layout(takumi_core::geometry::NodeId::ROOT)
        .map_err(|_| MeasureError::NoBox)?;
    let (width, height) = (f64::from(layout.size.width), f64::from(layout.size.height));
    if !width.is_finite() || !height.is_finite() {
        return Err(MeasureError::NoBox);
    }
    Ok(MeasuredBox { width, height })
}

fn parse(declarations: &str) -> Result<Style, MeasureError> {
    Style::from_str(declarations).map_err(|error| MeasureError::BadStyle {
        declarations: declarations.to_owned(),
        reason: error.to_string(),
    })
}
