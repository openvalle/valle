//! Computed styles and their construction.

use std::rc::Rc;

use crate::svg;
use taffy::{Display, Overflow, Position};

use super::color::Rgba;
use super::declaration::{COLUMNS, Declared, ROWS, Shown};
use super::font::{Decoration, Font, Leading, LineHeight, TextShadow, Weight, bolder, lighter};
use super::math::{Calc, Math};
use super::property::{INLINE_INSTEAD, NO_INLINE, USE_FLEX};
use super::units::{Len, Lengths, Units};
use super::{background, color, filter, gradient, offset, position, shape, transform};

use crate::front::Slot;
use crate::shader::Program;

/// What fills or strokes an SVG shape.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    None,
    Color(Rgba),
    /// The shape's own `color`, wherever it inherits the paint from.
    CurrentColor,
    /// The first gradient of this id in the frame, as Chrome finds one in its document; none if
    /// there is none.
    Url(Rc<str>),
}

/// The operations ordered by `paint-order`; text has no markers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintPart {
    Fill,
    Stroke,
    Markers,
}

pub(super) const PAINT_ORDER: [PaintPart; 3] =
    [PaintPart::Fill, PaintPart::Stroke, PaintPart::Markers];

/// How an SVG shape is stroked (`stroke-*`): its paint and the opacity it strokes at; its width,
/// its dashes and their offset, px and fractions of the viewport's normalized diagonal; how its
/// open ends are capped and its corners joined, and how far a miter may reach, in widths.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub paint: Paint,
    pub opacity: f64,
    pub width: Calc,
    pub cap: Cap,
    pub join: Join,
    pub miter_limit: f64,
    /// Lengths of dashes and of the gaps between them in turn; none for a solid stroke.
    pub dashes: Rc<[Calc]>,
    pub dash_offset: Calc,
}

impl Stroke {
    pub(super) fn initial() -> Stroke {
        Stroke {
            paint: Paint::None,
            opacity: 1.0,
            width: Math::Sum([1.0, 0.0]),
            cap: Cap::Butt,
            join: Join::Miter,
            miter_limit: 4.0,
            dashes: Rc::new([]),
            dash_offset: Math::Sum([0.0, 0.0]),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    Miter,
    Round,
    Bevel,
}

/// A node's computed style.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub layout: taffy::Style,
    /// Where a flex or grid item goes among its siblings, which `layout::ordered` sorts by.
    pub order: i32,
    /// `z-index`; `None` is `auto`.
    pub z_index: Option<i32>,
    pub background: Rgba,
    /// The background's images, the first on top, with their sizes, positions and repeats.
    pub layers: Vec<background::Layer>,
    /// `border-radius`: each corner's horizontal and vertical radius, from the top left
    /// clockwise, as px and fractions of the border box's width and height.
    pub radii: [[Calc; 2]; 4],
    /// The border's sides, top, right, bottom and left: their widths in whole px, 0 without a
    /// line, their colors and their styles.
    pub border: [f64; 4],
    pub border_colors: [Rgba; 4],
    pub border_styles: [BorderStyle; 4],
    /// The outline's width in whole px, 0 without a line, its color, its style, and how far
    /// outside the border box it is, in whole px toward 0, as Chrome draws it.
    pub outline: f64,
    pub outline_color: Rgba,
    pub outline_style: BorderStyle,
    pub outline_offset: f64,
    /// `box-shadow`, the first on top.
    pub shadows: Vec<Shadow>,
    /// Within [0, 1].
    pub opacity: f64,
    /// `translate`, `rotate` and `scale`, then `transform`: one list, empty when all are `none`;
    /// how many of it are the first three's, which go before the motion path's move; and
    /// `transform-origin`, from the border box's top left, and its z in px.
    pub transform: Vec<transform::Function>,
    pub individual: usize,
    pub transform_origin: [position::Offset; 2],
    pub transform_origin_z: f64,
    /// `offset-*`: the motion path the box moves along, `None` for `offset-path: none`.
    pub motion: Option<offset::Motion>,
    /// `perspective` in px, `None` for `none`, which its children's transforms are seen in, from
    /// `perspective-origin`.
    pub perspective: Option<f64>,
    pub perspective_origin: [position::Offset; 2],
    /// An Image's `object-fit`, how its image is sized in its content box, and `object-position`,
    /// where it goes there.
    pub object_fit: Fit,
    pub object_position: [position::Offset; 2],
    /// `backface-visibility: visible`; a box that shows its back otherwise hides with all in it.
    pub backface_visible: bool,
    /// `transform-style: preserve-3d`: its children with transforms stay in its space rather
    /// than being flattened onto its plane, unless what is in it is grouped.
    pub preserve_3d: bool,
    /// `mix-blend-mode`, and `isolation: isolate`, which keeps the boxes in it from blending
    /// with what is under it.
    pub blend: Blend,
    pub isolate: bool,
    /// `filter` and `backdrop-filter`: empty for `none`. A `motionBlur` is the last filter.
    pub filter: Vec<filter::Function>,
    /// `motionBlur: "auto"`: the scene measures the trail of its last filter.
    pub motion_auto: bool,
    pub backdrop: Vec<filter::Function>,
    /// `clip-path`; `None` for `none`.
    pub clip: Option<shape::Clip>,
    /// `mask-*`: its layers, the first on top; none when every image is `none`.
    pub masks: Vec<background::Layer>,
    /// A `<Mask>`'s source, which masks all the box paints after its `mask-*` do.
    pub source: Option<Source>,
    /// `visibility: visible`; hidden boxes paint nothing of their own.
    pub visible: bool,
    /// `image-rendering: pixelated`: images read their nearest px.
    pub pixelated: bool,
    /// SVG `fill`, `fill-opacity` and `fill-rule: evenodd`: what fills a shape, the opacity it
    /// fills at, and whether it fills where the shape winds around an odd number of times only.
    pub fill: Paint,
    pub fill_opacity: f64,
    pub even_odd: bool,
    pub stroke: Stroke,
    /// A gradient stop's color, a color or `currentColor`, and its opacity.
    pub stop_color: Paint,
    pub stop_opacity: f64,
    /// An SVG element's `clip-path: url(#id)` and `mask: url(#id)`, `clip-rule: evenodd` for a
    /// shape in a clip path, and `mask-type: alpha` for a `<mask>`.
    pub clip_ref: Option<Rc<str>>,
    pub mask_ref: Option<Rc<str>>,
    pub clip_even_odd: bool,
    pub mask_alpha: bool,
    /// The `<marker>`s at a path's start, middle and end vertices, by id.
    pub markers: [Option<Rc<str>>; 3],
    /// Inherited `paint-order`, including the operations omitted by the declaration.
    pub paint_order: [PaintPart; 3],
    /// An SVG element's geometry, as its attributes have it at the frame.
    pub svg: Option<Rc<svg::Computed>>,
    /// A `<use>`'s copy.
    pub instance: Option<Rc<Instance>>,
    /// A `<ShaderLayer>`'s shader, with its uniforms' values at the frame.
    pub shader: Option<Shaded>,
    pub color: Rgba,
    pub font: Rc<Font>,
    /// The text decorations its text takes: those of the boxes around it, outermost first, then
    /// its own. A box out of flow takes none from around it.
    pub decorations: Vec<Decoration>,
    /// `text-overflow: ellipsis`: a Text that clips what overflows it ends a line that overflows
    /// with an ellipsis.
    pub ellipsis: bool,
    /// How many lines a Text shows before an ellipsis, its height theirs: its
    /// `-webkit-line-clamp`, with `display: -webkit-box` and `-webkit-box-orient: vertical`.
    pub clamp: Option<u32>,
    /// The math functions with percentages in them that `layout` holds handles to.
    pub calcs: Vec<Calc>,
}

/// The styles of the elements a `<use>` copies, which inherit from it: the one it names, `root`,
/// and those in it, in template order.
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    pub root: usize,
    pub styles: Vec<Style>,
}

impl Instance {
    /// Node `i`'s style in the copy, if the copy holds it.
    pub fn style(&self, i: usize) -> Option<&Style> {
        self.styles.get(i.checked_sub(self.root)?)
    }
}

/// `object-fit`: an image filling its box, as large as fits in it whole or as covers it, at its
/// natural size, or the smaller of that and fitting whole.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fit {
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

/// A `<Mask>`'s source: the root node whose painting masks the box, by its alpha or its
/// luminance, inverted or not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    pub root: usize,
    pub luminance: bool,
    pub invert: bool,
}

/// A shader as a frame has it: its program, its uniform block, the resources of the images it
/// reads by name, the px it paints out past the box on each side, and whether it is a
/// `<Transition>`'s, which reads its two children rather than its content.
#[derive(Clone, Debug, PartialEq)]
pub struct Shaded {
    pub program: Rc<Program>,
    pub block: Rc<[u8]>,
    pub inputs: Vec<(Rc<str>, u32)>,
    pub outset: f64,
    pub transition: bool,
}

impl Style {
    /// Whether the box is the containing block of every box in it, fixed ones too, and a
    /// stacking context: it has a transform, a perspective, a filter or a backdrop filter, or
    /// preserves 3D, as CSS has it, or is a `<ShaderLayer>`, which a filter's rules hold for.
    pub fn contains_all(&self) -> bool {
        self.transformed()
            || self.perspective.is_some()
            || self.preserve_3d
            || !self.filter.is_empty()
            || !self.backdrop.is_empty()
            || self.shader.is_some()
    }

    /// Whether it has a transform: functions, or a motion path.
    pub fn transformed(&self) -> bool {
        !self.transform.is_empty() || self.motion.is_some()
    }

    /// Whether layout lays out a box of this style as one of `other`: they differ at most in
    /// what paints the box or moves it once laid out. Every field is named, so that a new one
    /// takes a side.
    pub fn lays_out_as(&self, other: &Style) -> bool {
        let Style {
            layout,
            order,
            source,
            svg,
            font,
            decorations,
            ellipsis,
            clamp,
            calcs,
            // Only whether they make a containing block of all, below.
            transform: _,
            motion: _,
            perspective: _,
            preserve_3d: _,
            filter: _,
            backdrop: _,
            shader: _,
            z_index: _,
            background: _,
            layers: _,
            radii: _,
            border: _,
            border_colors: _,
            border_styles: _,
            outline: _,
            outline_color: _,
            outline_style: _,
            outline_offset: _,
            shadows: _,
            opacity: _,
            individual: _,
            transform_origin: _,
            transform_origin_z: _,
            perspective_origin: _,
            object_fit: _,
            object_position: _,
            backface_visible: _,
            blend: _,
            isolate: _,
            motion_auto: _,
            clip: _,
            masks: _,
            visible: _,
            pixelated: _,
            fill: _,
            fill_opacity: _,
            even_odd: _,
            stroke: _,
            stop_color: _,
            stop_opacity: _,
            clip_ref: _,
            mask_ref: _,
            clip_even_odd: _,
            mask_alpha: _,
            markers: _,
            paint_order: _,
            instance: _,
            color: _,
        } = self;
        *layout == other.layout
            && *order == other.order
            && *source == other.source
            && *svg == other.svg
            && *font == other.font
            && *decorations == other.decorations
            && *ellipsis == other.ellipsis
            && *clamp == other.clamp
            && *calcs == other.calcs
            && self.contains_all() == other.contains_all()
    }

    /// Whether its children with transforms stay in its space: it preserves 3D and nothing
    /// groups what is in it into one flat layer, as an opacity, a clip, a clip path, a mask or a
    /// `<Mask>` source, a blend, isolation, a filter of it or of its backdrop, or a shader does.
    pub fn keeps_3d(&self) -> bool {
        let o = self.layout.overflow;
        self.preserve_3d
            && self.opacity >= 1.0
            && self.filter.is_empty()
            && self.backdrop.is_empty()
            && self.shader.is_none()
            && self.blend == Blend::Normal
            && !self.isolate
            && self.clip.is_none()
            && self.masks.is_empty()
            && self.source.is_none()
            && o.x == taffy::Overflow::Visible
            && o.y == taffy::Overflow::Visible
    }
}

/// `mix-blend-mode`: how a box's colors mix with those under it, numbered as shaders take them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Blend {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    PlusLighter,
}

/// The last is plus-lighter, which only `mix-blend-mode` takes.
pub(super) const BLENDS: [(&str, Blend); 17] = [
    ("normal", Blend::Normal),
    ("multiply", Blend::Multiply),
    ("screen", Blend::Screen),
    ("overlay", Blend::Overlay),
    ("darken", Blend::Darken),
    ("lighten", Blend::Lighten),
    ("color-dodge", Blend::ColorDodge),
    ("color-burn", Blend::ColorBurn),
    ("hard-light", Blend::HardLight),
    ("soft-light", Blend::SoftLight),
    ("difference", Blend::Difference),
    ("exclusion", Blend::Exclusion),
    ("hue", Blend::Hue),
    ("saturation", Blend::Saturation),
    ("color", Blend::Color),
    ("luminosity", Blend::Luminosity),
    ("plus-lighter", Blend::PlusLighter),
];

/// A computed box shadow, px.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadow {
    pub offset: [f64; 2],
    pub blur: f64,
    pub spread: f64,
    pub color: Rgba,
    pub inset: bool,
}

/// A border's or an outline's line style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderStyle {
    None,
    Solid,
    /// Two lines with a gap between them.
    Double,
    /// Lines that look carved into the canvas, out of it, sunk into it and raised from it: in
    /// darker and lighter shades of their color.
    Groove,
    Ridge,
    Inset,
    Outset,
    /// Round dots, or square ones under 4 px, and dashes.
    Dotted,
    Dashed,
}

/// A border width as browsers compute it: whole px, and 1 px for one between 0 and 1.
fn snapped(width: f64) -> f64 {
    if width > 0.0 && width < 1.0 {
        1.0
    } else {
        width.floor()
    }
}

pub(super) fn computed_transforms(d: &Declared, units: Units) -> Vec<transform::Function> {
    [&d.translate, &d.rotate, &d.scale]
        .into_iter()
        .flatten()
        .chain(&d.transform)
        .map(|f| f.computed(units))
        .collect()
}

/// The computed style of a node with these declared values under `parent` (`None`: the canvas,
/// a block container with CSS's initial text properties), after `first` calcs in the frame.
pub(super) fn compute(
    d: &Declared,
    parent: Option<&Style>,
    canvas: [f64; 2],
    first: usize,
    node: usize,
    text: bool,
    resources: &[Slot],
) -> Result<Style, String> {
    let initial;
    let (inherited, inherited_color, inherited_visible, container) = match parent {
        Some(p) => (p.font.as_ref(), p.color, p.visible, p.layout.display),
        None => {
            initial = Font::initial();
            (&initial, color::BLACK, true, Display::Block)
        }
    };
    let (fill, fill_opacity, even_odd) = match parent {
        Some(p) => (p.fill.clone(), p.fill_opacity, p.even_odd),
        None => (Paint::Color(color::BLACK), 1.0, false),
    };
    let mut stroke = parent.map_or_else(Stroke::initial, |p| p.stroke.clone());
    let units = |em| Units { em, canvas };
    let size = match &d.font_size {
        None => inherited.size,
        Some(l) => l.px(units(inherited.size), inherited.size),
    };
    let font = Font {
        size,
        weight: match d.font_weight {
            None => inherited.weight,
            Some(Weight::Absolute(w)) => w,
            Some(Weight::Bolder) => bolder(inherited.weight),
            Some(Weight::Lighter) => lighter(inherited.weight),
        },
        stretch: d.stretch.unwrap_or(inherited.stretch),
        style: d.font_style.unwrap_or(inherited.style),
        family: match &d.font_family {
            None => inherited.family.clone(),
            Some(f) => f.clone(),
        },
        line_height: match &d.line_height {
            None => inherited.line_height,
            Some(Leading::Normal) => LineHeight::Normal,
            Some(Leading::Factor(f)) => LineHeight::Factor(*f),
            Some(Leading::Len(l)) => LineHeight::Px(l.px(units(size), size)),
        },
        align: d.text_align.unwrap_or(inherited.align),
        indent: match &d.text_indent {
            None => inherited.indent.clone(),
            Some(l) => Some(l.calc(units(size))),
        },
        spacing: match &d.letter_spacing {
            None => inherited.spacing,
            Some(l) => l.px(units(size), 0.0),
        },
        word_spacing: match &d.word_spacing {
            None => inherited.word_spacing,
            Some(l) => l.px(units(size), 0.0),
        },
        collapse: d.collapse.unwrap_or(inherited.collapse),
        wrap: d.wrap.unwrap_or(inherited.wrap),
        balance: d.balance.unwrap_or(inherited.balance),
        word_break: d.word_break.unwrap_or(inherited.word_break),
        overflow_wrap: d.overflow_wrap.unwrap_or(inherited.overflow_wrap),
        transform: d.text_transform.unwrap_or(inherited.transform),
        variants: std::array::from_fn(|i| {
            (d.variants[i].clone()).unwrap_or_else(|| inherited.variants[i].clone())
        }),
        features: (d.features.clone()).unwrap_or_else(|| inherited.features.clone()),
        variations: (d.variations.clone()).unwrap_or_else(|| inherited.variations.clone()),
        fill: d.fill.unwrap_or(inherited.fill),
        stroke: match &d.stroke {
            None => inherited.stroke,
            Some(l) => l.px(units(size), 0.0),
        },
        stroke_color: d.stroke_color.unwrap_or(inherited.stroke_color),
        shadows: match &d.text_shadows {
            None => inherited.shadows.clone(),
            Some(list) => (list.iter())
                .map(|s| TextShadow {
                    offset: s.offset.each_ref().map(|l| l.px(units(size), 0.0)),
                    blur: s.blur.px(units(size), 0.0),
                    color: s.color,
                })
                .collect(),
        },
        skip_ink: d.skip_ink.unwrap_or(inherited.skip_ink),
        underline_offset: match &d.underline_offset {
            None => inherited.underline_offset.clone(),
            Some(Len::Auto) => None,
            Some(l) => Some(l.calc(units(size))),
        },
    };
    let display = match d.display {
        Shown::Block => Display::Block,
        Shown::WebkitBox if text => Display::Block,
        Shown::WebkitBox => {
            return Err(format!(
                "display: -webkit-box is only for a Text, which -webkit-line-clamp clamps: other \
                 boxes would lay out their children in a row, as old flexbox did; {USE_FLEX}"
            ));
        }
        Shown::Flex => Display::Flex,
        Shown::Grid => Display::Grid,
        Shown::None => Display::None,
        inline => {
            let out_of_flow = matches!(d.position, Position::Absolute | Position::Fixed);
            if !out_of_flow && !matches!(container, Display::Flex | Display::Grid) {
                let name = match inline {
                    Shown::InlineFlex => "inline-flex",
                    Shown::InlineGrid => "inline-grid",
                    Shown::InlineBlock => "inline-block",
                    _ => "inline",
                };
                return Err(format!(
                    "display: {name} needs a flex or grid parent: {NO_INLINE}; {INLINE_INSTEAD}"
                ));
            }
            // Blockified, as CSS does to flex and grid items and out-of-flow boxes.
            match inline {
                Shown::InlineFlex => Display::Flex,
                Shown::InlineGrid => Display::Grid,
                _ => Display::Block,
            }
        }
    };
    let u = units(size);
    let color = d.color.unwrap_or(inherited_color);
    if let Some(p) = &d.svg_stroke {
        stroke.paint = p.clone();
    }
    stroke.opacity = d.stroke_opacity.unwrap_or(stroke.opacity);
    if let Some(w) = &d.stroke_width {
        stroke.width = w.calc(u);
    }
    stroke.cap = d.cap.unwrap_or(stroke.cap);
    stroke.join = d.join.unwrap_or(stroke.join);
    stroke.miter_limit = d.miter_limit.unwrap_or(stroke.miter_limit);
    if let Some(dashes) = &d.dashes {
        stroke.dashes = dashes.iter().map(|l| l.calc(u)).collect();
    }
    if let Some(o) = &d.dash_offset {
        stroke.dash_offset = o.calc(u);
    }
    let out_of_flow = matches!(d.position, Position::Absolute | Position::Fixed);
    let mut decorations = match parent {
        Some(p) if !out_of_flow => p.decorations.clone(),
        _ => Vec::new(),
    };
    if d.decoration_lines.contains(&true) {
        decorations.push(Decoration {
            lines: d.decoration_lines,
            style: d.decoration_style,
            color: d.decoration_color.unwrap_or(font.decoration_color(color)),
            thickness: match &d.decoration_thickness {
                Len::Auto => None,
                l => Some(l.calc(u)),
            },
            offset: font.underline_offset.clone(),
            by: node,
        });
    }
    let width = |style, width: &Len| match style {
        BorderStyle::None => 0.0,
        _ => snapped(width.px(u, 0.0)),
    };
    let border: [f64; 4] = std::array::from_fn(|i| width(d.border_style[i], &d.border_width[i]));
    let mut l = Lengths {
        u,
        first,
        calcs: Vec::new(),
    };
    let (grid_template_rows, grid_template_row_names) = l.template(&d.grid_template[ROWS]);
    let (grid_template_columns, grid_template_column_names) = l.template(&d.grid_template[COLUMNS]);
    let [grid_auto_rows, grid_auto_columns]: [Vec<_>; 2] =
        [ROWS, COLUMNS].map(|axis| d.grid_auto[axis].iter().map(|t| l.track(t)).collect());
    // Hidden on one axis makes the other, visible or clip, hidden too: CSS computes it to auto or
    // hidden, which clip alike when nothing scrolls.
    let [x, y] = match d.overflow {
        o if o.contains(&Overflow::Hidden) => [Overflow::Hidden; 2],
        o => o,
    };
    let layout: taffy::Style = taffy::Style {
        display,
        position: d.position,
        overflow: taffy::Point { x, y },
        inset: l.rect(&d.inset, Lengths::lpa),
        size: taffy::Size {
            width: l.dim(&d.size[0]),
            height: l.dim(&d.size[1]),
        },
        min_size: taffy::Size {
            width: l.lpa(&d.min_size[0]),
            height: l.lpa(&d.min_size[1]),
        },
        max_size: taffy::Size {
            width: l.lpa(&d.max_size[0]),
            height: l.lpa(&d.max_size[1]),
        },
        box_sizing: d.box_sizing,
        aspect_ratio: d.aspect_ratio.map(|r| r as f32),
        margin: l.rect(&d.margin, Lengths::lpa),
        padding: l.rect(&d.padding, Lengths::lp),
        border: l.rect(&border.map(Len::Px), Lengths::lp),
        flex_direction: d.flex_direction,
        flex_wrap: d.flex_wrap,
        flex_grow: d.flex_grow as f32,
        flex_shrink: d.flex_shrink as f32,
        flex_basis: l.dim(&d.flex_basis),
        justify_content: d.justify_content,
        align_items: d.align_items,
        align_self: d.align_self,
        align_content: d.align_content,
        gap: taffy::Size {
            width: l.lp(&d.gap[1]),
            height: l.lp(&d.gap[0]),
        },
        grid_template_rows,
        grid_template_columns,
        grid_template_row_names,
        grid_template_column_names,
        grid_template_areas: d.grid_areas.clone(),
        grid_auto_rows,
        grid_auto_columns,
        grid_auto_flow: d.grid_auto_flow,
        grid_row: d.grid_lines[ROWS].clone(),
        grid_column: d.grid_lines[COLUMNS].clone(),
        justify_items: d.justify_items,
        justify_self: d.justify_self,
        ..taffy::Style::DEFAULT
    };
    Ok(Style {
        layout,
        order: d.order,
        z_index: d.z_index,
        background: d.background.unwrap_or(color),
        layers: (d.layers).computed(u, color, background::Area::PaddingBox, resources)?,
        radii: d.radii.each_ref().map(|r| r.each_ref().map(|l| l.calc(u))),
        border,
        border_colors: d.border_color.map(|c| c.unwrap_or(color)),
        border_styles: d.border_style,
        outline: width(d.outline_style, &d.outline_width),
        outline_color: d.outline_color.unwrap_or(color),
        outline_style: d.outline_style,
        outline_offset: d.outline_offset.px(u, 0.0).trunc(),
        shadows: (d.shadows.iter())
            .map(|s| Shadow {
                offset: s.offset.each_ref().map(|l| l.px(u, 0.0)),
                blur: s.blur.px(u, 0.0),
                spread: s.spread.px(u, 0.0),
                color: s.color.unwrap_or(color),
                inset: s.inset,
            })
            .collect(),
        opacity: d.opacity,
        transform: computed_transforms(d, u),
        individual: [&d.translate, &d.rotate, &d.scale]
            .into_iter()
            .flatten()
            .count(),
        transform_origin: d.transform_origin.each_ref().map(|o| o.computed(u)),
        transform_origin_z: d.transform_origin_z.px(u, 0.0),
        motion: d.offset_path.as_ref().map(|(path, area)| offset::Motion {
            path: path.computed(u),
            area: *area,
            distance: d.offset_distance.calc(u),
            along: d.offset_rotate.0,
            angle: d.offset_rotate.1,
            anchor: (d.offset_anchor.as_ref()).map(|a| a.each_ref().map(|o| o.computed(u))),
            position: d.offset_position.computed(u),
        }),
        perspective: d.perspective.as_ref().map(|l| l.px(u, 0.0)),
        perspective_origin: d.perspective_origin.each_ref().map(|o| o.computed(u)),
        object_fit: d.object_fit,
        object_position: d.object_position.each_ref().map(|o| o.computed(u)),
        backface_visible: d.backface_visible,
        preserve_3d: d.preserve_3d,
        blend: d.blend,
        isolate: d.isolate,
        filter: (d.filter.iter().map(|f| f.computed(u, color)))
            .chain(
                (d.motion_blur)
                    .map(|trail| filter::Function::MotionBlur(trail.unwrap_or([0.0; 2]))),
            )
            .collect(),
        motion_auto: d.motion_blur == Some(None),
        backdrop: d.backdrop.iter().map(|f| f.computed(u, color)).collect(),
        clip: d.clip.as_ref().map(|c| c.computed(u)),
        masks: match d
            .masks
            .images
            .iter()
            .all(|i| matches!(i, gradient::Image::None))
        {
            true => Vec::new(),
            false => (d.masks).computed(u, color, background::Area::BorderBox, resources)?,
        },
        source: None,
        visible: d.visible.unwrap_or(inherited_visible),
        pixelated: (d.pixelated).unwrap_or(parent.is_some_and(|p| p.pixelated)),
        fill: d.svg_fill.clone().unwrap_or(fill),
        fill_opacity: d.fill_opacity.unwrap_or(fill_opacity),
        even_odd: d.even_odd.unwrap_or(even_odd),
        stroke,
        stop_color: d.stop_color.clone().unwrap_or(Paint::Color(color::BLACK)),
        stop_opacity: d.stop_opacity.unwrap_or(1.0),
        clip_ref: d.clip_ref.clone(),
        mask_ref: d.mask_ref.clone(),
        clip_even_odd: d
            .clip_even_odd
            .unwrap_or(parent.is_some_and(|p| p.clip_even_odd)),
        mask_alpha: d.mask_alpha.unwrap_or(false),
        markers: std::array::from_fn(|k| match &d.markers[k] {
            Some(m) => m.clone(),
            None => parent.and_then(|p| p.markers[k].clone()),
        }),
        paint_order: d
            .paint_order
            .unwrap_or_else(|| parent.map_or(PAINT_ORDER, |p| p.paint_order)),
        svg: None,
        instance: None,
        shader: None,
        color,
        font: Rc::new(font),
        decorations,
        ellipsis: d.ellipsis,
        clamp: match (d.display, d.box_vertical) {
            (Shown::WebkitBox, true) => d.line_clamp,
            _ => None,
        },
        calcs: l.calcs,
    })
}

#[cfg(test)]
mod tests {

    use taffy::{
        AlignContent, AlignItems, Dimension, Display, FlexDirection, LengthPercentageAuto, Position,
    };

    use crate::template::Prop;

    use crate::testing::styles::{auto_px, error, num, one, px, styles, text, view};

    #[test]
    fn declarations_compute_into_a_layout_style() {
        let s = one(&[
            ("fontSize", num(20.0)),
            ("display", text("flex")),
            ("flexDirection", text("COLUMN")),
            ("justifyContent", text("space-between")),
            ("alignItems", text("center")),
            ("width", num(100.0)),
            ("height", text("50%")),
            ("maxWidth", text("none")),
            ("minHeight", text("2rem")),
            ("padding", text("1px 2px 3px")),
            ("margin", text("auto 1rem")),
            ("top", text("-2em")),
            ("gap", text("4px 8px")),
        ]);
        let l = &s.layout;
        assert_eq!(l.display, Display::Flex);
        assert_eq!(l.position, Position::Static);
        assert_eq!(l.flex_direction, FlexDirection::Column);
        assert_eq!(l.justify_content, AlignContent::SPACE_BETWEEN);
        assert_eq!(l.align_items, AlignItems::CENTER);
        assert_eq!(l.size.width, Dimension::length(100.0));
        assert_eq!(l.size.height, Dimension::percent(0.5));
        assert_eq!(l.max_size.width, LengthPercentageAuto::auto());
        assert_eq!(l.min_size.height, auto_px(32.0));
        let padding = taffy::Rect {
            top: px(1.0),
            right: px(2.0),
            bottom: px(3.0),
            left: px(2.0),
        };
        assert_eq!(l.padding, padding);
        assert_eq!(l.margin.top, LengthPercentageAuto::auto());
        assert_eq!(l.margin.left, auto_px(16.0));
        // `em` is the element's own font size.
        assert_eq!(l.inset.top, auto_px(-40.0));
        assert_eq!(l.inset.left, LengthPercentageAuto::auto());
        // Rows, then columns.
        assert_eq!(l.gap.height, px(4.0));
        assert_eq!(l.gap.width, px(8.0));

        let initial = one(&[]).layout;
        assert_eq!(initial.display, Display::Block);
        assert_eq!(initial.flex_shrink, 1.0);
        assert_eq!(initial.gap.width, px(0.0));
    }

    #[test]
    fn a_box_lays_out_alike_with_what_only_paints_or_moves_it() {
        let alike = |a: &[(&str, Prop)], b: &[(&str, Prop)]| one(a).lays_out_as(&one(b));
        let wide = || ("width", num(10.0));
        assert!(alike(
            &[wide(), ("translate", text("5px")), ("opacity", num(0.5))],
            &[wide(), ("translate", text("6px 2px"))],
        ));
        assert!(!alike(&[wide()], &[("width", num(11.0))]));
        assert!(!alike(
            &[("fontSize", num(20.0))],
            &[("fontSize", num(21.0))]
        ));
        assert!(alike(&[], &[("paintOrder", text("stroke fill"))]));
        // A transform makes it the containing block of the boxes out of flow in it.
        assert!(!alike(&[wide()], &[wide(), ("translate", text("5px"))]));
    }

    #[test]
    fn inline_displays_blockify_in_flex_and_grid_containers() {
        let tree = |parent: &str, child: &[(&str, Prop)]| {
            let tree = vec![view(&[("display", text(parent))], &[1]), view(child, &[])];
            styles(tree, &[]).map(|s| s[1].layout.display)
        };
        assert_eq!(
            tree("flex", &[("display", text("inline-flex"))]),
            Ok(Display::Flex)
        );
        assert_eq!(
            tree("flex", &[("display", text("inline"))]),
            Ok(Display::Block)
        );
        assert_eq!(
            tree("grid", &[("display", text("inline-grid"))]),
            Ok(Display::Grid)
        );
        let absolute = [
            ("display", text("inline-block")),
            ("position", text("absolute")),
        ];
        assert_eq!(tree("block", &absolute), Ok(Display::Block));
        assert_eq!(
            tree("block", &[("display", text("inline-flex"))]),
            Err(
                "s.tsx:1:1: display: inline-flex needs a flex or grid parent: there is no HTML \
             inline flow; use flex layout, and Span for styled runs of text"
                    .into()
            )
        );
        // The canvas is a block container.
        assert!(
            error(&[("display", text("inline-block"))]).contains("needs a flex or grid parent")
        );
    }
}
