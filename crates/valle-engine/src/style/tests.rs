use super::*;
use crate::template::Kind;

const CANVAS: [f32; 2] = [200.0, 100.0];

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
    sheet(nodes)?.compute(values, CANVAS)
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
fn logical_sides_are_the_physical_ones_in_declaration_order() {
    let l = one(&[
        ("padding", num(1.0)),
        ("paddingInline", text("2px 3px")),
        ("paddingTop", num(4.0)),
        ("paddingBlock", num(5.0)),
        ("marginInlineEnd", num(6.0)),
        ("marginRight", num(7.0)),
        ("insetBlockEnd", text("10%")),
    ])
    .layout;
    let padding = taffy::Rect {
        top: px(5.0),
        right: px(3.0),
        bottom: px(5.0),
        left: px(2.0),
    };
    assert_eq!(l.padding, padding);
    assert_eq!(l.margin.right, auto_px(7.0));
    assert_eq!(l.inset.bottom, LengthPercentageAuto::percent(0.1));
    assert!(error(&[("marginBlock", text("1px 2px 3px"))]).ends_with("takes one or two lengths"));
}

#[test]
fn box_sizing_aspect_ratio_and_order() {
    let initial = one(&[]);
    assert_eq!(initial.layout.box_sizing, BoxSizing::BorderBox);
    assert_eq!((initial.layout.aspect_ratio, initial.order), (None, 0));
    let content = one(&[("boxSizing", text(" Content-Box "))]);
    assert_eq!(content.layout.box_sizing, BoxSizing::ContentBox);
    let ratio = |p: Prop| one(&[("aspectRatio", p)]).layout.aspect_ratio;
    assert_eq!(ratio(num(2.0)), Some(2.0));
    assert_eq!(ratio(text("16/10")), Some(1.6));
    assert_eq!(ratio(text("auto 3 / 2")), Some(1.5));
    assert_eq!(ratio(text("1 auto")), Some(1.0));
    for none in [text("auto"), text("0"), text("2 / 0"), num(0.0)] {
        assert_eq!(ratio(none), None);
    }
    for bad in ["16:9", "1 / -2", "1 2", "auto auto", "1px", ""] {
        let e = error(&[("aspectRatio", text(bad))]);
        assert!(
            e.starts_with("s.tsx:1:1: style aspectRatio: "),
            "{bad}: {e}"
        );
    }
    assert!(error(&[("aspectRatio", num(-1.0))]).ends_with("-1 is negative"));
    assert_eq!(one(&[("order", text("-3"))]).order, -3);
    assert!(error(&[("order", num(1.5))]).ends_with("1.5 is not an integer"));
}

#[test]
fn viewport_units_take_the_canvas() {
    let l = one(&[
        ("width", text("10vw")),
        ("height", text("10vh")),
        ("paddingLeft", text("10vmin")),
        ("paddingRight", text("10vmax")),
        ("paddingTop", text("10dvh")),
        ("paddingBottom", text("10svi")),
        ("marginLeft", text("-5lvb")),
    ])
    .layout;
    assert_eq!(l.size.width, Dimension::length(20.0));
    assert_eq!(l.size.height, Dimension::length(10.0));
    let padding = taffy::Rect {
        top: px(10.0),
        right: px(20.0),
        bottom: px(20.0),
        left: px(10.0),
    };
    assert_eq!(l.padding, padding);
    assert_eq!(l.margin.left, auto_px(-5.0));
    assert_eq!(one(&[("fontSize", text("10vw"))]).font.size, 20.0);
    assert!(
        error(&[("width", text("1vq"))])
            .ends_with("1vq: vq is not supported; use px, em, rem, %, vw, vh, vmin or vmax")
    );
}

#[test]
fn math_functions_are_px_or_calcs_for_layout() {
    let s = one(&[
        ("fontSize", text("calc(1em + 4px)")),
        ("width", text("calc(100% - 2em)")),
        ("height", text("calc(2 * (1vw + 3px))")),
        ("minWidth", text("clamp(10px, 50%, 2em)")),
        ("paddingLeft", text("calc(10px - 20px)")),
        ("marginLeft", text("calc(10px - 20px)")),
    ]);
    assert_eq!(s.font.size, 20.0);
    let l = &s.layout;
    // With a percentage in it, a calc layout resolves; it clamps at 0 where lengths have to.
    assert_eq!(l.size.width, Dimension::calc(math::handle(0)));
    assert_eq!(
        l.min_size.width,
        LengthPercentageAuto::calc(math::handle(1))
    );
    let calcs = &s.calcs;
    let [width, min] = [0, 1].map(|i| move |basis| calcs[i].resolve(basis));
    assert_eq!((width(200.0), width(10.0)), (160.0, 0.0));
    assert_eq!((min(200.0), min(60.0), min(10.0)), (40.0, 30.0, 10.0));
    // Without one, it is px.
    assert_eq!(l.size.height, Dimension::length(10.0));
    assert_eq!((l.padding.left, l.margin.left), (px(0.0), auto_px(-10.0)));
    for (bad, e) in [
        ("calc(1px+2px)", "+ and - need white space around them"),
        ("calc(1px * 2px)", "two lengths do not multiply"),
        ("calc(1px / 0)", "it divides by 0"),
        ("calc(1 + 1px)", "a number and a length do not add"),
        ("min(1px, 2)", "numbers and lengths do not compare"),
        ("calc(2)", "calc(2) is a number, not a length"),
        ("calc(1px, 2px)", "calc() takes one sum"),
        (
            "clamp(1px, 2px)",
            "clamp() takes a minimum, a value and a maximum",
        ),
        ("calc((1px)", "it ends too soon"),
        ("calc(1px) 2px", "calc(1px) 2px is not a length"),
        (
            "calc(1in)",
            "in is not supported; use px, em, rem, %, vw, vh, vmin or vmax",
        ),
    ] {
        let got = error(&[("width", text(bad))]);
        assert!(
            got.starts_with("s.tsx:1:1: style width: ") && got.ends_with(e),
            "{got}"
        );
    }
    let radius = error(&[("borderRadius", text("calc(1% + 1px)"))]);
    assert!(radius.ends_with("percentages are not supported here"));
}

#[test]
fn sizing_keywords_go_to_taffy() {
    let l = one(&[
        ("width", text("fit-content")),
        ("height", text("max-content")),
        ("flexBasis", text("content")),
    ])
    .layout;
    assert_eq!(l.size.width, Dimension::fit_content());
    assert_eq!(l.size.height, Dimension::max_content());
    assert_eq!(l.flex_basis, Dimension::content());
    let stretch = one(&[("width", text("Stretch"))]).layout.size.width;
    assert_eq!(stretch, Dimension::stretch());
    let e = error(&[("minWidth", text("min-content"))]);
    assert!(
        e.ends_with("min-content: only width, height and flex-basis take sizing keywords so far")
    );
    assert!(error(&[("width", text("content"))]).ends_with("content is not a length"));
    let e = error(&[("width", text("fit-content(30%)"))]);
    assert!(e.ends_with(
        "browsers take fit-content() only for grid tracks; use fit-content and max-width"
    ));
}

/// Sets of line names.
fn names(ns: &[&[&str]]) -> Vec<Vec<String>> {
    ns.iter()
        .map(|n| n.iter().map(|s| s.to_string()).collect())
        .collect()
}

#[test]
fn grid_tracks_and_lines_go_to_taffy() {
    use taffy::{
        GridTemplateComponent as C, GridTemplateRepetition, MaxTrackSizingFunction as Max, MinMax,
        MinTrackSizingFunction as Min, RepetitionCount,
    };
    let s = one(&[
        ("display", text("grid")),
        ("fontSize", num(10.0)),
        (
            "gridTemplateColumns",
            text("[Start] 2em repeat(2, [a] minmax(10%, 1FR)) fit-content(calc(50% - 5px)) [End]"),
        ),
        ("gridTemplateRows", text("repeat(auto-fill, 20px)")),
        ("gridAutoRows", text("min-content 3fr")),
        ("gridAutoFlow", text("column dense")),
        ("gridRow", text("span 2 / Main")),
        ("gridColumn", text("Main")),
        ("gridColumnEnd", num(-1.0)),
        ("justifyItems", text("center")),
        ("justifySelf", text("auto")),
    ]);
    let l = &s.layout;
    let both = |min, max| MinMax { min, max };
    assert_eq!(l.display, Display::Grid);
    assert_eq!(
        l.grid_template_columns,
        [
            C::Single(both(Min::length(20.0), Max::length(20.0))),
            C::Repeat(GridTemplateRepetition {
                count: RepetitionCount::Count(2),
                tracks: vec![both(Min::percent(0.1), Max::fr(1.0))],
                line_names: names(&[&["a"], &[]]),
            }),
            C::Single(both(Min::auto(), Max::fit_content_calc(math::handle(0)))),
        ]
    );
    // Names are as written; a set for each line around the parts.
    assert_eq!(
        l.grid_template_column_names,
        names(&[&["Start"], &[], &[], &["End"]])
    );
    assert_eq!(s.calcs.len(), 1);
    assert_eq!(
        l.grid_template_rows,
        [C::Repeat(GridTemplateRepetition {
            count: RepetitionCount::AutoFill,
            tracks: vec![both(Min::length(20.0), Max::length(20.0))],
            line_names: names(&[&[], &[]]),
        })]
    );
    assert_eq!(
        l.grid_auto_rows,
        [
            both(Min::min_content(), Max::min_content()),
            both(Min::auto(), Max::fr(3.0)),
        ]
    );
    assert_eq!(l.grid_auto_flow, taffy::GridAutoFlow::ColumnDense);
    let main = || GridPlacement::NamedLine("Main".to_string(), 0);
    assert_eq!(l.grid_row.start, GridPlacement::Span(2));
    assert_eq!(l.grid_row.end, main());
    // A name alone ends where it starts, unless something sets the end.
    assert_eq!(l.grid_column.start, main());
    assert_eq!(l.grid_column.end, GridPlacement::Line((-1i16).into()));
    assert_eq!(l.justify_items, AlignItems::CENTER);
    assert_eq!(l.justify_self, None);

    let line = |v: &str| one(&[("gridRowStart", text(v))]).layout.grid_row.start;
    assert_eq!(line("2 a"), GridPlacement::NamedLine("a".into(), 2));
    assert_eq!(line("a -1"), GridPlacement::NamedLine("a".into(), -1));
    assert_eq!(line("span B 2"), GridPlacement::NamedSpan("B".into(), 2));
    assert_eq!(line("SPAN b"), GridPlacement::NamedSpan("b".into(), 1));
    assert_eq!(line("Auto"), GridPlacement::Auto);
    let flow = |v: &str| one(&[("gridAutoFlow", text(v))]).layout.grid_auto_flow;
    assert_eq!(flow("dense"), taffy::GridAutoFlow::RowDense);
    assert_eq!(flow("COLUMN"), taffy::GridAutoFlow::Column);
    let none = one(&[("gridTemplateColumns", text("none"))]);
    assert!(none.layout.grid_template_columns.is_empty());
    assert!(none.layout.grid_template_column_names.is_empty());
}

#[test]
fn grid_areas_and_shorthands_go_to_taffy() {
    use taffy::{GridAutoFlow, GridTemplateArea};
    let area = |name: &str, rows: [u16; 2], columns: [u16; 2]| GridTemplateArea {
        name: name.into(),
        row_start: rows[0],
        row_end: rows[1],
        column_start: columns[0],
        column_end: columns[1],
    };
    let s = one(&[(
        "gridTemplateAreas",
        text("'Head Head' \"side .\" 'side main'"),
    )]);
    let a = s.layout.grid_template_areas.unwrap();
    assert_eq!(
        a.areas,
        [
            area("Head", [1, 2], [1, 3]),
            area("side", [2, 4], [1, 2]),
            area("main", [3, 4], [2, 3]),
        ]
    );
    assert_eq!((a.row_count, a.column_count), (3, 2));

    // Rows of names, a string and a size; columns after the `/`.
    let s = one(&[(
        "gridTemplate",
        text("[top] \"a a\" 20px [mid] \"b c\" / [l] 1fr 2fr"),
    )]);
    let l = &s.layout;
    assert_eq!(l.grid_template_rows.len(), 2);
    assert_eq!(l.grid_template_row_names, names(&[&["top"], &["mid"], &[]]));
    assert_eq!(l.grid_template_column_names, names(&[&["l"], &[], &[]]));
    assert_eq!(l.grid_template_areas.as_ref().unwrap().areas.len(), 3);

    // `grid` sets what items flow into, and resets the rest.
    let s = one(&[
        ("gridAutoRows", text("5px")),
        ("grid", text("auto-flow dense 10px / 1fr 1fr")),
    ]);
    assert_eq!(s.layout.grid_auto_flow, GridAutoFlow::RowDense);
    assert_eq!(s.layout.grid_auto_rows.len(), 1);
    assert_eq!(s.layout.grid_template_columns.len(), 2);
    assert!(s.layout.grid_template_rows.is_empty());
    let s = one(&[
        ("gridAutoRows", text("5px")),
        ("grid", text("20px / auto-flow")),
    ]);
    assert_eq!(s.layout.grid_auto_flow, GridAutoFlow::Column);
    assert!(s.layout.grid_auto_rows.is_empty());

    // Lines `grid-area` leaves out take the name of their start.
    let s = one(&[("gridArea", text("main"))]);
    let main = || GridPlacement::NamedLine("main".to_string(), 0);
    let line = |start, end| Line { start, end };
    assert_eq!(s.layout.grid_row, line(main(), main()));
    assert_eq!(s.layout.grid_column, line(main(), main()));
    let s = one(&[("gridArea", text("2 / a / span 2"))]);
    let a = || GridPlacement::NamedLine("a".to_string(), 0);
    let row = line(GridPlacement::Line(2i16.into()), GridPlacement::Span(2));
    assert_eq!(s.layout.grid_row, row);
    assert_eq!(s.layout.grid_column, line(a(), a()));

    // One value of a `place-` shorthand is both.
    let s = one(&[
        ("placeItems", text("center")),
        ("placeSelf", text("end start")),
        ("placeContent", text("space-between center")),
    ]);
    let l = &s.layout;
    assert_eq!(
        (l.align_items, l.justify_items),
        (AlignItems::CENTER, AlignItems::CENTER)
    );
    assert_eq!(
        (l.align_self, l.justify_self),
        (Some(AlignItems::END), Some(AlignItems::START))
    );
    assert_eq!(
        (l.align_content, l.justify_content),
        (AlignContent::SPACE_BETWEEN, AlignContent::CENTER)
    );
}

#[test]
fn grid_values_out_of_css_are_errors() {
    let cases: &[(&str, &str, &str)] = &[
        ("gridTemplateColumns", "1fr 2", "2 needs a unit: 2px"),
        ("gridTemplateColumns", "", "\"\" has no tracks"),
        (
            "gridTemplateColumns",
            "repeat(auto-fill, 1fr)",
            "beside auto-fill or auto-fit, every track needs a length",
        ),
        (
            "gridTemplateColumns",
            "repeat(auto-fit, 10px) auto",
            "beside auto-fill or auto-fit",
        ),
        (
            "gridTemplateColumns",
            "repeat(auto-fill, 10px) repeat(auto-fit, 10px)",
            "only one repeat() may be auto-fill or auto-fit",
        ),
        ("gridTemplateColumns", "repeat(0, 10px)", "0 is not a count"),
        (
            "gridTemplateColumns",
            "repeat(2, repeat(2, 10px))",
            "repeat() does not nest",
        ),
        (
            "gridTemplateColumns",
            "minmax(1fr, 10px)",
            "the minimum is not flexible",
        ),
        (
            "gridTemplateColumns",
            "fit-content",
            "tracks take fit-content(limit)",
        ),
        (
            "gridTemplateColumns",
            "[a] [b] 10px",
            "put names of one line in one [ ]",
        ),
        (
            "gridTemplateColumns",
            "[span] 10px",
            "span is a keyword, not a line name",
        ),
        ("gridTemplateColumns", "[1a] 10px", "1a is not a line name"),
        ("gridTemplateColumns", "[a 10px", "a [ is not closed"),
        ("gridTemplateColumns", "-1fr", "-1fr is negative"),
        ("gridTemplateRows", "subgrid", "subgrid is not supported"),
        (
            "gridAutoRows",
            "repeat(2, 10px)",
            "implicit tracks take sizes only",
        ),
        ("gridAutoFlow", "row column", "is not row or column"),
        ("gridRow", "0", "there is no line 0"),
        ("gridRow", "1 / 2 / 3", "takes at most 2 lines"),
        ("gridRow", "span 0", "a span is at least 1"),
        ("gridRow", "span", "is not a grid line"),
        ("gridRowStart", "1.5", "1.5 is not a whole number"),
        ("gridColumnStart", "a b", "is not a grid line"),
        ("gridColumnEnd", "auto 2", "auto is a keyword"),
        ("justifyItems", "left", "left is not one of normal, stretch"),
        (
            "gridTemplateColumns",
            "'a' 10px",
            "strings of areas go in grid-template-areas",
        ),
        (
            "gridTemplateAreas",
            "'a b' 'a'",
            "every row needs as many cells as the others",
        ),
        (
            "gridTemplateAreas",
            "'a b' 'b a'",
            "area a is not a rectangle",
        ),
        ("gridTemplateAreas", "'a #'", "# is not in a name"),
        ("gridTemplateAreas", "a b", "each row is a string"),
        ("gridTemplateAreas", "'a", "a string is not closed"),
        (
            "gridTemplate",
            "'a' / repeat(2, 1fr)",
            "columns with areas take no repeat()",
        ),
        (
            "gridTemplate",
            "1fr",
            "takes rows / columns, or rows with areas",
        ),
        (
            "gridTemplate",
            "'a' [x] [y] [z] 'b'",
            "each row is names, a string of areas",
        ),
        (
            "grid",
            "10px auto-flow / 1fr",
            "auto-flow and dense come before the sizes",
        ),
        ("gridArea", "1 / 2 / 3 / 4 / 5", "takes at most 4 lines"),
        ("placeItems", "center end start", "takes one or two values"),
        ("placeContent", "baseline", "baseline is not one of"),
    ];
    for (key, value, message) in cases {
        let e = error(&[(*key, text(value))]);
        assert!(
            e.starts_with(&format!("s.tsx:1:1: style {key}: ")) && e.contains(message),
            "{key}: {value}: {e}"
        );
    }
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
    assert!(error(&[("display", text("inline-block"))]).contains("needs a flex or grid parent"));
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
        sheet.compute(&[V::Num(5.0)], CANVAS).unwrap()[0]
            .layout
            .size
            .width,
        Dimension::length(5.0)
    );
    assert_eq!(
        sheet.compute(&[V::Str("abc".into())], CANVAS).unwrap_err(),
        "s.tsx:1:1: style width: abc is not a length"
    );
    assert_eq!(
        sheet.compute(&[V::Bool(true)], CANVAS).unwrap_err(),
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
