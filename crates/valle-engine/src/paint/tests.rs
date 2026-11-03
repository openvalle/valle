use super::*;
use crate::layout::layout;
use crate::style::{Sheet, color};
use crate::template::Node;
use crate::testing::{mask, text, text_node, view};

/// The display list on a 200 × 100 canvas, and the layout it paints.
fn paint_of(nodes: Vec<Node>, roots: &[u32]) -> (Vec<Item>, Layout) {
    let template = Template {
        nodes,
        roots: roots.to_vec(),
        ..Default::default()
    };
    let canvas = [200.0, 100.0];
    let styles = Sheet::new(&template)
        .unwrap()
        .compute(&[], true, canvas, &[])
        .unwrap();
    let laid = layout(&template, &[], &styles, canvas, &mut text(), &[]).unwrap();
    (paint(&template, &styles, &laid, &[]), laid)
}

/// Items by the named color they are painted in, with any opacity folded into it: `red`,
/// `red@0.5`, `text blue`, `group 0.5 [...]`.
fn names(items: &[Item]) -> Vec<String> {
    let name = |c: [f32; 4]| {
        for n in ["red", "green", "blue", "gray", "black", "white"] {
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
            Item::Gradient { ramp, .. } => format!("gradient of {}", ramp.segments.len()),
            Item::Image { image, .. } => format!("image {image}"),
            Item::Glyphs { color, .. } => format!("text {}", name(*color)),
            Item::Path { color, .. } => format!("path {}", name(*color)),
            Item::Mask { mask, items } => format!("mask {:?} {:?}", names(mask), names(items)),
            Item::Group {
                opacity,
                blend: Blend::Normal,
                items,
            } => format!("group {opacity} {:?}", names(items)),
            Item::Group {
                opacity,
                blend,
                items,
            } => format!("group {opacity} {blend:?} {:?}", names(items)),
            Item::Clip { items, .. } => format!("clip {:?}", names(items)),
            Item::Transform { items, .. } => format!("transform {:?}", names(items)),
            Item::Cut { items, .. } => format!("cut {:?}", names(items)),
            Item::Backdrop { filters, .. } => format!("backdrop of {}", filters.len()),
            Item::Filter { filters, items } => {
                let filters: Vec<&str> = (filters.iter())
                    .map(|f| match f {
                        Filter::Blur(_) => "blur",
                        Filter::Colors(_) => "colors",
                        Filter::Shadow { .. } => "shadow",
                        Filter::Luminance => "luminance",
                        Filter::Aberration(_) => "aberration",
                        Filter::Lens { .. } => "lens",
                        Filter::Grain { .. } => "grain",
                        Filter::Radial { .. } => "radial",
                        Filter::Motion(_) => "motion",
                        Filter::Displacement { .. } => "displacement",
                        Filter::Bloom { .. } => "bloom",
                    })
                    .collect();
                format!("filter {filters:?} {:?}", names(items))
            }
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

    // A block that is neither positioned nor an item takes no z-index, even as a stacking
    // context: at 0, after the in-flow content.
    let order = painted(
        vec![
            view(&[("height", "10px"), ("background", "red")], &[1, 2]),
            view(
                &[
                    ("opacity", "0.5"),
                    ("zIndex", "-1"),
                    ("height", "5px"),
                    ("background", "blue"),
                ],
                &[],
            ),
            view(&[("height", "5px"), ("background", "green")], &[]),
        ],
        &[0],
    );
    assert_eq!(order, ["red", "green", "blue@0.5"]);
}

#[test]
fn transformed_boxes_paint_in_their_transform_as_stacking_contexts() {
    // After the in-flow content, its items in its transform.
    let order = painted(
        vec![
            view(&[("height", "30px"), ("background", "red")], &[1, 2]),
            view(
                &[
                    ("transform", "rotate(90deg)"),
                    ("height", "10px"),
                    ("background", "blue"),
                ],
                &[],
            ),
            view(&[("height", "10px"), ("background", "green")], &[]),
        ],
        &[0],
    );
    assert_eq!(order, ["red", "green", r#"transform ["blue"]"#]);

    // Around its origin, the functions last first.
    let matrix = |origin: &str| {
        let style = [
            ("position", "absolute"),
            ("left", "10px"),
            ("top", "20px"),
            ("width", "40px"),
            ("height", "20px"),
            ("background", "blue"),
            ("transform", "translate(5px, 10%) scale(2)"),
            ("transformOrigin", origin),
        ];
        match &paint_of(vec![view(&style, &[])], &[0]).0[..] {
            [Item::Transform { matrix, .. }] => *matrix,
            items => panic!("{items:?}"),
        }
    };
    assert_eq!(matrix("center").apply([10.0, 20.0]), [-5.0, 12.0]);
    assert_eq!(matrix("left top").apply([10.0, 20.0]), [15.0, 22.0]);
    assert_eq!(matrix("left top").apply([30.0, 30.0]), [55.0, 42.0]);

    // One that flattens the plane shows nothing.
    let flat = [
        ("transform", "scale(0, 1)"),
        ("height", "10px"),
        ("background", "blue"),
    ];
    assert_eq!(painted(vec![view(&flat, &[])], &[0]), Vec::<String>::new());
}

#[test]
fn clips_around_a_transformed_box_go_around_its_transform() {
    let clipping = [("overflow", "hidden"), ("height", "40px")];
    let transformed = [
        ("transform", "rotate(10deg)"),
        ("height", "20px"),
        ("background", "blue"),
    ];
    let absolute = [
        ("position", "absolute"),
        ("width", "5px"),
        ("height", "5px"),
        ("background", "green"),
    ];
    let (items, _) = paint_of(
        vec![
            view(&[("position", "relative")], &[1]),
            view(&clipping, &[2]),
            view(&transformed, &[3]),
            view(&absolute, &[]),
        ],
        &[0],
    );
    // The positioned box's containing block is the transformed one, so the clip holds it too.
    let [Item::Clip { items, .. }] = &items[..] else {
        panic!("{items:?}")
    };
    assert_eq!(names(items), [r#"transform ["blue", "green"]"#]);

    // Its own clip is inside its transform.
    let clipped = [transformed.as_slice(), &[("overflow", "hidden")]].concat();
    let overflowing = [
        ("height", "50px"),
        ("borderRadius", "4px"),
        ("background", "green"),
    ];
    let (items, _) = paint_of(vec![view(&clipped, &[1]), view(&overflowing, &[])], &[0]);
    assert_eq!(names(&items), [r#"transform ["blue", "clip [\"green\"]"]"#]);
}

#[test]
fn transforms_take_a_box_to_the_canvas_through_its_ancestors() {
    let template = Template {
        nodes: vec![
            view(&[("transform", "translate(10px, 0)")], &[1]),
            view(
                &[
                    ("width", "20px"),
                    ("height", "20px"),
                    ("transform", "rotate(90deg)"),
                    ("transformOrigin", "0 0"),
                ],
                &[2],
            ),
            view(&[("height", "5px")], &[]),
        ],
        roots: vec![0],
        ..Default::default()
    };
    let canvas = [200.0, 100.0];
    let styles = Sheet::new(&template)
        .unwrap()
        .compute(&[], true, canvas, &[])
        .unwrap();
    let laid = layout(&template, &[], &styles, canvas, &mut text(), &[]).unwrap();
    let transforms = transforms(&template, &styles, &laid);
    let at = |i: usize, p| transforms[i].apply(p).map(rounded);
    assert_eq!(at(0, [0.0, 0.0]), [10.0, 0.0]);
    // A quarter turn clockwise about its top left, then moved by its parent.
    assert_eq!(at(1, [20.0, 0.0]), [10.0, 20.0]);
    assert_eq!(at(2, [0.0, 5.0]), [5.0, 0.0]);
}

#[test]
fn motion_paths_move_a_box_along_and_turn_it_after_its_own_transforms() {
    let moving = |more: &[(&'static str, &'static str)]| {
        let place = [
            ("position", "absolute"),
            ("left", "50px"),
            ("top", "20px"),
            ("width", "40px"),
            ("height", "20px"),
        ];
        view(&[&place[..], more].concat(), &[])
    };
    let container = [
        ("position", "relative"),
        ("height", "100px"),
        ("padding", "10px"),
        ("border", "5px solid black"),
    ];
    let template = Template {
        nodes: vec![
            view(&container, &[1, 2, 3, 4, 5]),
            moving(&[
                ("offsetPath", "path('M0 0 L100 0')"),
                ("offsetDistance", "50%"),
            ]),
            moving(&[
                ("offsetPath", "path('M0 0 H100 V100 Z')"),
                ("offsetDistance", "150%"),
            ]),
            moving(&[
                ("offsetPath", "ray(90deg)"),
                ("offsetDistance", "30px"),
                ("offsetAnchor", "0 0"),
            ]),
            moving(&[
                ("offsetPath", "circle(20px at 100px 50px) content-box"),
                ("offsetDistance", "25%"),
            ]),
            moving(&[
                ("offsetPath", "path('M0 0')"),
                ("translate", "10px 0"),
                ("transform", "rotate(90deg)"),
            ]),
        ],
        roots: vec![0],
        ..Default::default()
    };
    let canvas = [200.0, 100.0];
    let styles = Sheet::new(&template)
        .unwrap()
        .compute(&[], true, canvas, &[])
        .unwrap();
    let laid = layout(&template, &[], &styles, canvas, &mut text(), &[]).unwrap();
    let transforms = transforms(&template, &styles, &laid);
    // Each box's center and the point 20 px right of it, from its border box at 55, 25.
    let at = |i: usize| [[75.0, 35.0], [95.0, 35.0]].map(|p| transforms[i].apply(p));
    let near = |i: usize, want: [[f32; 2]; 2]| {
        let got = at(i);
        let off = (0..4).map(|k| (got[k / 2][k % 2] - want[k / 2][k % 2]).abs());
        assert!(
            off.fold(0.0, f32::max) < 0.1,
            "box {i}: {got:?}, not {want:?}"
        );
    };
    // A path is from the box's own top left, its center on it.
    near(1, [[105.0, 25.0], [125.0, 25.0]]);
    // Closed, it goes on around, and the box turns down with it.
    near(2, [[155.0, 25.0 + 70.711], [155.0, 45.0 + 70.711]]);
    // A ray from the containing block's center, its anchor the box's top left on it.
    near(3, [[150.0, 60.0], [170.0, 60.0]]);
    // A circle in the containing block's content box, a quarter around, turned left.
    near(4, [[115.0, 85.0], [95.0, 85.0]]);
    // After `translate` and before `transform`.
    near(5, [[65.0, 25.0], [65.0, 45.0]]);
}

#[test]
fn a_perspective_reaches_only_the_boxs_children() {
    let toward = [
        ("width", "40px"),
        ("height", "20px"),
        ("transform", "translateZ(50px)"),
    ];
    let template = Template {
        nodes: vec![
            view(&[("height", "100px"), ("perspective", "100px")], &[1, 2]),
            view(&toward, &[]),
            view(&[("height", "20px")], &[3]),
            view(&toward, &[]),
        ],
        roots: vec![0],
        ..Default::default()
    };
    let canvas = [200.0, 100.0];
    let styles = Sheet::new(&template)
        .unwrap()
        .compute(&[], true, canvas, &[])
        .unwrap();
    let laid = layout(&template, &[], &styles, canvas, &mut text(), &[]).unwrap();
    let transforms = transforms(&template, &styles, &laid);
    let at = |i: usize, p| transforms[i].apply(p).map(rounded);
    // Halfway to a viewer 100 px in front of the center: twice as far from it.
    assert_eq!(at(1, [0.0, 0.0]), [-100.0, -50.0]);
    // Its grandchild is flattened as if from infinitely far.
    assert_eq!(at(3, [0.0, 20.0]), [0.0, 20.0]);
}

#[test]
fn a_box_showing_its_back_hides_with_all_in_it_when_asked() {
    let turned = |transform: &'static str, backface: &'static str| {
        let style = [
            ("height", "10px"),
            ("background", "blue"),
            ("transform", transform),
            ("backfaceVisibility", backface),
        ];
        painted(
            vec![
                view(&style, &[1]),
                view(&[("height", "5px"), ("background", "green")], &[]),
            ],
            &[0],
        )
    };
    assert_eq!(
        turned("rotateY(180deg)", "visible"),
        [r#"transform ["blue", "green"]"#]
    );
    assert_eq!(turned("rotateY(180deg)", "hidden"), [] as [&str; 0]);
    assert_eq!(turned("rotateX(-120deg)", "hidden"), [] as [&str; 0]);
    // A mirror does not show the back.
    assert_eq!(
        turned("scaleX(-1)", "hidden"),
        [r#"transform ["blue", "green"]"#]
    );
}

#[test]
fn a_3d_scene_paints_its_planes_back_to_front_cut_where_they_cross() {
    let card = |color: &'static str, transform: &'static str| {
        let style = [
            ("position", "absolute"),
            ("width", "20px"),
            ("height", "20px"),
            ("background", color),
            ("transform", transform),
        ];
        view(&style, &[])
    };
    let scene = |style: &[(&'static str, &'static str)], cards| {
        let mut nodes = vec![view(style, &[1, 2])];
        nodes.extend(cards);
        painted(nodes, &[0])
    };
    let keeps = [("height", "20px"), ("transformStyle", "preserve-3d")];
    // The nearer one last, whatever the tree order.
    let order = scene(
        &keeps,
        [
            card("red", "translateZ(10px)"),
            card("blue", "translateZ(-10px)"),
        ],
    );
    assert_eq!(order, [r#"transform ["blue"]"#, r#"transform ["red"]"#]);
    // Two that cross: the second cut in two, its far half before the first and its near half
    // after.
    let order = scene(
        &keeps,
        [
            card("red", "rotateY(30deg)"),
            card("blue", "rotateY(-30deg)"),
        ],
    );
    assert_eq!(
        order,
        [
            r#"transform ["cut [\"blue\"]"]"#,
            r#"transform ["red"]"#,
            r#"transform ["cut [\"blue\"]"]"#,
        ]
    );
    // An opacity groups them, which flattens them in tree order.
    let flat = [keeps[0], keeps[1], ("opacity", "0.5")];
    let order = scene(
        &flat,
        [
            card("red", "translateZ(10px)"),
            card("blue", "translateZ(-10px)"),
        ],
    );
    assert_eq!(
        order,
        [r#"group 0.5 ["transform [\"red\"]", "transform [\"blue\"]"]"#]
    );
}

#[test]
fn transforms_in_a_3d_scene_add_up_before_they_are_flattened() {
    let template = Template {
        nodes: vec![
            view(
                &[
                    ("width", "20px"),
                    ("height", "20px"),
                    ("transform", "rotateY(45deg)"),
                    ("transformStyle", "preserve-3d"),
                ],
                &[1],
            ),
            view(&[("height", "20px"), ("transform", "rotateY(45deg)")], &[]),
        ],
        roots: vec![0],
        ..Default::default()
    };
    let canvas = [200.0, 100.0];
    let styles = Sheet::new(&template)
        .unwrap()
        .compute(&[], true, canvas, &[])
        .unwrap();
    let laid = layout(&template, &[], &styles, canvas, &mut text(), &[]).unwrap();
    let transforms = transforms(&template, &styles, &laid);
    // A quarter turn about y in all: edge on.
    assert_eq!(transforms[1].inverse(), None);
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
            r#"clip ["text black", "text black"]"#.to_string(),
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
fn a_filter_takes_all_a_box_paints_inside_its_transform_and_clips() {
    let (items, _) = paint_of(
        vec![
            view(&[("overflow", "hidden"), ("height", "50px")], &[1]),
            view(
                &[
                    (
                        "filter",
                        "blur(2px) grayscale(1) invert(0.5) drop-shadow(1px 2px red)",
                    ),
                    ("transform", "rotate(10deg)"),
                    ("opacity", "0.5"),
                    ("height", "20px"),
                    ("background", "blue"),
                ],
                &[2],
            ),
            // A fixed box in a filtered one is in its filter.
            view(
                &[
                    ("position", "fixed"),
                    ("width", "10px"),
                    ("height", "10px"),
                    ("background", "green"),
                ],
                &[],
            ),
        ],
        &[0],
    );
    let [Item::Group { items, .. }] = &items[..] else {
        panic!("{items:?}")
    };
    let [Item::Clip { items, .. }] = &items[..] else {
        panic!("{items:?}")
    };
    let [Item::Transform { items, .. }] = &items[..] else {
        panic!("{items:?}")
    };
    let [Item::Filter { filters, items }] = &items[..] else {
        panic!("{items:?}")
    };
    assert_eq!(names(items), ["blue", "green"]);
    let [
        Filter::Blur(2.0),
        Filter::Colors(m),
        Filter::Shadow {
            offset,
            sigma,
            color,
        },
    ] = &filters[..]
    else {
        panic!("{filters:?}")
    };
    assert_eq!(
        (m.len(), *offset, *sigma, *color),
        (2, [1.0, 2.0], 0.0, [1.0, 0.0, 0.0, 1.0])
    );
    assert_eq!(m[0][..3], [0.2126, 0.7152, 0.0722]);
    assert_eq!(m[1][..4], [0.0; 4]);
    assert_eq!(m[1][16..], [0.5, 0.5, 0.5, 0.0]);
}

#[test]
fn a_backdrop_filter_paints_under_its_box_in_a_layer() {
    let order = painted(
        vec![
            view(&[("background", "gray")], &[1]),
            view(
                &[
                    ("backdropFilter", "blur(4px) grayscale(1)"),
                    ("margin", "10px"),
                    ("height", "20px"),
                    ("borderRadius", "5px"),
                    ("background", "red"),
                ],
                &[2],
            ),
            // Fixed, in it.
            view(
                &[
                    ("position", "fixed"),
                    ("width", "5px"),
                    ("height", "5px"),
                    ("background", "blue"),
                ],
                &[],
            ),
        ],
        &[0],
    );
    assert_eq!(
        order,
        [
            "gray",
            r#"group 1 ["backdrop of 2", "group 1 [\"red\", \"blue\"]"]"#
        ]
    );
}

#[test]
fn a_clip_path_masks_all_its_stacking_context_paints() {
    let order = painted(
        vec![
            view(&[("background", "gray")], &[1]),
            view(
                &[
                    ("clipPath", "inset(2px round 4px)"),
                    ("opacity", "0.5"),
                    ("margin", "10px"),
                    ("height", "20px"),
                    ("background", "red"),
                ],
                &[2],
            ),
            // Absolute, its containing block the canvas: in the box's stacking context all the
            // same.
            view(
                &[
                    ("position", "absolute"),
                    ("width", "5px"),
                    ("height", "5px"),
                    ("background", "blue"),
                ],
                &[],
            ),
        ],
        &[0],
    );
    assert_eq!(
        order,
        [
            "gray",
            r#"group 0.5 ["mask [\"white\"] [\"red\", \"blue\"]"]"#
        ]
    );
    // Around its backdrop and filter, inside its transform.
    let (items, _) = paint_of(
        vec![
            view(&[("background", "gray")], &[1]),
            view(
                &[
                    ("clipPath", "circle()"),
                    ("filter", "blur(1px)"),
                    ("backdropFilter", "invert(1)"),
                    ("transform", "rotate(10deg)"),
                    ("height", "20px"),
                    ("background", "red"),
                ],
                &[],
            ),
        ],
        &[0],
    );
    let [_, Item::Group { items, .. }] = &items[..] else {
        panic!("{items:?}")
    };
    let [Item::Transform { items, .. }] = &items[..] else {
        panic!("{items:?}")
    };
    let [Item::Mask { mask, items }] = &items[..] else {
        panic!("{items:?}")
    };
    assert_eq!(names(mask), ["white"]);
    assert_eq!(
        names(items),
        [
            "backdrop of 1",
            r#"group 1 ["filter [\"blur\"] [\"red\"]"]"#
        ]
    );
}

#[test]
fn a_mask_shows_what_its_box_paints_through_its_layers() {
    let (items, _) = paint_of(
        vec![
            view(&[("background", "gray")], &[1]),
            view(
                &[
                    (
                        "maskImage",
                        "linear-gradient(black, transparent), radial-gradient(red, blue, green)",
                    ),
                    ("maskMode", "alpha, luminance"),
                    ("clipPath", "circle()"),
                    ("margin", "10px"),
                    ("height", "20px"),
                    ("borderRadius", "5px"),
                    ("background", "red"),
                ],
                &[2],
            ),
            view(
                &[
                    ("position", "absolute"),
                    ("width", "5px"),
                    ("height", "5px"),
                    ("background", "blue"),
                ],
                &[],
            ),
        ],
        &[0],
    );
    // The last layer first, under the first; through the clip path first.
    let [_, Item::Mask { mask, items }] = &items[..] else {
        panic!("{items:?}")
    };
    assert_eq!(
        names(mask),
        [r#"filter ["luminance"] ["gradient of 2"]"#, "gradient of 1"]
    );
    assert_eq!(names(items), [r#"mask ["white"] ["red", "blue"]"#]);
    let Item::Gradient { rect, radii, .. } = &mask[1] else {
        panic!("{mask:?}")
    };
    // In the border box, rounded by its corners.
    assert_eq!(
        (*rect, *radii),
        (
            Rect {
                x: 10.0,
                y: 10.0,
                width: 180.0,
                height: 20.0
            },
            [[5.0; 2]; 4]
        )
    );
}

#[test]
fn spans_paint_their_text_in_their_own_style() {
    use crate::template::{Kind, Piece, Prop, V};
    let piece = |s: &str| Piece::Text(Prop::Const(V::Str(s.into())));
    let mut text = text_node("", &[("color", "black")]);
    text.kind = Kind::Text {
        content: vec![piece("a "), Piece::Span(1), piece(" c")],
    };
    text.children = vec![1];
    let mut span = view(&[("color", "red"), ("fontSize", "20px")], &[]);
    span.kind = Kind::Span {
        content: vec![piece("b"), Piece::Span(2)],
    };
    span.children = vec![2];
    let mut hidden = view(&[("visibility", "hidden")], &[]);
    hidden.kind = Kind::Span {
        content: vec![piece("d")],
    };
    let (items, _) = paint_of(vec![text, span, hidden], &[0]);
    assert_eq!(names(&items), ["text black", "text red", "text black"]);
    let sizes: Vec<f32> = (items.iter())
        .map(|i| match i {
            Item::Glyphs { size, .. } => *size,
            _ => 0.0,
        })
        .collect();
    assert_eq!(sizes, [16.0, 20.0, 16.0]);
}

#[test]
fn text_paints_line_throughs_over_its_glyphs_and_other_lines_under() {
    let style = [
        ("color", "black"),
        ("textDecoration", "underline overline line-through red"),
    ];
    let (items, _) = paint_of(vec![text_node("x", &style)], &[0]);
    assert_eq!(names(&items), ["red", "red", "text black", "red"]);
    let hidden = [("visibility", "hidden"), ("textDecoration", "underline")];
    assert!(paint_of(vec![text_node("x", &hidden)], &[0]).0.is_empty());
}

#[test]
fn text_shadows_go_under_each_part_of_a_run_the_last_lowest() {
    let style = [
        ("color", "black"),
        ("textDecoration", "underline white"),
        ("textShadow", "1px 1px red, 2px 2px 4px blue"),
    ];
    let (items, _) = paint_of(vec![text_node("x", &style)], &[0]);
    let blurred = |what: &str| format!("filter [\"blur\"] [\"{what}\"]");
    assert_eq!(
        names(&items),
        [
            blurred("blue"),
            "red".into(),
            "white".into(),
            blurred("text blue"),
            "text red".into(),
            "text black".into(),
        ]
    );
    let Item::Filter { filters, .. } = &items[3] else {
        unreachable!()
    };
    assert!(matches!(filters[..], [Filter::Blur(2.0)]));
    // Text with no fill still casts its shadows.
    let unfilled = [
        ("WebkitTextFillColor", "transparent"),
        ("textShadow", "1px 1px red"),
    ];
    let (items, _) = paint_of(vec![text_node("x", &unfilled)], &[0]);
    assert_eq!(names(&items), ["text red"]);
}

#[test]
fn text_strokes_go_over_the_fill_and_cast_one_shadow_with_it() {
    let style = [
        ("color", "black"),
        ("WebkitTextStroke", "2px blue"),
        ("textShadow", "1px 1px rgb(255 0 0 / 0.5), 2px 2px green"),
    ];
    let (items, _) = paint_of(vec![text_node("x", &style)], &[0]);
    // A translucent shadow is one layer, so that it is no darker where the two overlap.
    assert_eq!(
        names(&items),
        [
            "text green",
            "text green",
            "group 0.5 [\"text red\", \"text red\"]",
            "text black",
            "text blue",
        ]
    );
    let strokes: Vec<f32> = [&items[..2], &items[3..]]
        .concat()
        .iter()
        .map(|item| match item {
            Item::Glyphs { stroke, .. } => *stroke,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(strokes, [0.0, 2.0, 0.0, 2.0]);
    // Its currentcolor is the color of the text, whatever its fill.
    let outlined = [
        ("color", "red"),
        ("WebkitTextFillColor", "transparent"),
        ("WebkitTextStrokeWidth", "1px"),
    ];
    assert_eq!(painted(vec![text_node("x", &outlined)], &[0]), ["text red"]);
}

#[test]
fn a_background_clipped_to_text_shows_through_the_text_in_the_box() {
    let underlined = [("color", "black"), ("textDecoration", "underline red")];
    let painted = painted(
        vec![
            view(
                &[("background", "linear-gradient(red, blue) text green")],
                &[1, 2],
            ),
            text_node("x", &underlined),
            // Text in a transform of its own is not the box's.
            view(&[("transform", "translate(1px)")], &[3]),
            text_node("y", &[("color", "black")]),
        ],
        &[0],
    );
    // The glyphs and their decorations, opaque, for the color as for the image.
    let mask = r#"["text white", "white"]"#;
    assert_eq!(
        painted,
        [
            format!(r#"mask {mask} ["green"]"#),
            format!(r#"mask {mask} ["gradient of 1"]"#),
            "red".into(),
            "text black".into(),
            r#"transform ["text black"]"#.into(),
        ]
    );
}

#[test]
fn a_mask_element_shows_what_its_box_paints_through_its_source() {
    let masked = |source: (u32, bool, bool), shown: &[u32]| {
        paint_of(
            vec![
                mask(
                    source,
                    &[
                        ("margin", "10px"),
                        ("height", "20px"),
                        ("borderRadius", "5px"),
                        ("background", "red"),
                        ("color", "green"),
                    ],
                    &[1],
                ),
                view(&[("height", "10px"), ("background", "blue")], &[]),
                view(&[], shown),
                view(&[("height", "10px"), ("background", "green")], &[]),
            ],
            &[0],
        )
        .0
    };
    // What the source paints, inheriting from the mask, in the border box rounded.
    let items = masked((2, false, false), &[3]);
    let [Item::Mask { mask, items }] = &items[..] else {
        panic!("{items:?}")
    };
    assert_eq!(names(items), ["red", "blue"]);
    assert_eq!(names(mask), [r#"mask ["white"] ["green"]"#]);
    let [Item::Mask { mask: area, .. }] = &mask[..] else {
        panic!("{mask:?}")
    };
    let [Item::Rect { rect, radii, .. }] = &area[..] else {
        panic!("{area:?}")
    };
    assert_eq!((rect.y, rect.height, *radii), (10.0, 20.0, [[5.0; 2]; 4]));
    // Inverted, by luminance: inside the box, where the source's luminance is not.
    let items = masked((2, true, true), &[3]);
    let [Item::Mask { mask, .. }] = &items[..] else {
        panic!("{items:?}")
    };
    let [Item::Mask { mask: area, items }] = &mask[..] else {
        panic!("{mask:?}")
    };
    assert_eq!(names(area), ["white"]);
    let [Item::Filter { filters, items }] = &items[..] else {
        panic!("{items:?}")
    };
    assert_eq!(filters, &[Filter::Colors(vec![INVERTED])]);
    assert_eq!(names(items), [r#"filter ["luminance"] ["green"]"#]);
    // A source that paints nothing shows nothing, or inverted, all of the box.
    assert!(masked((2, false, false), &[]).is_empty());
    let items = masked((2, false, true), &[]);
    assert_eq!(names(&items), [r#"mask ["white"] ["red", "blue"]"#]);
}

#[test]
fn clip_path_shapes_are_in_the_box_they_name() {
    use crate::path::Segment;
    // A box at 10, 10, 100 × 40, its border 2 px, its padding 3 px, its corners rounded 10 px.
    let mask = |clip: &str| {
        let style = [
            ("margin", "10px"),
            ("width", "100px"),
            ("height", "40px"),
            ("border", "2px solid black"),
            ("padding", "3px"),
            ("borderRadius", "10px"),
            ("background", "red"),
            ("clipPath", clip),
        ];
        let (items, _) = paint_of(vec![view(&style, &[])], &[0]);
        match <[Item; 1]>::try_from(items) {
            Ok([Item::Mask { mask, .. }]) => Some(mask),
            Ok(other) => panic!("{other:?}"),
            Err(none) if none.is_empty() => None,
            Err(items) => panic!("{items:?}"),
        }
    };
    let rect = |x, y, width, height, radii: [f32; 2]| Item::Rect {
        rect: Rect {
            x,
            y,
            width,
            height,
        },
        radii: [radii; 4],
        color: [1.0; 4],
    };
    assert_eq!(
        mask("padding-box"),
        Some(vec![rect(12.0, 12.0, 96.0, 36.0, [8.0; 2])])
    );
    // Radii of the box, fitting the rectangle.
    assert_eq!(
        mask("inset(5px 10% round 50%)"),
        Some(vec![rect(20.0, 15.0, 80.0, 30.0, [37.5, 15.0])])
    );
    assert_eq!(
        mask("circle(at 20px 50%)"),
        Some(vec![rect(10.0, 10.0, 40.0, 40.0, [20.0; 2])])
    );
    assert_eq!(
        mask("ellipse(farthest-side 25% at left)"),
        Some(vec![rect(-90.0, 20.0, 200.0, 20.0, [100.0, 10.0])])
    );
    assert_eq!(
        mask("content-box polygon(evenodd, 0 0, 100% 0, 50% 100%)"),
        Some(vec![Item::Path {
            path: Path(vec![
                Segment::Move([15.0, 15.0]),
                Segment::Line([105.0, 15.0]),
                Segment::Line([60.0, 45.0]),
            ]),
            even_odd: true,
            color: [1.0; 4],
        }])
    );
    assert_eq!(
        mask("path('M0 0 h5')"),
        Some(vec![Item::Path {
            path: Path(vec![
                Segment::Move([10.0, 10.0]),
                Segment::Line([15.0, 10.0])
            ]),
            even_odd: false,
            color: [1.0; 4],
        }])
    );
    // Insets that meet leave nothing to show.
    assert_eq!(mask("inset(20px 0)"), None);
}

#[test]
fn color_filters_at_no_effect_are_the_identity() {
    use crate::style::filter::Function as F;
    let none = [
        F::Brightness(1.0),
        F::Contrast(1.0),
        F::Grayscale(0.0),
        F::HueRotate(360.0),
        F::Invert(0.0),
        F::Opacity(1.0),
        F::Saturate(1.0),
        F::Sepia(0.0),
    ];
    for f in none {
        let m = colors(&f).unwrap();
        for (k, v) in m.iter().enumerate() {
            let one = f32::from(u8::from(k < 16 && k % 4 == k / 4));
            assert!((v - one).abs() < 1e-6, "{f:?}: {m:?}");
        }
    }
    assert_eq!(colors(&F::Blur(1.0)), None);
}

#[test]
fn a_box_that_blends_is_a_layer_and_its_stacking_context_one_too() {
    let order = painted(
        vec![
            view(&[("background", "gray")], &[1, 3, 6]),
            view(&[("overflow", "hidden"), ("height", "20px")], &[2]),
            view(
                &[
                    ("mixBlendMode", "multiply"),
                    ("height", "30px"),
                    ("background", "red"),
                ],
                &[],
            ),
            // A stacking context that is no layer of its own groups a box blending in it.
            view(
                &[
                    ("position", "relative"),
                    ("zIndex", "1"),
                    ("background", "green"),
                ],
                &[4, 5],
            ),
            view(&[("height", "10px"), ("background", "blue")], &[]),
            view(
                &[
                    ("mixBlendMode", "screen"),
                    ("height", "10px"),
                    ("background", "black"),
                ],
                &[],
            ),
            // Isolation alone groups nothing.
            view(
                &[
                    ("isolation", "isolate"),
                    ("height", "5px"),
                    ("background", "red"),
                ],
                &[],
            ),
        ],
        &[0],
    );
    assert_eq!(
        order,
        [
            "gray",
            r#"group 1 Multiply ["red"]"#,
            "red",
            r#"group 1 ["green", "blue", "group 1 Screen [\"black\"]"]"#,
        ]
    );
    // The clips around a box that blends go in its layer, so it blends with all under them.
    let (items, _) = paint_of(
        vec![
            view(&[("background", "gray")], &[1]),
            view(&[("overflow", "hidden"), ("height", "20px")], &[2]),
            view(
                &[
                    ("mixBlendMode", "difference"),
                    ("transform", "rotate(10deg)"),
                    ("height", "30px"),
                    ("background", "red"),
                ],
                &[],
            ),
        ],
        &[0],
    );
    let Item::Group { items: layer, .. } = &items[1] else {
        panic!("{items:?}")
    };
    assert!(matches!(&layer[..], [Item::Clip { .. }]), "{layer:?}");
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
fn border_lines_split_and_shade_sides_as_chrome_draws_them() {
    let rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 40.0,
        height: 20.0,
    };
    let green = color::parse("#4caf50").unwrap();
    let [dark, light] = color::shades(green).map(work);
    // Each ring: how deep it is in the top and the bottom, and the colors of those sides.
    let rings = |widths: [f32; 4], styles: [BorderStyle; 4]| {
        (rings(rect, SQUARE, widths, &lines(widths, styles, [green; 4])).iter())
            .map(|ring| match ring {
                Item::Ring {
                    rect: r,
                    inner,
                    colors,
                    ..
                } => (
                    [
                        r.y,
                        inner.y,
                        20.0 - r.y - r.height,
                        20.0 - inner.y - inner.height,
                    ],
                    [colors[0], colors[2]],
                ),
                item => panic!("{item:?}"),
            })
            .collect::<Vec<_>>()
    };
    use BorderStyle::{Double, Groove, Inset, Outset, Ridge, Solid};
    let g = work(green);
    assert_eq!(
        rings([10.0; 4], [Double; 4]),
        [
            ([0.0, 3.0, 0.0, 3.0], [g; 2]),
            ([7.0, 10.0, 7.0, 10.0], [g; 2])
        ]
    );
    assert_eq!(rings([7.0; 4], [Double; 4])[1].0[..2], [5.0, 7.0]);
    // Under 3 px a double line is solid, as is a side of another style.
    assert_eq!(
        rings([2.0; 4], [Double; 4]),
        [([0.0, 2.0, 0.0, 2.0], [g; 2])]
    );
    assert_eq!(
        rings([4.0, 4.0, 6.0, 4.0], [Double, Solid, Solid, Solid]),
        [
            ([0.0, 1.0, 0.0, 6.0], [g; 2]),
            ([3.0, 4.0, 6.0, 6.0], [g, [0.0; 4]])
        ]
    );
    // Inset is darker at the top, outset at the bottom.
    assert_eq!(rings([3.0; 4], [Inset; 4])[0].1, [dark, light]);
    assert_eq!(rings([3.0; 4], [Outset; 4])[0].1, [light, dark]);
    // A groove is inset outside outset, the odd px in the top's outer half and the bottom's
    // inner one; a ridge the other way round. An opaque outer half goes on under the inner one.
    assert_eq!(
        rings([5.0; 4], [Groove; 4]),
        [
            ([0.0, 5.0, 0.0, 5.0], [dark, light]),
            ([3.0, 5.0, 2.0, 5.0], [light, dark])
        ]
    );
    assert_eq!(rings([4.0; 4], [Ridge; 4])[0].1, [light, dark]);
    let clear = [green[0], green[1], green[2], 0.5];
    let halves = lines([5.0; 4], [Groove; 4], [clear; 4]).map(|l| l[0].depth);
    assert_eq!(halves, [[0.0, 3.0], [0.0, 2.0], [0.0, 2.0], [0.0, 3.0]]);
    // Under 2 px it is solid.
    assert_eq!(
        rings([1.0; 4], [Groove; 4]),
        [([0.0, 1.0, 0.0, 1.0], [g; 2])]
    );
}

/// The gradients a box paints, 100 × 50 px and with a 10 px border, with its `background`.
fn ramps(background: &str) -> Vec<(Rect, Rect, Ramp)> {
    let style = [
        ("width", "100px"),
        ("height", "50px"),
        ("border", "10px solid transparent"),
        ("boxSizing", "content-box"),
        ("background", background),
    ];
    let (items, _) = paint_of(vec![view(&style, &[])], &[0]);
    (items.into_iter())
        .filter_map(|item| match item {
            Item::Gradient {
                rect, tiles, ramp, ..
            } => Some((rect, tiles.first, ramp)),
            _ => None,
        })
        .collect()
}

/// The first tile and the steps of the gradients a box like `ramps`'s paints, rounded.
fn tiled(background: &str) -> Vec<([f32; 4], [Option<f32>; 2])> {
    let style = [
        ("width", "100px"),
        ("height", "50px"),
        ("border", "10px solid transparent"),
        ("boxSizing", "content-box"),
        ("background", background),
    ];
    let (items, _) = paint_of(vec![view(&style, &[])], &[0]);
    (items.into_iter())
        .filter_map(|item| match item {
            Item::Gradient { tiles, .. } => {
                let r = tiles.first;
                let first = [r.x, r.y, r.width, r.height].map(rounded);
                Some((first, tiles.step.map(|s| s.map(rounded))))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn layers_are_placed_in_and_clipped_to_their_boxes() {
    // The color's rect, or a gradient's and its first tile.
    let shown = |background: &str| {
        let style = [
            ("width", "100px"),
            ("height", "50px"),
            ("border", "10px solid transparent"),
            ("padding", "5px 10px"),
            ("boxSizing", "content-box"),
            ("background", background),
        ];
        let (items, _) = paint_of(vec![view(&style, &[])], &[0]);
        let xywh = |r: Rect| [r.x, r.y, r.width, r.height];
        (items.into_iter())
            .filter_map(|item| match item {
                Item::Rect { rect, .. } => Some((xywh(rect), None)),
                Item::Gradient { rect, tiles, .. } => Some((xywh(rect), Some(xywh(tiles.first)))),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let border = [0.0, 0.0, 140.0, 80.0];
    let padding = [10.0, 10.0, 120.0, 60.0];
    let content = [20.0, 15.0, 100.0, 50.0];
    // Placed in the padding box and shown under the border.
    assert_eq!(
        shown("linear-gradient(red, blue), gray"),
        [(border, None), (border, Some(padding))]
    );
    assert_eq!(
        shown("linear-gradient(red, blue) content-box"),
        [(content, Some(content))]
    );
    // The color shows where the last layer does.
    assert_eq!(
        shown("linear-gradient(red, blue) border-box content-box, gray padding-box"),
        [(padding, None), (content, Some(border))]
    );
}

#[test]
fn tiles_are_sized_placed_and_repeated_in_the_padding_box() {
    let tiles = |b: &str| tiled(&format!("linear-gradient(red, blue) {b}"))[0];
    // As big as the padding box, and repeated.
    assert_eq!(
        tiles(""),
        ([10.0, 10.0, 100.0, 50.0], [Some(100.0), Some(50.0)])
    );
    assert_eq!(tiles("0 0 / cover"), tiles(""));
    // Placed from a side, in the room the tile leaves.
    assert_eq!(
        tiles("right 10px bottom / 20px 50% no-repeat"),
        ([80.0, 35.0, 20.0, 25.0], [None, None])
    );
    assert_eq!(
        tiles("-5px 50% / 20px 10px repeat-y"),
        ([5.0, 30.0, 20.0, 10.0], [None, Some(10.0)])
    );
    // Spaced out from edge to edge, or one where the position puts it.
    assert_eq!(
        tiles("center / 30px 20px space"),
        ([10.0, 10.0, 30.0, 20.0], [Some(35.0), Some(30.0)])
    );
    assert_eq!(
        tiles("center / 60px space"),
        ([30.0, 10.0, 60.0, 50.0], [None, None])
    );
    // Rounded to whole tiles, an auto side kept in proportion.
    assert_eq!(
        tiles("0 0 / 30px 15px round"),
        ([10.0, 10.0, 33.333, 16.667], [Some(33.333), Some(16.667)])
    );
    assert_eq!(
        tiles("0 0 / 30px round no-repeat"),
        ([10.0, 10.0, 33.333, 55.556], [Some(33.333), None])
    );
    // Placed as if it were not rounded.
    assert_eq!(
        tiles("center / 28px 10px round"),
        ([46.0, 30.0, 25.0, 10.0], [Some(25.0), Some(10.0)])
    );
    // No tiles without a size.
    assert!(tiled("linear-gradient(red, blue) 0 0 / 0 10px").is_empty());
}

fn rounded(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

/// Where a linear ramp's line starts and ends.
fn line(ramp: &Ramp) -> [[f32; 2]; 2] {
    match ramp.geometry {
        Geometry::Line(ends) => ends.map(|p| p.map(rounded)),
        _ => panic!("{ramp:?}"),
    }
}

/// Where a ramp's segments start and end.
fn spans(ramp: &Ramp) -> Vec<[f32; 2]> {
    ramp.segments.iter().map(|s| s.at.map(rounded)).collect()
}

#[test]
fn gradients_paint_over_the_color_from_the_padding_box() {
    let (items, _) = paint_of(
        vec![view(
            &[
                ("width", "40px"),
                ("height", "20px"),
                ("border", "2px solid black"),
                (
                    "background",
                    "linear-gradient(red, blue), linear-gradient(red, green 50%, blue), gray",
                ),
            ],
            &[],
        )],
        &[0],
    );
    assert_eq!(
        names(&items),
        ["gray", "gradient of 2", "gradient of 1", "ring black"]
    );
    // An image that blends makes the background a layer of its own.
    let (items, _) = paint_of(
        vec![view(
            &[
                ("height", "20px"),
                (
                    "background",
                    "linear-gradient(red, blue), linear-gradient(red, green 50%, blue), gray",
                ),
                ("backgroundBlendMode", "multiply, normal"),
            ],
            &[],
        )],
        &[0],
    );
    assert_eq!(
        names(&items),
        [r#"group 1 ["gray", "gradient of 2", "group 1 Multiply [\"gradient of 1\"]"]"#]
    );
    let ramps = ramps("linear-gradient(red, blue)");
    let [(rect, tile, ramp)] = &ramps[..] else {
        panic!("{ramps:?}")
    };
    // Under the border too, the image the padding box.
    assert_eq!((rect.width, rect.height), (120.0, 70.0));
    assert_eq!(
        (tile.x, tile.y, tile.width, tile.height),
        (10.0, 10.0, 100.0, 50.0)
    );
    // Top to bottom through the middle.
    assert_eq!(line(ramp), [[50.0, 0.0], [50.0, 50.0]]);
    assert_eq!(spans(ramp), [[0.0, 1.0]]);
}

#[test]
fn gradient_lines_span_the_image_corner_to_corner() {
    let line = |g: &str| line(&ramps(g)[0].2);
    assert_eq!(
        line("linear-gradient(45deg, red, blue)"),
        [[12.5, 62.5], [87.5, -12.5]]
    );
    // Square to the diagonal from the top left to the bottom right, so both are halfway.
    assert_eq!(
        line("linear-gradient(to top right, red, blue)"),
        [[30.0, 65.0], [70.0, -15.0]]
    );
    assert_eq!(
        line("linear-gradient(to left, red, blue)"),
        [[100.0, 25.0], [0.0, 25.0]]
    );
}

#[test]
fn radial_gradients_end_at_a_side_a_corner_or_their_radii() {
    // The center and radii in an image 100 × 50 px, and where the stops are.
    let ellipse = |g: &str| {
        let ramp = ramps(g).remove(0).2;
        match ramp.geometry {
            Geometry::Ellipse(c, r) => (c.map(rounded), r.map(rounded), spans(&ramp)),
            _ => panic!("{ramp:?}"),
        }
    };
    let shape = |g: &str| {
        let (c, r, _) = ellipse(g);
        (c, r)
    };
    // Through the farthest corner, as the farthest sides are in proportion.
    assert_eq!(
        shape("radial-gradient(red, blue)"),
        ([50.0, 25.0], [70.711, 35.355])
    );
    assert_eq!(
        shape("radial-gradient(circle closest-side at 20px 10px, red, blue)"),
        ([20.0, 10.0], [10.0, 10.0])
    );
    assert_eq!(
        shape("radial-gradient(circle farthest-side at 20px 10px, red, blue)").1,
        [80.0, 80.0]
    );
    assert_eq!(
        shape("radial-gradient(circle at 20px 10px, red, blue)").1,
        [89.443, 89.443]
    );
    assert_eq!(
        shape("radial-gradient(closest-corner at 20px 10px, red, blue)").1,
        [28.284, 14.142]
    );
    assert_eq!(
        shape("radial-gradient(farthest-side at right 10px bottom 20%, red, blue)"),
        ([90.0, 40.0], [90.0, 40.0])
    );
    // Sides measured from a center outside the image.
    assert_eq!(
        shape("radial-gradient(closest-side at -10px 25px, red, blue)").1,
        [10.0, 25.0]
    );
    // Stops along the horizontal radius; percentages of the image's sides.
    assert_eq!(
        ellipse("radial-gradient(10px 50%, red 5px, blue)"),
        ([50.0, 25.0], [10.0, 25.0], vec![[0.5, 1.0]])
    );
    // A radius of 0 is the last color all around.
    assert_eq!(
        shape("radial-gradient(circle 0px, red, blue)").1,
        [0.001, 0.001]
    );
}

#[test]
fn conic_gradients_turn_around_their_center() {
    let ramp = ramps("conic-gradient(from 90deg at 20px 10px, red, blue 25%, green 180deg)")
        .remove(0)
        .2;
    assert_eq!(ramp.geometry, Geometry::Turn([20.0, 10.0], 0.25));
    // Stops are fractions of a turn.
    assert_eq!(spans(&ramp), [[0.0, 0.25], [0.25, 0.5]]);
    let ramp = ramps("repeating-conic-gradient(red 0 10deg, blue 10deg 20deg)")
        .remove(0)
        .2;
    assert_eq!(ramp.geometry, Geometry::Turn([50.0, 25.0], 0.0));
    assert_eq!(spans(&ramp), [[0.0, 0.028], [0.028, 0.028], [0.028, 0.056]]);
    assert_eq!(ramp.spread, Spread::Repeat);
}

#[test]
fn stops_go_where_css_puts_them() {
    let ramp = |g: &str| ramps(g).remove(0).2;
    let (red, green, blue) = (
        work(color::parse("red").unwrap()),
        work(color::parse("green").unwrap()),
        work(color::parse("blue").unwrap()),
    );
    // Black is no earlier than the hint before it, green halfway between blue and black, and
    // the hint at black keeps the way to it green.
    let stops = ramp("linear-gradient(to right, red, blue 30%, green, 60%, black 50px, gray)");
    assert_eq!(
        spans(&stops),
        [[0.0, 0.3], [0.3, 0.45], [0.45, 0.6], [0.6, 0.6], [0.6, 1.0]]
    );
    assert_eq!(stops.segments[2].colors, [green; 2]);
    // Half and half a quarter of the way along, lines between nine points on the curve.
    let hinted = ramp("linear-gradient(to right, red, 25%, blue)");
    assert_eq!(
        spans(&hinted)[..4],
        [[0.0, 0.083], [0.083, 0.167], [0.167, 0.25], [0.25, 0.308]]
    );
    assert_eq!(spans(&hinted)[9], [0.596, 1.0]);
    assert_eq!(
        hinted.segments[3].colors[0].map(rounded),
        [0.5, 0.0, 0.5, 1.0]
    );
    // Points closer together toward the stop the hint is near.
    let late = ramp("linear-gradient(to right, red, 75%, blue)");
    assert_eq!(
        spans(&late)[7..],
        [[0.75, 0.833], [0.833, 0.917], [0.917, 1.0]]
    );
    // A hint at a stop changes colors there; one halfway changes nothing.
    let sudden = ramp("linear-gradient(to right, red, 0%, blue)");
    assert_eq!(spans(&sudden), [[0.0, 0.0], [0.0, 1.0]]);
    assert_eq!(sudden.segments[1].colors, [blue; 2]);
    let even = ramp("linear-gradient(to right, red, 50%, blue)");
    assert_eq!(even.segments[0].colors, [red, blue]);
    let one = ramp("linear-gradient(red 20%)");
    assert_eq!((spans(&one), one.spread), (vec![[0.2, 0.2]], Spread::Pad));
    let repeating = ramp("repeating-linear-gradient(to right, red 10px, blue 30px)");
    assert_eq!(spans(&repeating), [[0.1, 0.3]]);
    assert_eq!(repeating.spread, Spread::Repeat);
    // Stops 0 px apart that repeat are the last color, as in Chrome.
    let flat = ramp("repeating-linear-gradient(red 10px, blue 10px)");
    assert_eq!(flat.segments[0].colors, [blue; 2]);
    assert_eq!(flat.spread, Spread::Pad);
}

#[test]
fn gradient_colors_mix_premultiplied_in_their_space() {
    let ends = |g: &str| ramps(g).remove(0).2.segments[0].colors;
    let both = |c: [[f32; 4]; 2]| c.map(|c| c.map(rounded));
    assert_eq!(
        ends("linear-gradient(rgb(255 0 0 / 50%), transparent)"),
        [[0.5, 0.0, 0.0, 0.5], [0.0; 4]]
    );
    assert_eq!(
        both(ends("linear-gradient(in srgb-linear, #808080, red)")),
        [[0.216, 0.216, 0.216, 1.0], [1.0, 0.0, 0.0, 1.0]]
    );
    // Oklab's red, as CSS Color 4 gives it.
    assert_eq!(
        both(ends("linear-gradient(in oklab, red, red)"))[0],
        [0.628, 0.225, 0.126, 1.0]
    );
    // Oklch: lightness and chroma premultiplied, hue in degrees. White's hue is powerless, so it
    // takes blue's; the shorter way from red to blue goes down through 0.
    let [white, blue] = ends("linear-gradient(in oklch, white, blue)");
    assert_eq!(white[2], blue[2]);
    let [red, blue] = ends("linear-gradient(in oklch, red, rgb(0 0 255 / 50%))");
    assert_eq!(
        [red, blue].map(|c| (c[2] * 10.0).round() / 10.0),
        [389.2, 264.1]
    );
    assert_eq!(rounded(blue[0]), 0.226);
    let [red, blue] = ends("linear-gradient(in oklch longer hue, red, blue)");
    assert_eq!([red, blue].map(|c| c[2].round()), [29.0, 264.0]);
    // CIE Lab's red, D50 white, as CSS Color 4 gives it; and in LCH a gray's hue is powerless.
    assert_eq!(
        both(ends("linear-gradient(in lab, red, red)"))[0],
        [54.291, 80.805, 69.891, 1.0]
    );
    // A wide color keeps its channels past sRGB until the gradient's pixels clip them.
    let [green, _] = ends("linear-gradient(in srgb, color(display-p3 0 1 0), red)");
    assert!(green[0] < 0.0 && green[1] > 1.0, "{green:?}");
    let [red, gray] = ends("linear-gradient(in lch, red, gray)");
    assert_eq!(
        [red, gray].map(|c| (c[2] * 10.0).round() / 10.0),
        [40.9, 40.9]
    );
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
            stroke: 0.0,
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
