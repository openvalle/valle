//! CSS-wide keywords operate on longhands at their declaration position. Only parents whose
//! children request explicit inheritance retain a resolved declaration snapshot.
use super::{
    Style, computed,
    declaration::{self, Declared, Shown},
    font::{Leading, LineHeight, Weight},
    math::Calc,
    property::{BorderPart, Field},
    units::{Len, Units},
};
use crate::template::V;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Keyword {
    Inherit,
    Initial,
    Unset,
}
impl Keyword {
    pub fn of(value: &V) -> Option<Self> {
        let V::Str(value) = value else { return None };
        match value.trim() {
            s if s.eq_ignore_ascii_case("inherit") => Some(Self::Inherit),
            s if s.eq_ignore_ascii_case("initial") => Some(Self::Initial),
            s if s.eq_ignore_ascii_case("unset") => Some(Self::Unset),
            _ => None,
        }
    }
}

/// A resolved declaration snapshot, kept only where a descendant explicitly inherits.
#[derive(Clone, Debug)]
pub(crate) struct Snapshot(pub(super) Declared);
impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

pub(super) fn apply(
    d: &mut Declared,
    field: Field,
    value: &V,
    parent: Option<&Style>,
) -> Result<(), String> {
    let Some(keyword) = Keyword::of(value) else {
        return declaration::set(d, field, value);
    };
    if field == Field::XmlSpace {
        return Err("xml:space takes default or preserve".into());
    }
    let inherit = keyword == Keyword::Inherit || (keyword == Keyword::Unset && inherited(field));
    let initial;
    let source = if inherit && let Some(snapshot) = parent.and_then(|s| s.specified.as_ref()) {
        &snapshot.0
    } else {
        initial = initial_values()?;
        &initial
    };
    copy(d, source, field);
    Ok(())
}

fn initial_values() -> Result<Declared, String> {
    let d = Declared::default();
    let style = computed::compute(&d, None, [0.0; 2], 0, 0, false, &[])?;
    let mut d = resolved(&d, &style, [0.0; 2]);
    // Valle's element defaults differ from these CSS initial values.
    d.box_sizing = taffy::BoxSizing::ContentBox;
    d.display = Shown::Inline;
    d.border_width = std::array::from_fn(|_| Len::Px(3.0));
    d.outline_width = Len::Px(3.0);
    Ok(d)
}

fn inherited(field: Field) -> bool {
    use Field::*;
    matches!(
        field,
        Font | Color
            | FontSize
            | FontWeight
            | FontStretch
            | FontStyle
            | FontFamily
            | FontFeatureSettings
            | FontVariant(_)
            | FontVariationSettings
            | LineHeight
            | TextAlign
            | TextAlignLast
            | LineBreak
            | WritingMode
            | TextOrientation
            | Direction
            | TextCombineUpright
            | TextIndent
            | LetterSpacing
            | WordSpacing
            | WhiteSpace
            | WhiteSpaceCollapse
            | TextWrapMode
            | TextWrap
            | TextWrapStyle
            | WordBreak
            | OverflowWrap
            | TextTransform
            | TextEmphasis
            | TextEmphasisStyle
            | TextEmphasisColor
            | TextEmphasisPosition
            | TextDecorationSkipInk
            | TextUnderlineOffset
            | TextUnderlinePosition
            | WebkitTextFillColor
            | WebkitTextStroke
            | WebkitTextStrokeWidth
            | WebkitTextStrokeColor
            | TextShadow
            | Visibility
            | ImageRendering
            | Fill
            | FillOpacity
            | FillRule
            | Stroke
            | StrokeOpacity
            | StrokeWidth
            | StrokeLinecap
            | StrokeLinejoin
            | StrokeMiterlimit
            | StrokeDasharray
            | StrokeDashoffset
            | TextAnchor
            | ClipRule
            | Marker(_)
            | PaintOrder
            | ColorInterpolation
            | ColorInterpolationFilters
    )
}

/// Assign precisely the longhands affected by a property, keeping shorthand/longhand order.
fn copy(d: &mut Declared, s: &Declared, field: Field) {
    use Field as F;
    macro_rules! fields { ($($name:ident),+ $(,)?) => {{ $(d.$name.clone_from(&s.$name);)+ }}; }
    match field {
        F::Display => fields!(display),
        F::Position => fields!(position),
        F::BoxSizing => fields!(box_sizing),
        F::AspectRatio => fields!(aspect_ratio),
        F::FlexDirection => fields!(flex_direction),
        F::FlexWrap => fields!(flex_wrap),
        F::FlexGrow => fields!(flex_grow),
        F::FlexShrink => fields!(flex_shrink),
        F::FlexBasis => fields!(flex_basis),
        F::Flex => fields!(flex_grow, flex_shrink, flex_basis),
        F::Order => fields!(order),
        F::ZIndex => fields!(z_index),
        F::Overflow => fields!(overflow),
        F::JustifyContent => fields!(justify_content),
        F::AlignItems => fields!(align_items),
        F::AlignSelf => fields!(align_self),
        F::AlignContent => fields!(align_content),
        F::Gap => fields!(gap),
        F::GridTemplateAreas => fields!(grid_areas),
        F::GridTemplateAll => fields!(grid_template, grid_areas),
        F::GridAutoFlow => fields!(grid_auto_flow),
        F::Grid => fields!(grid_template, grid_areas, grid_auto, grid_auto_flow),
        F::GridArea => fields!(grid_lines),
        F::JustifyItems => fields!(justify_items),
        F::JustifySelf => fields!(justify_self),
        F::PlaceItems => fields!(align_items, justify_items),
        F::PlaceSelf => fields!(align_self, justify_self),
        F::PlaceContent => fields!(align_content, justify_content),
        F::BackgroundColor => fields!(background),
        F::BorderRadius => fields!(radii),
        F::OutlineOffset => fields!(outline_offset),
        F::BoxShadow => fields!(shadows),
        F::Translate => fields!(translate),
        F::Rotate => fields!(rotate),
        F::Scale => fields!(scale),
        F::Transform => {
            fields!(transform);
            d.transform_set = true;
        }
        F::TransformBox => fields!(transform_box),
        F::TransformOrigin => fields!(transform_origin, transform_origin_z),
        F::Perspective => fields!(perspective),
        F::PerspectiveOrigin => fields!(perspective_origin),
        F::BackfaceVisibility => fields!(backface_visible),
        F::TransformStyle => fields!(preserve_3d),
        F::OffsetPath => fields!(offset_path),
        F::OffsetDistance => fields!(offset_distance),
        F::OffsetRotate => fields!(offset_rotate),
        F::OffsetAnchor => fields!(offset_anchor),
        F::OffsetPosition => fields!(offset_position),
        F::ObjectFit => fields!(object_fit),
        F::ObjectPosition => fields!(object_position),
        F::ImageRendering => fields!(pixelated),
        F::Opacity => fields!(opacity),
        F::MixBlendMode => fields!(blend),
        F::Isolation => fields!(isolate),
        F::Filter => fields!(filter, filter_ref),
        F::LightingColor => fields!(lighting_color),
        F::FloodColor => fields!(flood_color),
        F::FloodOpacity => fields!(flood_opacity),
        F::ColorInterpolation => fields!(gradient_linear),
        F::ColorInterpolationFilters => fields!(filter_linear),
        F::MotionBlur => fields!(motion_blur),
        F::BackdropFilter => fields!(backdrop),
        F::ClipPath => fields!(clip, clip_ref),
        F::Visibility => fields!(visible),
        F::Fill => fields!(svg_fill),
        F::FillOpacity => fields!(fill_opacity),
        F::FillRule => fields!(even_odd),
        F::PaintOrder => fields!(paint_order),
        F::Stroke => fields!(svg_stroke),
        F::StrokeOpacity => fields!(stroke_opacity),
        F::TextAnchor => fields!(text_anchor),
        F::DominantBaseline => fields!(dominant_baseline),
        F::AlignmentBaseline => fields!(alignment_baseline),
        F::BaselineShift => fields!(baseline_shift),
        F::VectorEffect => fields!(non_scaling_stroke),
        F::StrokeWidth => fields!(stroke_width),
        F::StrokeLinecap => fields!(cap),
        F::StrokeLinejoin => fields!(join),
        F::StrokeMiterlimit => fields!(miter_limit),
        F::StrokeDasharray => fields!(dashes),
        F::StrokeDashoffset => fields!(dash_offset),
        F::StopColor => fields!(stop_color),
        F::StopOpacity => fields!(stop_opacity),
        F::ClipRule => fields!(clip_even_odd),
        F::Mask => fields!(mask_ref),
        F::MaskType => fields!(mask_alpha),
        F::Color => fields!(color),
        F::Font => fields!(
            font_size,
            font_weight,
            stretch,
            font_style,
            font_family,
            variants,
            features,
            variations,
            line_height
        ),
        F::FontSize => fields!(font_size),
        F::FontWeight => fields!(font_weight),
        F::FontStretch => fields!(stretch),
        F::FontStyle => fields!(font_style),
        F::FontFamily => fields!(font_family),
        F::FontFeatureSettings => fields!(features),
        F::FontVariationSettings => fields!(variations),
        F::LineHeight => fields!(line_height),
        F::WritingMode => fields!(writing),
        F::TextCombineUpright => fields!(combine),
        F::Direction => fields!(rtl),
        F::UnicodeBidi => fields!(bidi),
        F::TextOrientation => fields!(orientation),
        F::TextAlign => fields!(text_align),
        F::TextAlignLast => fields!(align_last),
        F::LineBreak => fields!(line_break),
        F::TextIndent => fields!(text_indent),
        F::LetterSpacing => fields!(letter_spacing),
        F::WordSpacing => fields!(word_spacing),
        F::WhiteSpace => fields!(collapse, wrap),
        F::WhiteSpaceCollapse => fields!(collapse),
        F::TextWrapMode => fields!(wrap),
        F::TextWrap => fields!(wrap, wrap_style),
        F::TextWrapStyle => fields!(wrap_style),
        F::WordBreak => fields!(word_break),
        F::OverflowWrap => fields!(overflow_wrap),
        F::TextTransform => fields!(text_transform),
        F::TextEmphasis => fields!(emphasis, emphasis_color),
        F::TextEmphasisStyle => fields!(emphasis),
        F::TextEmphasisColor => fields!(emphasis_color),
        F::TextEmphasisPosition => fields!(emphasis_position),
        F::TextDecoration => fields!(
            decoration_lines,
            decoration_style,
            decoration_color,
            decoration_thickness
        ),
        F::TextDecorationLine => fields!(decoration_lines),
        F::TextDecorationColor => fields!(decoration_color),
        F::TextDecorationStyle => fields!(decoration_style),
        F::TextDecorationThickness => fields!(decoration_thickness),
        F::TextDecorationSkipInk => fields!(skip_ink),
        F::TextUnderlineOffset => fields!(underline_offset),
        F::TextUnderlinePosition => fields!(underline_position),
        F::WebkitTextFillColor => fields!(fill),
        F::WebkitTextStroke => fields!(stroke, stroke_color),
        F::WebkitTextStrokeWidth => fields!(stroke),
        F::WebkitTextStrokeColor => fields!(stroke_color),
        F::TextShadow => fields!(text_shadows),
        F::TextOverflow => fields!(ellipsis),
        F::WebkitBoxOrient => fields!(box_vertical),
        F::WebkitLineClamp => fields!(line_clamp),
        F::Width => d.size[0] = s.size[0].clone(),
        F::Height => d.size[1] = s.size[1].clone(),
        F::MinWidth => d.min_size[0] = s.min_size[0].clone(),
        F::MinHeight => d.min_size[1] = s.min_size[1].clone(),
        F::MaxWidth => d.max_size[0] = s.max_size[0].clone(),
        F::MaxHeight => d.max_size[1] = s.max_size[1].clone(),
        F::OverflowX => d.overflow[0] = s.overflow[0],
        F::OverflowY => d.overflow[1] = s.overflow[1],
        F::RowGap => d.gap[0] = s.gap[0].clone(),
        F::ColumnGap => d.gap[1] = s.gap[1].clone(),
        F::GridTemplate(k) => d.grid_template[k] = s.grid_template[k].clone(),
        F::GridAuto(k) => d.grid_auto[k] = s.grid_auto[k].clone(),
        F::GridLines(k) => d.grid_lines[k] = s.grid_lines[k].clone(),
        F::GridStart(k) => d.grid_lines[k].start = s.grid_lines[k].start.clone(),
        F::GridEnd(k) => d.grid_lines[k].end = s.grid_lines[k].end.clone(),
        F::CornerRadius(k) => d.radii[k] = s.radii[k].clone(),
        F::FontVariant(Some(k)) => d.variants[k as usize] = s.variants[k as usize].clone(),
        F::Marker(Some(k)) => d.markers[k] = s.markers[k].clone(),
        F::BackgroundImage => d.layers.images = s.layers.images.clone(),
        F::BackgroundSize => d.layers.sizes = s.layers.sizes.clone(),
        F::BackgroundPosition => d.layers.positions = s.layers.positions.clone(),
        F::BackgroundRepeat => d.layers.repeats = s.layers.repeats.clone(),
        F::BackgroundOrigin => d.layers.origins = s.layers.origins.clone(),
        F::BackgroundClip => d.layers.clips = s.layers.clips.clone(),
        F::BackgroundBlendMode => d.layers.blends = s.layers.blends.clone(),
        F::MaskImage => d.masks.images = s.masks.images.clone(),
        F::MaskSize => d.masks.sizes = s.masks.sizes.clone(),
        F::MaskPosition => d.masks.positions = s.masks.positions.clone(),
        F::MaskRepeat => d.masks.repeats = s.masks.repeats.clone(),
        F::MaskOrigin => d.masks.origins = s.masks.origins.clone(),
        F::MaskClip => d.masks.clips = s.masks.clips.clone(),
        F::MaskMode => d.masks.modes = s.masks.modes.clone(),
        F::MaskComposite => d.masks.composites = s.masks.composites.clone(),
        F::BackgroundPositionAxis(k) => d.layers.positions[k] = s.layers.positions[k].clone(),
        F::Background => {
            let blends = d.layers.blends.clone();
            fields!(background, layers);
            d.layers.blends = blends;
        }
        F::Inset(sides) | F::Margin(sides) | F::Padding(sides) => {
            let (to, from) = match field {
                F::Inset(_) => (&mut d.inset, &s.inset),
                F::Margin(_) => (&mut d.margin, &s.margin),
                _ => (&mut d.padding, &s.padding),
            };
            for &k in declaration::named(sides) {
                to[k] = from[k].clone();
            }
        }
        F::Border(sides, part) => {
            for &k in declaration::named(sides) {
                if matches!(part, BorderPart::Line | BorderPart::Width) {
                    d.border_width[k] = s.border_width[k].clone();
                }
                if matches!(part, BorderPart::Line | BorderPart::Style) {
                    d.border_style[k] = s.border_style[k];
                }
                if matches!(part, BorderPart::Line | BorderPart::Color) {
                    d.border_color[k] = s.border_color[k].clone();
                }
            }
        }
        F::Outline(part) => {
            if matches!(part, BorderPart::Line | BorderPart::Width) {
                fields!(outline_width);
            }
            if matches!(part, BorderPart::Line | BorderPart::Style) {
                fields!(outline_style);
            }
            if matches!(part, BorderPart::Line | BorderPart::Color) {
                fields!(outline_color);
            }
        }
        F::FontVariant(None) => {
            // font-variant does not set font-kerning.
            d.variants[1..].clone_from_slice(&s.variants[1..]);
        }
        F::Marker(None) => fields!(markers),
        F::XmlSpace => unreachable!("XML whitespace is not CSS"),
    }
}

fn from_calc(c: &Calc) -> Len {
    Len::Calc(Rc::new(c.map(&|&[px, percent]| {
        vec![Len::Px(px), Len::Percent(percent * 100.0)]
    })))
}

pub(super) fn resolved(d: &Declared, s: &Style, canvas: [f64; 2]) -> Declared {
    let mut d = d.clone();
    let u = Units {
        em: s.font.size,
        canvas,
    };
    freeze::declaration(&mut d, u);
    let f = &s.font;
    macro_rules! some { ($($target:ident = $value:expr),+ $(,)?) => { $(d.$target = Some($value);)+ }; }
    some!(
        color = s.color.into(),
        font_size = Len::Px(f.size),
        font_weight = Weight::Absolute(f.weight),
        stretch = f.stretch,
        font_style = f.style,
        font_family = f.family.clone(),
        features = f.features.clone(),
        variations = f.variations.clone(),
        text_align = f.align,
        align_last = f.align_last,
        line_break = f.line_break,
        writing = f.writing,
        orientation = f.orientation,
        rtl = f.rtl,
        bidi = f.bidi,
        combine = f.combine.is_some(),
        text_indent = f.indent.as_ref().map_or(Len::Px(0.0), from_calc),
        letter_spacing = Len::Px(f.spacing),
        word_spacing = Len::Px(f.word_spacing),
        collapse = f.collapse,
        wrap = f.wrap,
        wrap_style = f.wrap_style,
        word_break = f.word_break,
        overflow_wrap = f.overflow_wrap,
        text_transform = f.transform,
        fill = f.fill.clone(),
        stroke = Len::Px(f.stroke),
        stroke_color = f.stroke_color.clone(),
        skip_ink = f.skip_ink,
        underline_offset = f.underline_offset.as_ref().map_or(Len::Auto, from_calc),
        underline_position = f.underline_position,
        emphasis = f.emphasis.clone(),
        emphasis_color = f.emphasis_color.clone(),
        emphasis_position = f.emphasis_position,
        visible = s.visible,
        pixelated = s.pixelated,
        svg_fill = s.fill.clone(),
        fill_opacity = s.fill_opacity,
        even_odd = s.even_odd,
        svg_stroke = s.stroke.paint.clone(),
        stroke_opacity = s.stroke.opacity,
        stroke_width = from_calc(&s.stroke.width),
        cap = s.stroke.cap,
        join = s.stroke.join,
        miter_limit = s.stroke.miter_limit,
        dashes = s
            .stroke
            .dashes
            .iter()
            .map(from_calc)
            .collect::<Vec<_>>()
            .into(),
        dash_offset = from_calc(&s.stroke.dash_offset),
        text_anchor = s.text_anchor,
        dominant_baseline = s.dominant_baseline,
        stop_color = s.stop_color.clone(),
        stop_opacity = s.stop_opacity,
        clip_even_odd = s.clip_even_odd,
        mask_alpha = s.mask_alpha,
        paint_order = s.paint_order,
        flood_color = s.flood_color.clone(),
        lighting_color = s.lighting_color.clone(),
        flood_opacity = s.flood_opacity,
        gradient_linear = s.gradient_linear,
        filter_linear = s.filter_linear
    );
    d.variants = f.variants.clone().map(Some);
    d.markers = s.markers.clone().map(Some);
    d.line_height = Some(match f.line_height {
        LineHeight::Normal => Leading::Normal,
        LineHeight::Factor(n) => Leading::Factor(n),
        LineHeight::Px(n) => Leading::Len(Len::Px(n)),
    });
    d.text_shadows = Some(
        f.shadows
            .iter()
            .map(|shadow| declaration::Shadowed {
                offset: shadow.offset.map(Len::Px),
                blur: Len::Px(shadow.blur),
                spread: Len::Px(0.0),
                color: shadow.color.clone(),
                inset: false,
            })
            .collect(),
    );
    d.border_width = s.border.map(Len::Px);
    d.outline_width = Len::Px(s.outline);
    d
}

pub(super) mod freeze;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::Prop;
    use crate::testing::styles::{num, px, styles, text, view};

    #[test]
    fn inheritance_copies_computed_units_and_shorthand_longhands() {
        let tree = vec![
            view(
                &[
                    ("fontSize", num(20.0)),
                    ("fontWeight", num(700.0)),
                    ("width", text("3em")),
                    ("padding", text("2em 1em")),
                    ("border", text("4px solid red")),
                    ("opacity", num(0.5)),
                    ("color", text("blue")),
                ],
                &[1],
            ),
            view(
                &[
                    ("fontSize", num(10.0)),
                    ("fontWeight", text("unset")),
                    ("width", text("inherit")),
                    ("padding", text("inherit")),
                    ("paddingLeft", num(7.0)),
                    ("border", text("inherit")),
                    ("borderColor", text("initial")),
                    ("opacity", text("inherit")),
                    ("color", text("unset")),
                ],
                &[],
            ),
        ];
        let s = styles(tree, &[]).unwrap();
        assert_eq!(s[1].layout.size.width, taffy::Dimension::length(60.0));
        assert_eq!(s[1].layout.padding.top, px(40.0));
        assert_eq!(s[1].layout.padding.right, px(20.0));
        assert_eq!(s[1].layout.padding.left, px(7.0));
        assert_eq!(s[1].border, [4.0; 4]);
        assert_eq!(s[1].border_colors, [[0.0, 0.0, 1.0, 1.0]; 4]);
        assert_eq!(s[1].font.weight, 700.0);
        assert_eq!(s[1].opacity, 0.5);
    }

    #[test]
    fn initial_resets_inherited_properties_and_unset_non_inherited_ones() {
        let tree = vec![
            view(
                &[
                    ("font", text("italic 700 24px/2 sans-serif")),
                    ("padding", num(12.0)),
                    ("opacity", num(0.3)),
                ],
                &[1, 2],
            ),
            view(
                &[
                    ("font", text("initial")),
                    ("padding", text("unset")),
                    ("opacity", text("unset")),
                    ("boxSizing", text("initial")),
                ],
                &[],
            ),
            view(&[("font", text("inherit")), ("fontSize", num(10.0))], &[]),
        ];
        let s = styles(tree, &[]).unwrap();
        assert_eq!(s[1].font.size, 16.0);
        assert_eq!(s[1].font.weight, 400.0);
        assert_eq!(s[1].font.style, super::super::FontStyle::Normal);
        assert_eq!(s[1].opacity, 1.0);
        assert_eq!(s[1].layout.padding.left, px(0.0));
        assert_eq!(s[1].layout.box_sizing, taffy::BoxSizing::ContentBox);
        assert_eq!(s[2].font.size, 10.0);
        assert_eq!(s[2].font.line_height, LineHeight::Factor(2.0));
        assert_eq!(s[2].font.weight, 700.0);
    }

    #[test]
    fn dynamic_keywords_and_relative_parents_do_not_reuse_stale_styles() {
        let template = crate::template::Template {
            nodes: vec![
                view(
                    &[("fontSize", Prop::Expr(0)), ("padding", text("2em"))],
                    &[1],
                ),
                view(&[("padding", Prop::Expr(1))], &[]),
            ],
            roots: vec![0],
            ..Default::default()
        };
        let sheet = crate::style::Sheet::new(&template).unwrap();
        for (size, keyword, expected) in [
            (20.0, "inherit", 40.0),
            (30.0, "inherit", 60.0),
            (30.0, "initial", 0.0),
            (20.0, "inherit", 40.0),
        ] {
            let values = [V::Num(size), V::Str(keyword.into())];
            let s = sheet.compute(&values, true, [200.0, 100.0], &[]).unwrap();
            assert_eq!(s[1].layout.padding.top, px(expected));
        }
    }
    #[test]
    fn every_css_property_accepts_all_three_keywords() {
        let initial = initial_values().unwrap();
        let mut parent =
            computed::compute(&Declared::default(), None, [100.0; 2], 0, 0, false, &[]).unwrap();
        parent.specified = Some(Rc::new(Snapshot(initial)));
        for name in super::super::properties() {
            let field = Field::by_name(name).unwrap();
            if field == Field::XmlSpace {
                continue;
            }
            for keyword in ["inherit", "initial", "unset"] {
                let mut declared = Declared::default();
                apply(&mut declared, field, &V::Str(keyword.into()), Some(&parent))
                    .unwrap_or_else(|e| panic!("{name}: {keyword}: {e}"));
            }
        }
    }
}
