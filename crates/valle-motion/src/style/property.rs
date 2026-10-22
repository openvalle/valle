//! Motion property policy shared by source, artifacts, layout and cache admission.
//!
//! CSS grammar, shorthand expansion, initial values and inheritance stay with the
//! shared parser/cascade. This table describes Motion's consumers and dependencies;
//! recognizing a property is never a promise that every CSS value is implemented.
use std::str::FromStr;

use crate::{Expr, ExprId, MotionValue, StyleValue, expr::ExprType};
use takumi_core::style::{Style, StyleDeclaration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyLowering {
    Css,
    Motion,
    Unavailable {
        reason: &'static str,
        suggestion: &'static str,
    },
}

/// Effects are cumulative. Containing-block changes can move positioned descendants
/// even when the property's own border box has not changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertyImpact {
    pub text: bool,
    pub layout: bool,
    pub paint: bool,
    pub compositing: bool,
    pub containing_block: bool,
}
impl PropertyImpact {
    const LAYOUT: Self = Self {
        text: false,
        layout: true,
        paint: true,
        compositing: false,
        containing_block: false,
    };
    const TEXT: Self = Self {
        text: true,
        ..Self::LAYOUT
    };
    const PAINT: Self = Self {
        layout: false,
        ..Self::LAYOUT
    };
    const COMPOSITE: Self = Self {
        compositing: true,
        ..Self::PAINT
    };
    const TRANSFORM: Self = Self {
        containing_block: true,
        ..Self::COMPOSITE
    };
}

/// Proven geometry-cache behavior, including data retained in text layout snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryReuse {
    Rebuild,
    WhenStable,
    AcrossFrames,
}

#[derive(Debug, Clone, Copy)]
enum ValueSyntax {
    Css,
    Typed {
        types: &'static [ExprType],
        static_css: bool,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct PropertySpec<'a> {
    pub name: &'a str,
    pub lowering: PropertyLowering,
    pub impact: PropertyImpact,
    pub geometry_reuse: GeometryReuse,
    pub reads_destination: bool,
    /// String expressions must expose a finite CSS structure, not arbitrary text.
    pub closed_css: bool,
    post_layout: bool,
    syntax: ValueSyntax,
}

/// A CSS property family Motion recognizes and rejects. The same records produce the admission
/// diagnostic and the Unsupported CSS table of the Motion guide.
pub struct UnavailableProperty {
    /// Property names; a name ending in `*` stands for every name with that prefix.
    pub names: &'static [&'static str],
    pub reason: &'static str,
    /// Paste-able replacement for the rejected declaration.
    pub suggestion: &'static str,
}
pub const UNAVAILABLE_PROPERTIES: &[UnavailableProperty] = &[
    UnavailableProperty {
        names: &["animation", "animation-*"],
        reason: "CSS keyframes are permanently unsupported: the timeline must be a function of the sampled frame",
        suggestion: "`interpolate` over local time, for example `interpolate(seconds % 1, [0, 1], [0, 360])`",
    },
    UnavailableProperty {
        names: &["transition", "transition-*"],
        reason: "transitions need an explicit state timeline; a previous rendered frame cannot be used as an event",
        suggestion: "an explicit range: `interpolate(seconds, [start, end], [a, b])`",
    },
    UnavailableProperty {
        names: &["clip-path"],
        reason: "CSS clip-path lowering is not implemented",
        suggestion: "`<Clip path={...} />`",
    },
    UnavailableProperty {
        names: &[
            "mask",
            "mask-image",
            "mask-size",
            "mask-position",
            "mask-repeat",
            "mask-origin",
            "mask-clip",
            "mask-mode",
            "mask-composite",
            "mask-type",
        ],
        reason: "CSS mask lowering is not implemented",
        suggestion: "`<Mask paint={...} />` or a bound image source",
    },
    UnavailableProperty {
        names: &[
            "offset",
            "offset-path",
            "offset-distance",
            "offset-rotate",
            "offset-anchor",
            "offset-position",
        ],
        reason: "CSS offset-path lowering is not implemented",
        suggestion: "the `motionPath` helper",
    },
    UnavailableProperty {
        names: &[
            "border-image",
            "border-image-source",
            "border-image-slice",
            "border-image-width",
            "border-image-outset",
            "border-image-repeat",
        ],
        reason: "border images have no Motion paint lowering",
        suggestion: "a border, a background gradient, or an explicit `<Image>`",
    },
];
/// Author CSS variables are a permanent boundary: a shared value is a `const` in the file. The
/// internal `--tw-*` composition slots belong to the utility lowerer and are not author API.
pub const CUSTOM_PROPERTIES: UnavailableProperty = UnavailableProperty {
    names: &["--*"],
    reason: "author CSS custom properties are not supported",
    suggestion: "a `const` at the top of the file, or an expression for a value that changes per \
                 frame",
};

impl UnavailableProperty {
    fn covers(&self, name: &str) -> bool {
        self.names
            .iter()
            .any(|pattern| match pattern.strip_suffix('*') {
                Some(prefix) => name.starts_with(prefix),
                None => name == *pattern,
            })
    }
}

/// Known value families that must fail explicitly until their lowering is implemented.
/// The same records describe the restriction, produce the admission diagnostic and the
/// Unsupported CSS table of the Motion guide.
pub struct ValueLimit {
    /// The properties whose values are checked; empty for every property.
    pub properties: &'static [&'static str],
    pub keywords: &'static [&'static str],
    pub functions: &'static [&'static str],
    pub reason: &'static str,
    /// Paste-able replacement for the rejected value.
    pub suggestion: &'static str,
}
pub const VALUE_LIMITS: &[ValueLimit] = &[
    ValueLimit {
        properties: &[
            "width",
            "height",
            "min-width",
            "max-width",
            "min-height",
            "max-height",
            "flex-basis",
        ],
        keywords: &["min-content", "max-content", "fit-content"],
        functions: &["fit-content"],
        reason: "intrinsic sizing is not implemented",
        suggestion: "an explicit length or percentage, or `measureText` for text-sized boxes",
    },
    ValueLimit {
        properties: &[
            "width",
            "height",
            "min-width",
            "max-width",
            "min-height",
            "max-height",
            "flex-basis",
            "margin",
            "margin-inline",
            "margin-block",
            "margin-top",
            "margin-right",
            "margin-bottom",
            "margin-left",
            "margin-inline-start",
            "margin-inline-end",
            "padding",
            "padding-inline",
            "padding-block",
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
            "padding-inline-start",
            "padding-inline-end",
            "gap",
            "column-gap",
            "row-gap",
            "inset",
            "inset-inline",
            "inset-block",
            "top",
            "right",
            "bottom",
            "left",
            "font-size",
            "letter-spacing",
            "word-spacing",
        ],
        keywords: &[],
        functions: &["min", "max", "clamp"],
        reason: "min()/max()/clamp() length math is not implemented",
        suggestion: "`calc()`, or a Motion expression for the numeric part",
    },
    ValueLimit {
        properties: &[
            "grid-template-columns",
            "grid-template-rows",
            "grid-auto-columns",
            "grid-auto-rows",
        ],
        keywords: &["subgrid", "masonry"],
        functions: &[],
        reason: "subgrid and masonry layout are not implemented",
        suggestion: "explicit tracks such as `grid-cols-[1fr_2fr]`",
    },
    ValueLimit {
        properties: &["overflow", "overflow-x", "overflow-y"],
        keywords: &["auto", "scroll"],
        functions: &[],
        reason: "scrolling overflow has no Motion layout/paint implementation",
        suggestion: "`overflow-hidden`, `overflow-clip`, or `overflow-visible`",
    },
];
/// Variable consumers fail with the custom properties they would read.
pub const VARIABLE_REFERENCES: ValueLimit = ValueLimit {
    properties: &[],
    keywords: &[],
    functions: &["var"],
    reason: "CSS variable references are not supported",
    suggestion: "a `const` at the top of the file, or a literal value",
};

/// The Unsupported CSS tables of the Motion guide, generated from the admission records.
pub fn unsupported_css_markdown() -> String {
    fn code(names: impl Iterator<Item = String>) -> String {
        names
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }
    fn row(cells: &[&str]) -> String {
        let cells = cells
            .iter()
            .map(|cell| cell.replace('|', "\\|"))
            .collect::<Vec<_>>();
        format!("| {} |\n", cells.join(" | "))
    }
    let mut out = row(&["Property", "Why", "Write instead"]);
    out += "| --- | --- | --- |\n";
    for family in UNAVAILABLE_PROPERTIES.iter().chain([&CUSTOM_PROPERTIES]) {
        let names = code(family.names.iter().map(|name| name.to_string()));
        out += &row(&[&names, family.reason, family.suggestion]);
    }
    out += "\n";
    out += &row(&["Value", "Properties", "Why", "Write instead"]);
    out += "| --- | --- | --- | --- |\n";
    for limit in VALUE_LIMITS.iter().chain([&VARIABLE_REFERENCES]) {
        let values = code(
            limit
                .keywords
                .iter()
                .map(|keyword| keyword.to_string())
                .chain(
                    limit
                        .functions
                        .iter()
                        .map(|function| format!("{function}()")),
                ),
        );
        let properties = if limit.properties.is_empty() {
            "every property".to_string()
        } else {
            code(limit.properties.iter().map(|name| name.to_string()))
        };
        out += &row(&[&values, &properties, limit.reason, limit.suggestion]);
    }
    out
}

impl ValueLimit {
    fn matches(&self, value: &str) -> bool {
        fn scan(limit: &ValueLimit, input: &mut cssparser::Parser<'_, '_>) -> bool {
            use cssparser::Token;
            while let Ok(token) = input.next() {
                match token {
                    Token::Ident(name)
                        if limit
                            .keywords
                            .iter()
                            .any(|word| name.eq_ignore_ascii_case(word)) =>
                    {
                        return true;
                    }
                    Token::Function(name)
                        if limit
                            .functions
                            .iter()
                            .any(|word| name.eq_ignore_ascii_case(word)) =>
                    {
                        return true;
                    }
                    Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock => {
                        if input
                            .parse_nested_block(|input| {
                                Ok::<_, cssparser::ParseError<'_, ()>>(scan(limit, input))
                            })
                            .unwrap_or(false)
                        {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
            false
        }
        scan(
            self,
            &mut cssparser::Parser::new(&mut cssparser::ParserInput::new(value)),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyAdmissionError {
    Unknown(String),
    Unavailable {
        property: String,
        reason: &'static str,
        /// Paste-able replacement for the rejected declaration.
        suggestion: &'static str,
    },
}
impl std::fmt::Display for PropertyAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "unknown or noncanonical CSS property `{name}`"),
            Self::Unavailable {
                property,
                reason,
                suggestion,
            } => write!(
                f,
                "unsupported property `{property}`: {reason}; use {suggestion}"
            ),
        }
    }
}
impl std::error::Error for PropertyAdmissionError {}

impl PropertySpec<'_> {
    /// The value families this property cannot lower yet.
    pub fn value_limits(&self) -> impl Iterator<Item = &'static ValueLimit> + '_ {
        VALUE_LIMITS
            .iter()
            .filter(|limit| limit.properties.contains(&self.name))
    }

    /// Reject a value this property cannot lower, with the replacement to write instead.
    pub fn check_value(&self, value: &str) -> Result<(), super::StyleIssue> {
        if crate::style::contains_variable(value) {
            return Err(super::StyleIssue::new(
                super::StyleIssueKind::UnsupportedValue,
                self.name,
                value,
                VARIABLE_REFERENCES.reason,
            )
            .with_suggestion(VARIABLE_REFERENCES.suggestion));
        }
        for limit in self.value_limits() {
            if limit.matches(value) {
                return Err(super::StyleIssue::new(
                    super::StyleIssueKind::UnsupportedValue,
                    self.name,
                    value,
                    limit.reason,
                )
                .with_suggestion(limit.suggestion));
            }
        }
        Ok(())
    }

    pub fn admit(&self) -> Result<(), PropertyAdmissionError> {
        if CUSTOM_PROPERTIES.covers(self.name) {
            return Err(PropertyAdmissionError::Unavailable {
                property: self.name.into(),
                reason: CUSTOM_PROPERTIES.reason,
                suggestion: CUSTOM_PROPERTIES.suggestion,
            });
        }
        if self.name.is_empty()
            || !self
                .name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err(PropertyAdmissionError::Unknown(self.name.into()));
        }
        match self.lowering {
            PropertyLowering::Unavailable { reason, suggestion } => {
                Err(PropertyAdmissionError::Unavailable {
                    property: self.name.into(),
                    reason,
                    suggestion,
                })
            }
            PropertyLowering::Motion => Ok(()),
            PropertyLowering::Css => {
                if Style::from_str(&format!("{}: initial", self.name))
                    .is_ok_and(|style| !style.declarations.is_empty())
                {
                    Ok(())
                } else {
                    Err(PropertyAdmissionError::Unknown(self.name.into()))
                }
            }
        }
    }

    pub fn accepts_post_layout(&self) -> bool {
        self.post_layout
    }

    pub(crate) fn validate_post_layout(
        &self,
        expr: ExprId,
        exprs: &[Expr],
        types: &[Option<ExprType>],
    ) -> Result<(), &'static str> {
        if !self.requires_css_structure(types.get(expr.0 as usize).copied().flatten())
            || !self.impact.containing_block
        {
            return Ok(());
        }
        let variants = crate::artifact::css_expression_variants(expr, exprs, types)
            .ok_or("post-layout CSS requires a finite property structure")?;
        let mut presence = None;
        let mut importance = None;
        for source in variants {
            let style = super::parse_property(self.name, &source)
                .map_err(|_| "post-layout CSS has an invalid branch")?;
            let important = !style.declarations.importance.is_empty();
            if importance.is_some_and(|previous| previous != important) {
                return Err(
                    "post-layout transforms/filters cannot switch !important: that can change positioned descendants' containing block",
                );
            }
            importance = Some(important);
            let current = match style.declarations.iter().next() {
                _ if matches!(self.name, "translate" | "rotate" | "scale") => {
                    match super::css_keyword(&source).as_deref() {
                        Some("none" | "initial" | "unset") => false,
                        Some("inherit" | "revert" | "revert-layer") => {
                            return Err(
                                "post-layout transforms/filters cannot resolve inherited presence; use an explicit list or none",
                            );
                        }
                        _ => true,
                    }
                }
                Some(StyleDeclaration::Transform(value)) => {
                    value.as_ref().is_some_and(|v| !v.is_empty())
                }
                Some(StyleDeclaration::Filter(value) | StyleDeclaration::BackdropFilter(value)) => {
                    !value.is_empty()
                }
                _ if matches!(
                    super::css_keyword(&source).as_deref(),
                    Some("initial" | "unset")
                ) =>
                {
                    false
                }
                _ => {
                    return Err(
                        "post-layout transforms/filters cannot resolve inherited presence; use an explicit list or none",
                    );
                }
            };
            if presence.is_some_and(|previous| previous != current) {
                return Err(
                    "post-layout transforms/filters cannot switch between none and a list: that changes positioned descendants' containing block",
                );
            }
            presence = Some(current);
        }
        Ok(())
    }

    pub fn can_reuse_geometry(&self, frame_stable: bool) -> bool {
        match self.geometry_reuse {
            GeometryReuse::Rebuild => false,
            GeometryReuse::WhenStable => frame_stable,
            GeometryReuse::AcrossFrames => true,
        }
    }

    pub(crate) fn accepts_typed_value(&self, actual: Option<ExprType>, value: &StyleValue) -> bool {
        match self.syntax {
            ValueSyntax::Css => true,
            ValueSyntax::Typed { types, static_css } => {
                actual.is_some_and(|actual| types.contains(&actual))
                    || self.closed_css && matches!(actual, Some(ExprType::String | ExprType::Enum))
                    || static_css
                        && matches!(
                            value,
                            StyleValue::Static {
                                value: MotionValue::Str(_)
                            }
                        )
            }
        }
    }

    /// Shared source/artifact type check, before any sampled frame or CSS cascade.
    pub fn check_type(
        &self,
        value: &StyleValue,
        expr_types: &[Option<ExprType>],
    ) -> Result<(), super::StyleIssue> {
        let actual = match value {
            StyleValue::Static { value } => Some(ExprType::of_value(value)),
            StyleValue::Expr { expr } => expr_types.get(expr.0 as usize).copied().flatten(),
        };
        if self.accepts_typed_value(actual, value) {
            Ok(())
        } else {
            Err(self.typed_value_issue(actual))
        }
    }

    pub(crate) fn requires_css_structure(&self, actual: Option<ExprType>) -> bool {
        self.closed_css
            && (matches!(self.syntax, ValueSyntax::Css)
                || matches!(actual, Some(ExprType::String | ExprType::Enum)))
    }

    pub(crate) fn typed_value_issue(&self, actual: Option<ExprType>) -> super::StyleIssue {
        let expected = match self.syntax {
            ValueSyntax::Typed { types, .. } => types
                .iter()
                .map(|ty| format!("{ty:?}"))
                .collect::<Vec<_>>()
                .join(", "),
            ValueSyntax::Css => "CSS".into(),
        };
        let mut issue = super::StyleIssue::new(
            super::StyleIssueKind::InvalidValue,
            self.name,
            "<expression>",
            format!(
                "expected {expected}{}; received {}",
                if self.closed_css {
                    " or a finite CSS string"
                } else {
                    ""
                },
                actual.map_or_else(|| "an invalid expression".into(), |ty| format!("{ty:?}")),
            ),
        );
        if let Some(suggestion) = self.dynamic_example() {
            issue = issue.with_suggestion(suggestion);
        }
        issue
    }

    pub(crate) fn css_structure_issue(&self) -> super::StyleIssue {
        let mut issue = super::StyleIssue::new(
            super::StyleIssueKind::InvalidValue,
            self.name,
            "<expression>",
            "dynamic CSS must use literal strings, typed templates, or finite conditional branches; template holes must be numbers, lengths, angles, or colors",
        );
        if let Some(suggestion) = self.dynamic_example() {
            issue = issue.with_suggestion(suggestion);
        }
        issue
    }

    pub(crate) fn dynamic_example(&self) -> Option<&'static str> {
        match self.name {
            "translate" => {
                Some("translate: point(seconds * 100, 0) or translate: `${seconds * 100}px 0px`")
            }
            "rotate" => Some("rotate: `${seconds * 90}deg`"),
            "scale" => Some("scale: 1 + seconds or scale: `${1 + seconds} 1`"),
            "transform" => Some(
                "transform: `translateX(${seconds * 100}px) rotate(${seconds * 90}deg) skew(${seconds * 10}deg)`",
            ),
            _ => None,
        }
    }

    fn typed(mut self, types: &'static [ExprType], static_css: bool) -> Self {
        self.syntax = ValueSyntax::Typed { types, static_css };
        self
    }
}

/// Returns conservative policy even for unknown names; `admit` checks recognition.
/// Policy lookup itself never parses CSS, so prepared artifact consumers can use it
/// without repeating property-name validation in frame/cache decisions.
pub fn property_spec(name: &str) -> PropertySpec<'_> {
    use ExprType as T;
    use GeometryReuse as Reuse;
    use PropertyImpact as Impact;
    use PropertyLowering as Lowering;
    let mut spec = PropertySpec {
        name,
        lowering: Lowering::Css,
        impact: Impact::LAYOUT,
        geometry_reuse: Reuse::WhenStable,
        reads_destination: false,
        closed_css: false,
        post_layout: false,
        syntax: ValueSyntax::Css,
    };
    if name.starts_with("--") {
        return spec.typed(&[T::String, T::Number], false);
    }
    if let Some(family) = UNAVAILABLE_PROPERTIES
        .iter()
        .find(|family| family.covers(name))
    {
        spec.lowering = Lowering::Unavailable {
            reason: family.reason,
            suggestion: family.suggestion,
        };
        spec.geometry_reuse = Reuse::Rebuild;
        return spec;
    }
    if let Some(suffix) = name.strip_prefix("motion-transform-3d-") {
        let matrix_component = suffix.rsplit_once('-').is_some_and(|(function, index)| {
            let count = match function {
                "matrix" => 6,
                "matrix3d" => 16,
                _ => 0,
            };
            index
                .parse::<usize>()
                .is_ok_and(|at| at < count && at.to_string() == index)
        });
        if matrix_component
            || matches!(
                suffix,
                "translate-x"
                    | "translate-y"
                    | "translate-z"
                    | "translate-x-percent"
                    | "translate-y-percent"
                    | "rotate-x"
                    | "rotate-y"
                    | "rotate-z"
                    | "rotate-axis-x"
                    | "rotate-axis-y"
                    | "rotate-axis-z"
                    | "rotate-axis-angle"
                    | "scale-x"
                    | "scale-y"
                    | "scale-z"
                    | "skew-x"
                    | "skew-y"
                    | "perspective"
            )
        {
            spec.lowering = Lowering::Motion;
            spec.impact = Impact::TRANSFORM;
            spec.geometry_reuse = Reuse::AcrossFrames;
            spec.post_layout = true;
            return spec.typed(&[T::Number], false);
        }
    }
    if let Some(suffix) = name.strip_prefix("motion-perspective-origin-") {
        if matches!(
            suffix,
            "x" | "y" | "x-px" | "y-px" | "x-percent" | "y-percent"
        ) {
            spec.lowering = Lowering::Motion;
            spec.impact = Impact::COMPOSITE;
            spec.geometry_reuse = Reuse::AcrossFrames;
            spec.post_layout = true;
            return spec.typed(
                if matches!(suffix, "x" | "y") {
                    &[T::Length]
                } else {
                    &[T::Number]
                },
                false,
            );
        }
    }
    for prefix in ["motion-displacement-", "motion-backdrop-displacement-"] {
        if let Some(suffix) = name.strip_prefix(prefix)
            && matches!(suffix, "scale" | "seed" | "frequency" | "octaves" | "mode")
        {
            spec.lowering = Lowering::Motion;
            spec.impact = Impact::COMPOSITE;
            spec.geometry_reuse = Reuse::Rebuild;
            spec.reads_destination = prefix == "motion-backdrop-displacement-";
            return spec;
        }
    }
    match name {
        "motion-filter-frame" => {
            spec.lowering = Lowering::Motion;
            spec.geometry_reuse = Reuse::AcrossFrames;
            spec.typed(&[T::Number], false)
        }
        "motion-inline-image" => {
            spec.lowering = Lowering::Motion;
            spec.geometry_reuse = Reuse::Rebuild;
            spec.typed(&[T::Bool], false)
        }
        "translate" | "rotate" | "scale" => {
            spec.impact = Impact::TRANSFORM;
            spec.geometry_reuse = Reuse::AcrossFrames;
            // Typed inputs have fixed presence. CSS branches must expose their structure;
            // cache/post-layout admission additionally checks containing-block presence.
            spec.closed_css = true;
            spec.post_layout = true;
            spec.typed(
                match name {
                    "translate" => &[T::Length2, T::Length, T::Point, T::Vec2],
                    "rotate" => &[T::Angle],
                    _ => &[T::Number, T::Point, T::Length, T::Length2],
                },
                true,
            )
        }
        "transform" | "filter" | "backdrop-filter" => {
            spec.impact = Impact::TRANSFORM;
            // Reuse requires a fixed containing-block presence, proved by validate_post_layout.
            spec.geometry_reuse = Reuse::AcrossFrames;
            spec.closed_css = true;
            spec.reads_destination = name == "backdrop-filter";
            // Admission proves presence stable; the layout probe keeps that CSS structure.
            spec.post_layout = true;
            spec
        }
        "opacity" => {
            spec.impact = Impact::COMPOSITE;
            spec.geometry_reuse = Reuse::AcrossFrames;
            spec.post_layout = true;
            spec.typed(&[T::Number], true)
        }
        "rotate-x" | "rotate-y" | "perspective" | "transform-style" | "backface-visibility" => {
            spec.lowering = Lowering::Motion;
            spec.impact = Impact::TRANSFORM;
            // These are post-layout projection inputs; boxes and text metrics do not change.
            spec.geometry_reuse = Reuse::AcrossFrames;
            spec.post_layout = true;
            spec.typed(
                match name {
                    "rotate-x" | "rotate-y" => &[T::Number, T::Angle],
                    "perspective" => &[T::Number, T::Length],
                    // A keyword, or a CSS string chosen per frame.
                    _ => &[T::Enum, T::String],
                },
                false,
            )
        }
        "motion-path-anchor" | "motion-path-angle-offset" => {
            spec.lowering = Lowering::Motion;
            spec.impact = Impact::COMPOSITE;
            spec.geometry_reuse = Reuse::Rebuild;
            spec
        }
        "motion-velocity-blur-auto"
        | "motion-velocity-blur-velocity"
        | "motion-velocity-blur-shutter" => {
            spec.lowering = Lowering::Motion;
            spec.impact = Impact::COMPOSITE;
            spec.geometry_reuse = Reuse::AcrossFrames;
            spec
        }
        "motion-chromatic-aberration-offset"
        | "motion-glow-radius"
        | "motion-glow-intensity"
        | "motion-bloom-threshold"
        | "motion-bloom-knee"
        | "motion-bloom-intensity"
        | "motion-bloom-radius"
        | "motion-radial-blur-center-x"
        | "motion-radial-blur-center-y"
        | "motion-radial-blur-amount"
        | "motion-film-grain-seed"
        | "motion-film-grain-amount"
        | "motion-film-grain-size"
        | "motion-lens-distortion-k1"
        | "motion-lens-distortion-k2" => {
            spec.lowering = Lowering::Motion;
            spec.impact = Impact::COMPOSITE;
            spec.geometry_reuse = Reuse::AcrossFrames;
            spec.typed(&[T::Number], false)
        }
        "motion-glow-color" => {
            spec.lowering = Lowering::Motion;
            spec.impact = Impact::COMPOSITE;
            spec.geometry_reuse = Reuse::AcrossFrames;
            spec.typed(&[T::Color], false)
        }
        "mix-blend-space" => {
            spec.lowering = Lowering::Motion;
            spec.impact = Impact::COMPOSITE;
            spec.geometry_reuse = Reuse::AcrossFrames;
            spec.post_layout = true;
            spec.typed(&[T::String, T::Enum], false)
        }
        "paper-grain" | "contact-shadow" => {
            spec.lowering = Lowering::Motion;
            spec.impact = Impact::COMPOSITE;
            spec.post_layout = true;
            spec.typed(&[T::Number], false)
        }
        "color" | "background-color" | "background-image" | "border-color" | "border-radius"
        | "outline-color" | "box-shadow" | "text-shadow" | "fill" | "stroke" | "mix-blend-mode"
        | "isolation" | "visibility" => {
            spec.impact = if matches!(name, "mix-blend-mode" | "isolation") {
                Impact::COMPOSITE
            } else {
                Impact::PAINT
            };
            spec.post_layout = true;
            spec
        }
        "transform-origin"
        | "object-fit"
        | "object-position"
        | "background-position"
        | "background-size"
        | "background-repeat"
        | "background-clip"
        | "background-origin"
        | "background-blend-mode"
        | "image-rendering"
        | "outline-width"
        | "outline-style"
        | "outline-offset"
        | "z-index" => {
            spec.impact = Impact::PAINT;
            spec
        }
        name if name.starts_with("font-")
            || name.starts_with("text-")
            || matches!(
                name,
                "line-height"
                    | "letter-spacing"
                    | "word-spacing"
                    | "white-space"
                    | "white-space-collapse"
                    | "word-break"
                    | "overflow-wrap"
                    | "direction"
                    | "tab-size"
                    | "line-clamp"
                    | "max-lines"
            ) =>
        {
            spec.impact = Impact::TEXT;
            spec
        }
        _ => spec,
    }
}

/// A deterministic syntax-valid value for the first layout pass. The completed
/// post-layout pass evaluates the real expression from scratch. A shared expression
/// gets the same probe for every property that consumes it.
pub(crate) fn layout_probe_value(
    expr: ExprId,
    exprs: &[Expr],
    types: &[Option<ExprType>],
) -> MotionValue {
    if let Some(source) = crate::artifact::css_expression_variants(expr, exprs, types)
        .and_then(|variants| variants.into_iter().next())
    {
        return MotionValue::Str(source);
    }
    match types.get(expr.0 as usize).copied().flatten() {
        Some(ExprType::Color) => MotionValue::Color(valle_draw::program::AuthorColor::from_srgb8(
            valle_draw::Rgba::new(0, 0, 0, 0),
        )),
        Some(ExprType::String | ExprType::Enum) => MotionValue::Str("initial".into()),
        _ => MotionValue::Number(0.0),
    }
}
