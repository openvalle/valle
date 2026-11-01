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

/// The images of a style's background layers.
fn images(style: &[(&str, Prop)]) -> Vec<gradient::Image> {
    one(style).layers.into_iter().map(|l| l.image).collect()
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
    let spacing = error(&[("letterSpacing", text("calc(1% + 1px)"))]);
    assert!(spacing.ends_with("percentages are not supported here"));
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
fn z_index_is_auto_or_an_integer() {
    assert_eq!(one(&[]).z_index, None);
    assert_eq!(one(&[("zIndex", num(-3.0))]).z_index, Some(-3));
    assert_eq!(one(&[("zIndex", text(" 2 "))]).z_index, Some(2));
    assert_eq!(one(&[("zIndex", text("AUTO"))]).z_index, None);
    assert!(error(&[("zIndex", num(1.5))]).contains("1.5 is not an integer"));
}

#[test]
fn overflow_hidden_on_one_axis_hides_both() {
    use taffy::Overflow::{Clip, Hidden, Visible};
    let overflow = |style: &[(&str, Prop)]| {
        let o = one(style).layout.overflow;
        [o.x, o.y]
    };
    assert_eq!(overflow(&[]), [Visible, Visible]);
    assert_eq!(overflow(&[("overflow", text("clip"))]), [Clip, Clip]);
    assert_eq!(overflow(&[("overflowX", text("clip"))]), [Clip, Visible]);
    assert_eq!(
        overflow(&[("overflowBlock", text("HIDDEN"))]),
        [Hidden, Hidden]
    );
    assert_eq!(
        overflow(&[("overflow", text("clip hidden"))]),
        [Hidden, Hidden]
    );
    assert!(error(&[("overflow", text("auto"))]).contains("the canvas does not scroll"));
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
fn visibility_inherits() {
    let tree = vec![
        view(&[("visibility", text("hidden"))], &[1, 2]),
        view(&[], &[]),
        view(&[("visibility", text("Visible"))], &[]),
    ];
    let s = styles(tree, &[]).unwrap();
    assert_eq!(
        s.iter().map(|s| s.visible).collect::<Vec<_>>(),
        [false, false, true]
    );
    assert!(!one(&[("visibility", text("collapse"))]).visible);
    assert!(error(&[("visibility", text("none"))]).contains("none is not one of visible"));
}

#[test]
fn a_mask_source_inherits_from_the_mask() {
    let mut tree = vec![
        view(&[("color", text("red"))], &[2]),
        view(&[], &[]),
        view(&[], &[]),
    ];
    tree[0].kind = Kind::Mask {
        source: 1,
        luminance: true,
        invert: false,
    };
    let s = styles(tree, &[]).unwrap();
    let source = Source {
        root: 1,
        luminance: true,
        invert: false,
    };
    assert_eq!(s[0].source, Some(source));
    assert_eq!((s[1].source, s[1].color), (None, s[0].color));
    assert!(!s[0].keeps_3d());
}

#[test]
fn a_span_takes_only_text_properties() {
    let mut tree = vec![view(&[("fontSize", num(20.0))], &[1]), view(&[], &[])];
    let span = |style: &[(&str, Prop)]| Node {
        kind: Kind::Span {
            content: Vec::new(),
        },
        ..view(style, &[])
    };
    tree[1] = span(&[("color", text("red")), ("letterSpacing", num(1.0))]);
    let s = styles(tree.clone(), &[]).unwrap();
    assert_eq!((s[1].font.size, s[1].font.spacing), (20.0, 1.0));
    tree[1] = span(&[("padding", num(1.0))]);
    let e = styles(tree, &[]).unwrap_err();
    assert!(
        e.ends_with(
            "style padding: a <Span> is part of a text, not a box, so it takes only text properties"
        ),
        "{e}"
    );
}

#[test]
fn white_space_sets_its_collapsing_and_wrapping() {
    let font = |v: &str| one(&[("whiteSpace", text(v))]).font;
    for (v, collapse, wrap) in [
        ("pre", Collapse::Preserve, false),
        ("nowrap", Collapse::Collapse, false),
        ("break-spaces", Collapse::BreakSpaces, true),
        ("Preserve-Breaks NOWRAP", Collapse::PreserveBreaks, false),
        ("nowrap preserve", Collapse::Preserve, false),
        ("wrap", Collapse::Collapse, true),
    ] {
        let f = font(v);
        assert_eq!((f.collapse, f.wrap), (collapse, wrap), "{v}");
    }
    let f = one(&[
        ("whiteSpace", text("nowrap")),
        ("whiteSpaceCollapse", text("preserve")),
    ])
    .font;
    assert_eq!((f.collapse, f.wrap), (Collapse::Preserve, false));
    for bad in ["pre pre", "wrap nowrap", "auto"] {
        let e = error(&[("whiteSpace", text(bad))]);
        assert!(e.contains("is not a white-space"), "{bad}: {e}");
    }
}

#[test]
fn text_decorations_pass_to_the_text_in_flow_inside() {
    let tree = vec![
        view(
            &[
                ("color", text("red")),
                ("fontSize", num(20.0)),
                ("textDecoration", text("underline overline 10%")),
                ("textUnderlineOffset", text("0.5em")),
            ],
            &[1, 2],
        ),
        view(
            &[
                ("textDecorationLine", text("line-through")),
                ("textDecorationColor", text("blue")),
                ("textDecorationThickness", num(3.0)),
                ("textUnderlineOffset", text("20%")),
                ("textDecorationSkipInk", text("none")),
            ],
            &[],
        ),
        view(&[("position", text("absolute"))], &[]),
    ];
    let s = styles(tree, &[]).unwrap();
    let red = color::parse("red").unwrap();
    let px = |c: &Option<Calc>| c.as_ref().map(|c| c.resolve(10.0));
    let d = &s[0].decorations;
    assert_eq!(d.len(), 1);
    assert_eq!(
        (d[0].lines, d[0].color, d[0].by),
        ([true, true, false], red, 0)
    );
    // Percentages are of the decorating box's font size; em is of the declaring box's.
    assert_eq!(
        (px(&d[0].thickness), px(&d[0].offset)),
        (Some(1.0), Some(10.0))
    );
    let d = &s[1].decorations;
    assert_eq!(d.len(), 2);
    assert_eq!(d[0], s[0].decorations[0]);
    assert_eq!((d[1].lines, d[1].by), ([false, false, true], 1));
    assert_eq!(d[1].color, color::parse("blue").unwrap());
    assert_eq!(
        (px(&d[1].thickness), px(&d[1].offset)),
        (Some(3.0), Some(2.0))
    );
    assert!(s[0].font.skip_ink && !s[1].font.skip_ink);
    assert!(s[2].decorations.is_empty());
}

#[test]
fn text_decoration_sets_its_longhands_or_refuses() {
    let d = |v: &str| one(&[("textDecoration", text(v))]).decorations;
    assert!(d("none").is_empty());
    let all = &d("Line-Through underline solid 2px #00f")[0];
    assert_eq!(
        (all.lines, all.style),
        ([true, false, true], DecorationStyle::Solid)
    );
    let wavy = one(&[
        ("textDecoration", text("underline wavy")),
        ("textDecorationStyle", text("dotted")),
    ]);
    assert_eq!(wavy.decorations[0].style, DecorationStyle::Dotted);
    assert_eq!(all.color, color::parse("#00f").unwrap());
    assert_eq!(all.thickness.as_ref().map(|t| t.resolve(16.0)), Some(2.0));
    let reset = one(&[
        ("textDecorationColor", text("red")),
        ("textDecoration", text("underline")),
    ]);
    assert_eq!(reset.decorations[0].color, color::BLACK);
    // currentcolor is the glyphs' fill color, as in Chrome.
    let filled = one(&[
        ("WebkitTextFillColor", text("red")),
        ("textDecoration", text("underline")),
    ]);
    assert_eq!(filled.decorations[0].color, color::parse("red").unwrap());
    for (bad, message) in [
        ("underline red overline", "twice"),
        ("none underline", "is not a text-decoration-line"),
        ("underline wavy dashed", "twice"),
        ("underline from-font", "from-font is not supported yet"),
        ("underline 2", "needs a unit"),
        ("blink", "is not a color"),
    ] {
        let e = error(&[("textDecoration", text(bad))]);
        assert!(e.contains(message), "{bad}: {e}");
    }
    let e = error(&[("textDecorationThickness", num(-1.0))]);
    assert!(e.contains("is negative"), "{e}");
}

#[test]
fn text_overflow_is_clip_or_an_ellipsis_of_the_box_alone() {
    let tree = vec![
        view(&[("textOverflow", text("Ellipsis"))], &[1]),
        view(&[], &[]),
    ];
    let s = styles(tree, &[]).unwrap();
    assert!(s[0].ellipsis && !s[1].ellipsis);
    assert!(!one(&[("textOverflow", text("clip"))]).ellipsis);
    let e = error(&[("textOverflow", text("'…'"))]);
    assert!(e.contains("is not one of clip, ellipsis"), "{e}");
}

#[test]
fn line_clamp_needs_a_vertical_webkit_box_of_a_text() {
    let clamp = |style: &[(&str, Prop)]| {
        let node = Node {
            kind: Kind::Text {
                content: Vec::new(),
            },
            ..view(style, &[])
        };
        styles(vec![node], &[]).map(|s| s[0].clamp)
    };
    let boxed = [
        ("display", text("-webkit-box")),
        ("WebkitBoxOrient", text("vertical")),
    ];
    let lines = |v| [&boxed[..], &[("WebkitLineClamp", v)]].concat();
    assert_eq!(clamp(&lines(num(2.0))), Ok(Some(2)));
    assert_eq!(clamp(&lines(text("3"))), Ok(Some(3)));
    assert_eq!(clamp(&lines(text("none"))), Ok(None));
    assert_eq!(clamp(&[("WebkitLineClamp", num(2.0))]), Ok(None));
    let row = [boxed[0].clone(), ("WebkitLineClamp", num(2.0))];
    assert_eq!(clamp(&row), Ok(None));
    for (bad, message) in [
        (num(0.0), "0 is not a number of lines"),
        (num(1.5), "1.5 is not an integer"),
    ] {
        let e = clamp(&lines(bad)).unwrap_err();
        assert!(e.contains(message), "{e}");
    }
    let e = error(&boxed);
    assert!(e.contains("display: -webkit-box is only for a Text"), "{e}");
}

#[test]
fn text_wrap_sets_its_mode_and_style_or_refuses() {
    let wrap = |v: &str| {
        let f = one(&[("textWrap", text(v))]).font;
        (f.wrap, f.balance)
    };
    assert_eq!(wrap("balance"), (true, true));
    assert_eq!(wrap("nowrap"), (false, false));
    assert_eq!(wrap("Balance NoWrap"), (false, true));
    assert_eq!(wrap("stable"), (true, false));
    let kept = one(&[
        ("textWrapStyle", text("balance")),
        ("whiteSpace", text("nowrap")),
    ]);
    assert_eq!((kept.font.wrap, kept.font.balance), (false, true));
    for (bad, message) in [
        ("pretty", "pretty is not supported yet"),
        ("wrap nowrap", "twice"),
        ("balance auto", "twice"),
        ("even", "is not one of auto, balance"),
    ] {
        let e = error(&[("textWrap", text(bad))]);
        assert!(e.contains(message), "{bad}: {e}");
    }
}

#[test]
fn text_stroke_sets_its_longhands_or_refuses() {
    let font = |style: &[(&str, Prop)]| one(style).font;
    // The keywords are in proportion to the font size, as in Chrome.
    let both = font(&[
        ("fontSize", num(24.0)),
        ("WebkitTextStroke", text("thick rgb(0 0 255)")),
    ]);
    let blue = color::parse("blue").unwrap();
    assert_eq!((both.stroke, both.stroke_color), (2.5, Some(blue)));
    // Left out of the shorthand, the width is 0 and the color currentcolor.
    let reset = font(&[
        ("WebkitTextStrokeWidth", num(2.0)),
        ("WebkitTextStrokeColor", text("red")),
        ("WebkitTextStroke", text("currentcolor")),
    ]);
    assert_eq!((reset.stroke, reset.stroke_color), (0.0, None));
    assert_eq!(font(&[("WebkitTextStroke", num(1.5))]).stroke, 1.5);
    // A decoration's currentcolor is the stroke's color where the stroke shows in a color of
    // its own, as in Chrome, and else the fill color.
    let decoration = |stroke: &str| {
        one(&[
            ("color", text("red")),
            ("WebkitTextFillColor", text("yellow")),
            ("WebkitTextStroke", text(stroke)),
            ("textDecoration", text("underline")),
        ])
        .decorations[0]
            .color
    };
    let yellow = color::parse("yellow").unwrap();
    assert_eq!(decoration("2px blue"), blue);
    for hidden in ["2px currentcolor", "2px transparent", "0 blue"] {
        assert_eq!(decoration(hidden), yellow, "{hidden}");
    }
    for (key, bad, message) in [
        ("WebkitTextStroke", text("2px red 3px"), "twice"),
        ("WebkitTextStroke", text("2 red"), "needs a unit"),
        ("WebkitTextStrokeWidth", num(-1.0), "is negative"),
        (
            "WebkitTextStrokeWidth",
            text("10%"),
            "percentages are not supported",
        ),
        ("WebkitTextStrokeColor", text("2px"), "is not a color"),
    ] {
        let e = error(&[(key, bad)]);
        assert!(e.contains(message), "{key}: {e}");
    }
}

#[test]
fn text_shadows_inherit_in_px_with_currentcolor_kept() {
    let tree = vec![
        view(
            &[
                ("fontSize", num(20.0)),
                ("textShadow", text("0.1em 2px, -1px 0 3px blue")),
            ],
            &[1],
        ),
        view(&[("color", text("red"))], &[]),
    ];
    let s = styles(tree, &[]).unwrap();
    let blue = color::parse("blue").unwrap();
    let shadows = [
        TextShadow {
            offset: [2.0, 2.0],
            blur: 0.0,
            color: None,
        },
        TextShadow {
            offset: [-1.0, 0.0],
            blur: 3.0,
            color: Some(blue),
        },
    ];
    assert_eq!(&s[0].font.shadows[..], shadows);
    assert_eq!(s[1].font.shadows, s[0].font.shadows);
    for (bad, message) in [
        ("1px 1px 1px 1px", "is not a text shadow"),
        ("inset 1px 1px", "is not a text shadow"),
        ("1px", "needs two to four lengths"),
        ("1px 1px -2px", "is negative"),
    ] {
        let e = error(&[("textShadow", text(bad))]);
        assert!(e.contains(message), "{bad}: {e}");
    }
    assert!(one(&[("textShadow", text("none"))]).font.shadows.is_empty());
}

#[test]
fn font_features_are_quoted_tags_with_their_values() {
    let features = |v: &str| {
        let f = one(&[("fontFeatureSettings", text(v))]).font.features;
        (f.iter())
            .map(|f| (std::str::from_utf8(&f.tag).unwrap().to_owned(), f.value))
            .collect::<Vec<_>>()
    };
    let feature = |tag: &str, value| (tag.to_owned(), value);
    assert_eq!(
        features("'liga' OFF, \"ss03\", 'salt' 2, 'Kern' on"),
        [
            feature("liga", 0),
            feature("ss03", 1),
            feature("salt", 2),
            feature("Kern", 1),
        ]
    );
    assert_eq!(features("Normal"), []);
    for (bad, message) in [
        ("liga", "a tag of 4 characters, quoted"),
        ("'lig'", "a tag of 4 characters, quoted"),
        ("'liga' -1", "-1 is not a feature value"),
        ("'liga' 1.5", "1.5 is not a feature value"),
        ("'liga' 70000", "70000 is over 65535"),
        ("'liga' 1 2", "is not a quoted tag and its value"),
        ("'liga',", "is not a quoted tag and its value"),
    ] {
        let e = error(&[("fontFeatureSettings", text(bad))]);
        assert!(e.contains(message), "{bad}: {e}");
    }
}

#[test]
fn font_style_is_italic_or_oblique_by_an_angle() {
    let style = |v: &str| one(&[("fontStyle", text(v))]).font.style;
    assert_eq!(style("Italic"), FontStyle::Italic);
    assert_eq!(style("oblique"), FontStyle::Oblique(14.0));
    assert_eq!(style("oblique -0.25turn"), FontStyle::Oblique(-90.0));
    assert_eq!(style("normal"), FontStyle::Normal);
    // Chrome slants a face for italic, and for oblique by 14deg or more.
    let slants = |v: &str| style(v).slants();
    assert!(slants("italic") && slants("oblique") && slants("oblique 90deg"));
    assert!(!slants("oblique 13deg") && !slants("oblique -20deg") && !slants("normal"));
    for (bad, message) in [
        (
            "oblique 91deg",
            "91deg is not an angle from -90deg to 90deg",
        ),
        ("oblique 10", "10 is not an angle"),
        (
            "italic 10deg",
            "is not normal, italic, or oblique and an angle",
        ),
        ("slanted", "is not normal, italic, or oblique and an angle"),
    ] {
        let e = error(&[("fontStyle", text(bad))]);
        assert!(e.contains(message), "{bad}: {e}");
    }
}

#[test]
fn font_stretch_is_a_keyword_or_a_percentage() {
    let stretch = |v: &str| one(&[("fontStretch", text(v))]).font.stretch;
    assert_eq!(stretch("Semi-Condensed"), 87.5);
    assert_eq!(stretch("ultra-expanded"), 200.0);
    assert_eq!(stretch("62.5%"), 62.5);
    assert_eq!(stretch("0%"), 0.0);
    for bad in ["-10%", "75", "wide"] {
        let e = error(&[("fontStretch", text(bad))]);
        assert!(e.contains("is not a percentage, 0% or more"), "{bad}: {e}");
    }
}

#[test]
fn font_variations_are_quoted_axes_with_numbers() {
    let variations = |v: &str| {
        let f = one(&[("fontVariationSettings", text(v))]).font.variations;
        (f.iter())
            .map(|v| (std::str::from_utf8(&v.tag).unwrap().to_owned(), v.value))
            .collect::<Vec<_>>()
    };
    let axis = |tag: &str, value| (tag.to_owned(), value);
    assert_eq!(
        variations("'wght' 650, \"wdth\" 75.5, 'XOPQ' -0"),
        [axis("wght", 650.0), axis("wdth", 75.5), axis("XOPQ", 0.0)]
    );
    assert!(variations("'XOPQ' -0")[0].1.is_sign_positive());
    assert_eq!(variations("normal"), []);
    for (bad, message) in [
        ("'wght'", "an axis takes a number after its tag"),
        ("'wght' bold", "bold is not an axis value"),
        ("'wght' 10px", "10px is not an axis value"),
        (
            "wght 400",
            "does not start with a tag of 4 characters, quoted",
        ),
        ("'wght' 1 2", "is not a quoted tag and its value"),
    ] {
        let e = error(&[("fontVariationSettings", text(bad))]);
        assert!(e.contains(message), "{bad}: {e}");
    }
}

#[test]
fn font_variants_set_their_features_one_longhand_at_a_time() {
    let variants = |style: &[(&str, Prop)]| {
        let f = one(style).font;
        (f.variants.iter())
            .map(|v| {
                (v.iter())
                    .map(|f| format!("{}={}", std::str::from_utf8(&f.tag).unwrap(), f.value))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        variants(&[("fontVariant", text("Small-Caps tabular-nums ordinal sub"))]),
        ["", "", "smcp=1", "tnum=1 ordn=1", "", "", "subs=1"]
    );
    assert_eq!(
        variants(&[
            (
                "fontVariantNumeric",
                text("oldstyle-nums diagonal-fractions")
            ),
            (
                "fontVariantLigatures",
                text("no-common-ligatures contextual")
            ),
            ("fontVariantCaps", text("all-small-caps")),
            ("fontVariantEastAsian", text("jis04 full-width ruby")),
            ("fontVariantAlternates", text("historical-forms")),
            ("fontVariantPosition", text("super")),
            ("fontKerning", text("none")),
        ]),
        [
            "kern=0",
            "liga=0 clig=0 calt=1",
            "c2sc=1 smcp=1",
            "onum=1 frac=1",
            "jp04=1 fwid=1 ruby=1",
            "hist=1",
            "sups=1",
        ]
    );
    // The shorthand sets every longhand but kerning; none turns ligatures off.
    assert_eq!(
        variants(&[
            ("fontKerning", text("none")),
            ("fontVariantCaps", text("small-caps")),
            ("fontVariant", text("none")),
        ]),
        [
            "kern=0",
            "liga=0 clig=0 dlig=0 hlig=0 calt=0",
            "",
            "",
            "",
            "",
            ""
        ]
    );
    assert_eq!(
        variants(&[("fontKerning", text("normal"))]),
        variants(&[("fontKerning", text("auto"))])
    );
    for (key, bad, message) in [
        (
            "fontVariantCaps",
            "petite-caps",
            "petite-caps is not supported yet",
        ),
        ("fontVariant", "unicase", "unicase is not supported yet"),
        (
            "fontVariantAlternates",
            "swash(fancy)",
            "needs @font-feature-values",
        ),
        (
            "fontVariantNumeric",
            "lining-nums oldstyle-nums",
            "set the same: give one",
        ),
        (
            "fontVariantNumeric",
            "ordinal ordinal",
            "set the same: give one",
        ),
        (
            "fontVariantCaps",
            "none",
            "none is not one of normal, small-caps",
        ),
        (
            "fontVariant",
            "none small-caps",
            "none is not a keyword of a font-variant",
        ),
        ("fontKerning", "on", "on is not one of normal, auto, none"),
    ] {
        let e = error(&[(key, text(bad))]);
        assert!(e.contains(message), "{bad}: {e}");
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
                ("wordSpacing", text("0.25em")),
                ("whiteSpace", text("pre-line")),
                ("textTransform", text("uppercase")),
                ("wordBreak", text("break-all")),
                ("wordWrap", text("break-word")),
                ("textWrapStyle", text("balance")),
                ("WebkitTextFillColor", text("blue")),
                ("WebkitTextStroke", text("0.1em #0f0")),
                ("fontFeatureSettings", text("'smcp', \"tnum\" 1")),
                ("fontKerning", text("none")),
                ("fontVariant", text("small-caps slashed-zero")),
                ("fontVariationSettings", text("'wdth' 80")),
                ("fontStretch", text("condensed")),
                ("fontStyle", text("oblique 20deg")),
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
    let feature = |tag: &[u8; 4], value| Feature { tag: *tag, value };
    let initial = Font {
        size: 16.0,
        weight: 400.0,
        stretch: 100.0,
        style: FontStyle::Normal,
        family: Rc::new([Family::SansSerif]),
        line_height: LineHeight::Normal,
        align: TextAlign::Start,
        spacing: 0.0,
        word_spacing: 0.0,
        collapse: Collapse::Collapse,
        wrap: true,
        balance: false,
        word_break: WordBreak::Normal,
        overflow_wrap: OverflowWrap::Normal,
        transform: TextTransform::None,
        variants: Default::default(),
        features: Rc::new([]),
        variations: Rc::new([]),
        fill: None,
        stroke: 0.0,
        stroke_color: None,
        shadows: Rc::new([]),
        skip_ink: true,
        underline_offset: None,
    };
    assert_eq!(s[0].font, initial);
    assert_eq!(s[0].color, color::BLACK);
    assert_eq!(s[4].font, initial);

    let brand = Font {
        size: 20.0,
        weight: 700.0,
        stretch: 75.0,
        style: FontStyle::Oblique(20.0),
        family: Rc::new([
            Family::Named("Brand Sans".into()),
            Family::Named("Noto Serif".into()),
            Family::Serif,
        ]),
        line_height: LineHeight::Factor(1.5),
        align: TextAlign::Center,
        spacing: 2.0,
        word_spacing: 5.0,
        collapse: Collapse::PreserveBreaks,
        wrap: true,
        balance: true,
        word_break: WordBreak::BreakAll,
        overflow_wrap: OverflowWrap::BreakWord,
        transform: TextTransform::Uppercase,
        variants: [
            Rc::new([feature(b"kern", 0)]),
            Rc::new([]),
            Rc::new([feature(b"smcp", 1)]),
            Rc::new([feature(b"zero", 1)]),
            Rc::new([]),
            Rc::new([]),
            Rc::new([]),
        ],
        features: Rc::new([
            Feature {
                tag: *b"smcp",
                value: 1,
            },
            Feature {
                tag: *b"tnum",
                value: 1,
            },
        ]),
        variations: Rc::new([Variation {
            tag: *b"wdth",
            value: 80.0,
        }]),
        fill: Some(color::parse("blue").unwrap()),
        stroke: 2.0,
        stroke_color: Some(color::parse("#0f0").unwrap()),
        shadows: Rc::new([]),
        skip_ink: true,
        underline_offset: None,
    };
    assert_eq!(s[1].font, brand);
    assert_eq!(s[1].color, [1.0, 0.0, 0.0, 1.0]);
    // Spacing inherits as px; a factor line height inherits as the factor.
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
fn borders_take_lines_widths_styles_and_colors() {
    let c = |name| color::parse(name).unwrap();
    let border = |style: &[(&str, Prop)]| {
        let s = one(style);
        let b = s.layout.border;
        assert_eq!(
            [b.top, b.right, b.bottom, b.left],
            s.border.map(|w| px(w as f32))
        );
        (s.border, s.border_colors)
    };
    // No line until a style gives one; then `medium`, in `currentcolor`.
    assert_eq!(border(&[]), ([0.0; 4], [color::BLACK; 4]));
    assert_eq!(
        border(&[("color", text("blue")), ("border", text("solid"))]),
        ([3.0; 4], [c("blue"); 4])
    );
    assert_eq!(
        border(&[("border", text("red 2px solid"))]),
        ([2.0; 4], [c("red"); 4])
    );
    assert_eq!(
        border(&[
            ("borderStyle", text("solid none")),
            ("borderWidth", text("thin medium thick")),
            ("borderLeftStyle", text("solid")),
            ("borderLeftWidth", text("2.75px")),
            ("borderColor", text("red green")),
            ("borderBlockEndColor", text("currentcolor")),
        ]),
        (
            [1.0, 0.0, 5.0, 2.0],
            [c("red"), c("green"), color::BLACK, c("green")]
        )
    );
    // Logical sides, and widths between 0 and 1 px.
    assert_eq!(
        border(&[
            ("borderInline", text("0.25px solid blue")),
            ("borderBlockStyle", text("hidden solid")),
            ("borderBlockWidth", num(4.0)),
        ]),
        (
            [0.0, 1.0, 4.0, 1.0],
            [color::BLACK, c("blue"), color::BLACK, c("blue")]
        )
    );
    // Their styles, for paint.
    use BorderStyle::{Dashed, Dotted, Double, Groove, Inset, Outset, Ridge, Solid};
    let styles = |style: &[(&str, Prop)]| (one(style).border_styles, one(style).outline_style);
    assert_eq!(
        styles(&[
            ("borderStyle", text("double solid")),
            ("borderLeftStyle", text("hidden")),
            ("outline", text("double")),
        ]),
        ([Double, Solid, Double, BorderStyle::None], Double)
    );
    assert_eq!(
        styles(&[
            ("borderStyle", text("groove ridge inset outset")),
            ("outlineStyle", text("ridge")),
        ]),
        ([Groove, Ridge, Inset, Outset], Ridge)
    );
    assert_eq!(
        styles(&[
            ("borderStyle", text("dotted dashed")),
            ("outline", text("2px dashed"))
        ]),
        ([Dotted, Dashed, Dotted, Dashed], Dashed)
    );
}

#[test]
fn outlines_take_a_line_and_an_offset() {
    let c = |name| color::parse(name).unwrap();
    let outline = |style: &[(&str, Prop)]| {
        let s = one(style);
        (s.outline, s.outline_color, s.outline_offset)
    };
    assert_eq!(outline(&[]), (0.0, color::BLACK, 0.0));
    assert_eq!(
        outline(&[("color", text("red")), ("outline", text("solid"))]),
        (3.0, c("red"), 0.0)
    );
    assert_eq!(
        outline(&[
            ("outline", text("thick blue solid")),
            ("outlineOffset", text("-0.5em")),
        ]),
        (5.0, c("blue"), -8.0)
    );
    assert_eq!(
        outline(&[
            ("outlineStyle", text("solid")),
            ("outlineWidth", text("2.5px")),
            ("outlineColor", text("green")),
            ("outlineColor", text("currentcolor")),
        ]),
        (2.0, color::BLACK, 0.0)
    );
    let offset = |px: &str| one(&[("outlineOffset", text(px))]).outline_offset;
    assert_eq!([offset("2.5px"), offset("-1.5px")], [2.0, -1.0]);
}

#[test]
fn box_shadows_take_lengths_a_color_and_inset() {
    let c = |name| color::parse(name).unwrap();
    let shadows = |v: &str| one(&[("color", text("blue")), ("boxShadow", text(v))]).shadows;
    let shadow = |offset, blur, spread, color, inset| Shadow {
        offset,
        blur,
        spread,
        color,
        inset,
    };
    assert_eq!(shadows("none"), []);
    assert_eq!(
        shadows("1px 2px"),
        [shadow([1.0, 2.0], 0.0, 0.0, c("blue"), false)]
    );
    assert_eq!(
        shadows("inset -1px 0 3px -2px red, 0.5em 0 GREEN"),
        [
            shadow([-1.0, 0.0], 3.0, -2.0, c("red"), true),
            shadow([8.0, 0.0], 0.0, 0.0, c("green"), false),
        ]
    );
    assert_eq!(
        shadows("rgb(0 0 0 / 50%) 1px 1px calc(1px + 1px) inset"),
        [shadow([1.0, 1.0], 2.0, 0.0, [0.0, 0.0, 0.0, 0.5], true)]
    );
}

#[test]
fn backgrounds_take_gradients_over_a_color() {
    use gradient::{Hue, Image, Line, Shape, Space, Stop};
    let c = |name| color::parse(name).unwrap();
    // Colors, `None` for a hint, at px of a 100 px line.
    let stops = |image: &Image| match image {
        Image::Gradient(g) => (g.stops.iter())
            .map(|s| match s {
                Stop::Color(c, at) => (Some(*c), at.as_ref().map(|a| a.resolve(100.0))),
                Stop::Hint(at) => (None, Some(at.resolve(100.0))),
            })
            .collect(),
        Image::None => Vec::new(),
    };
    let s = one(&[
        ("color", text("blue")),
        (
            "background",
            text("linear-gradient(red, 10px, currentColor 20% 2em), none, Green"),
        ),
    ]);
    assert_eq!(s.background, c("green"));
    assert_eq!(s.layers[1].image, Image::None);
    assert_eq!(
        stops(&s.layers[0].image),
        [
            (Some(c("red")), None),
            (None, Some(10.0)),
            (Some(c("blue")), Some(20.0)),
            (Some(c("blue")), Some(32.0)),
        ]
    );
    let gradient = |v: &str| match &images(&[("backgroundImage", text(v))])[..] {
        [Image::Gradient(g)] => match g.shape {
            Shape::Linear(line) => (line, g.repeating, g.space),
            _ => panic!("{g:?}"),
        },
        other => panic!("{other:?}"),
    };
    assert_eq!(
        gradient("linear-gradient(red, blue)"),
        (Line::Angle(180.0), false, Space::Srgb)
    );
    assert_eq!(
        gradient("linear-gradient(to left, red)").0,
        Line::Angle(270.0)
    );
    assert_eq!(gradient("linear-gradient(0, red)").0, Line::Angle(0.0));
    assert_eq!(
        gradient("repeating-linear-gradient(0.25turn in oklab, red, blue)"),
        (Line::Angle(90.0), true, Space::Oklab)
    );
    assert_eq!(
        gradient("linear-gradient(in oklch longer hue to top left, red, blue)"),
        (Line::Corner([-1.0, -1.0]), false, Space::Oklch(Hue::Longer))
    );
    assert_eq!(
        gradient("linear-gradient(to bottom right in srgb-linear, red, blue)"),
        (Line::Corner([1.0, 1.0]), false, Space::SrgbLinear)
    );
    assert_eq!(
        gradient("linear-gradient(in oklch, red, blue)").2,
        Space::Oklch(Hue::Shorter)
    );
    assert_eq!(images(&[("background", text("none"))]), [Image::None]);
}

#[test]
fn background_layers_take_sizes_positions_and_repeats() {
    use background::{ImageSize, Repeat};
    // Each layer's width and height, and position, in 100 px, and repeat.
    let layers = |style: &[(&str, Prop)]| {
        (one(style).layers.into_iter())
            .map(|l| {
                let size = match l.size {
                    ImageSize::Sized(s) => s.map(|v| v.map(|v| v.resolve(100.0))),
                    other => panic!("{other:?}"),
                };
                let at = l.position.each_ref().map(|o| o.resolve(100.0));
                (size, at, l.repeat)
            })
            .collect::<Vec<_>>()
    };
    let three = text("linear-gradient(red), linear-gradient(blue), linear-gradient(green)");
    let no = [Repeat::NoRepeat; 2];
    // Lists repeat for the images.
    assert_eq!(
        layers(&[
            ("backgroundImage", three.clone()),
            ("backgroundSize", text("10px, 50% auto")),
            ("backgroundPosition", text("right 10px top, center")),
            ("backgroundRepeat", text("no-repeat")),
        ]),
        [
            ([Some(10.0), None], [90.0, 0.0], no),
            ([Some(50.0), None], [50.0, 50.0], no),
            ([Some(10.0), None], [90.0, 0.0], no),
        ]
    );
    // x and y are lists of their own.
    assert_eq!(
        layers(&[
            ("backgroundImage", three),
            ("backgroundPosition", text("left 10px top 20px")),
            ("backgroundPositionX", text("right 5px, 30%")),
        ])
        .into_iter()
        .map(|l| l.1)
        .collect::<Vec<_>>(),
        [[95.0, 20.0], [30.0, 20.0], [95.0, 20.0]]
    );
    // The shorthand sets each, and what it leaves out goes back to its initial value. The color
    // goes with a last layer of its own.
    let background = text(
        "linear-gradient(red) center / 20px 30% repeat-x, \
         linear-gradient(blue) space round bottom 5px right 10%, white",
    );
    let s = one(&[
        ("backgroundSize", text("10px")),
        ("background", background.clone()),
    ]);
    assert_eq!(s.background, [1.0; 4]);
    assert_eq!(s.layers[1].size, ImageSize::Sized([None, None]));
    assert_eq!(
        layers(&[("background", background)]),
        [
            (
                [Some(20.0), Some(30.0)],
                [50.0, 50.0],
                [Repeat::Repeat, Repeat::NoRepeat]
            ),
            ([None, None], [90.0, 95.0], [Repeat::Space, Repeat::Round]),
            ([None, None], [0.0, 0.0], [Repeat::Repeat; 2]),
        ]
    );
    let sizes = one(&[
        ("backgroundImage", text("none, none")),
        ("backgroundSize", text("cover, contain")),
    ]);
    assert_eq!(
        sizes.layers.iter().map(|l| &l.size).collect::<Vec<_>>(),
        [&ImageSize::Cover, &ImageSize::Contain]
    );
}

#[test]
fn background_layers_take_origins_and_clips() {
    use background::Area::{BorderBox, ContentBox, PaddingBox, Text};
    let areas = |style: &[(&str, Prop)]| {
        (one(style).layers.iter())
            .map(|l| (l.origin, l.clip))
            .collect::<Vec<_>>()
    };
    // A layer with no image by default.
    assert_eq!(areas(&[]), [(PaddingBox, BorderBox)]);
    assert_eq!(
        areas(&[
            ("backgroundImage", text("none, none, none")),
            ("backgroundOrigin", text("content-box, border-box")),
            ("backgroundClip", text("padding-box")),
        ]),
        [
            (ContentBox, PaddingBox),
            (BorderBox, PaddingBox),
            (ContentBox, PaddingBox)
        ]
    );
    // In the shorthand one box is both, and of two the first is the origin.
    assert_eq!(
        areas(&[(
            "background",
            text("none content-box, border-box none padding-box, red")
        )]),
        [
            (ContentBox, ContentBox),
            (BorderBox, PaddingBox),
            (PaddingBox, BorderBox)
        ]
    );
    // Text is a clip alone, so a box before it is the origin.
    assert_eq!(
        areas(&[("background", text("none text, none content-box text, red"))]),
        [
            (PaddingBox, Text),
            (ContentBox, Text),
            (PaddingBox, BorderBox)
        ]
    );
    assert_eq!(
        areas(&[("backgroundClip", text("text"))]),
        [(PaddingBox, Text)]
    );
}

#[test]
fn filters_take_css_functions_in_order() {
    use filter::Function as F;
    let s = one(&[
        ("color", text("blue")),
        (
            "filter",
            text(
                "blur(0.5em) brightness(150%) GRAYSCALE(2) hue-rotate(0.5turn) \
                 drop-shadow(red 1px 2px 3px) drop-shadow(0 1px) opacity()",
            ),
        ),
    ]);
    assert_eq!(
        s.filter,
        [
            F::Blur(8.0),
            F::Brightness(1.5),
            F::Grayscale(1.0),
            F::HueRotate(180.0),
            F::DropShadow {
                offset: [1.0, 2.0],
                blur: 3.0,
                color: [1.0, 0.0, 0.0, 1.0]
            },
            F::DropShadow {
                offset: [0.0, 1.0],
                blur: 0.0,
                color: [0.0, 0.0, 1.0, 1.0]
            },
            F::Opacity(1.0),
        ]
    );
    assert!(s.contains_all());
    assert!(one(&[("filter", text("none"))]).filter.is_empty());
}

#[test]
fn filters_take_valles_functions_in_px_within_their_ranges() {
    use filter::Function as F;
    let filter = |v: &str| one(&[("filter", text(v))]).filter;
    assert_eq!(
        filter("chromatic-aberration(-4.5px) blur(1px) chromatic-aberration(0)"),
        [
            F::ChromaticAberration(-4.5),
            F::Blur(1.0),
            F::ChromaticAberration(0.0)
        ]
    );
    let backdrop = one(&[("backdropFilter", text("chromatic-aberration(256px)"))]);
    assert_eq!(backdrop.backdrop, [F::ChromaticAberration(256.0)]);
    for (v, says) in [
        (
            "chromatic-aberration(300px)",
            "the offset 300px is not within -256px and 256px",
        ),
        ("chromatic-aberration(1em)", "the offset 1em is not px"),
        ("chromatic-aberration(4)", "the offset 4 is not px"),
        ("chromatic-aberration()", "takes the offset"),
        ("chromatic-aberration(1px 2px)", "takes the offset"),
    ] {
        let e = error(&[("filter", text(v))]);
        assert!(e.contains(says), "{v}: {e}");
    }
}

#[test]
fn clip_paths_take_basic_shapes_in_a_box() {
    use background::Area;
    use shape::{Clip, Radius, Shape};
    let clip = |v: &str| one(&[("clipPath", text(v))]).clip;
    let px = |v: f64| Math::Sum([v, 0.0]);
    let percent = |v: f64| Math::Sum([0.0, v / 100.0]);
    let center = || position::Offset {
        end: false,
        by: percent(50.0),
    };
    assert_eq!(clip("none"), None);
    assert_eq!(
        clip("Padding-Box"),
        Some(Clip {
            shape: None,
            area: Area::PaddingBox
        })
    );
    let Some(Clip {
        shape: Some(Shape::Inset { sides, radii }),
        area: Area::BorderBox,
    }) = clip("inset(1px 10% round 2px 50% / 1em)")
    else {
        panic!()
    };
    assert_eq!(sides, [px(1.0), percent(10.0), px(1.0), percent(10.0)]);
    assert_eq!(radii[0], [px(2.0), px(16.0)]);
    assert_eq!(radii[1], [percent(50.0), px(16.0)]);
    assert_eq!(
        clip("content-box CIRCLE(at right 5px top 0)"),
        Some(Clip {
            shape: Some(Shape::Circle {
                radius: Radius::ClosestSide,
                at: [
                    position::Offset {
                        end: true,
                        by: px(5.0)
                    },
                    position::Offset {
                        end: false,
                        by: percent(0.0)
                    }
                ],
            }),
            area: Area::ContentBox,
        })
    );
    assert_eq!(
        clip("ellipse(farthest-side 20%)").unwrap().shape,
        Some(Shape::Ellipse {
            radii: [Radius::FarthestSide, Radius::Len(percent(20.0))],
            at: [center(), center()],
        })
    );
    assert_eq!(
        clip("polygon(EVENODD, 0 0, 100% 0, 50% calc(100% - 2px))")
            .unwrap()
            .shape,
        Some(Shape::Polygon {
            even_odd: true,
            points: vec![
                [px(0.0), px(0.0)],
                [percent(100.0), px(0.0)],
                [percent(50.0), Math::Sum([-2.0, 1.0])],
            ],
        })
    );
    // Path data keeps its case: a lower case command is relative.
    assert_eq!(
        clip("path(\"M1 2 l3 0 Z\") padding-box"),
        Some(Clip {
            shape: Some(Shape::Path {
                even_odd: false,
                path: crate::path::parse("M1 2 L4 2 Z").unwrap(),
            }),
            area: Area::PaddingBox,
        })
    );
    let s = one(&[("clipPath", text("circle()"))]);
    assert!(!s.contains_all());
    let flat = one(&[
        ("transformStyle", text("preserve-3d")),
        ("clipPath", text("circle()")),
    ]);
    assert!(!flat.keeps_3d());
}

#[test]
fn masks_are_layers_of_images_shown_by_alpha_or_luminance() {
    use background::{Area, ImageSize, Repeat};
    assert!(one(&[]).masks.is_empty());
    assert!(one(&[("maskImage", text("none, none"))]).masks.is_empty());
    let s = one(&[
        ("maskImage", text("linear-gradient(#000, #0000), none")),
        ("maskSize", text("10px auto")),
        ("maskPosition", text("right 4px bottom 2px")),
        ("maskRepeat", text("no-repeat")),
        ("maskMode", text("LUMINANCE, match-source")),
    ]);
    let [a, b] = &s.masks[..] else {
        panic!("{:?}", s.masks)
    };
    assert!(a.luminance && !b.luminance);
    assert!(matches!(b.image, gradient::Image::None));
    // Placed in the border box, where backgrounds are placed in the padding box.
    assert_eq!(a.origin, Area::BorderBox);
    assert_eq!(
        one(&[("backgroundImage", text("linear-gradient(red, blue)"))]).layers[0].origin,
        Area::PaddingBox
    );
    assert_eq!(
        a.size,
        ImageSize::Sized([Some(Math::Sum([10.0, 0.0])), None])
    );
    let from_end = |px| position::Offset {
        end: true,
        by: Math::Sum([px, 0.0]),
    };
    assert_eq!(b.position, [from_end(4.0), from_end(2.0)]);
    assert_eq!(b.repeat, [Repeat::NoRepeat; 2]);
    assert!(!s.contains_all());
    let flat = one(&[
        ("transformStyle", text("preserve-3d")),
        ("maskImage", text("linear-gradient(#000, #0000)")),
    ]);
    assert!(!flat.keeps_3d());
}

#[test]
fn background_layers_take_blend_modes_which_the_shorthand_leaves() {
    let blends = |style: &[(&str, Prop)]| {
        (one(style).layers.iter())
            .map(|l| l.blend)
            .collect::<Vec<_>>()
    };
    assert_eq!(blends(&[]), [Blend::Normal]);
    let style = [
        ("backgroundBlendMode", text("multiply, Screen")),
        ("background", text("none, none, none, red")),
    ];
    assert_eq!(
        blends(&style),
        [
            Blend::Multiply,
            Blend::Screen,
            Blend::Multiply,
            Blend::Screen
        ]
    );
}

/// The transform functions of a style, translations resolved against a 100 px box.
fn functions(style: &[(&str, Prop)]) -> Vec<transform::Function<f32>> {
    use transform::Function as F;
    (one(style).transform.into_iter())
        .map(|f| match f {
            F::Translate(t) => F::Translate(t.map(|l| l.resolve(100.0))),
            F::Scale(s) => F::Scale(s),
            F::Rotate(a) => F::Rotate(a),
            F::Rotate3d(axis, a) => F::Rotate3d(axis, a),
            F::Skew(a) => F::Skew(a),
            F::Matrix(m) => F::Matrix(m),
            F::Matrix3d(m) => F::Matrix3d(m),
            F::Perspective(d) => F::Perspective(d.map(|l| l.resolve(0.0))),
        })
        .collect()
}

#[test]
fn transforms_take_2d_functions_around_an_origin() {
    use transform::Function as F;
    let functions = |v: &str| functions(&[("transform", text(v))]);
    assert_eq!(functions("none"), []);
    assert_eq!(
        functions("translate(10px, 50%) translateX(1em) translateY(-2px) translate(5%)"),
        [
            F::Translate([10.0, 50.0, 0.0]),
            F::Translate([16.0, 0.0, 0.0]),
            F::Translate([0.0, -2.0, 0.0]),
            F::Translate([5.0, 0.0, 0.0]),
        ]
    );
    assert_eq!(
        functions("scale(2) scale(50%, 3) scaleX(-1) scaleY(0.5)"),
        [
            F::Scale([2.0, 2.0, 1.0]),
            F::Scale([0.5, 3.0, 1.0]),
            F::Scale([-1.0, 1.0, 1.0]),
            F::Scale([1.0, 0.5, 1.0]),
        ]
    );
    assert_eq!(
        functions("rotate(0.25turn) rotateZ(-10deg) rotate(0) skew(10deg) skewY(100grad)"),
        [
            F::Rotate(90.0),
            F::Rotate(-10.0),
            F::Rotate(0.0),
            F::Skew([10.0, 0.0]),
            F::Skew([0.0, 90.0]),
        ]
    );
    assert_eq!(
        functions(" MATRIX(1, 0, 0.5, 1, 10, -5) "),
        [F::Matrix([1.0, 0.0, 0.5, 1.0, 10.0, -5.0])]
    );
    // From the border box's top left, the center by default.
    let origin = |v: Option<Prop>| {
        let style: Vec<_> = v.into_iter().map(|v| ("transformOrigin", v)).collect();
        one(&style).transform_origin.map(|o| o.resolve(100.0))
    };
    assert_eq!(origin(None), [50.0, 50.0]);
    assert_eq!(origin(Some(text("left"))), [0.0, 50.0]);
    assert_eq!(origin(Some(text("bottom right"))), [100.0, 100.0]);
    assert_eq!(origin(Some(text("10px 20%"))), [10.0, 20.0]);
    assert_eq!(origin(Some(num(5.0))), [5.0, 50.0]);
}

#[test]
fn motion_paths_take_paths_rays_and_shapes_with_their_longhands() {
    use background::Area;
    use offset::{Route, Size, Start};
    let motion = |style: &[(&str, Prop)]| one(style).motion;
    assert_eq!(motion(&[]), None);
    assert_eq!(motion(&[("offsetDistance", text("10px"))]), None);
    assert_eq!(motion(&[("offsetPath", text("none"))]), None);
    // Path data as written; its longhands as they start: auto, no angle, the origin, normal.
    let m = motion(&[("offsetPath", text("path('M0 0 L10 0 Z')"))]).unwrap();
    assert_eq!(
        m.path,
        Route::Path(crate::path::parse("M0 0 L10 0 Z").unwrap())
    );
    assert_eq!(
        (m.area, m.along, m.angle, &m.anchor, &m.position),
        (Area::BorderBox, true, 0.0, &None, &Start::Normal)
    );
    assert_eq!(m.distance.resolve(100.0), 0.0);
    let m = motion(&[
        (
            "offsetPath",
            text("Ray(45DEG sides at 10px 20%) padding-box"),
        ),
        ("offsetDistance", text("calc(50% + 1em)")),
        ("offsetRotate", text("30deg reverse")),
        ("offsetAnchor", text("left top")),
        ("offsetPosition", text("auto")),
    ])
    .unwrap();
    let Route::Ray { angle, size, at } = &m.path else {
        panic!("{:?} is not a ray", m.path);
    };
    let at = at.as_ref().map(|a| a.each_ref().map(|o| o.resolve(100.0)));
    assert_eq!((*angle, *size, at), (45.0, Size::Sides, Some([10.0, 20.0])));
    assert_eq!(m.area, Area::PaddingBox);
    assert_eq!(m.distance.resolve(100.0), 66.0);
    assert_eq!((m.along, m.angle), (true, 210.0));
    let anchor = m.anchor.map(|a| a.map(|o| o.resolve(100.0)));
    assert_eq!((anchor, m.position), (Some([0.0, 0.0]), Start::Auto));
    let m = motion(&[
        ("offsetPath", text("ray(0deg)")),
        ("offsetRotate", text("-10deg")),
    ]);
    assert_eq!(m.map(|m| (m.along, m.angle)), Some((false, -10.0)));
    // A circle or an ellipse with no `at` is around the offset position; a box alone is a path.
    let shape = |v: &str| match motion(&[("offsetPath", text(v))]).unwrap().path {
        Route::Shape { shape, at } => (shape.is_some(), at),
        other => panic!("{other:?} is not a shape"),
    };
    assert_eq!(shape("circle(10px)"), (true, false));
    assert_eq!(shape("ellipse(1px 2px at center)"), (true, true));
    assert_eq!(shape("inset(1px round 2px)"), (true, true));
    assert_eq!(shape("content-box"), (false, true));
    let m = motion(&[("offsetPath", text("content-box polygon(0 0, 1px 0)"))]);
    assert_eq!(m.map(|m| m.area), Some(Area::ContentBox));
    // It is a transform, so it contains all.
    assert!(one(&[("offsetPath", text("ray(0deg)"))]).contains_all());
    for (property, v, says) in [
        (
            "offsetPath",
            "ray(0deg contain)",
            "contain is not supported",
        ),
        (
            "offsetPath",
            "ray(0deg 10px)",
            "10px is not an angle or a size",
        ),
        ("offsetPath", "ray(closest-side)", "takes an angle"),
        ("offsetPath", "path('M0 0') content-box", "takes no box"),
        ("offsetPath", "url(#a)", "url() is not an offset path"),
        ("offsetPath", "circle() ray(0deg)", "takes one path"),
        ("offsetRotate", "auto reverse", "is not an offset rotate"),
        ("offsetAnchor", "normal", "is not an offset anchor"),
        (
            "offsetPosition",
            "left top 10px",
            "is not an offset position",
        ),
        ("offsetDistance", "auto", "auto is not a length"),
    ] {
        let e = error(&[(property, text(v))]);
        assert!(e.contains(says), "{property}: {v}: {e}");
    }
}

#[test]
fn transforms_take_3d_functions_and_an_origin_z() {
    use transform::Function as F;
    assert_eq!(
        functions(&[(
            "transform",
            text("translateZ(2em) translate3d(10%, 1px, -5px) scaleZ(2) scale3d(1, 50%, 3)")
        )]),
        [
            F::Translate([0.0, 0.0, 32.0]),
            F::Translate([10.0, 1.0, -5.0]),
            F::Scale([1.0, 1.0, 2.0]),
            F::Scale([1.0, 0.5, 3.0]),
        ]
    );
    assert_eq!(
        functions(&[(
            "transform",
            text(
                "rotateX(90deg) rotateY(-0.5turn) rotate3d(1, 1, 0, 0) perspective(0) perspective(none)"
            )
        )]),
        [
            F::Rotate3d([1.0, 0.0, 0.0], 90.0),
            F::Rotate3d([0.0, 1.0, 0.0], -180.0),
            F::Rotate3d([1.0, 1.0, 0.0], 0.0),
            F::Perspective(Some(0.0)),
            F::Perspective(None),
        ]
    );
    let identity = "1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 5, 6, 7, 1";
    assert_eq!(
        functions(&[("transform", text(&format!("matrix3d({identity})")))]),
        [F::Matrix3d([
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, 6.0, 7.0, 1.0
        ])]
    );
    // Each property's z and axes.
    assert_eq!(
        functions(&[
            ("translate", text("1px 2px 3px")),
            ("rotate", text("10deg y")),
            ("scale", text("1 2 3")),
        ]),
        [
            F::Translate([1.0, 2.0, 3.0]),
            F::Rotate3d([0.0, 1.0, 0.0], 10.0),
            F::Scale([1.0, 2.0, 3.0]),
        ]
    );
    assert_eq!(
        functions(&[("rotate", text("1 0 0 10deg"))]),
        [F::Rotate3d([1.0, 0.0, 0.0], 10.0)]
    );
    assert_eq!(
        functions(&[("rotate", text("20deg 0 0 1"))]),
        [F::Rotate(20.0)]
    );
    let z = |v: &str| one(&[("transformOrigin", text(v))]).transform_origin_z;
    assert_eq!(z("left"), 0.0);
    assert_eq!(z("left top 10px"), 10.0);
    assert_eq!(z("10px 20% -1em"), -16.0);
    let seen = |style: &[(&str, Prop)]| {
        let s = one(style);
        (
            s.perspective,
            s.perspective_origin.map(|o| o.resolve(100.0)),
        )
    };
    assert_eq!(seen(&[]), (None, [50.0, 50.0]));
    assert_eq!(
        seen(&[
            ("perspective", num(200.0)),
            ("perspectiveOrigin", text("left 10px bottom 5%"))
        ]),
        (Some(200.0), [10.0, 95.0])
    );
    assert_eq!(seen(&[("perspective", text("0"))]).0, Some(0.0));
    assert!(one(&[]).backface_visible);
    assert!(!one(&[("backfaceVisibility", text("Hidden"))]).backface_visible);
    let keeps = |style: &[(&str, Prop)]| one(style).keeps_3d();
    assert!(!keeps(&[]));
    assert!(keeps(&[("transformStyle", text("preserve-3d"))]));
    let grouped = [
        ("transformStyle", text("preserve-3d")),
        ("overflow", text("hidden")),
    ];
    assert!(!keeps(&grouped));
}

#[test]
fn translate_rotate_and_scale_come_before_transform() {
    use transform::Function as F;
    assert_eq!(
        functions(&[
            ("transform", text("skew(10deg)")),
            ("scale", text("2 50%")),
            ("rotate", text("Z 90DEG")),
            ("translate", text("10px 5%")),
        ]),
        [
            F::Translate([10.0, 5.0, 0.0]),
            F::Rotate(90.0),
            F::Scale([2.0, 0.5, 1.0]),
            F::Skew([10.0, 0.0]),
        ]
    );
    assert_eq!(
        functions(&[
            ("translate", num(4.0)),
            ("rotate", text("-0.25turn z")),
            ("scale", num(3.0))
        ]),
        [
            F::Translate([4.0, 0.0, 0.0]),
            F::Rotate(-90.0),
            F::Scale([3.0, 3.0, 1.0]),
        ]
    );
    let none = [
        ("translate", text("none")),
        ("rotate", text("none")),
        ("scale", text("none")),
    ];
    assert_eq!(functions(&none), []);
}

#[test]
fn radial_gradients_take_a_shape_a_size_and_a_center() {
    use gradient::{Image, Shape, Size, Space};
    // The shape, the size in px of 100 px, and the center from the top left of 100 × 100 px.
    let radial = |v: &str| match &images(&[("backgroundImage", text(v))])[..] {
        [Image::Gradient(g)] => match &g.shape {
            Shape::Radial { circle, size, at } => {
                let size = match size {
                    Size::Radii(r) => Some(r.each_ref().map(|r| r.resolve(100.0))),
                    _ => None,
                };
                let at = at.each_ref().map(|o| o.resolve(100.0));
                (*circle, size, at, g.repeating, g.space)
            }
            _ => panic!("{g:?}"),
        },
        other => panic!("{other:?}"),
    };
    let size = |v: &str| match &images(&[("backgroundImage", text(v))])[..] {
        [Image::Gradient(g)] => match &g.shape {
            Shape::Radial { size, .. } => size.clone(),
            _ => panic!("{g:?}"),
        },
        other => panic!("{other:?}"),
    };
    assert_eq!(
        radial("radial-gradient(red, blue)"),
        (false, None, [50.0, 50.0], false, Space::Srgb)
    );
    assert_eq!(size("radial-gradient(red, blue)"), Size::FarthestCorner);
    assert_eq!(
        size("radial-gradient(closest-side circle, red)"),
        Size::ClosestSide
    );
    assert_eq!(
        radial("radial-gradient(20px, red)"),
        (true, Some([20.0, 20.0]), [50.0, 50.0], false, Space::Srgb)
    );
    assert_eq!(
        radial("repeating-radial-gradient(10px 20% at left 5px bottom 10%, red)"),
        (false, Some([10.0, 20.0]), [5.0, 90.0], true, Space::Srgb)
    );
    assert_eq!(
        radial("radial-gradient(in oklab circle closest-corner at top, red)"),
        (true, None, [50.0, 0.0], false, Space::Oklab)
    );
    assert_eq!(
        radial("radial-gradient(circle at 25% calc(10px + 5%) in oklab, red)").2,
        [25.0, 15.0]
    );
    assert_eq!(
        radial("radial-gradient(at right center, red)").2,
        [100.0, 50.0]
    );
    assert_eq!(radial("radial-gradient(at 0, red)").2, [0.0, 50.0]);
}

#[test]
fn conic_gradients_take_an_angle_a_center_and_angles() {
    use gradient::{Image, Shape, Space, Stop};
    // From, the center in 100 × 100 px, and the stops in degrees.
    let conic = |v: &str| match &images(&[("backgroundImage", text(v))])[..] {
        [Image::Gradient(g)] => match &g.shape {
            Shape::Conic { from, at } => {
                let stops: Vec<Option<f32>> = (g.stops.iter())
                    .map(|s| match s {
                        Stop::Color(_, at) => at.as_ref().map(|a| a.resolve(360.0)),
                        Stop::Hint(at) => Some(at.resolve(360.0)),
                    })
                    .collect();
                let at = at.each_ref().map(|o| o.resolve(100.0));
                (*from, at, stops, g.repeating, g.space)
            }
            _ => panic!("{g:?}"),
        },
        other => panic!("{other:?}"),
    };
    assert_eq!(
        conic("conic-gradient(red, blue)"),
        (0.0, [50.0, 50.0], vec![None, None], false, Space::Srgb)
    );
    assert_eq!(
        conic("conic-gradient(from 90deg at 25% 0, red 10%, 25%, blue 0.25turn 180deg, green)"),
        (
            90.0,
            [25.0, 0.0],
            vec![Some(36.0), Some(90.0), Some(90.0), Some(180.0), None],
            false,
            Space::Srgb
        )
    );
    assert_eq!(
        conic("repeating-conic-gradient(in oklab from 0.5turn, red 0 10deg)"),
        (
            180.0,
            [50.0, 50.0],
            vec![Some(0.0), Some(10.0)],
            true,
            Space::Oklab
        )
    );
    assert_eq!(
        conic("conic-gradient(at left in oklab, red)").1,
        [0.0, 50.0]
    );
}

#[test]
fn corner_radii_expand_as_sides_do() {
    // Each corner's radii, percentages of 100 px.
    let radii = |style: &[(&str, Prop)]| one(style).radii.map(|c| c.map(|r| r.resolve(100.0)));
    assert_eq!(
        radii(&[("borderRadius", text("1px 2px 3px / 4px 5%"))]),
        [[1.0, 4.0], [2.0, 5.0], [3.0, 4.0], [2.0, 5.0]]
    );
    assert_eq!(radii(&[("borderRadius", num(6.0))]), [[6.0; 2]; 4]);
    assert_eq!(
        radii(&[
            ("fontSize", num(20.0)),
            ("borderRadius", text("8px")),
            ("borderTopLeftRadius", text("0.5em")),
            ("borderStartEndRadius", text("calc(10% + 1px)")),
            ("borderEndEndRadius", text("max(-5%, 2px) 3px")),
            ("borderEndStartRadius", text("2px 3px")),
        ]),
        [[10.0, 10.0], [11.0, 11.0], [2.0, 3.0], [2.0, 3.0]]
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
    assert_eq!(s.radii[2][1].resolve(0.0), 10.0);
    assert_eq!(s.opacity, 0.5);
    assert_eq!(
        one(&[("background", text("none"))]).background,
        color::TRANSPARENT
    );
    assert_eq!(one(&[("background", text("white"))]).background, [1.0; 4]);
    assert_eq!(one(&[("opacity", num(2.0))]).opacity, 1.0);
}

#[test]
fn blends_and_isolation_group_what_is_in_a_3d_box() {
    assert_eq!(one(&[]).blend, Blend::Normal);
    let s = one(&[("mixBlendMode", text(" Color-Dodge "))]);
    assert_eq!(s.blend, Blend::ColorDodge);
    assert_eq!(
        one(&[("mixBlendMode", text("plus-lighter"))]).blend,
        Blend::PlusLighter
    );
    assert!(!one(&[]).isolate);
    assert!(one(&[("isolation", text("isolate"))]).isolate);
    let keeps = |style: &[(&str, Prop)]| one(style).keeps_3d();
    let flat = [
        ("transformStyle", text("preserve-3d")),
        ("mixBlendMode", text("screen")),
    ];
    assert!(!keeps(&flat));
    let flat = [
        ("transformStyle", text("preserve-3d")),
        ("isolation", text("isolate")),
    ];
    assert!(!keeps(&flat));
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
            "textIndent",
            text("1em"),
            "style textIndent is not supported",
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
            "in hsl is not supported yet; use srgb, srgb-linear, oklab or oklch",
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
            "math functions of angles are not supported yet",
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
            text("rotate(calc(10deg + 5deg))"),
            "rotate(): math functions of angles are not supported yet",
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
            text("bloom(1)"),
            "bloom() is not a filter function: blur, brightness, contrast, drop-shadow, \
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
            "url() images are not supported yet",
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
