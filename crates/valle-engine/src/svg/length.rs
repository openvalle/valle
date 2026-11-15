use super::{Element, Shape};

/// An SVG length: user units, and a percentage of the viewport's width, height or normalized
/// diagonal, as the attribute measures.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Length {
    pub units: f64,
    pub percent: f64,
    pub(crate) em: f64,
}

impl Length {
    pub fn resolve(self, basis: f64) -> f64 {
        self.units + self.percent / 100.0 * basis
    }

    pub(crate) fn nonnegative(self, name: &str) -> Result<Self, String> {
        if self.units < 0.0 || self.percent < 0.0 || self.em < 0.0 {
            Err(format!("{name} is negative"))
        } else {
            Ok(self)
        }
    }
}

impl Shape {
    pub(crate) fn lengths(&mut self, f: &mut impl FnMut(&mut Length)) {
        match self {
            Self::Circle { center, r } => {
                center.iter_mut().for_each(&mut *f);
                f(r);
            }
            Self::Ellipse { center, r } => {
                center.iter_mut().for_each(&mut *f);
                r.iter_mut().flatten().for_each(f);
            }
            Self::Rect { at, size, r } => {
                at.iter_mut().chain(size).for_each(&mut *f);
                r.iter_mut().flatten().for_each(f);
            }
            Self::Line(points) => points.iter_mut().for_each(f),
            Self::Path { .. } | Self::Points { .. } | Self::Text { .. } => {}
        }
    }

    pub(crate) fn resolve_font(&mut self, size: f64) {
        self.lengths(&mut |l| l.resolve_font(size));
    }
}

impl Length {
    fn resolve_font(&mut self, size: f64) {
        self.units += self.em * size;
        self.em = 0.0;
    }
}

impl Element {
    pub(crate) fn resolve_font(&mut self, size: f64) {
        self.lengths(&mut |l| l.resolve_font(size));
    }

    pub(super) fn lengths(&mut self, f: &mut impl FnMut(&mut Length)) {
        match self {
            Self::Svg { at, size, .. }
            | Self::Use { at, size, .. }
            | Self::Image { at, size, .. } => {
                at.iter_mut().for_each(&mut *f);
                size.iter_mut().flatten().for_each(f);
            }
            Self::Shape(s) => s.lengths(f),
            Self::Filter(d) => d.region.iter_mut().for_each(f),
            Self::Primitive(p) => p.region.iter_mut().flatten().for_each(f),
            Self::Text(p) => {
                p.xy.iter_mut()
                    .chain(&mut p.delta)
                    .flatten()
                    .for_each(&mut *f);
                f(&mut p.offset);
                p.length.iter_mut().for_each(f);
            }
            Self::Gradient(g) => g.lengths.iter_mut().flatten().for_each(f),
            Self::Pattern(p) => p.region.iter_mut().flatten().for_each(f),
            Self::Mask(m) => m.region.iter_mut().for_each(f),
            Self::Marker(m) => m.reference.iter_mut().chain(&mut m.size).for_each(f),
            Self::G
            | Self::TextRun(_)
            | Self::Defs
            | Self::Stop(_)
            | Self::ClipPath { .. }
            | Self::Symbol { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::template::V;

    #[test]
    fn em_resolves_after_font_inheritance_and_rejects_negative_sizes() {
        let mut length = crate::svg::length(&V::Str("1.5em".into())).unwrap();
        length.resolve_font(24.0);
        assert_eq!(length.resolve(100.0), 36.0);
        assert_eq!(length.em, 0.0);
        let negative = crate::svg::length(&V::Str("-2em".into())).unwrap();
        assert!(negative.nonnegative("width").is_err());
        for bad in ["em", "1 2em", "NaNem", "1rem"] {
            assert!(crate::svg::length(&V::Str(bad.into())).is_err(), "{bad}");
        }
    }
}
