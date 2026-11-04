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
