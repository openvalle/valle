use super::*;
use crate::layout::layout;
use crate::style::{Sheet, color};
use crate::template::Node;
use crate::testing::{text, text_node, view};

/// The display list on a 200 × 100 canvas, and the layout it paints.
fn paint_of(nodes: Vec<Node>, roots: &[u32]) -> (Vec<Item>, Layout) {
    let template = Template {
        nodes,
        roots: roots.to_vec(),
        ..Default::default()
    };
    let canvas = [200.0, 100.0];
    let styles = Sheet::new(&template).unwrap().compute(&[], canvas).unwrap();
    let laid = layout(&template, &[], &styles, canvas, &mut text()).unwrap();
    (paint(&template, &styles, &laid), laid)
}

/// Items by the named color they are painted in, with any opacity folded into it: `red`,
/// `red@0.5`, `text blue`, `group 0.5 [...]`.
fn names(items: &[Item]) -> Vec<String> {
    let name = |c: [f32; 4]| {
        for n in ["red", "green", "blue", "gray", "black"] {
            let w = work(color::parse(n).unwrap());
            if w.map(|v| v * c[3]) == c {
                return match c[3] {
                    1.0 => n.to_string(),
                    a => format!("{n}@{a}"),
                };
            }
        }
        panic!("no name for {c:?}")
    };
    items
        .iter()
        .map(|item| match item {
            Item::Rect { color, .. } => name(*color),
            Item::Ring { colors, .. } => format!("ring {}", name(colors[0])),
            Item::Glyphs { color, .. } => format!("text {}", name(*color)),
            Item::Group { opacity, items } => format!("group {opacity} {:?}", names(items)),
            Item::Clip { items, .. } => format!("clip {:?}", names(items)),
            Item::Shadow { inside, items, .. } => {
                let side = if *inside { "inner" } else { "outer" };
                format!("{side} shadow {:?}", names(items))
            }
        })
        .collect()
}

fn painted(nodes: Vec<Node>, roots: &[u32]) -> Vec<String> {
    names(&paint_of(nodes, roots).0)
}

#[test]
fn blocks_paint_backgrounds_then_text_then_positioned_boxes() {
    let order = painted(
        vec![
            view(&[("background", "red")], &[1, 2, 3]),
            text_node("Hi", &[("background", "green"), ("color", "black")]),
            view(
                &[
                    ("position", "relative"),
                    ("height", "10px"),
                    ("background", "blue"),
                ],
                &[],
            ),
            view(&[("height", "10px"), ("background", "gray")], &[]),
        ],
        &[0],
    );
    // The positioned box comes after the text of the block before it, and after the block
    // after it.
    assert_eq!(order, ["red", "green", "gray", "text black", "blue"]);
}

#[test]
fn flex_and_grid_items_paint_all_at_once() {
    let children = |display| {
        painted(
            vec![
                view(&[("display", display), ("background", "red")], &[1, 2]),
                text_node("Hi", &[("background", "green"), ("color", "black")]),
                view(
                    &[
                        ("width", "10px"),
                        ("height", "10px"),
                        ("background", "blue"),
                    ],
                    &[],
                ),
            ],
            &[0],
        )
    };
    assert_eq!(children("flex"), ["red", "green", "text black", "blue"]);
    assert_eq!(children("grid"), ["red", "green", "text black", "blue"]);
    assert_eq!(children("block"), ["red", "green", "blue", "text black"]);
}

#[test]
fn flex_items_paint_in_order() {
    let item = |color, order| {
        let style = [("width", "10px"), ("background", color), ("order", order)];
        view(&style, &[])
    };
    let order = painted(
        vec![
            view(&[("display", "flex"), ("height", "10px")], &[1, 2, 3]),
            item("red", "1"),
            item("green", "0"),
            view(
                &[
                    ("position", "absolute"),
                    ("width", "10px"),
                    ("height", "10px"),
                    ("order", "-1"),
                    ("background", "blue"),
                ],
                &[],
            ),
        ],
        &[0],
    );
    // The absolute box paints later, with the positioned boxes.
    assert_eq!(order, ["green", "red", "blue"]);
}

#[test]
fn opacity_makes_a_layer_painted_with_the_positioned_boxes() {
    let half = [("opacity", "0.5"), ("height", "10px")];
    let order = painted(
        vec![
            // A layer of one rectangle is the rectangle.
            view(&[("background", "red"), half[0], half[1]], &[]),
            view(&[("background", "green"), half[0], half[1]], &[2]),
            view(&[("background", "blue"), ("height", "5px")], &[]),
            // Nested layers multiply.
            view(&[half[0]], &[4]),
            view(&[half[0]], &[5, 6]),
            view(&[("height", "5px"), ("background", "gray")], &[]),
            view(&[("height", "5px"), ("background", "black")], &[]),
            // Nothing shows through no opacity, or without a background.
            view(&[("opacity", "0"), ("background", "red"), half[1]], &[]),
            view(&[("height", "10px")], &[]),
        ],
        &[0, 1, 3, 7, 8],
    );
    assert_eq!(
        order,
        [
            "red@0.5",
            r#"group 0.5 ["green", "blue"]"#,
            r#"group 0.25 ["gray", "black"]"#,
        ]
    );
    // A layer paints after the in-flow content around it.
    let order = painted(
        vec![
            view(&[("background", "red"), half[0], half[1]], &[]),
            view(&[("background", "green"), ("height", "10px")], &[]),
        ],
        &[0, 1],
    );
    assert_eq!(order, ["green", "red@0.5"]);
}

#[test]
fn positioned_boxes_leave_their_positioned_descendants_to_the_context() {
    let order = painted(
        vec![
            view(&[("position", "relative"), ("background", "red")], &[1, 2]),
            view(
                &[
                    ("position", "absolute"),
                    ("width", "10px"),
                    ("height", "10px"),
                    ("background", "green"),
                ],
                &[],
            ),
            view(&[("height", "10px"), ("background", "blue")], &[]),
            view(&[("height", "10px"), ("background", "gray")], &[]),
        ],
        &[0, 3],
    );
    assert_eq!(order, ["gray", "red", "blue", "green"]);
}

#[test]
fn z_index_orders_stacking_contexts() {
    let at = |z: &str, background| {
        let style = [
            ("position", "relative"),
            ("zIndex", z),
            ("height", "5px"),
            ("background", background),
        ];
        view(&style, &[])
    };
    let order = painted(
        vec![
            view(&[("height", "10px"), ("background", "red")], &[1, 2, 3, 4]),
            at("2", "green"),
            at("-1", "blue"),
            at("auto", "gray"),
            at("1", "black"),
        ],
        &[0],
    );
    // Below the in-flow content, then in tree order at 0, then up.
    assert_eq!(order, ["blue", "red", "gray", "black", "green"]);

    // A flex item with a z-index is a stacking context, painted after the in-flow content.
    let item = |z: &[(&'static str, &'static str)], background| {
        let mut style = vec![("width", "10px"), ("background", background)];
        style.extend_from_slice(z);
        view(&style, &[])
    };
    let flex = |z| {
        painted(
            vec![
                view(&[("display", "flex"), ("height", "10px")], &[1, 2]),
                item(z, "green"),
                item(&[], "red"),
            ],
            &[0],
        )
    };
    assert_eq!(flex(&[("zIndex", "0")]), ["red", "green"]);
    assert_eq!(flex(&[]), ["green", "red"]);

    // A fixed box is a stacking context: what it has below 0 stays in it.
    let order = painted(
        vec![
            view(
                &[
                    ("position", "fixed"),
                    ("width", "10px"),
                    ("height", "10px"),
                    ("background", "red"),
                ],
                &[1],
            ),
            at("-1", "blue"),
            view(&[("height", "10px"), ("background", "green")], &[]),
        ],
        &[0, 2],
    );
    assert_eq!(order, ["green", "red", "blue"]);
}

#[test]
fn hidden_boxes_paint_only_what_is_visible_in_them() {
    let order = painted(
        vec![
            view(
                &[("visibility", "hidden"), ("background", "red")],
                &[1, 2, 3],
            ),
            view(&[("height", "5px"), ("background", "green")], &[]),
            text_node("Hi", &[("color", "black")]),
            view(
                &[
                    ("visibility", "visible"),
                    ("height", "5px"),
                    ("background", "blue"),
                ],
                &[],
            ),
        ],
        &[0],
    );
    assert_eq!(order, ["blue"]);
}

#[test]
fn overflow_clips_what_the_box_contains() {
    let hidden = [("overflow", "hidden"), ("height", "20px")];
    let at = |position, top: &str, background| {
        let style = [
            ("position", position),
            ("top", top),
            ("width", "10px"),
            ("height", "10px"),
            ("background", background),
        ];
        view(&style, &[])
    };
    let (items, _) = paint_of(
        vec![
            view(&[hidden[0], hidden[1], ("background", "red")], &[1, 2, 3]),
            view(&[("height", "30px"), ("background", "green")], &[]),
            // Its containing block is the canvas, outside the clip.
            at("absolute", "30px", "blue"),
            at("relative", "-15px", "gray"),
            view(&[hidden[0], hidden[1], ("position", "relative")], &[5, 6]),
            at("absolute", "15px", "black"),
            at("fixed", "50px", "red"),
        ],
        &[0, 4],
    );
    let rect = |x: f32, y: f32, width: f32, height: f32| Rect {
        x,
        y,
        width,
        height,
    };
    let rects: Vec<Rect> = (items.iter())
        .map(|item| match item {
            Item::Rect { rect, .. } => *rect,
            _ => panic!("{items:?}"),
        })
        .collect();
    // A square clip cuts a lone rectangle, or leaves one inside it as it is.
    assert_eq!(
        rects,
        [
            rect(0.0, 0.0, 200.0, 20.0),
            rect(0.0, 0.0, 200.0, 20.0),
            rect(0.0, 30.0, 10.0, 10.0),
            rect(0.0, 15.0, 10.0, 5.0),
            rect(0.0, 35.0, 10.0, 5.0),
            rect(0.0, 50.0, 10.0, 10.0),
        ]
    );
}

#[test]
fn clips_round_and_go_on_each_item_in_a_layer() {
    let (items, _) = paint_of(
        vec![
            view(
                &[
                    ("overflow", "clip"),
                    ("height", "20px"),
                    ("borderRadius", "4px"),
                ],
                &[1],
            ),
            view(
                &[
                    ("opacity", "0.5"),
                    ("height", "30px"),
                    ("background", "green"),
                ],
                &[2],
            ),
            view(
                &[
                    ("position", "absolute"),
                    ("top", "40px"),
                    ("width", "10px"),
                    ("height", "10px"),
                    ("background", "blue"),
                ],
                &[],
            ),
            view(&[("overflowX", "clip"), ("width", "10px")], &[4]),
            text_node("Clipped across", &[("color", "black")]),
        ],
        &[0, 3],
    );
    assert_eq!(
        names(&items),
        [
            r#"clip ["text black"]"#.to_string(),
            r#"group 0.5 ["clip [\"green\"]", "blue"]"#.into(),
        ]
    );
    let Item::Group { items: layer, .. } = &items[1] else {
        panic!("{items:?}")
    };
    let Item::Clip { rect, radii, .. } = &layer[0] else {
        panic!("{layer:?}")
    };
    assert_eq!((rect.height, *radii), (20.0, [[4.0; 2]; 4]));
}

#[test]
fn boxes_paint_on_whole_pixels() {
    let at = |left: &str, top: &str, width: &str, height: &str| {
        let style = [
            ("position", "absolute"),
            ("left", left),
            ("top", top),
            ("width", width),
            ("height", height),
            ("background", "red"),
        ];
        view(&style, &[])
    };
    let (items, _) = paint_of(
        vec![
            at("10.5px", "3.4px", "20.25px", "0.5px"),
            at("1.25px", "3.7px", "0.75px", "0.5px"),
            at("0px", "0px", "8px", "0.05px"),
        ],
        &[0, 1, 2],
    );
    let rects: Vec<[f32; 4]> = (items.iter())
        .map(|item| match item {
            Item::Rect { rect: r, .. } => [r.x, r.y, r.width, r.height],
            _ => panic!("{items:?}"),
        })
        .collect();
    // Edges round, halves up; a side that rounds away keeps a pixel unless it is thinner than
    // 4/64 px, which then paints nothing.
    assert_eq!(rects, [[11.0, 3.0, 20.0, 1.0], [1.0, 4.0, 1.0, 1.0]]);
}

#[test]
fn rectangles_round_within_their_box() {
    let (items, _) = paint_of(
        vec![view(
            &[
                ("width", "40px"),
                ("height", "20px"),
                ("borderRadius", "100px"),
                ("background", "rgb(255 0 0 / 50%)"),
            ],
            &[],
        )],
        &[0],
    );
    let Item::Rect { rect, radii, color } = items[0] else {
        panic!("{items:?}")
    };
    assert_eq!(
        rect,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 20.0
        }
    );
    assert_eq!(radii, [[10.0; 2]; 4]);
    // Premultiplied.
    assert_eq!(color, [0.5, 0.0, 0.0, 0.5]);
}

#[test]
fn corners_take_percentages_of_the_box_and_shrink_together() {
    let radii = |style: &[(&str, &str)]| {
        let style = [&[("width", "40px"), ("height", "20px")][..], style].concat();
        let (items, _) = paint_of(vec![view(&style, &[])], &[0]);
        match items[..] {
            [Item::Rect { radii, .. }] => radii,
            _ => panic!("{items:?}"),
        }
    };
    let bg = ("background", "red");
    assert_eq!(
        radii(&[bg, ("borderRadius", "50% / 25%")]),
        [[20.0, 5.0]; 4]
    );
    // The top's radii are twice its width, the left's twice its height: all halve.
    assert_eq!(
        radii(&[bg, ("borderRadius", "60px 20px / 20px")]),
        [[30.0, 10.0], [10.0, 10.0], [30.0, 10.0], [10.0, 10.0]]
    );
    // A corner with a radius of 0 is square.
    assert_eq!(radii(&[bg, ("borderTopLeftRadius", "8px 0px")]), SQUARE);
}

#[test]
fn borders_paint_over_backgrounds_and_clips_go_inside_them() {
    let (items, _) = paint_of(
        vec![
            view(
                &[
                    ("width", "40px"),
                    ("height", "20px"),
                    ("background", "red"),
                    ("border", "2px solid blue"),
                    ("borderLeftColor", "green"),
                    ("borderRadius", "6px"),
                    ("overflow", "hidden"),
                ],
                &[1],
            ),
            view(&[("height", "30px"), ("background", "gray")], &[]),
        ],
        &[0],
    );
    assert_eq!(names(&items), ["red", "ring blue", r#"clip ["gray"]"#]);
    let Item::Ring {
        rect,
        radii,
        inner,
        inner_radii,
        colors,
    } = &items[1]
    else {
        panic!("{items:?}")
    };
    let padding = Rect {
        x: 2.0,
        y: 2.0,
        width: 36.0,
        height: 16.0,
    };
    assert_eq!((rect.width, *radii), (40.0, [[6.0; 2]; 4]));
    assert_eq!((*inner, *inner_radii), (padding, [[4.0; 2]; 4]));
    assert_eq!(colors[3], work(color::parse("green").unwrap()));
    let Item::Clip { rect, radii, .. } = &items[2] else {
        panic!("{items:?}")
    };
    assert_eq!((*rect, *radii), (padding, [[4.0; 2]; 4]));
}

#[test]
fn outlines_paint_after_in_flow_content_around_the_border_box() {
    let (items, _) = paint_of(
        vec![
            view(
                &[
                    ("width", "40px"),
                    ("height", "20px"),
                    ("outline", "2px solid red"),
                    ("outlineOffset", "3px"),
                    ("borderRadius", "4px"),
                ],
                &[1, 2],
            ),
            view(
                &[
                    ("height", "10px"),
                    ("background", "green"),
                    ("outline", "1px solid blue"),
                ],
                &[],
            ),
            view(
                &[
                    ("position", "relative"),
                    ("height", "5px"),
                    ("background", "gray"),
                ],
                &[],
            ),
        ],
        &[0],
    );
    assert_eq!(names(&items), ["green", "ring red", "ring blue", "gray"]);
    let Item::Ring {
        rect, radii, inner, ..
    } = &items[1]
    else {
        panic!("{items:?}")
    };
    let around = Rect {
        x: -5.0,
        y: -5.0,
        width: 50.0,
        height: 30.0,
    };
    // The radii grow by the offset and the width.
    assert_eq!((*rect, *radii, inner.x), (around, [[9.0; 2]; 4], -3.0));
}

#[test]
fn shadows_paint_outer_under_the_background_and_inner_over_it() {
    let (items, _) = paint_of(
        vec![view(
            &[
                ("width", "40px"),
                ("height", "20px"),
                ("background", "gray"),
                ("border", "2px solid black"),
                ("borderRadius", "6px"),
                (
                    "boxShadow",
                    "inset 1px 1px blue, 0 0 4px red, 3px 4px 0 2px green, inset 0 0 2px -1px red",
                ),
            ],
            &[],
        )],
        &[0],
    );
    assert_eq!(
        names(&items),
        [
            r#"outer shadow ["green"]"#,
            r#"outer shadow ["red"]"#,
            "gray",
            r#"inner shadow ["ring red"]"#,
            r#"inner shadow ["ring blue"]"#,
            "ring black",
        ]
    );
    let shadow = |item: &Item| match item {
        Item::Shadow {
            sigma,
            rect,
            inside,
            items,
            ..
        } => (*sigma, *rect, *inside, items[0].clone()),
        _ => panic!("{items:?}"),
    };
    // Moved and spread, with the radii grown by the spread, outside the border box less a px
    // under its opaque background.
    let (sigma, rect, inside, shape) = shadow(&items[0]);
    let less = Rect {
        x: 1.0,
        y: 1.0,
        width: 38.0,
        height: 18.0,
    };
    assert_eq!((sigma, rect, inside), (0.0, less, false));
    let Item::Shadow { radii, .. } = &items[0] else {
        panic!("{items:?}")
    };
    assert_eq!(*radii, [[6.0 * 0.9; 2]; 4]);
    let Item::Rect { rect, radii, .. } = shape else {
        panic!("{shape:?}")
    };
    let moved = Rect {
        x: 1.0,
        y: 2.0,
        width: 44.0,
        height: 24.0,
    };
    assert_eq!((rect, radii), (moved, [[8.0; 2]; 4]));
    // Half the blur is the standard deviation.
    assert_eq!(shadow(&items[1]).0, 2.0);
    // All but a hole shrunk by the spread, inside the padding box, out past the blur.
    let (sigma, rect, inside, shape) = shadow(&items[3]);
    let padding = Rect {
        x: 2.0,
        y: 2.0,
        width: 36.0,
        height: 16.0,
    };
    assert_eq!((sigma, rect, inside), (1.0, padding, true));
    let Item::Ring {
        rect,
        inner,
        inner_radii,
        ..
    } = shape
    else {
        panic!("{shape:?}")
    };
    assert_eq!(rect.x, -2.0);
    let hole = Rect {
        x: 1.0,
        y: 1.0,
        width: 38.0,
        height: 18.0,
    };
    assert_eq!((inner, inner_radii), (hole, [[5.0; 2]; 4]));
}

#[test]
fn spread_grows_small_radii_less() {
    let radii = [[0.0; 2], [2.0; 2], [4.0, 8.0], [10.0; 2]];
    // 2 is half the spread: it grows by 4 × (1 + (0.5 − 1)³).
    assert_eq!(
        spread(radii, 4.0),
        [[0.0; 2], [5.5; 2], [8.0, 12.0], [14.0; 2]]
    );
    assert_eq!(
        spread(radii, -3.0),
        [[0.0; 2], [0.0; 2], [1.0, 5.0], [7.0; 2]]
    );
}

#[test]
fn a_border_rounds_its_inside_less_its_widths() {
    let rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 40.0,
        height: 20.0,
    };
    let radii = [[10.0, 4.0], [3.0; 2], [3.0; 2], [3.0; 2]];
    let (inside, inner_radii) = inner(rect, radii, [2.0, 3.0, 4.0, 6.0]);
    assert_eq!(
        inside,
        Rect {
            x: 6.0,
            y: 2.0,
            width: 31.0,
            height: 14.0
        }
    );
    // Corners whose radius a width takes up are square.
    assert_eq!(inner_radii, [[4.0, 2.0], [0.0; 2], [0.0; 2], [0.0; 2]]);
    // Sides that overlap leave nothing inside, and no radii.
    let (inside, inner_radii) = inner(rect, radii, [12.0, 1.0, 12.0, 1.0]);
    assert_eq!((inside.height, inner_radii), (-4.0, SQUARE));
}

#[test]
fn glyphs_sit_in_the_content_box() {
    let (items, laid) = paint_of(
        vec![
            view(&[("paddingLeft", "30px")], &[1]),
            text_node("Hi there", &[("padding", "5px"), ("color", "blue")]),
        ],
        &[0],
    );
    let lines = laid.lines[1].as_ref().unwrap();
    let [
        Item::Glyphs {
            face,
            size,
            color: c,
            glyphs,
        },
    ] = &items[..]
    else {
        panic!("{items:?}")
    };
    assert_eq!((*face, *size), (0, 16.0));
    assert_eq!(*c, work(color::parse("blue").unwrap()));
    let expected: Vec<(u32, [f32; 2])> = lines
        .glyphs
        .iter()
        .map(|g| (g.id, [35.0 + g.x, 5.0 + g.y]))
        .collect();
    assert_eq!(*glyphs, expected);
}
