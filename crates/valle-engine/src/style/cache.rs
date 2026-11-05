//! Computed declarations whose only inputs are their parent, viewport and resource table.
//! SVG geometry and shader uniforms are evaluated separately, even when this part is reused.

use std::rc::Rc;

use super::color::Rgba;
use super::computed::{Paint, Stroke, Style, computed_transforms};
use super::declaration::Declared;
use super::font::{Decoration, Font};
use super::property::Field;
use super::units::{Lengths, Units};

use crate::front::Slot;
use crate::template::Prop;

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
    pub fn new(rest: &[(Field, Prop, std::rc::Rc<str>, bool)]) -> Option<Self> {
        let mut updates = Self::default();
        for (field, prop, ..) in rest {
            if !matches!(prop, Prop::Expr(_)) {
                continue;
            }
            match field {
                Field::Inset(_) => updates.inset = true,
                Field::Translate | Field::Rotate | Field::Scale | Field::Transform => {
                    updates.transform = true
                }
                Field::Opacity => updates.opacity = true,
                _ => return None,
            }
        }
        Some(updates)
    }

    pub fn apply(&self, style: &mut Style, d: &Declared, canvas: [f64; 2]) -> bool {
        let u = Units {
            em: style.font.size,
            canvas,
        };
        if self.inset {
            // A calc handle is an index into all the frame's calcs. When insets add or remove
            // handles, recompute the whole style so later properties keep their correct indices.
            if !style.calcs.is_empty() {
                return false;
            }
            let mut lengths = Lengths {
                u,
                first: 0,
                calcs: Vec::new(),
            };
            let inset = lengths.rect(&d.inset, Lengths::lpa);
            if !lengths.calcs.is_empty() {
                return false;
            }
            style.layout.inset = inset;
        }
        if self.transform {
            style.transform = computed_transforms(d, u);
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
mod tests {
    use crate::front::{Holds, Slot};
    use crate::style::Sheet;
    use crate::template::{Kind, Measure, Node, Op, Prop, Tag, Template, V};

    fn node(style: &[(&str, Prop)], children: &[u32]) -> Node {
        Node {
            kind: Kind::View,
            path: None,
            classes: Vec::new(),
            style: style
                .iter()
                .map(|(k, v)| ((*k).into(), v.clone()))
                .collect(),
            children: children.to_vec(),
            site: None,
        }
    }

    fn text(s: &str) -> Prop {
        Prop::Const(V::Str(s.into()))
    }

    #[test]
    fn changing_inherited_styles_and_calc_offsets_matches_a_fresh_sheet() {
        let template = Template {
            nodes: vec![
                node(
                    &[
                        ("fontSize", Prop::Expr(0)),
                        ("color", Prop::Expr(1)),
                        ("textDecoration", Prop::Expr(2)),
                    ],
                    &[1, 2],
                ),
                node(&[("width", Prop::Expr(3))], &[]),
                node(
                    &[
                        ("position", text("absolute")),
                        ("left", Prop::Expr(4)),
                        ("top", text("2em")),
                        ("translate", Prop::Expr(5)),
                        ("opacity", Prop::Expr(6)),
                        ("backgroundColor", text("currentColor")),
                    ],
                    &[],
                ),
            ],
            roots: vec![0],
            ..Default::default()
        };
        let sheet = Sheet::new(&template).unwrap();
        for i in [0, 1, 2, 1, 0] {
            let values = [
                V::Num(12.0 + 4.0 * i as f64),
                V::Str(["red", "blue", "green"][i].into()),
                V::Str(["none", "underline", "line-through"][i].into()),
                V::Str(["10px", "calc(50% + 1px)", "20px"][i].into()),
                V::Str(["1em", "calc(10% + 2px)", "20%"][i].into()),
                V::Str(["none", "1em 10%", "2vw"][i].into()),
                V::Num(0.5 + i as f64 / 4.0),
            ];
            let fresh = Sheet::new(&template)
                .unwrap()
                .compute(&values, true, [200.0, 100.0], &[])
                .unwrap();
            let reused = sheet.compute(&values, true, [200.0, 100.0], &[]).unwrap();
            assert_eq!(reused, fresh);
            assert_eq!(
                sheet
                    .compute_only(&[0, 2], &values, [200.0, 100.0], &[], &reused)
                    .unwrap(),
                [fresh[0].clone(), fresh[2].clone()]
            );
        }
    }

    #[test]
    fn geometry_inheritance_and_declaration_order_survive_reuse() {
        let mut svg = node(
            &[("fill", Prop::Expr(0)), ("strokeWidth", Prop::Expr(1))],
            &[1],
        );
        svg.kind = Kind::Svg {
            tag: Tag::Svg,
            attributes: Vec::new(),
        };
        let mut circle = node(
            &[("opacity", Prop::Expr(2)), ("opacity", text("0.75"))],
            &[],
        );
        circle.kind = Kind::Svg {
            tag: Tag::Circle,
            attributes: vec![
                ("r".into(), Prop::Expr(4)),
                ("transform".into(), Prop::Expr(3)),
            ],
        };
        let template = Template {
            nodes: vec![svg, circle],
            roots: vec![0],
            ..Default::default()
        };
        let sheet = Sheet::new(&template).unwrap();
        for i in [0, 1, 2, 0] {
            let values = [
                V::Str(["red", "blue", "green"][i].into()),
                V::Num(i as f64),
                V::Num(0.1 * i as f64),
                V::Str(format!("rotate({})", i * 30).into()),
                V::Num(5.0 + i as f64),
            ];
            let fresh = Sheet::new(&template)
                .unwrap()
                .compute(&values, true, [200.0, 100.0], &[])
                .unwrap();
            assert_eq!(
                sheet.compute(&values, true, [200.0, 100.0], &[]).unwrap(),
                fresh
            );
            assert_eq!(fresh[1].opacity, 0.75);
        }
        let invalid = [
            V::Str("red".into()),
            V::Num(1.0),
            V::Num(1.0),
            V::Str("broken".into()),
            V::Num(8.0),
        ];
        assert!(sheet.compute(&invalid, true, [200.0, 100.0], &[]).is_err());
    }

    #[test]
    fn viewport_resources_and_late_values_do_not_leave_stale_styles() {
        let mut template = Template {
            nodes: vec![node(
                &[
                    ("width", text("10vw")),
                    ("backgroundImage", text("url(resource://photo)")),
                    ("opacity", Prop::Expr(0)),
                ],
                &[],
            )],
            roots: vec![0],
            ..Default::default()
        };
        template
            .program
            .ops
            .push(Op::Bounds("box".into(), Measure::Width));
        let sheet = Sheet::new(&template).unwrap();
        let mut resources = ["photo", "other"].map(|name| Slot {
            name: name.into(),
            kind: Holds::Image,
            default: Vec::new(),
        });
        for (width, settled) in [(200.0, false), (400.0, true), (200.0, true), (200.0, false)] {
            resources.reverse();
            let values = [V::Num(0.25)];
            let canvas = [width, 100.0];
            let fresh = Sheet::new(&template)
                .unwrap()
                .compute(&values, settled, canvas, &resources)
                .unwrap();
            assert_eq!(
                sheet.compute(&values, settled, canvas, &resources).unwrap(),
                fresh
            );
            assert_eq!(fresh[0].opacity, if settled { 0.25 } else { 1.0 });
        }
    }

    #[test]
    fn a_parent_switching_out_of_flex_still_rejects_an_inline_child() {
        let template = Template {
            nodes: vec![
                node(&[("display", Prop::Expr(0))], &[1]),
                node(&[("display", text("inline-block"))], &[]),
            ],
            roots: vec![0],
            ..Default::default()
        };
        let sheet = Sheet::new(&template).unwrap();
        sheet
            .compute(&[V::Str("flex".into())], true, [200.0, 100.0], &[])
            .unwrap();
        assert!(
            sheet
                .compute(&[V::Str("block".into())], true, [200.0, 100.0], &[])
                .is_err()
        );
    }
}
