//! Intrinsic text measurement during preparation. Use the same Takumi nodes, fonts, viewport, zero
//! CSS clock, and layout engine as [`crate::build_tree`] so measured dimensions match rendering.
//! Callers must provide fonts and reject an empty font environment before measuring.
//!
//! Options are explicit style properties, not a CSS declaration string: measurement goes through
//! the same typed-value admission as an authored `style` object, so there is no second syntax that
//! could admit a value the renderer would reject.

use std::{collections::BTreeMap, rc::Rc};

use takumi_core::geometry::{AvailableSpace, Size};
use takumi_core::layout::inline::{
    InlineLayoutMode, InlineLayoutRequest, collect_inline_items, create_inline_layout,
    resolve_inline_runs,
};
use takumi_core::layout::tree::{LayoutTree as TakumiLayoutTree, RenderNode};
use takumi_core::resources::font::Fonts;
use takumi_core::resources::glyph::ResolvedGlyph;
use takumi_core::style::{ComputedStyle, SizingContext};
use takumi_core::viewport::Viewport;
use valle_draw::{PathVerb, Point};

use crate::geometry::{MAX_PATH_POINTS, PathData};
use crate::{
    ARTIFACT_FORMAT_VERSION, CapabilitySet, ChildRange, ControlsSchema, LayoutOptions, MotionValue,
    NodeId, NodeKind, SceneArtifact, SceneNode, StyleBinding, StyleValue, TextValue,
    prepare_owned_scene,
};

/// Text measurement request, corresponding to `measureText(text, options)`.
///
/// Every option is the same value, with the same meaning, as the identically named property in an
/// authored `style` object, and the request lowers to the same typed values. Measurement therefore
/// admits exactly what rendering admits: there is no second syntax to keep in sync. String lengths
/// such as `"1rem"` or `"36px"` are the permanent boundary — options are numbers except for
/// `fontFamily` and `fontVariationSettings`, which are strings.
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
    /// CSS OpenType axis settings, e.g. `"wght" 625, "wdth" 80`.
    pub font_variation_settings: Option<&'a str>,
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

/// Placement of the first line's baseline at `origin`, in logical CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOutlineAlign {
    Left,
    Center,
    Right,
}

/// One visible shaped glyph. `cluster` is its source UTF-8 byte offset; `word` and `line` are
/// zero-based logical indices. A whitespace glyph without ink is omitted.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlinedGlyph {
    pub path: String,
    pub x: f64,
    pub y: f64,
    pub advance: f64,
    pub cluster: usize,
    pub word: usize,
    pub line: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OutlineBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Static outline in the same logical coordinate space as a JSX Path.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OutlinedText {
    pub path: String,
    pub glyphs: Vec<OutlinedGlyph>,
    pub bounds: OutlineBounds,
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
    if let Some(settings) = request.font_variation_settings {
        styles.push(binding(
            "font-variation-settings",
            MotionValue::Str(settings.into()),
        ));
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

/// Shape and outline text with exactly the font registry, style admission, and Takumi inline
/// layout used by `measureText` and ordinary Text painting. The first line's baseline is placed
/// at `origin`; alignment shifts its layout box around that x coordinate.
pub fn text_outline(
    request: &TextMeasure<'_>,
    fonts: &Fonts,
    viewport: Viewport,
    origin: Point,
    align: TextOutlineAlign,
) -> Result<OutlinedText, String> {
    use takumi_core::geometry::PathCommand as C;

    if !origin.x.is_finite() || !origin.y.is_finite() {
        return Err("origin must be a finite point".into());
    }
    if request.text.len() > 4096 {
        return Err("text exceeds the 4096-byte outline preparation limit".into());
    }
    let styles = request_styles(request).map_err(|error| error.to_string())?;
    let prepared = prepare_owned_scene(text_artifact(request, styles))
        .map_err(|error| format!("text style: {error}"))?;
    let (wrapper_node, stylesheet) = prepared
        .intrinsic_node(NodeId(0), viewport)
        .map_err(|error| error.to_string())?;
    let context = takumi_core::context::RenderContext::builder()
        .fonts(fonts.snapshot())
        .sizing(SizingContext::builder().viewport(viewport).build())
        .images(Rc::new(Default::default()))
        .stylesheet(stylesheet)
        .time_ms(LayoutOptions::TIME_MS)
        .draw_debug_border(false)
        .style(Box::new(ComputedStyle::default()))
        .build();
    let wrapper = RenderNode::from_node(&context, wrapper_node);
    let (text_node, _) = prepared
        .intrinsic_node(NodeId(1), viewport)
        .map_err(|error| error.to_string())?;
    let root = RenderNode::from_node(&wrapper.context, text_node);
    let mut tree = TakumiLayoutTree::from_render_node(&root);
    let dpr = f64::from(viewport.device_pixel_ratio);
    let available_width = match request.max_width {
        Some(width) => {
            let physical = (width * dpr) as f32;
            if !width.is_finite() || width <= 0.0 || !physical.is_finite() || physical <= 0.0 {
                return Err(format!("maxWidth must be finite and positive, got {width}"));
            }
            AvailableSpace::Definite(physical)
        }
        None => AvailableSpace::MaxContent,
    };
    tree.compute_layout(Size {
        width: available_width,
        height: AvailableSpace::MaxContent,
    });
    let results = tree.into_results();
    let layout = results
        .layout(takumi_core::geometry::NodeId::ROOT)
        .map_err(|_| "layout produced no text box")?;
    let font_style =
        takumi_core::font_style::SizedFontStyle::from_style(&root.context.style, &root.context);
    let built = create_inline_layout(InlineLayoutRequest::in_content_box(
        collect_inline_items(&root),
        Size {
            width: layout.content_box_width(),
            height: layout.content_box_height(),
        },
        &font_style,
        &root.context,
        InlineLayoutMode::Draw,
    ));
    let resolved = resolve_inline_runs(&built, &root.context, layout)
        .map_err(|error| format!("text shaping failed: {error}"))?;
    let first_baseline = resolved
        .runs
        .iter()
        .find_map(|run| {
            run.glyph_run
                .glyphs
                .first()
                .map(|glyph| f64::from(run.glyph_offset(layout).y + glyph.y) / dpr)
        })
        .ok_or("text produced no glyphs")?;
    let width = f64::from(layout.size.width) / dpr;
    let x_shift = origin.x
        - match align {
            TextOutlineAlign::Left => 0.0,
            TextOutlineAlign::Center => width / 2.0,
            TextOutlineAlign::Right => width,
        };
    let y_shift = origin.y - first_baseline;
    // Takumi may prepend a U+2060 direction mark while shaping. Report cluster offsets in the
    // author's text, not in that internal buffer.
    let source_start = built.text.find(request.text).unwrap_or(0);
    let mut verbs = Vec::new();
    let mut points = Vec::new();
    let mut glyphs = Vec::new();
    let mut line_baselines = Vec::<f64>::new();
    let mut bounds = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );

    for run in &resolved.runs {
        let shaped = &run.glyph_run;
        let offset = run.glyph_offset(layout);
        let Some(first) = shaped.glyphs.first() else {
            continue;
        };
        let baseline = f64::from(offset.y + first.y) / dpr;
        let line = match line_baselines
            .iter()
            .position(|at| (at - baseline).abs() <= 1.0)
        {
            Some(line) => line,
            None => {
                line_baselines.push(baseline);
                line_baselines.len() - 1
            }
        };
        for (index, glyph) in shaped.glyphs.iter().enumerate() {
            let Some(resolved) = run.resolved_glyphs.get(&glyph.id) else {
                return Err(format!("font did not resolve glyph {}", glyph.id));
            };
            let outline = match resolved.as_ref() {
                ResolvedGlyph::Outline(outline) => outline,
                ResolvedGlyph::Bitmap(_) => {
                    return Err(format!("glyph {} is a bitmap and has no outline", glyph.id));
                }
            };
            let x = f64::from(offset.x + glyph.x) / dpr + x_shift;
            let y = f64::from(offset.y + glyph.y) / dpr + y_shift;
            let mut glyph_verbs = Vec::new();
            let mut glyph_points = Vec::new();
            let point = |p: &takumi_core::geometry::Point<f32>| {
                Point::new(x + f64::from(p.x) / dpr, y + f64::from(p.y) / dpr)
            };
            for command in outline.paths() {
                match command {
                    C::MoveTo(a) => {
                        glyph_verbs.push(PathVerb::Move);
                        glyph_points.push(point(a));
                    }
                    C::LineTo(a) => {
                        glyph_verbs.push(PathVerb::Line);
                        glyph_points.push(point(a));
                    }
                    C::QuadTo(a, b) => {
                        glyph_verbs.push(PathVerb::Quad);
                        glyph_points.extend([point(a), point(b)]);
                    }
                    C::CubicTo(a, b, c) => {
                        glyph_verbs.push(PathVerb::Cubic);
                        glyph_points.extend([point(a), point(b), point(c)]);
                    }
                    C::Close => glyph_verbs.push(PathVerb::Close),
                }
            }
            if glyph_verbs.is_empty() {
                continue;
            }
            if points.len() + glyph_points.len() > MAX_PATH_POINTS {
                return Err(format!(
                    "text outline exceeds the {MAX_PATH_POINTS}-point path limit"
                ));
            }
            for p in &glyph_points {
                bounds.0 = bounds.0.min(p.x);
                bounds.1 = bounds.1.min(p.y);
                bounds.2 = bounds.2.max(p.x);
                bounds.3 = bounds.3.max(p.y);
            }
            let glyph_path = PathData::new(glyph_verbs.clone(), glyph_points.clone())
                .map_err(|error| format!("invalid glyph outline: {error}"))?;
            let cluster = shaped
                .cluster_ranges
                .get(index)
                .map_or(0, |range| range.start.saturating_sub(source_start));
            let word = word_index(request.text, cluster);
            let next_x = shaped
                .glyphs
                .get(index + 1)
                .map_or(shaped.offset + shaped.advance, |next| next.x);
            glyphs.push(OutlinedGlyph {
                path: glyph_path.to_svg_path(),
                x,
                y,
                advance: f64::from(next_x - glyph.x) / dpr,
                cluster,
                word,
                line,
            });
            verbs.extend(glyph_verbs);
            points.extend(glyph_points);
        }
    }
    if verbs.is_empty() {
        return Err("text produced no vector outline".into());
    }
    let path =
        PathData::new(verbs, points).map_err(|error| format!("invalid text outline: {error}"))?;
    Ok(OutlinedText {
        path: path.to_svg_path(),
        glyphs,
        bounds: OutlineBounds {
            x: bounds.0,
            y: bounds.1,
            width: bounds.2 - bounds.0,
            height: bounds.3 - bounds.1,
        },
    })
}

fn word_index(text: &str, byte_offset: usize) -> usize {
    let mut words: usize = 0;
    let mut in_word = false;
    for (at, ch) in text.char_indices() {
        if at >= byte_offset {
            break;
        }
        if ch.is_whitespace() {
            in_word = false;
        } else if !in_word {
            words += 1;
            in_word = true;
        }
    }
    words.saturating_sub(usize::from(in_word))
}

// This artifact only exists during measurement. It has no frame inputs or resources, and passes
// exactly the same variable, utility and property admission as an authored Text node.
fn text_artifact(request: &TextMeasure<'_>, styles: Vec<StyleBinding>) -> SceneArtifact {
    SceneArtifact {
        format_version: ARTIFACT_FORMAT_VERSION,
        capability_set: CapabilitySet::base(),
        component: "measureText".into(),
        // Measurement is a prepare-time helper; the delivery contract stays with the caller.
        composition: None,
        controls: ControlsSchema {
            props: BTreeMap::new(),
            data: BTreeMap::new(),
            assets: BTreeMap::new(),
        },
        camera: None,
        resource_refs: vec![],
        exprs: vec![],
        instance_groups: vec![],
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
