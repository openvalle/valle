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
    let styles = Sheet::new(&template).unwrap().compute(&[]).unwrap();
    let laid = layout(&template, &[], &styles, [200.0, 100.0], &mut text()).unwrap();
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
            Item::Glyphs { color, .. } => format!("text {}", name(*color)),
            Item::Group { opacity, items } => format!("group {opacity} {:?}", names(items)),
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
fn flex_items_paint_all_at_once() {
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
    assert_eq!(children("block"), ["red", "green", "blue", "text black"]);
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
    let Item::Rect {
        rect,
        radius,
        color,
    } = items[0]
    else {
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
    assert_eq!(radius, 10.0);
    // Premultiplied.
    assert_eq!(color, [0.5, 0.0, 0.0, 0.5]);
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
