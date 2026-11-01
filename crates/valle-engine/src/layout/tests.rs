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
    let styles = Sheet::new(&template)?.compute(&[])?;
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
