use super::*;
use crate::style::Sheet;
use crate::template::Node;
use crate::testing::{text, text_node, view};

/// Laid out on a 200 × 100 canvas.
fn lay(nodes: Vec<Node>, roots: &[u32]) -> Result<Layout, Error> {
    let template = Template {
        nodes,
        roots: roots.to_vec(),
        ..Default::default()
    };
    let styles = Sheet::new(&template)?.compute(&[], [200.0, 100.0])?;
    layout(&template, &[], &styles, [200.0, 100.0], &mut text())
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

#[test]
fn blocks_stack_and_fill_their_container() {
    let l = lay(
        vec![
            view(&[("height", "20px"), ("marginTop", "5px")], &[]),
            view(
                &[("width", "50%"), ("height", "30px"), ("marginLeft", "10px")],
                &[],
            ),
        ],
        &[0, 1],
    )
    .unwrap();
    // The canvas clips, so its first child's margin stays inside it.
    assert_eq!(l.boxes[0].border, rect(0.0, 5.0, 200.0, 20.0));
    assert_eq!(l.boxes[1].border, rect(10.0, 25.0, 100.0, 30.0));
}

#[test]
fn flex_rows_place_and_grow() {
    let row = [
        ("display", "flex"),
        ("padding", "10px"),
        ("gap", "5px"),
        ("height", "50px"),
    ];
    let l = lay(
        vec![
            view(&row, &[1, 2]),
            view(&[("width", "20px")], &[]),
            view(&[("flexGrow", "1")], &[]),
        ],
        &[0],
    )
    .unwrap();
    assert_eq!(l.boxes[0].content, rect(10.0, 10.0, 180.0, 30.0));
    assert_eq!(l.boxes[1].border, rect(10.0, 10.0, 20.0, 30.0));
    assert_eq!(l.boxes[2].border, rect(35.0, 10.0, 155.0, 30.0));
}

#[test]
fn flex_items_go_in_order_then_as_written() {
    let l = lay(
        vec![
            view(&[("display", "flex")], &[1, 2, 3, 4]),
            view(&[("width", "10px"), ("order", "1")], &[]),
            view(&[("width", "20px")], &[]),
            view(&[("width", "30px"), ("order", "-1")], &[]),
            view(&[("width", "40px")], &[]),
        ],
        &[0],
    )
    .unwrap();
    let x: Vec<f32> = (1..5).map(|i| l.boxes[i].border.x).collect();
    assert_eq!(x, [90.0, 30.0, 0.0, 50.0]);
}

#[test]
fn grids_place_items_on_tracks_and_lines() {
    let grid = [
        ("display", "grid"),
        ("position", "relative"),
        ("gridTemplateColumns", "[a] 50px [b] 1fr [c]"),
        ("gridTemplateRows", "20px"),
        ("gridAutoRows", "10px"),
        ("columnGap", "10px"),
    ];
    let l = lay(
        vec![
            view(&grid, &[1, 2, 3, 4]),
            view(&[("gridColumn", "span 2")], &[]),
            view(&[], &[]),
            view(&[("gridColumn", "b")], &[]),
            view(
                &[
                    ("position", "absolute"),
                    ("inset", "0"),
                    ("gridColumn", "b / c"),
                    ("gridRow", "1 / 2"),
                ],
                &[],
            ),
            view(&[("display", "flex")], &[6]),
            view(
                &[
                    ("display", "grid"),
                    ("gridTemplateColumns", "30px 20px"),
                    ("gridTemplateRows", "15px"),
                ],
                &[],
            ),
        ],
        &[0, 5],
    )
    .unwrap();
    assert_eq!(l.boxes[1].border, rect(0.0, 0.0, 200.0, 20.0));
    assert_eq!(l.boxes[2].border, rect(0.0, 20.0, 50.0, 10.0));
    assert_eq!(l.boxes[3].border, rect(60.0, 20.0, 140.0, 10.0));
    // An absolute box in a grid takes its grid area.
    assert_eq!(l.boxes[4].border, rect(60.0, 0.0, 140.0, 20.0));
    // A grid's tracks size it without items.
    assert_eq!(l.boxes[6].border, rect(0.0, 30.0, 50.0, 15.0));
}

#[test]
fn grid_areas_place_items_by_name() {
    let l = lay(
        vec![
            view(
                &[
                    ("display", "grid"),
                    (
                        "gridTemplate",
                        "'head head' 10px 'side main' 30px / 40px 1fr",
                    ),
                ],
                &[1, 2, 3],
            ),
            view(&[("gridArea", "main")], &[]),
            view(&[("gridArea", "head")], &[]),
            // An area names the lines around it too.
            view(&[("gridColumn", "side")], &[]),
        ],
        &[0],
    )
    .unwrap();
    assert_eq!(l.boxes[1].border, rect(40.0, 10.0, 160.0, 30.0));
    assert_eq!(l.boxes[2].border, rect(0.0, 0.0, 200.0, 10.0));
    assert_eq!(l.boxes[3].border, rect(0.0, 10.0, 40.0, 30.0));
}

#[test]
fn sizes_take_box_sizing_and_aspect_ratio() {
    let l = lay(
        vec![
            view(&[("width", "50px"), ("padding", "5px")], &[]),
            view(
                &[
                    ("width", "50px"),
                    ("padding", "5px"),
                    ("boxSizing", "content-box"),
                ],
                &[],
            ),
            view(&[("width", "50px"), ("aspectRatio", "5 / 2")], &[]),
        ],
        &[0, 1, 2],
    )
    .unwrap();
    assert_eq!(l.boxes[0].border, rect(0.0, 0.0, 50.0, 10.0));
    assert_eq!(l.boxes[1].border, rect(0.0, 10.0, 60.0, 10.0));
    assert_eq!(l.boxes[2].border, rect(0.0, 20.0, 50.0, 20.0));
}

#[test]
fn math_functions_resolve_against_what_their_percentages_are_of() {
    let l = lay(
        vec![
            view(
                &[
                    ("width", "calc(50% - 10px)"),
                    ("height", "min(10px, 50%)"),
                    ("paddingLeft", "calc(10% + 5px)"),
                ],
                &[],
            ),
            text_node(
                "Hi",
                &[
                    ("width", "max(20%, 10px)"),
                    ("paddingLeft", "calc(5% - 2px)"),
                ],
            ),
        ],
        &[0, 1],
    )
    .unwrap();
    assert_eq!(l.boxes[0].border, rect(0.0, 0.0, 90.0, 10.0));
    assert_eq!(l.boxes[0].content.x, 25.0);
    assert_eq!(l.boxes[1].border.width, 40.0);
    assert_eq!(l.boxes[1].content.x, 8.0);
}

#[test]
fn widths_from_content_break_text_or_not() {
    let l = lay(
        vec![
            view(&[("width", "min-content")], &[1]),
            text_node("Hi there", &[]),
            view(&[("width", "max-content")], &[3]),
            text_node("Hi there", &[]),
        ],
        &[0, 2],
    )
    .unwrap();
    let lines = |i: usize| l.lines[i].as_ref().map_or(0, |lines| lines.text.len());
    assert_eq!((lines(1), lines(3)), (2, 1));
    assert!(l.boxes[0].border.width < l.boxes[2].border.width);
    assert!(l.boxes[2].border.width < 100.0);
}

#[test]
fn absolute_boxes_take_the_nearest_positioned_ancestor() {
    let l = lay(
        vec![
            view(
                &[
                    ("position", "relative"),
                    ("left", "50px"),
                    ("top", "20px"),
                    ("width", "100px"),
                    ("height", "50px"),
                ],
                &[1],
            ),
            view(&[("padding", "10px")], &[2]),
            view(
                &[
                    ("position", "absolute"),
                    ("top", "5px"),
                    ("left", "5px"),
                    ("width", "10px"),
                    ("height", "10px"),
                ],
                &[],
            ),
            view(&[], &[4]),
            view(
                &[
                    ("position", "absolute"),
                    ("right", "0"),
                    ("bottom", "0"),
                    ("width", "10px"),
                    ("height", "10px"),
                ],
                &[],
            ),
        ],
        &[0, 3],
    )
    .unwrap();
    assert_eq!(l.boxes[0].border, rect(50.0, 20.0, 100.0, 50.0));
    assert_eq!(l.boxes[1].content, rect(60.0, 30.0, 80.0, 0.0));
    assert_eq!(l.boxes[2].border, rect(55.0, 25.0, 10.0, 10.0));
    // No positioned ancestor: the canvas contains it.
    assert_eq!(l.boxes[4].border, rect(190.0, 90.0, 10.0, 10.0));
}

#[test]
fn fixed_boxes_take_the_canvas() {
    let fixed = [("position", "fixed"), ("width", "10px"), ("height", "10px")];
    let l = lay(
        vec![
            view(
                &[
                    ("position", "relative"),
                    ("left", "50px"),
                    ("top", "20px"),
                    ("padding", "10px"),
                    ("height", "50px"),
                ],
                &[1, 2],
            ),
            view(
                &[
                    ("right", "0"),
                    ("bottom", "0"),
                    fixed[0],
                    fixed[1],
                    fixed[2],
                ],
                &[],
            ),
            view(&fixed, &[3]),
            view(
                &[
                    ("position", "absolute"),
                    ("left", "50%"),
                    ("width", "10px"),
                    ("height", "10px"),
                ],
                &[],
            ),
        ],
        &[0],
    )
    .unwrap();
    assert_eq!(l.boxes[1].border, rect(190.0, 90.0, 10.0, 10.0));
    // Without insets, where it would be in flow.
    assert_eq!(l.boxes[2].border, rect(60.0, 30.0, 10.0, 10.0));
    // A fixed box is positioned, and contains absolute boxes.
    assert_eq!(l.boxes[3].border, rect(65.0, 30.0, 10.0, 10.0));
}

#[test]
fn text_is_measured_and_set_in_its_box() {
    let l = lay(
        vec![
            view(&[("width", "60px")], &[1]),
            text_node("one two three", &[("padding", "2px")]),
            view(&[("display", "flex")], &[3]),
            text_node("Hi", &[]),
        ],
        &[0, 2],
    )
    .unwrap();
    let wrapped = l.lines[1].as_ref().unwrap();
    assert!(wrapped.boxes.len() >= 2);
    let height = 22.0 * wrapped.boxes.len() as f32;
    assert_eq!(l.boxes[1].border, rect(0.0, 0.0, 60.0, height + 4.0));
    assert_eq!(l.boxes[1].content, rect(2.0, 2.0, 56.0, height));
    // A flex item shrinks to fit its text.
    let hi = l.boxes[3].border;
    assert!(hi.width > 0.0 && hi.width < 30.0, "{hi:?}");
    assert_eq!([hi.x, hi.y, hi.height], [0.0, height + 4.0, 22.0]);
    assert!(l.lines[0].is_none());
}

#[test]
fn flex_items_align_on_the_first_baseline_of_text() {
    let l = lay(
        vec![
            view(
                &[("display", "flex"), ("alignItems", "baseline")],
                &[1, 2, 3],
            ),
            view(&[("width", "20px"), ("height", "6px")], &[]),
            text_node("Ag", &[("paddingTop", "3px")]),
            view(&[("width", "20px")], &[]),
        ],
        &[0],
    )
    .unwrap();
    let baseline = l.boxes[2].content.y + l.lines[2].as_ref().unwrap().glyphs[0].y;
    assert_eq!(l.boxes[2].border.y, 0.0);
    // A box without text aligns its bottom edge, an empty one its top.
    assert_eq!(l.boxes[1].border.y + 6.0, baseline);
    assert_eq!(l.boxes[3].border.y, baseline);
}

#[test]
fn hidden_subtrees_take_no_room() {
    let l = lay(
        vec![
            view(&[("display", "none"), ("height", "50px")], &[1]),
            text_node("hidden", &[]),
            view(&[("height", "10px")], &[]),
        ],
        &[0, 2],
    )
    .unwrap();
    assert_eq!(l.boxes[0], Boxes::default());
    assert_eq!(l.boxes[1], Boxes::default());
    assert!(l.lines[1].is_none());
    assert_eq!(l.boxes[2].border, rect(0.0, 0.0, 200.0, 10.0));
}

#[test]
fn text_errors_point_at_the_node() {
    let e = lay(vec![text_node("a", &[("fontFamily", "serif")])], &[0]);
    assert_eq!(
        e.err().unwrap(),
        "s.tsx:1:1: font family serif is not built in yet"
    );
}
