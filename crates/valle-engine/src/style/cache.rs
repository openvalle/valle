//! Computed declarations whose only inputs are their parent, viewport and resource table.
//! SVG geometry and shader uniforms are evaluated separately, even when this part is reused.

use std::rc::Rc;

use super::{Decoration, Font, Paint, Rgba, Slot, Stroke, Style};

/// Only the inputs `compute` reads from its parent, not another copy of the parent box.
struct Parent {
    font: Rc<Font>,
    color: Rgba,
    visible: bool,
    display: taffy::Display,
    fill: Paint,
    fill_opacity: f64,
    even_odd: bool,
    stroke: Stroke,
    decorations: Vec<Decoration>,
    pixelated: bool,
    clip_even_odd: bool,
    markers: [Option<Rc<str>>; 3],
}

impl Parent {
    fn new(s: &Style) -> Self {
        Self {
            font: s.font.clone(),
            color: s.color,
            visible: s.visible,
            display: s.layout.display,
            fill: s.fill.clone(),
            fill_opacity: s.fill_opacity,
            even_odd: s.even_odd,
            stroke: s.stroke.clone(),
            decorations: s.decorations.clone(),
            pixelated: s.pixelated,
            clip_even_odd: s.clip_even_odd,
            markers: s.markers.clone(),
        }
    }
}

pub(super) struct Base {
    parent: Option<Parent>,
    canvas: [f64; 2],
    first: usize,
    resources: Vec<Slot>,
    pub style: Style,
}

impl Base {
    pub fn new(
        parent: Option<&Style>,
        canvas: [f64; 2],
        first: usize,
        resources: &[Slot],
        style: Style,
    ) -> Self {
        Self {
            parent: parent.map(Parent::new),
            canvas,
            first,
            resources: resources.to_vec(),
            style,
        }
    }

    pub fn matches(
        &self,
        parent: Option<&Style>,
        canvas: [f64; 2],
        first: usize,
        resources: &[Slot],
    ) -> bool {
        self.canvas == canvas
            && self.first == first
            && self.resources == resources
            && match (self.parent.as_ref(), parent) {
                (None, None) => true,
                (Some(a), Some(b)) => {
                    // Keep in step with what `compute` reads from its parent. In particular,
                    // container display blockifies children and decorations propagate even
                    // though they are not inherited CSS declarations.
                    a.font == b.font
                        && a.color == b.color
                        && a.visible == b.visible
                        && a.display == b.layout.display
                        && a.fill == b.fill
                        && a.fill_opacity == b.fill_opacity
                        && a.even_odd == b.even_odd
                        && a.stroke == b.stroke
                        && a.decorations == b.decorations
                        && a.pixelated == b.pixelated
                        && a.clip_even_odd == b.clip_even_odd
                        && a.markers == b.markers
                }
                _ => false,
            }
    }
}

/// Declarations that can change without recomputing fonts, paints and the rest of the box.
/// Any other dynamic declaration keeps the full computation path.
#[derive(Default)]
pub(super) struct Updates {
    inset: bool,
    transform: bool,
    opacity: bool,
}

impl Updates {
    pub fn new(rest: &[(super::Field, super::Prop, std::rc::Rc<str>, bool)]) -> Option<Self> {
        let mut updates = Self::default();
        for (field, prop, ..) in rest {
            if !matches!(prop, super::Prop::Expr(_)) {
                continue;
            }
            match field {
                super::Field::Inset(_) => updates.inset = true,
                super::Field::Translate
                | super::Field::Rotate
                | super::Field::Scale
                | super::Field::Transform => updates.transform = true,
                super::Field::Opacity => updates.opacity = true,
                _ => return None,
            }
        }
        Some(updates)
    }

    pub fn apply(&self, style: &mut Style, d: &super::Declared, canvas: [f64; 2]) -> bool {
        let u = super::Units {
            em: style.font.size,
            canvas,
        };
        if self.inset {
            // A calc handle is an index into all the frame's calcs. When insets add or remove
            // handles, recompute the whole style so later properties keep their correct indices.
            if !style.calcs.is_empty() {
                return false;
            }
            let mut lengths = super::Lengths {
                u,
                first: 0,
                calcs: Vec::new(),
            };
            let inset = lengths.rect(&d.inset, super::Lengths::lpa);
            if !lengths.calcs.is_empty() {
                return false;
            }
            style.layout.inset = inset;
        }
        if self.transform {
            style.transform = super::computed_transforms(d, u);
            style.individual = [&d.translate, &d.rotate, &d.scale]
                .into_iter()
                .flatten()
                .count();
        }
        if self.opacity {
            style.opacity = d.opacity;
        }
        true
    }
}

#[cfg(test)]
mod tests;
