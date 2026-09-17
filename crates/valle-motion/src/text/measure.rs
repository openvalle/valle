//! Intrinsic text measurement during preparation. Use the same Takumi nodes, fonts, viewport, zero
//! CSS clock, and layout engine as [`crate::build_tree`] so measured dimensions match rendering.
//! Callers must provide fonts and reject an empty font environment before measuring.
//!
//! Options are explicit style properties, not a CSS declaration string: measurement goes through
//! the same typed-value admission as an authored `style` object, so there is no second syntax that
//! could admit a value the renderer would reject.

use std::{collections::BTreeMap, rc::Rc};

use takumi_core::geometry::{AvailableSpace, Size};
use takumi_core::layout::tree::{LayoutTree as TakumiLayoutTree, RenderNode};
use takumi_core::resources::font::Fonts;
use takumi_core::style::{ComputedStyle, SizingContext};
use takumi_core::viewport::Viewport;

use crate::{
    ARTIFACT_FORMAT_VERSION, CapabilitySet, ChildRange, ControlsSchema, FrameControl,
    LayoutOptions, MotionValue, NodeId, NodeKind, OptionalFrameControl, SceneArtifact, SceneNode,
    StyleBinding, StyleValue, TextValue, TimingControls, prepare_owned_scene,
};

/// Text measurement request, corresponding to `measureText(text, options)`.
///
/// Every option is the same value, with the same meaning, as the identically named property in an
/// authored `style` object, and the request lowers to the same typed values. Measurement therefore
/// admits exactly what rendering admits: there is no second syntax to keep in sync. String lengths
/// such as `"1rem"` or `"36px"` are the permanent boundary — options are numbers, and `fontFamily`
/// is the one string.
#[derive(Debug, Clone, Default)]
pub struct TextMeasure<'a> {
    pub text: &'a str,
    /// Font size in CSS pixels. Required: a measurement without a size would silently assume one.
    pub font_size: f64,
    /// Font family, named the way an authored `fontFamily` names it (a bundled family, an
    /// asset alias, or a stack). `None` keeps the renderer's default stack.
    pub font_family: Option<&'a str>,
    /// Font weight, 1-1000.
    pub font_weight: Option<f64>,
    /// Letter spacing in CSS pixels.
    pub letter_spacing: Option<f64>,
    /// Unitless line-height multiplier, exactly like the CSS property: `1.5` is 150% of the font
    /// size. `None` keeps the font's own normal line height.
    pub line_height: Option<f64>,
    /// Available width in CSS pixels. `None` measures unconstrained intrinsic width; a value
    /// constrains wrapping and the resulting height.
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
    /// An option outside the admitted range.
    BadOption {
        option: &'static str,
        detail: String,
    },
    /// A composed style failed the same admission an authored style passes. The reason names the
    /// property, so no separate field repeats it.
    BadStyle { reason: String },
    /// Non-finite, non-positive, or unrepresentable `maxWidth`.
    BadConstraint { max_width: f64 },
    /// Invalid viewport for the shared layout environment.
    BadViewport,
    /// Missing root layout box or non-finite dimensions.
    NoBox,
}

impl core::fmt::Display for MeasureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MeasureError::BadOption { option, detail } => {
                write!(f, "measureText option `{option}`: {detail}")
            }
            MeasureError::BadStyle { reason } => write!(f, "measureText style: {reason}"),
            MeasureError::BadConstraint { max_width } => {
                write!(
                    f,
                    "maxWidth must be finite, positive, and representable by layout, got {max_width}"
                )
            }
            MeasureError::BadViewport => write!(f, "{}", crate::layout::LayoutError::BadViewport),
            MeasureError::NoBox => write!(f, "layout produced no finite box for the measured node"),
        }
    }
}

impl std::error::Error for MeasureError {}

/// Validate one option and report what the author wrote, not just that it was wrong.
fn bad_option(option: &'static str, detail: impl Into<String>) -> MeasureError {
    MeasureError::BadOption {
        option,
        detail: detail.into(),
    }
}

/// Preserve the authored CSS font list, including quoted names and fallback order. Resource
/// aliases are the sole non-CSS spelling admitted by `fontFamily`; serialize one alias as a name.
fn css_family(name: &str) -> String {
    if !name.starts_with("asset://") {
        return name.to_owned();
    }
    let mut quoted = String::new();
    cssparser::serialize_string(name, &mut quoted).expect("String writer");
    quoted
}

/// The style bindings an equivalent authored `Text` node would carry.
fn request_styles(request: &TextMeasure<'_>) -> Result<Vec<StyleBinding>, MeasureError> {
    let finite = |option: &'static str, value: f64, positive: bool| -> Result<f64, MeasureError> {
        if !value.is_finite() || (positive && value <= 0.0) {
            return Err(bad_option(
                option,
                format!(
                    "must be a finite{} number, got {value}",
                    if positive { " positive" } else { "" }
                ),
            ));
        }
        Ok(value)
    };
    let binding = |property: &str, value: MotionValue| StyleBinding {
        property: property.into(),
        value: StyleValue::Static { value },
    };
    let mut styles = vec![binding(
        "font-size",
        MotionValue::Number(finite("fontSize", request.font_size, true)?),
    )];
    if let Some(family) = request.font_family {
        if family.trim().is_empty() {
            return Err(bad_option(
                "fontFamily",
                "must name a font family; omit it to keep the default stack",
            ));
        }
        styles.push(binding("font-family", MotionValue::Str(css_family(family))));
    }
    if let Some(weight) = request.font_weight {
        let weight = finite("fontWeight", weight, true)?;
        if !(1.0..=1000.0).contains(&weight) {
            return Err(bad_option(
                "fontWeight",
                format!("must be between 1 and 1000, got {weight}"),
            ));
        }
        styles.push(binding("font-weight", MotionValue::Number(weight)));
    }
    if let Some(spacing) = request.letter_spacing {
        styles.push(binding(
            "letter-spacing",
            MotionValue::Number(finite("letterSpacing", spacing, false)?),
        ));
    }
    if let Some(height) = request.line_height {
        styles.push(binding(
            "line-height",
            MotionValue::Number(finite("lineHeight", height, false)?),
        ));
    }
    Ok(styles)
}

/// Measure text through the same layout path as [`crate::build_tree`].
pub fn measure_text(
    request: &TextMeasure<'_>,
    fonts: &Fonts,
    viewport: Viewport,
) -> Result<MeasuredBox, MeasureError> {
    let bad_style = |reason: String| MeasureError::BadStyle { reason };
    let styles = request_styles(request)?;
    let prepared = prepare_owned_scene(text_artifact(request, styles)).map_err(|error| {
        // LayoutError's Display intentionally abbreviates artifact errors. Measurements must
        // preserve their concrete property/utility and reason for authored diagnostics.
        let reason = match error {
            crate::layout::LayoutError::InvalidArtifact(errors) => errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
            error => error.to_string(),
        };
        bad_style(reason)
    })?;
    let (node, stylesheet) = prepared
        .intrinsic_node(NodeId(0), viewport)
        .map_err(|error| match error {
            crate::layout::LayoutError::BadViewport => MeasureError::BadViewport,
            error => bad_style(error.to_string()),
        })?;
    let dpr = f64::from(viewport.device_pixel_ratio);
    let available_width = match request.max_width {
        Some(max_width) => {
            let physical = (max_width * dpr) as f32;
            if !max_width.is_finite()
                || max_width <= 0.0
                || !physical.is_finite()
                || physical <= 0.0
            {
                return Err(MeasureError::BadConstraint { max_width });
            }
            AvailableSpace::Definite(physical)
        }
        None => AvailableSpace::MaxContent,
    };

    let context = takumi_core::context::RenderContext::builder()
        .fonts(fonts.snapshot())
        .sizing(SizingContext::builder().viewport(viewport).build())
        .images(Rc::new(Default::default()))
        .stylesheet(stylesheet)
        .time_ms(LayoutOptions::TIME_MS)
        .draw_debug_border(false)
        .style(Box::new(ComputedStyle::default()))
        .build();
    // Compute inheritance through the synthetic root, then measure the Text box alone.
    // Root padding/dimensions do not become part of the measured text geometry.
    let wrapper = RenderNode::from_node(&context, node);
    let (text_node, _) = prepared
        .intrinsic_node(NodeId(1), viewport)
        .map_err(|error| bad_style(error.to_string()))?;
    // Re-enter through the backend's root constructor: it blockifies inline Text roots
    // after applying the inherited context. Extracting a child directly bypasses this.
    let root = RenderNode::from_node(&wrapper.context, text_node);
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
        width: available_width,
        height: AvailableSpace::MaxContent,
    };
    tree.compute_layout(available);
    let results = tree.into_results();
    let layout = results
        .layout(takumi_core::geometry::NodeId::ROOT)
        .map_err(|_| MeasureError::NoBox)?;
    let (width, height) = (
        f64::from(layout.size.width) / dpr,
        f64::from(layout.size.height) / dpr,
    );
    if !width.is_finite() || !height.is_finite() {
        return Err(MeasureError::NoBox);
    }
    Ok(MeasuredBox { width, height })
}

// This artifact only exists during measurement. It has no frame inputs or resources, and passes
// exactly the same variable, utility and property admission as an authored Text node.
fn text_artifact(request: &TextMeasure<'_>, styles: Vec<StyleBinding>) -> SceneArtifact {
    let boundary = FrameControl {
        default: 0,
        min: 0,
        max: None,
    };
    SceneArtifact {
        format_version: ARTIFACT_FORMAT_VERSION,
        capability_set: CapabilitySet::base(),
        component: "measureText".into(),
        // Measurement is a prepare-time helper; the delivery contract stays with the caller.
        composition: None,
        controls: ControlsSchema {
            props: BTreeMap::new(),
            data: BTreeMap::new(),
            timing_seconds: None,
            timing: TimingControls {
                enter_frames: boundary.clone(),
                hold_cycle_frames: OptionalFrameControl {
                    default: None,
                    min: 1,
                    max: None,
                },
                exit_frames: boundary,
            },
            cues: BTreeMap::new(),
            assets: BTreeMap::new(),
            camera: Default::default(),
        },
        camera: None,
        resource_refs: vec![],
        exprs: vec![],
        nodes: vec![
            SceneNode {
                key: "measureRoot".into(),
                kind: NodeKind::Group,
                space: None,
                class_names: vec![],
                class_conditions: BTreeMap::new(),
                styles: vec![],
                visibility: None,
                children: ChildRange { start: 0, end: 1 },
                semantic: None,
            },
            SceneNode {
                key: "measureText".into(),
                kind: NodeKind::Text {
                    text: TextValue::Static {
                        value: request.text.to_owned(),
                    },
                    per_unit: None,
                    path: None,
                },
                space: None,
                class_names: vec![],
                class_conditions: BTreeMap::new(),
                styles,
                visibility: None,
                children: ChildRange::EMPTY,
                semantic: None,
            },
        ],
        node_children: vec![NodeId(1)],
        root: NodeId(0),
    }
}
