use super::*;
use crate::template::Kind;

fn text(s: &str) -> Prop {
    Prop::Const(V::Str(s.into()))
}

fn num(x: f64) -> Prop {
    Prop::Const(V::Num(x))
}

fn view(style: &[(&str, Prop)], children: &[u32]) -> Node {
    Node {
        kind: Kind::View,
        path: None,
        classes: Vec::new(),
        style: style
            .iter()
            .map(|(k, p)| (Rc::from(*k), p.clone()))
            .collect(),
        children: children.to_vec(),
        site: Some("s.tsx:1:1".into()),
    }
}

fn sheet(nodes: Vec<Node>) -> Result<Sheet, Error> {
    Sheet::new(&Template {
        nodes,
        roots: vec![0],
        ..Default::default()
    })
}

fn styles(nodes: Vec<Node>, values: &[V]) -> Result<Vec<Style>, Error> {
    sheet(nodes)?.compute(values)
}

fn one(style: &[(&str, Prop)]) -> Style {
    styles(vec![view(style, &[])], &[]).unwrap().remove(0)
}

fn error(style: &[(&str, Prop)]) -> String {
    styles(vec![view(style, &[])], &[]).unwrap_err()
}

fn px(v: f32) -> LengthPercentage {
    LengthPercentage::length(v)
}

fn auto_px(v: f32) -> LengthPercentageAuto {
    LengthPercentageAuto::length(v)
}

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
fn the_flex_shorthand_follows_css() {
    let flex = |p: Prop| {
        let l = one(&[("flex", p)]).layout;
        (l.flex_grow, l.flex_shrink, l.flex_basis)
    };
    let zero = Dimension::percent(0.0);
    assert_eq!(flex(num(2.0)), (2.0, 1.0, zero));
    assert_eq!(flex(text("2 3")), (2.0, 3.0, zero));
    assert_eq!(flex(text("1 1 0")), (1.0, 1.0, Dimension::length(0.0)));
    assert_eq!(flex(text("10px")), (1.0, 1.0, Dimension::length(10.0)));
    assert_eq!(flex(text("2 30%")), (2.0, 1.0, Dimension::percent(0.3)));
    assert_eq!(flex(text("30% 2 0")), (2.0, 0.0, Dimension::percent(0.3)));
    assert_eq!(flex(text("auto")), (1.0, 1.0, Dimension::auto()));
    assert_eq!(flex(text("none")), (0.0, 0.0, Dimension::auto()));
    for bad in ["1 10px 2", "1 2 3 4", "-1", "1px 2px", ""] {
        assert!(
            error(&[("flex", text(bad))]).starts_with("s.tsx:1:1: style flex: "),
            "{bad}"
        );
    }
}

#[test]
fn text_properties_inherit() {
    let tree = vec![
        view(&[], &[1, 4]),
        view(
            &[
                ("fontSize", num(20.0)),
                ("color", text("RED")),
                ("fontFamily", text("Brand Sans, 'Noto Serif', serif")),
                ("lineHeight", num(1.5)),
                ("letterSpacing", text("0.1em")),
                ("fontWeight", text("bold")),
                ("textAlign", text("center")),
            ],
            &[2],
        ),
        view(
            &[("fontSize", text("2em")), ("fontWeight", text("bolder"))],
            &[3],
        ),
        view(
            &[
                ("fontSize", text("50%")),
                ("lineHeight", text("150%")),
                ("fontWeight", text("lighter")),
            ],
            &[],
        ),
        view(&[], &[]),
    ];
    let s = styles(tree, &[]).unwrap();
    let initial = Font {
        size: 16.0,
        weight: 400.0,
        family: Rc::new([Family::SansSerif]),
        line_height: LineHeight::Normal,
        align: TextAlign::Start,
        spacing: 0.0,
    };
    assert_eq!(s[0].font, initial);
    assert_eq!(s[0].color, color::BLACK);
    assert_eq!(s[4].font, initial);

    let brand = Font {
        size: 20.0,
        weight: 700.0,
        family: Rc::new([
            Family::Named("Brand Sans".into()),
            Family::Named("Noto Serif".into()),
            Family::Serif,
        ]),
        line_height: LineHeight::Factor(1.5),
        align: TextAlign::Center,
        spacing: 2.0,
    };
    assert_eq!(s[1].font, brand);
    assert_eq!(s[1].color, [1.0, 0.0, 0.0, 1.0]);
    // Letter spacing inherits as px; a factor line height inherits as the factor.
    let bigger = Font {
        size: 40.0,
        weight: 900.0,
        ..brand.clone()
    };
    assert_eq!(s[2].font, bigger);
    assert_eq!(s[2].color, s[1].color);
    let smaller = Font {
        size: 20.0,
        weight: 700.0,
        line_height: LineHeight::Px(30.0),
        ..brand
    };
    assert_eq!(s[3].font, smaller);
}

#[test]
fn inline_displays_blockify_in_flex_containers() {
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
    let absolute = [
        ("display", text("inline-block")),
        ("position", text("absolute")),
    ];
    assert_eq!(tree("block", &absolute), Ok(Display::Block));
    assert_eq!(
        tree("block", &[("display", text("inline-flex"))]),
        Err(
            "s.tsx:1:1: display: inline-flex needs a flex parent: there is no HTML inline \
             flow; use flex layout, and Span for styled runs of text"
                .into()
        )
    );
    // The canvas is a block container.
    assert!(error(&[("display", text("inline-block"))]).contains("needs a flex parent"));
}

#[test]
fn expressions_are_set_in_declaration_order() {
    let padding = |style: &[(&str, Prop)]| {
        let s = styles(vec![view(style, &[])], &[V::Num(10.0)]).unwrap();
        (s[0].layout.padding.top, s[0].layout.padding.left)
    };
    // A constant shorthand after a traced side overrides it, and the other way round.
    assert_eq!(
        padding(&[("paddingTop", Prop::Expr(0)), ("padding", num(4.0))]),
        (px(4.0), px(4.0))
    );
    assert_eq!(
        padding(&[("padding", num(4.0)), ("paddingTop", Prop::Expr(0))]),
        (px(10.0), px(4.0))
    );
    let sheet = sheet(vec![view(&[("width", Prop::Expr(0))], &[])]).unwrap();
    assert_eq!(
        sheet.compute(&[V::Num(5.0)]).unwrap()[0].layout.size.width,
        Dimension::length(5.0)
    );
    assert_eq!(
        sheet.compute(&[V::Str("abc".into())]).unwrap_err(),
        "s.tsx:1:1: style width: abc is not a length"
    );
    assert_eq!(
        sheet.compute(&[V::Bool(true)]).unwrap_err(),
        "s.tsx:1:1: style width: true is not a length"
    );
}

#[test]
fn paint_values() {
    let s = one(&[
        ("fontSize", num(20.0)),
        ("backgroundColor", text("#ff000080")),
        ("borderRadius", text("0.5em")),
        ("opacity", text("50%")),
    ]);
    assert_eq!(s.background, [1.0, 0.0, 0.0, 128.0 / 255.0]);
    assert_eq!(s.radius, 10.0);
    assert_eq!(s.opacity, 0.5);
    assert_eq!(
        one(&[("background", text("none"))]).background,
        color::TRANSPARENT
    );
    assert_eq!(one(&[("background", text("white"))]).background, [1.0; 4]);
    assert_eq!(one(&[("opacity", num(2.0))]).opacity, 1.0);
}

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
            "boxShadow",
            text("none"),
            "style boxShadow is not supported",
        ),
        ("width", text("10"), "style width: 10 needs a unit: 10px"),
        (
            "width",
            text("10vw"),
            "style width: 10vw: vw is not supported yet",
        ),
        (
            "width",
            text("1in"),
            "style width: 1in: in is not supported; use px, em, rem or %",
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
            text("grid"),
            "style display: grid is not supported yet",
        ),
        ("display", text("table-cell"), "there is no document flow"),
        ("display", num(1.0), "style display: 1 is not a string"),
        ("position", text("sticky"), "the canvas does not scroll"),
        (
            "position",
            text("fixed"),
            "fixed is not one of static, relative, absolute",
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
            "images are not supported yet",
        ),
        (
            "borderRadius",
            text("1px 2px"),
            "one radius for every corner is supported so far",
        ),
        (
            "borderRadius",
            text("50%"),
            "percentages are not supported here",
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
    let mut classes = view(&[], &[]);
    classes.classes.push("flex".into());
    assert_eq!(
        styles(vec![classes], &[]).err().unwrap(),
        "s.tsx:1:1: className is not supported yet"
    );
}

#[test]
fn nodes_out_of_preorder_are_an_engine_bug() {
    let tree = vec![view(&[], &[]), view(&[], &[0])];
    assert!(sheet(tree).err().unwrap().starts_with("engine bug"));
}
