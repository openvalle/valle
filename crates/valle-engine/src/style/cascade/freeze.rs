//! Resolve relative units before an explicit inheritance copy, retaining percentages whose
//! containing box is not known until layout. This operates only on retained snapshots.
use super::*;
use crate::style::{background, filter, gradient, offset, position, shape, transform};

pub(in crate::style) trait Freeze {
    fn freeze(&mut self, u: Units);
}
impl Freeze for Len {
    fn freeze(&mut self, u: Units) {
        match self {
            Len::Auto | Len::Px(_) | Len::Percent(_) | Len::Sizing(_) => {}
            Len::Calc(m) => {
                *m = Rc::new(m.map(&|values| {
                    let mut values = values.clone();
                    values.freeze(u);
                    values
                }))
            }
            _ => *self = Len::Px(self.px(u, 0.0)),
        }
    }
}
impl<T: Freeze, const N: usize> Freeze for [T; N] {
    fn freeze(&mut self, u: Units) {
        for v in self {
            v.freeze(u);
        }
    }
}
impl<T: Freeze> Freeze for Vec<T> {
    fn freeze(&mut self, u: Units) {
        for v in self {
            v.freeze(u);
        }
    }
}
impl<T: Freeze> Freeze for Option<T> {
    fn freeze(&mut self, u: Units) {
        if let Some(v) = self {
            v.freeze(u);
        }
    }
}
impl Freeze for position::Offset<Len> {
    fn freeze(&mut self, u: Units) {
        self.by.freeze(u);
    }
}
impl Freeze for transform::Function<Len> {
    fn freeze(&mut self, u: Units) {
        match self {
            Self::Translate(v) => v.freeze(u),
            Self::Perspective(v) => v.freeze(u),
            _ => {}
        }
    }
}
impl Freeze for filter::Function<Len, Option<crate::style::color::Specified>> {
    fn freeze(&mut self, u: Units) {
        match self {
            Self::Blur(v) => v.freeze(u),
            Self::DropShadow { offset, blur, .. } => {
                offset.freeze(u);
                blur.freeze(u);
            }
            _ => {}
        }
    }
}
impl Freeze for declaration::Shadowed {
    fn freeze(&mut self, u: Units) {
        self.offset.freeze(u);
        self.blur.freeze(u);
        self.spread.freeze(u);
    }
}
impl Freeze for shape::Radius<Len> {
    fn freeze(&mut self, u: Units) {
        if let Self::Len(v) = self {
            v.freeze(u);
        }
    }
}
impl Freeze for shape::Shape<Len> {
    fn freeze(&mut self, u: Units) {
        match self {
            Self::Inset { sides, radii } => {
                sides.freeze(u);
                radii.freeze(u);
            }
            Self::Circle { radius, at } => {
                radius.freeze(u);
                at.freeze(u);
            }
            Self::Ellipse { radii, at } => {
                radii.freeze(u);
                at.freeze(u);
            }
            Self::Polygon { points, .. } => points.freeze(u),
            Self::Commands { path, .. } => path.freeze(u),
            Self::Path { .. } => {}
        }
    }
}
impl Freeze for shape::Clip<Len> {
    fn freeze(&mut self, u: Units) {
        self.shape.freeze(u);
    }
}
impl Freeze for offset::Route<Len> {
    fn freeze(&mut self, u: Units) {
        match self {
            Self::Ray { at, .. } => at.freeze(u),
            Self::Shape { shape, .. } => shape.freeze(u),
            Self::Path(_) => {}
        }
    }
}
impl Freeze for offset::Start<Len> {
    fn freeze(&mut self, u: Units) {
        if let Self::At(v) = self {
            v.freeze(u);
        }
    }
}
impl Freeze for background::Declared {
    fn freeze(&mut self, u: Units) {
        for size in &mut self.sizes {
            if let background::ImageSize::Sized(v) = size {
                v.freeze(u);
            }
        }
        self.positions.freeze(u);
        for image in &mut self.images {
            if let gradient::Image::Gradient(g) = image {
                match &mut g.shape {
                    gradient::Shape::Radial { size, at, .. } => {
                        if let gradient::Size::Radii(r) = size {
                            r.freeze(u);
                        }
                        at.freeze(u);
                    }
                    gradient::Shape::Conic { at, .. } => at.freeze(u),
                    gradient::Shape::Linear(_) => {}
                }
                for stop in &mut g.stops {
                    match stop {
                        gradient::Stop::Color(_, p) => p.freeze(u),
                        gradient::Stop::Hint(p) => p.freeze(u),
                    }
                }
            }
        }
    }
}

pub(super) fn declaration(d: &mut Declared, u: Units) {
    macro_rules! lengths { ($($f:ident),+ $(,)?) => { $(d.$f.freeze(u);)+ }; }
    lengths!(
        inset,
        size,
        min_size,
        max_size,
        margin,
        padding,
        flex_basis,
        gap,
        layers,
        radii,
        border_width,
        outline_width,
        outline_offset,
        shadows,
        translate,
        rotate,
        scale,
        transform,
        transform_origin,
        transform_origin_z,
        offset_distance,
        offset_anchor,
        offset_position,
        perspective,
        perspective_origin,
        object_position,
        filter,
        backdrop,
        clip,
        masks,
        decoration_thickness,
        stroke_width,
        dash_offset,
        text_indent,
        letter_spacing,
        word_spacing,
        stroke,
        text_shadows,
        underline_offset
    );
    if let Some((path, _)) = &mut d.offset_path {
        path.freeze(u);
    }
    if let Some(dashes) = &mut d.dashes {
        let mut v = dashes.to_vec();
        v.freeze(u);
        *dashes = v.into();
    }
    if let super::super::inline::Align::Offset(v) = &mut d.vertical_align {
        v.freeze(u);
    }
    if let crate::svg::text::Shift::Length(v) = &mut d.baseline_shift {
        v.freeze(u);
    }
    for tracks in &mut d.grid_template {
        tracks.freeze(u);
    }
    for tracks in &mut d.grid_auto {
        tracks.freeze(u);
    }
}
