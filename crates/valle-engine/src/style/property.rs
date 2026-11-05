//! Property names, supported fields and diagnostics.

use super::declaration::{COLUMNS, Declared, ROWS, set};
use super::font::Variant;

use crate::template::V;

/// Properties Valle will never take, by camelCase name or prefix (`*`): why, and what to write
/// instead.
const NEVER: &[(&str, &str, &str)] = &[
    ("animation*", FUNCTION_OF_TIME, ANIMATE),
    ("transition*", FUNCTION_OF_TIME, ANIMATE),
    ("scroll*", NO_SCROLL, NO_SCROLL_INSTEAD),
    ("overscroll*", NO_SCROLL, NO_SCROLL_INSTEAD),
    ("backgroundAttachment", NO_SCROLL, NO_SCROLL_INSTEAD),
    ("cursor", NO_INTERACTION, REMOVE),
    ("pointerEvents", NO_INTERACTION, REMOVE),
    ("userSelect", NO_INTERACTION, REMOVE),
    ("touchAction", NO_INTERACTION, REMOVE),
    ("resize", NO_INTERACTION, REMOVE),
    ("caretColor", NO_INTERACTION, REMOVE),
    ("accentColor", NO_INTERACTION, REMOVE),
    ("appearance", NO_INTERACTION, REMOVE),
    ("willChange", HINT, REMOVE),
    ("contain", HINT, REMOVE),
    ("contentVisibility", HINT, REMOVE),
    ("float", NO_FLOW, USE_FLEX),
    ("clear", NO_FLOW, USE_FLEX),
    ("columns", NO_FLOW, USE_FLEX),
    ("columnCount", NO_FLOW, USE_FLEX),
    ("columnWidth", NO_FLOW, USE_FLEX),
    ("columnFill", NO_FLOW, USE_FLEX),
    ("columnSpan", NO_FLOW, USE_FLEX),
    ("columnRule*", NO_FLOW, USE_FLEX),
    ("tableLayout", NO_FLOW, USE_FLEX),
    ("borderCollapse", NO_FLOW, USE_FLEX),
    ("borderSpacing", NO_FLOW, USE_FLEX),
    ("captionSide", NO_FLOW, USE_FLEX),
    ("emptyCells", NO_FLOW, USE_FLEX),
    (
        "listStyle*",
        NO_FLOW,
        "use flex or grid layout, and make markers and numbers in JS",
    ),
    ("counter*", NO_FLOW, "make numbers in JS"),
    ("breakBefore", NO_FLOW, USE_FLEX),
    ("breakAfter", NO_FLOW, USE_FLEX),
    ("breakInside", NO_FLOW, USE_FLEX),
    ("content", NO_FLOW, "put the text in a <Text>"),
    ("verticalAlign", NO_INLINE, INLINE_INSTEAD),
    (
        "borderImage*",
        "",
        "use a border with a gradient background, or an <Image>",
    ),
    ("zoom", "", "use scale"),
];

const FUNCTION_OF_TIME: &str = "a frame is a function of time, not of earlier frames or events";

const ANIMATE: &str = "animate with interpolate(seconds, …) or spring()";

pub(super) const NO_SCROLL: &str = "the canvas does not scroll";

pub(super) const NO_SCROLL_INSTEAD: &str = "use overflow: hidden, and translate to move content";

const NO_INTERACTION: &str = "a video has no interaction";

const HINT: &str = "it is a browser performance hint the engine does not need";

const REMOVE: &str = "remove it";

pub(super) const NO_FLOW: &str = "there is no document flow";

pub(super) const USE_FLEX: &str = "use flex or grid layout";

pub(super) const NO_INLINE: &str = "there is no HTML inline flow";

pub(super) const INLINE_INSTEAD: &str = "use flex layout, and Span for styled runs of text";

/// Whether a class's declaration `property: value` is one the engine takes, which errors call
/// `what`; what an element's kind allows is checked when its style is.
pub(super) fn accepts(what: &str, property: &str, value: &str) -> Result<(), String> {
    let key = camel(property);
    let field = Field::by_name(&key).ok_or_else(|| unsupported(what, &key))?;
    set(&mut Declared::default(), field, &V::Str(value.into())).map_err(|e| format!("{what}: {e}"))
}

/// That the property `key`, which errors call `what`, is not supported, and why.
pub(super) fn unsupported(what: &str, key: &str) -> String {
    let never = NEVER
        .iter()
        .find(|(name, ..)| match name.strip_suffix('*') {
            Some(prefix) => key.starts_with(prefix),
            None => key == *name,
        });
    match never {
        Some((_, "", instead)) => format!("{what} is not supported: {instead}"),
        Some((_, why, instead)) => format!("{what} is not supported: {why}; {instead}"),
        None => format!("{what} is not supported"),
    }
}

/// A CSS property's name as a style key: `margin-inline-start` as `marginInlineStart`,
/// `-webkit-line-clamp` as `WebkitLineClamp`.
pub(super) fn camel(property: &str) -> String {
    let mut out = String::with_capacity(property.len());
    let mut upper = false;
    for (i, c) in property.chars().enumerate() {
        match c {
            '-' => upper = i > 0 || !property.starts_with("-ms-"),
            c if upper => {
                out.push(c.to_ascii_uppercase());
                upper = false;
            }
            c => out.push(c),
        }
    }
    out
}

/// A style property the engine takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    Display,
    Position,
    Inset(Sides),
    Width,
    Height,
    MinWidth,
    MinHeight,
    MaxWidth,
    MaxHeight,
    BoxSizing,
    AspectRatio,
    /// Both axes, or x or y.
    Overflow,
    OverflowX,
    OverflowY,
    Margin(Sides),
    Padding(Sides),
    FlexDirection,
    FlexWrap,
    FlexGrow,
    FlexShrink,
    FlexBasis,
    Flex,
    Order,
    ZIndex,
    JustifyContent,
    AlignItems,
    AlignSelf,
    AlignContent,
    Gap,
    RowGap,
    ColumnGap,
    /// Of `ROWS` or `COLUMNS`.
    GridTemplate(usize),
    GridTemplateAreas,
    /// `grid-template`.
    GridTemplateAll,
    GridAuto(usize),
    GridAutoFlow,
    Grid,
    /// `grid-row` and `grid-column`.
    GridLines(usize),
    GridStart(usize),
    GridEnd(usize),
    GridArea,
    JustifyItems,
    JustifySelf,
    PlaceItems,
    PlaceSelf,
    PlaceContent,
    Background,
    BackgroundColor,
    BackgroundImage,
    BackgroundSize,
    BackgroundPosition,
    /// x or y.
    BackgroundPositionAxis(usize),
    BackgroundRepeat,
    BackgroundOrigin,
    BackgroundClip,
    BackgroundBlendMode,
    Translate,
    Rotate,
    Scale,
    Transform,
    TransformOrigin,
    Perspective,
    PerspectiveOrigin,
    BackfaceVisibility,
    TransformStyle,
    OffsetPath,
    OffsetDistance,
    OffsetRotate,
    OffsetAnchor,
    OffsetPosition,
    ObjectFit,
    ObjectPosition,
    ImageRendering,
    BorderRadius,
    /// Of the corners from the top left clockwise.
    CornerRadius(usize),
    Border(Sides, BorderPart),
    Outline(BorderPart),
    OutlineOffset,
    BoxShadow,
    Opacity,
    MixBlendMode,
    Isolation,
    Filter,
    MotionBlur,
    BackdropFilter,
    ClipPath,
    MaskImage,
    MaskSize,
    MaskPosition,
    MaskRepeat,
    MaskMode,
    MaskComposite,
    MaskClip,
    MaskOrigin,
    Visibility,
    Fill,
    FillOpacity,
    FillRule,
    Stroke,
    StrokeOpacity,
    StrokeWidth,
    StrokeLinecap,
    StrokeLinejoin,
    StrokeMiterlimit,
    StrokeDasharray,
    StrokeDashoffset,
    StopColor,
    StopOpacity,
    ClipRule,
    /// `marker-start`, `marker-mid` or `marker-end`; `None` for `marker`, all three.
    Marker(Option<usize>),
    Mask,
    MaskType,
    Color,
    FontSize,
    FontWeight,
    FontStretch,
    FontStyle,
    FontFamily,
    FontFeatureSettings,
    FontVariant(Option<Variant>),
    FontVariationSettings,
    LineHeight,
    TextAlign,
    TextIndent,
    LetterSpacing,
    WordSpacing,
    WhiteSpace,
    WhiteSpaceCollapse,
    TextWrapMode,
    TextWrap,
    TextWrapStyle,
    WordBreak,
    OverflowWrap,
    TextTransform,
    TextDecoration,
    TextDecorationLine,
    TextDecorationColor,
    TextDecorationStyle,
    TextDecorationThickness,
    TextDecorationSkipInk,
    TextUnderlineOffset,
    TextOverflow,
    WebkitBoxOrient,
    WebkitLineClamp,
    WebkitTextFillColor,
    WebkitTextStroke,
    WebkitTextStrokeWidth,
    WebkitTextStrokeColor,
    TextShadow,
}

/// What a border property sets on its sides: the line, its width, style and color at once, or one
/// of those.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BorderPart {
    Line,
    Width,
    Style,
    Color,
}

/// The sides a box-edge property sets: the four of a shorthand, one, or logical ones, which in
/// horizontal left-to-right text are left and right (inline) and top and bottom (block).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sides {
    All,
    Top,
    Right,
    Bottom,
    Left,
    Inline,
    Block,
    InlineStart,
    InlineEnd,
    BlockStart,
    BlockEnd,
}

/// The support table: the style properties the engine takes, by their React names. The case
/// runner compares each with Chrome, and makes each other property Chrome knows a case that must
/// fail.
const FIELDS: [(&str, Field); 257] = [
    ("display", Field::Display),
    ("position", Field::Position),
    ("inset", Field::Inset(Sides::All)),
    ("top", Field::Inset(Sides::Top)),
    ("right", Field::Inset(Sides::Right)),
    ("bottom", Field::Inset(Sides::Bottom)),
    ("left", Field::Inset(Sides::Left)),
    ("insetInline", Field::Inset(Sides::Inline)),
    ("insetBlock", Field::Inset(Sides::Block)),
    ("insetInlineStart", Field::Inset(Sides::InlineStart)),
    ("insetInlineEnd", Field::Inset(Sides::InlineEnd)),
    ("insetBlockStart", Field::Inset(Sides::BlockStart)),
    ("insetBlockEnd", Field::Inset(Sides::BlockEnd)),
    ("width", Field::Width),
    ("height", Field::Height),
    ("minWidth", Field::MinWidth),
    ("minHeight", Field::MinHeight),
    ("maxWidth", Field::MaxWidth),
    ("maxHeight", Field::MaxHeight),
    ("inlineSize", Field::Width),
    ("blockSize", Field::Height),
    ("minInlineSize", Field::MinWidth),
    ("minBlockSize", Field::MinHeight),
    ("maxInlineSize", Field::MaxWidth),
    ("maxBlockSize", Field::MaxHeight),
    ("boxSizing", Field::BoxSizing),
    ("aspectRatio", Field::AspectRatio),
    ("overflow", Field::Overflow),
    ("overflowX", Field::OverflowX),
    ("overflowY", Field::OverflowY),
    ("overflowInline", Field::OverflowX),
    ("overflowBlock", Field::OverflowY),
    ("margin", Field::Margin(Sides::All)),
    ("marginTop", Field::Margin(Sides::Top)),
    ("marginRight", Field::Margin(Sides::Right)),
    ("marginBottom", Field::Margin(Sides::Bottom)),
    ("marginLeft", Field::Margin(Sides::Left)),
    ("marginInline", Field::Margin(Sides::Inline)),
    ("marginBlock", Field::Margin(Sides::Block)),
    ("marginInlineStart", Field::Margin(Sides::InlineStart)),
    ("marginInlineEnd", Field::Margin(Sides::InlineEnd)),
    ("marginBlockStart", Field::Margin(Sides::BlockStart)),
    ("marginBlockEnd", Field::Margin(Sides::BlockEnd)),
    ("padding", Field::Padding(Sides::All)),
    ("paddingTop", Field::Padding(Sides::Top)),
    ("paddingRight", Field::Padding(Sides::Right)),
    ("paddingBottom", Field::Padding(Sides::Bottom)),
    ("paddingLeft", Field::Padding(Sides::Left)),
    ("paddingInline", Field::Padding(Sides::Inline)),
    ("paddingBlock", Field::Padding(Sides::Block)),
    ("paddingInlineStart", Field::Padding(Sides::InlineStart)),
    ("paddingInlineEnd", Field::Padding(Sides::InlineEnd)),
    ("paddingBlockStart", Field::Padding(Sides::BlockStart)),
    ("paddingBlockEnd", Field::Padding(Sides::BlockEnd)),
    ("flexDirection", Field::FlexDirection),
    ("flexWrap", Field::FlexWrap),
    ("flexGrow", Field::FlexGrow),
    ("flexShrink", Field::FlexShrink),
    ("flexBasis", Field::FlexBasis),
    ("flex", Field::Flex),
    ("order", Field::Order),
    ("zIndex", Field::ZIndex),
    ("justifyContent", Field::JustifyContent),
    ("alignItems", Field::AlignItems),
    ("alignSelf", Field::AlignSelf),
    ("alignContent", Field::AlignContent),
    ("gap", Field::Gap),
    ("rowGap", Field::RowGap),
    ("columnGap", Field::ColumnGap),
    ("gridTemplateRows", Field::GridTemplate(ROWS)),
    ("gridTemplateColumns", Field::GridTemplate(COLUMNS)),
    ("gridTemplateAreas", Field::GridTemplateAreas),
    ("gridTemplate", Field::GridTemplateAll),
    ("gridAutoRows", Field::GridAuto(ROWS)),
    ("gridAutoColumns", Field::GridAuto(COLUMNS)),
    ("gridAutoFlow", Field::GridAutoFlow),
    ("grid", Field::Grid),
    ("gridRow", Field::GridLines(ROWS)),
    ("gridColumn", Field::GridLines(COLUMNS)),
    ("gridRowStart", Field::GridStart(ROWS)),
    ("gridRowEnd", Field::GridEnd(ROWS)),
    ("gridColumnStart", Field::GridStart(COLUMNS)),
    ("gridColumnEnd", Field::GridEnd(COLUMNS)),
    ("gridArea", Field::GridArea),
    ("justifyItems", Field::JustifyItems),
    ("justifySelf", Field::JustifySelf),
    ("placeItems", Field::PlaceItems),
    ("placeSelf", Field::PlaceSelf),
    ("placeContent", Field::PlaceContent),
    ("background", Field::Background),
    ("backgroundColor", Field::BackgroundColor),
    ("backgroundImage", Field::BackgroundImage),
    ("backgroundSize", Field::BackgroundSize),
    ("backgroundPosition", Field::BackgroundPosition),
    ("backgroundPositionX", Field::BackgroundPositionAxis(0)),
    ("backgroundPositionY", Field::BackgroundPositionAxis(1)),
    ("backgroundRepeat", Field::BackgroundRepeat),
    ("backgroundOrigin", Field::BackgroundOrigin),
    ("backgroundClip", Field::BackgroundClip),
    ("backgroundBlendMode", Field::BackgroundBlendMode),
    ("borderRadius", Field::BorderRadius),
    ("borderTopLeftRadius", Field::CornerRadius(0)),
    ("borderTopRightRadius", Field::CornerRadius(1)),
    ("borderBottomRightRadius", Field::CornerRadius(2)),
    ("borderBottomLeftRadius", Field::CornerRadius(3)),
    ("borderStartStartRadius", Field::CornerRadius(0)),
    ("borderStartEndRadius", Field::CornerRadius(1)),
    ("borderEndEndRadius", Field::CornerRadius(2)),
    ("borderEndStartRadius", Field::CornerRadius(3)),
    ("border", Field::Border(Sides::All, BorderPart::Line)),
    ("borderWidth", Field::Border(Sides::All, BorderPart::Width)),
    ("borderStyle", Field::Border(Sides::All, BorderPart::Style)),
    ("borderColor", Field::Border(Sides::All, BorderPart::Color)),
    ("borderTop", Field::Border(Sides::Top, BorderPart::Line)),
    (
        "borderTopWidth",
        Field::Border(Sides::Top, BorderPart::Width),
    ),
    (
        "borderTopStyle",
        Field::Border(Sides::Top, BorderPart::Style),
    ),
    (
        "borderTopColor",
        Field::Border(Sides::Top, BorderPart::Color),
    ),
    ("borderRight", Field::Border(Sides::Right, BorderPart::Line)),
    (
        "borderRightWidth",
        Field::Border(Sides::Right, BorderPart::Width),
    ),
    (
        "borderRightStyle",
        Field::Border(Sides::Right, BorderPart::Style),
    ),
    (
        "borderRightColor",
        Field::Border(Sides::Right, BorderPart::Color),
    ),
    (
        "borderBottom",
        Field::Border(Sides::Bottom, BorderPart::Line),
    ),
    (
        "borderBottomWidth",
        Field::Border(Sides::Bottom, BorderPart::Width),
    ),
    (
        "borderBottomStyle",
        Field::Border(Sides::Bottom, BorderPart::Style),
    ),
    (
        "borderBottomColor",
        Field::Border(Sides::Bottom, BorderPart::Color),
    ),
    ("borderLeft", Field::Border(Sides::Left, BorderPart::Line)),
    (
        "borderLeftWidth",
        Field::Border(Sides::Left, BorderPart::Width),
    ),
    (
        "borderLeftStyle",
        Field::Border(Sides::Left, BorderPart::Style),
    ),
    (
        "borderLeftColor",
        Field::Border(Sides::Left, BorderPart::Color),
    ),
    (
        "borderInline",
        Field::Border(Sides::Inline, BorderPart::Line),
    ),
    (
        "borderInlineWidth",
        Field::Border(Sides::Inline, BorderPart::Width),
    ),
    (
        "borderInlineStyle",
        Field::Border(Sides::Inline, BorderPart::Style),
    ),
    (
        "borderInlineColor",
        Field::Border(Sides::Inline, BorderPart::Color),
    ),
    ("borderBlock", Field::Border(Sides::Block, BorderPart::Line)),
    (
        "borderBlockWidth",
        Field::Border(Sides::Block, BorderPart::Width),
    ),
    (
        "borderBlockStyle",
        Field::Border(Sides::Block, BorderPart::Style),
    ),
    (
        "borderBlockColor",
        Field::Border(Sides::Block, BorderPart::Color),
    ),
    (
        "borderInlineStart",
        Field::Border(Sides::InlineStart, BorderPart::Line),
    ),
    (
        "borderInlineStartWidth",
        Field::Border(Sides::InlineStart, BorderPart::Width),
    ),
    (
        "borderInlineStartStyle",
        Field::Border(Sides::InlineStart, BorderPart::Style),
    ),
    (
        "borderInlineStartColor",
        Field::Border(Sides::InlineStart, BorderPart::Color),
    ),
    (
        "borderInlineEnd",
        Field::Border(Sides::InlineEnd, BorderPart::Line),
    ),
    (
        "borderInlineEndWidth",
        Field::Border(Sides::InlineEnd, BorderPart::Width),
    ),
    (
        "borderInlineEndStyle",
        Field::Border(Sides::InlineEnd, BorderPart::Style),
    ),
    (
        "borderInlineEndColor",
        Field::Border(Sides::InlineEnd, BorderPart::Color),
    ),
    (
        "borderBlockStart",
        Field::Border(Sides::BlockStart, BorderPart::Line),
    ),
    (
        "borderBlockStartWidth",
        Field::Border(Sides::BlockStart, BorderPart::Width),
    ),
    (
        "borderBlockStartStyle",
        Field::Border(Sides::BlockStart, BorderPart::Style),
    ),
    (
        "borderBlockStartColor",
        Field::Border(Sides::BlockStart, BorderPart::Color),
    ),
    (
        "borderBlockEnd",
        Field::Border(Sides::BlockEnd, BorderPart::Line),
    ),
    (
        "borderBlockEndWidth",
        Field::Border(Sides::BlockEnd, BorderPart::Width),
    ),
    (
        "borderBlockEndStyle",
        Field::Border(Sides::BlockEnd, BorderPart::Style),
    ),
    (
        "borderBlockEndColor",
        Field::Border(Sides::BlockEnd, BorderPart::Color),
    ),
    ("outline", Field::Outline(BorderPart::Line)),
    ("outlineWidth", Field::Outline(BorderPart::Width)),
    ("outlineStyle", Field::Outline(BorderPart::Style)),
    ("outlineColor", Field::Outline(BorderPart::Color)),
    ("outlineOffset", Field::OutlineOffset),
    ("boxShadow", Field::BoxShadow),
    ("opacity", Field::Opacity),
    ("mixBlendMode", Field::MixBlendMode),
    ("isolation", Field::Isolation),
    ("filter", Field::Filter),
    ("motionBlur", Field::MotionBlur),
    ("backdropFilter", Field::BackdropFilter),
    ("clipPath", Field::ClipPath),
    ("maskImage", Field::MaskImage),
    ("maskSize", Field::MaskSize),
    ("maskPosition", Field::MaskPosition),
    ("maskRepeat", Field::MaskRepeat),
    ("maskMode", Field::MaskMode),
    ("maskComposite", Field::MaskComposite),
    ("maskClip", Field::MaskClip),
    ("maskOrigin", Field::MaskOrigin),
    ("translate", Field::Translate),
    ("rotate", Field::Rotate),
    ("scale", Field::Scale),
    ("transform", Field::Transform),
    ("transformOrigin", Field::TransformOrigin),
    ("perspective", Field::Perspective),
    ("perspectiveOrigin", Field::PerspectiveOrigin),
    ("backfaceVisibility", Field::BackfaceVisibility),
    ("transformStyle", Field::TransformStyle),
    ("offsetPath", Field::OffsetPath),
    ("offsetDistance", Field::OffsetDistance),
    ("offsetRotate", Field::OffsetRotate),
    ("offsetAnchor", Field::OffsetAnchor),
    ("offsetPosition", Field::OffsetPosition),
    ("objectFit", Field::ObjectFit),
    ("objectPosition", Field::ObjectPosition),
    ("imageRendering", Field::ImageRendering),
    ("visibility", Field::Visibility),
    ("fill", Field::Fill),
    ("fillOpacity", Field::FillOpacity),
    ("fillRule", Field::FillRule),
    ("stroke", Field::Stroke),
    ("strokeOpacity", Field::StrokeOpacity),
    ("strokeWidth", Field::StrokeWidth),
    ("strokeLinecap", Field::StrokeLinecap),
    ("strokeLinejoin", Field::StrokeLinejoin),
    ("strokeMiterlimit", Field::StrokeMiterlimit),
    ("strokeDasharray", Field::StrokeDasharray),
    ("strokeDashoffset", Field::StrokeDashoffset),
    ("stopColor", Field::StopColor),
    ("stopOpacity", Field::StopOpacity),
    ("clipRule", Field::ClipRule),
    ("markerStart", Field::Marker(Some(0))),
    ("markerMid", Field::Marker(Some(1))),
    ("markerEnd", Field::Marker(Some(2))),
    ("marker", Field::Marker(None)),
    ("mask", Field::Mask),
    ("maskType", Field::MaskType),
    ("color", Field::Color),
    ("fontSize", Field::FontSize),
    ("fontWeight", Field::FontWeight),
    ("fontStretch", Field::FontStretch),
    ("fontStyle", Field::FontStyle),
    ("fontFamily", Field::FontFamily),
    ("fontFeatureSettings", Field::FontFeatureSettings),
    ("fontKerning", Field::FontVariant(Some(Variant::Kerning))),
    ("fontVariant", Field::FontVariant(None)),
    (
        "fontVariantLigatures",
        Field::FontVariant(Some(Variant::Ligatures)),
    ),
    ("fontVariantCaps", Field::FontVariant(Some(Variant::Caps))),
    (
        "fontVariantNumeric",
        Field::FontVariant(Some(Variant::Numeric)),
    ),
    (
        "fontVariantEastAsian",
        Field::FontVariant(Some(Variant::EastAsian)),
    ),
    (
        "fontVariantAlternates",
        Field::FontVariant(Some(Variant::Alternates)),
    ),
    (
        "fontVariantPosition",
        Field::FontVariant(Some(Variant::Position)),
    ),
    ("fontVariationSettings", Field::FontVariationSettings),
    ("lineHeight", Field::LineHeight),
    ("textAlign", Field::TextAlign),
    ("textIndent", Field::TextIndent),
    ("letterSpacing", Field::LetterSpacing),
    ("wordSpacing", Field::WordSpacing),
    ("whiteSpace", Field::WhiteSpace),
    ("whiteSpaceCollapse", Field::WhiteSpaceCollapse),
    ("textWrapMode", Field::TextWrapMode),
    ("textWrap", Field::TextWrap),
    ("textWrapStyle", Field::TextWrapStyle),
    ("wordBreak", Field::WordBreak),
    ("overflowWrap", Field::OverflowWrap),
    ("wordWrap", Field::OverflowWrap),
    ("textTransform", Field::TextTransform),
    ("textDecoration", Field::TextDecoration),
    ("textDecorationLine", Field::TextDecorationLine),
    ("textDecorationColor", Field::TextDecorationColor),
    ("textDecorationStyle", Field::TextDecorationStyle),
    ("textDecorationThickness", Field::TextDecorationThickness),
    ("textDecorationSkipInk", Field::TextDecorationSkipInk),
    ("textUnderlineOffset", Field::TextUnderlineOffset),
    ("textOverflow", Field::TextOverflow),
    ("WebkitBoxOrient", Field::WebkitBoxOrient),
    ("WebkitLineClamp", Field::WebkitLineClamp),
    ("WebkitTextFillColor", Field::WebkitTextFillColor),
    ("WebkitTextStroke", Field::WebkitTextStroke),
    ("WebkitTextStrokeWidth", Field::WebkitTextStrokeWidth),
    ("WebkitTextStrokeColor", Field::WebkitTextStrokeColor),
    ("textShadow", Field::TextShadow),
];

impl Field {
    pub(super) fn by_name(key: &str) -> Option<Field> {
        FIELDS
            .iter()
            .find(|(name, _)| *name == key)
            .map(|&(_, f)| f)
    }

    /// Whether it changes what an element in an `<svg>` draws.
    pub(super) fn drawn_in_svg(self) -> bool {
        matches!(
            self,
            Field::Fill
                | Field::FillOpacity
                | Field::FillRule
                | Field::Stroke
                | Field::StrokeOpacity
                | Field::StrokeWidth
                | Field::StrokeLinecap
                | Field::StrokeLinejoin
                | Field::StrokeMiterlimit
                | Field::StrokeDasharray
                | Field::StrokeDashoffset
                | Field::StopColor
                | Field::StopOpacity
                | Field::ClipPath
                | Field::ClipRule
                | Field::Marker(_)
                | Field::Mask
                | Field::MaskType
                | Field::Opacity
                | Field::ImageRendering
                | Field::Visibility
                | Field::Display
                | Field::Color
        )
    }

    /// Whether it changes how a box paints and never where any box is laid out (a transform and
    /// a filter make a box the containing block of its absolute boxes; `Scene` checks that).
    pub(super) fn paints_only(self) -> bool {
        matches!(
            self,
            Field::Background
                | Field::BackgroundColor
                | Field::BackgroundImage
                | Field::BackgroundSize
                | Field::BackgroundPosition
                | Field::BackgroundPositionAxis(_)
                | Field::BackgroundRepeat
                | Field::BackgroundOrigin
                | Field::BackgroundClip
                | Field::BackgroundBlendMode
                | Field::Translate
                | Field::Rotate
                | Field::Scale
                | Field::Transform
                | Field::TransformOrigin
                | Field::Perspective
                | Field::PerspectiveOrigin
                | Field::OffsetPath
                | Field::OffsetDistance
                | Field::OffsetRotate
                | Field::OffsetAnchor
                | Field::OffsetPosition
                | Field::ObjectPosition
                | Field::BorderRadius
                | Field::CornerRadius(_)
                | Field::Border(_, BorderPart::Color)
                | Field::Outline(_)
                | Field::OutlineOffset
                | Field::BoxShadow
                | Field::Opacity
                | Field::Filter
                | Field::MotionBlur
                | Field::BackdropFilter
                | Field::ClipPath
                | Field::MaskImage
                | Field::MaskSize
                | Field::MaskPosition
                | Field::Visibility
                | Field::ZIndex
                | Field::Color
                | Field::TextDecorationColor
                | Field::TextShadow
                | Field::WebkitTextFillColor
                | Field::WebkitTextStrokeColor
        )
    }

    /// Whether a unit of a split text takes it besides the text properties.
    pub(super) fn on_units(self) -> bool {
        matches!(
            self,
            Field::Opacity
                | Field::Translate
                | Field::Rotate
                | Field::Scale
                | Field::Transform
                | Field::TransformOrigin
                | Field::Filter
        )
    }

    /// Whether it styles text, as a Span takes it.
    pub(super) fn textual(self) -> bool {
        matches!(
            self,
            Field::Visibility
                | Field::Color
                | Field::FontSize
                | Field::FontWeight
                | Field::FontStretch
                | Field::FontStyle
                | Field::FontFamily
                | Field::FontFeatureSettings
                | Field::FontVariant(_)
                | Field::FontVariationSettings
                | Field::LineHeight
                | Field::TextAlign
                | Field::TextIndent
                | Field::LetterSpacing
                | Field::WordSpacing
                | Field::WhiteSpace
                | Field::WhiteSpaceCollapse
                | Field::TextWrapMode
                | Field::TextWrap
                | Field::TextWrapStyle
                | Field::WordBreak
                | Field::OverflowWrap
                | Field::TextTransform
                | Field::TextDecoration
                | Field::TextDecorationLine
                | Field::TextDecorationColor
                | Field::TextDecorationStyle
                | Field::TextDecorationThickness
                | Field::TextDecorationSkipInk
                | Field::TextUnderlineOffset
                | Field::WebkitTextFillColor
                | Field::WebkitTextStroke
                | Field::WebkitTextStrokeWidth
                | Field::WebkitTextStrokeColor
                | Field::TextShadow
        )
    }
}

/// The style properties the engine takes, but `motionBlur`, which is Valle's and which the
/// frontend gives from what scenes write.
pub fn properties() -> impl Iterator<Item = &'static str> {
    FIELDS
        .iter()
        .map(|&(name, _)| name)
        .filter(|&name| name != "motionBlur")
}

#[cfg(test)]
mod tests {

    use crate::template::Prop;

    use crate::testing::styles::{error, num, text};

    #[test]
    fn what_is_not_supported_is_an_error() {
        let cases: &[(&str, Prop, &str)] = &[
            (
                "float",
                text("left"),
                "style float is not supported: there is no document flow; use flex or grid layout",
            ),
            (
                "transitionDuration",
                text("1s"),
                "style transitionDuration is not supported: a frame is a function of time, not of \
             earlier frames or events; animate with interpolate(seconds, …) or spring()",
            ),
            ("zoom", num(2.0), "style zoom is not supported: use scale"),
            (
                "textAlignLast",
                text("1em"),
                "style textAlignLast is not supported",
            ),
            ("width", text("10"), "style width: 10 needs a unit: 10px"),
            (
                "width",
                text("1in"),
                "style width: 1in: in is not supported; use px, em, rem, %, vw, vh, vmin or vmax",
            ),
            ("padding", num(-1.0), "style padding: -1 is negative"),
            (
                "padding",
                text("1px 2px 3px 4px 5px"),
                "takes one to four lengths",
            ),
            ("gap", text("1px 2px 3px"), "takes one or two lengths"),
            (
                "width",
                num(f64::NAN),
                "style width: NaN is not a finite length",
            ),
            (
                "display",
                text("contents"),
                "style display: contents is not one of block, flex",
            ),
            ("display", text("table-cell"), "there is no document flow"),
            ("display", num(1.0), "style display: 1 is not a string"),
            ("position", text("sticky"), "the canvas does not scroll"),
            (
                "position",
                text("float"),
                "float is not one of static, relative, absolute, fixed",
            ),
            (
                "justifyContent",
                text("around"),
                "around is not one of normal, stretch, center",
            ),
            ("color", text("#12"), "style color: #12 is not a color"),
            (
                "background",
                text("url(a.png)"),
                "url(a.png): an image is a resource declared in controls.resources",
            ),
            (
                "background",
                text("linear-gradient(10% red, blue)"),
                "\"10% red\" is not a color stop: a color, then up to two positions",
            ),
            (
                "background",
                text("linear-gradient(red 1px 2px 3px)"),
                "is not a color stop",
            ),
            (
                "background",
                text("linear-gradient(red, 10%, 20%, blue)"),
                "linear-gradient(): 20% is a hint, which goes between two color stops",
            ),
            ("background", text("linear-gradient(10%, red)"), "is a hint"),
            (
                "background",
                text("linear-gradient(to right)"),
                "needs a color stop",
            ),
            (
                "backgroundImage",
                text("linear-gradient(in hsl, red, blue)"),
                "in hsl is not supported yet; use srgb, srgb-linear, lab, oklab, lch or oklch",
            ),
            (
                "backgroundImage",
                text("linear-gradient(in oklch sideways hue, red)"),
                "sideways hue is not a way around the hue circle",
            ),
            (
                "backgroundImage",
                text("linear-gradient(to top bottom, red)"),
                "to top bottom is not a corner",
            ),
            (
                "backgroundImage",
                text("linear-gradient(45deg 90deg, red)"),
                "90deg: a gradient starts with an angle or a side, and a color space",
            ),
            (
                "backgroundImage",
                text("conic-gradient(red 10px)"),
                "conic-gradient(): 10px is not an angle or a percentage",
            ),
            (
                "backgroundImage",
                text("conic-gradient(red calc(10deg + 5%))"),
                "calc(10deg + 5%): 5% is not an angle",
            ),
            (
                "backgroundImage",
                text("conic-gradient(from 10px, red)"),
                "from takes an angle",
            ),
            (
                "backgroundImage",
                text("conic-gradient(at center from 10deg, red)"),
                "conic-gradient(): \"center from 10deg\" is not a position",
            ),
            (
                "backgroundImage",
                text("linear-gradient(red 10deg)"),
                "deg is not supported",
            ),
            (
                "backgroundImage",
                text("radial-gradient(circle 50%, red)"),
                "radial-gradient(): a circle's radius: 50%: percentages are not supported here",
            ),
            (
                "backgroundImage",
                text("radial-gradient(circle 1px 2px, red)"),
                "a circle takes one radius",
            ),
            (
                "backgroundImage",
                text("radial-gradient(ellipse 1px, red)"),
                "an ellipse takes two radii",
            ),
            (
                "backgroundImage",
                text("radial-gradient(-1px, red)"),
                "-1px is negative",
            ),
            (
                "backgroundImage",
                text("radial-gradient(at top 10px, red)"),
                "\"top 10px\" is not a position: one, two or four of left, center, right, top, \
             bottom and lengths",
            ),
            (
                "backgroundImage",
                text("radial-gradient(at left 1px right 2px, red)"),
                "is not a position",
            ),
            (
                "backgroundImage",
                text("radial-gradient(circle in oklab at center, red)"),
                "at: a radial gradient starts with its shape, its size and at its center",
            ),
            ("backgroundImage", text("red"), "red is not an image"),
            (
                "backgroundImage",
                text(""),
                "\"\" is not an image; use none",
            ),
            (
                "background",
                text("red, blue"),
                "only the last layer takes a color, and only one",
            ),
            ("background", text("red blue"), "and only one"),
            ("background", text("foo"), "foo is not a color or an image"),
            (
                "backgroundSize",
                text("1px 2px 3px"),
                "\"1px 2px 3px\" is not a size: cover, contain, or one or two lengths or auto",
            ),
            ("backgroundSize", text("-1px"), "-1px is negative"),
            ("backgroundSize", text(""), "\"\" is not a size; use auto"),
            (
                "backgroundPosition",
                text("center 10px top"),
                "is not a position: one to four of left, center, right, top, bottom and lengths",
            ),
            (
                "backgroundPositionX",
                text("top"),
                "\"top\" is not a position: center, left or right, a length, or a side and a length",
            ),
            (
                "backgroundRepeat",
                text("repeat-x repeat-y"),
                "is not a repeat: repeat-x, repeat-y, or one or two of repeat, space, round and \
             no-repeat",
            ),
            (
                "background",
                text("linear-gradient(red) / 10px"),
                "a slash goes between a position and a size",
            ),
            (
                "background",
                text("linear-gradient(red) fixed"),
                "fixed: the canvas does not scroll",
            ),
            (
                "background",
                text("none text border-box"),
                "text is a clip only, so it goes last",
            ),
            (
                "background",
                text("border-box padding-box content-box"),
                "it has more than two boxes",
            ),
            (
                "transform",
                text("translateZ(10%)"),
                "translatez(): 10%: percentages are not supported here",
            ),
            (
                "transform",
                text("perspective(-1px)"),
                "perspective(): -1px is negative",
            ),
            (
                "transform",
                text("matrix3d(1, 0)"),
                "matrix3d() takes 16 numbers, separated by commas",
            ),
            (
                "transform",
                text("translate(1px, 2px, 3px)"),
                "translate() takes one or two lengths, separated by commas",
            ),
            (
                "transform",
                text("translate(1px 2px)"),
                "translate(): 1px 2px is not a length",
            ),
            (
                "transform",
                text("translateX(10)"),
                "translatex(): 10 needs a unit: 10px",
            ),
            (
                "transform",
                text("scale(2px)"),
                "scale(): 2px is not a number",
            ),
            (
                "transform",
                text("matrix(1, 0, 0, 1, 0)"),
                "matrix() takes six numbers, separated by commas",
            ),
            (
                "transform",
                text("rotate(10)"),
                "rotate(): 10 is not an angle",
            ),
            (
                "transform",
                text("rotate(calc(10deg + 5px))"),
                "rotate(): calc(10deg + 5px): 5px is not an angle",
            ),
            (
                "transform",
                text("spin(1turn)"),
                "spin() is not a transform function",
            ),
            (
                "transform",
                text("translate"),
                "translate is not a transform function",
            ),
            ("transform", text(""), "\"\" is not a transform; use none"),
            (
                "translate",
                text("1px 2px 10%"),
                "10%: percentages are not supported here",
            ),
            ("translate", text("10"), "10 needs a unit: 10px"),
            (
                "translate",
                text(""),
                "\"\" is not a translation: none, or one to three lengths",
            ),
            ("rotate", text("0"), "0 needs a unit: 0deg"),
            (
                "rotate",
                text("10deg 20deg"),
                "\"10deg 20deg\" is not a rotation: none, or an angle about x, y, z or three numbers",
            ),
            ("rotate", num(10.0), "10 is not a string"),
            ("scale", text("2px"), "2px is not a number"),
            ("perspective", text("-10px"), "-10px is negative"),
            (
                "backfaceVisibility",
                text("collapse"),
                "collapse is not one of visible, hidden",
            ),
            (
                "transformStyle",
                text("3d"),
                "3d is not one of flat, preserve-3d",
            ),
            (
                "isolation",
                text("none"),
                "none is not one of auto, isolate",
            ),
            (
                "filter",
                text("brightness(-1)"),
                "brightness(): -1 is negative",
            ),
            (
                "filter",
                text("drop-shadow(1px)"),
                "drop-shadow(): takes a color and two or three lengths: x, y and a standard \
             deviation",
            ),
            (
                "filter",
                text("glow(1)"),
                "glow() is not a filter function: blur, brightness, contrast, drop-shadow, \
             grayscale, hue-rotate, invert, opacity, saturate or sepia",
            ),
            (
                "maskMode",
                text("luminance alpha"),
                "\"luminance alpha\" is not a mask mode: alpha, luminance or match-source",
            ),
            (
                "maskPosition",
                text("right 5px top"),
                "\"right 5px top\" is not a position: one, two or four",
            ),
            (
                "maskImage",
                text("url(a.png)"),
                "url(a.png): an image is a resource declared in controls.resources, such as \
             url(\"resource://logo\")",
            ),
            ("clipPath", text(""), "\"\" is not a clip path; use none"),
            ("clipPath", num(0.0), "0 is not a string"),
            (
                "clipPath",
                text("margin-box"),
                "margin-box is not a basic shape, inset(), circle(), ellipse(), polygon() or path(), \
             or a box, border-box, padding-box or content-box",
            ),
            (
                "clipPath",
                text("circle() ellipse()"),
                "\"circle() ellipse()\" takes one shape",
            ),
            (
                "clipPath",
                text("rect(0 1px 1px 0)"),
                "rect() is not a basic shape: inset(), circle(), ellipse(), polygon() or path()",
            ),
            (
                "clipPath",
                text("inset(1px 2px 3px 4px 5px)"),
                "inset(): takes one to four lengths",
            ),
            (
                "clipPath",
                text("inset(1px round)"),
                "inset(): round needs radii",
            ),
            (
                "clipPath",
                text("circle(-1px)"),
                "circle(): -1px is negative",
            ),
            (
                "clipPath",
                text("circle(closest-corner)"),
                "circle(): closest-corner is not a length",
            ),
            (
                "clipPath",
                text("ellipse(10px)"),
                "ellipse(): takes two radii or none",
            ),
            (
                "clipPath",
                text("polygon()"),
                "polygon(): takes points, each two lengths",
            ),
            (
                "clipPath",
                text("polygon(nonzero 0 0, 1px 1px)"),
                "polygon(): \"nonzero 0 0\" is not a point: two lengths, x and y",
            ),
            (
                "clipPath",
                text("path(M0 0)"),
                "path(): takes a fill rule or not, then path data in quotes",
            ),
            (
                "clipPath",
                text("path('M0 0 X')"),
                "path(): \"M0 0 X\" is not path data: X is not a command",
            ),
            (
                "backgroundBlendMode",
                text("normal, plus-lighter"),
                "plus-lighter is not one of normal, multiply, screen, overlay, darken, lighten, \
             color-dodge, color-burn, hard-light, soft-light, difference, exclusion, hue, \
             saturation, color, luminosity",
            ),
            (
                "mixBlendMode",
                text("add"),
                "add is not one of normal, multiply, screen, overlay, darken, lighten, color-dodge, \
             color-burn, hard-light, soft-light, difference, exclusion, hue, saturation, color, \
             luminosity, plus-lighter",
            ),
            (
                "perspective",
                text("10%"),
                "10%: percentages are not supported here",
            ),
            (
                "perspectiveOrigin",
                text("left top 10px"),
                "\"left top 10px\" is not a perspective origin: one or two of left, center, right, \
             top, bottom and lengths, or keywords with lengths after them",
            ),
            ("transform", num(0.0), "0 is not a string"),
            (
                "transformOrigin",
                text("top 10px"),
                "\"top 10px\" is not a transform origin: one or two of left, center, right, top, \
             bottom and lengths, and a length",
            ),
            (
                "transformOrigin",
                text("left top 10%"),
                "\"left top 10%\" is not a transform origin: one or two of left, center, right, top, \
             bottom and lengths, and a length",
            ),
            (
                "backgroundOrigin",
                text("text"),
                "\"text\" is not a box: border-box, padding-box or content-box",
            ),
            (
                "background",
                text("blue 1px 2px 3px 4px 5px"),
                "is not a position",
            ),
            (
                "borderRadius",
                text("1px / 2px / 3px"),
                "takes one slash at most",
            ),
            (
                "borderRadius",
                text("1px 2px 3px 4px 5px / 1px"),
                "takes one to four lengths",
            ),
            (
                "borderTopLeftRadius",
                text("1px 2px 3px"),
                "takes one or two lengths",
            ),
            ("borderEndStartRadius", text("-1px"), "-1px is negative"),
            (
                "border",
                text("1px 2px solid"),
                "sets a width, a style or a color twice",
            ),
            ("border", text(" "), "is not a line"),
            ("borderTop", text("1px solid nope"), "nope is not a color"),
            (
                "borderStyle",
                text("wavy"),
                "wavy is not one of none, hidden",
            ),
            ("outlineStyle", text("auto"), "focus ring; use solid"),
            (
                "outline",
                text("2px hidden"),
                "hidden is not an outline style",
            ),
            (
                "outlineOffset",
                text("10%"),
                "percentages are not supported here",
            ),
            ("outlineWidth", text("-1px"), "-1px is negative"),
            ("boxShadow", text("1px"), "needs two to four lengths"),
            (
                "boxShadow",
                text("1px 1px 1px 1px 1px"),
                "needs two to four lengths",
            ),
            (
                "boxShadow",
                text("1px 1px red,"),
                "needs two to four lengths",
            ),
            ("boxShadow", text("1px 1px -2px"), "-2px is negative"),
            (
                "boxShadow",
                text("1px 1px 10%"),
                "percentages are not supported here",
            ),
            ("boxShadow", text("1px red 1px"), "is not a shadow"),
            ("boxShadow", text("inset inset 1px 1px"), "is not a shadow"),
            ("boxShadow", text(""), "is not a shadow; use none"),
            (
                "borderStyle",
                text("wavy"),
                "wavy is not one of none, hidden, dotted",
            ),
            (
                "borderWidth",
                text("10%"),
                "percentages are not supported here",
            ),
            ("borderTopWidth", text("1px 2px"), "1px 2px is not a length"),
            (
                "borderInlineColor",
                text("red green blue"),
                "takes one or two colors",
            ),
            (
                "borderColor",
                text("red red red red red"),
                "takes one to four colors",
            ),
            ("fontWeight", num(0.0), "0 is not a weight from 1 to 1000"),
            (
                "fontFamily",
                text("Inter, cursive"),
                "cursive is not built in",
            ),
            ("fontFamily", text("Inter,,serif"), "names an empty family"),
            (
                "fontFamily",
                text("1Font"),
                "1Font is not a family name: quote it",
            ),
            (
                "letterSpacing",
                text("10%"),
                "percentages are not supported here",
            ),
        ];
        for (key, value, message) in cases {
            let e = error(&[(*key, value.clone())]);
            assert!(
                e.starts_with("s.tsx:1:1: ") && e.contains(message),
                "{key}: {e}"
            );
        }
    }
}
