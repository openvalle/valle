use super::*;
use crate::style::Sheet;
use crate::template::Node;
use crate::testing::{mask, text, text_node, view};

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
fn transformed_boxes_contain_absolute_and_fixed_boxes() {
    let at = |inset: [(&'static str, &'static str); 3]| {
        let mut style = vec![inset[0], inset[1], inset[2]];
        style.extend([("width", "10px"), ("height", "10px")]);
        view(&style, &[])
    };
    let l = lay(
        vec![
            view(
                &[
                    ("marginLeft", "20px"),
                    ("marginTop", "10px"),
                    ("width", "100px"),
                    ("height", "50px"),
                    ("transform", "rotate(10deg)"),
                ],
                &[1, 2],
            ),
            at([("position", "absolute"), ("right", "0"), ("bottom", "0")]),
            at([("position", "fixed"), ("left", "5px"), ("top", "5px")]),
        ],
        &[0],
    )
    .unwrap();
    // From its padding box, as laid out before it is transformed.
    assert_eq!(l.boxes[1].border, rect(110.0, 50.0, 10.0, 10.0));
    assert_eq!(l.boxes[2].border, rect(25.0, 15.0, 10.0, 10.0));
}

#[test]
fn a_mask_lays_out_its_source_in_its_border_box_as_the_canvas_is() {
    let l = lay(
        vec![
            mask(
                (2, false, false),
                &[
                    ("margin", "10px"),
                    ("width", "100px"),
                    ("height", "40px"),
                    ("border", "5px solid red"),
                    ("padding", "5px"),
                ],
                &[1],
            ),
            view(&[("height", "10px")], &[]),
            view(&[], &[3, 4]),
            view(&[("height", "50%")], &[]),
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
        &[0],
    )
    .unwrap();
    assert_eq!(l.boxes[1].border, rect(20.0, 20.0, 80.0, 10.0));
    // Its root is the mask's border box, which holds what is absolute in it.
    assert_eq!(l.boxes[2].border, rect(10.0, 10.0, 100.0, 40.0));
    assert_eq!(l.boxes[3].border, rect(10.0, 10.0, 100.0, 20.0));
    assert_eq!(l.boxes[4].border, rect(100.0, 40.0, 10.0, 10.0));
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
fn spans_are_boxes_of_their_text_on_its_lines() {
    let piece = |s: &str| template::Piece::Text(Prop::Const(V::Str(s.into())));
    let mut text = text_node("", &[("padding", "2px")]);
    text.kind = Kind::Text {
        content: vec![piece("Hi "), template::Piece::Span(1), piece(" there")],
    };
    text.children = vec![1];
    let mut span = view(&[("fontSize", "32px")], &[]);
    span.kind = Kind::Span {
        content: vec![piece("big")],
    };
    let l = lay(vec![text, span], &[0]).unwrap();
    let lines = l.lines[0].as_ref().unwrap();
    // As high as the big text: at 32px its ascent is 34 and its descent 9.
    assert_eq!(lines.boxes, [[0.0, 43.0]]);
    let big = lines.runs[1];
    assert_eq!(big.node, 1);
    let width = big.x[1] - big.x[0];
    assert_eq!(l.boxes[1].border, rect(2.0 + big.x[0], 2.0, width, 43.0));
}

#[test]
fn decorations_are_lines_on_runs_with_gaps_at_ink() {
    let style = |skip: &str| {
        let style = [
            ("fontSize", "20px"),
            ("lineHeight", "30px"),
            ("textDecoration", "underline line-through"),
            ("textDecorationSkipInk", skip),
        ];
        let l = lay(vec![text_node("Hugs", &style)], &[0]).unwrap();
        l.lines[0].clone().unwrap()
    };
    let lines = style("auto");
    let b = lines.baselines[0];
    let run = lines.runs[0];
    let (under, through): (Vec<&Stripe>, Vec<_>) =
        lines.decorations.iter().partition(|d| !d.through);
    // 2 px thick, 1 px under the baseline, and broken where the g goes through it, in whole px.
    let top = (b + 1.0).round();
    assert_eq!(under.len(), 2);
    for d in &under {
        assert_eq!([d.rect[1], d.rect[3]], [top, top + 2.0]);
    }
    assert_eq!([under[0].rect[0], under[1].rect[2]], run.x);
    let gap = [under[0].rect[2], under[1].rect[0]];
    let g = lines.glyphs[2].x;
    assert!(gap[0].fract() == 0.0 && gap[1].fract() == 0.0, "{gap:?}");
    assert!(
        gap[0] < g + 5.0 && g + 5.0 < gap[1],
        "{gap:?} for the g at {g}"
    );
    // A third of the ascent, 21 px, over the baseline, centered.
    assert_eq!(through.len(), 1);
    assert_eq!(through[0].rect[1], (b - 7.0 - 1.0).round());
    assert_eq!(style("none").decorations.len(), 2);
}

#[test]
fn decoration_styles_draw_two_lines_or_a_wave_or_marks_along_the_run() {
    let lines = |decoration: &str| {
        let style = [
            ("fontSize", "20px"),
            ("lineHeight", "30px"),
            ("textDecoration", decoration),
            ("textDecorationSkipInk", "none"),
        ];
        let l = lay(vec![text_node("mmm", &style)], &[0]).unwrap();
        l.lines[0].clone().unwrap()
    };
    let rows = |l: &Lines| -> Vec<[f32; 2]> {
        l.decorations
            .iter()
            .map(|d| [d.rect[1], d.rect[3]])
            .collect()
    };
    // A double line is two solid ones, a px past the thickness apart: under, and over upward.
    let solid = lines("underline 3px");
    let [top, bottom] = [solid.decorations[0].rect[1], solid.decorations[0].rect[3]];
    let double = lines("underline double 3px");
    assert_eq!(rows(&double), [[top, bottom], [top + 4.0, bottom + 4.0]]);
    assert!(
        double
            .decorations
            .iter()
            .all(|d| d.style == DecorationStyle::Solid)
    );
    let over = lines("overline double 3px");
    let [first, second] = [over.decorations[0].rect[1], over.decorations[1].rect[1]];
    assert_eq!(second, first - 4.0);
    // A wave is around an axis half a px under the line's top and as far again further out, in
    // a band its curve stays in; dots are spaced along the whole run.
    let wavy = lines("underline wavy 3px");
    let [y0, y1] = [wavy.decorations[0].rect[1], wavy.decorations[0].rect[3]];
    assert_eq!((y0 + y1) / 2.0, top + 0.5 + 4.0);
    assert!(y1 - y0 > 2.0 * 10.5);
    let dotted = lines("underline dotted");
    let d = dotted.decorations[0];
    assert_eq!(
        (d.style, d.along),
        (DecorationStyle::Dotted, dotted.runs[0].x)
    );
}

#[test]
fn a_decoration_takes_the_font_of_its_decorating_box() {
    let piece = |s: &str| template::Piece::Text(Prop::Const(V::Str(s.into())));
    let mut text = text_node("", &[("textDecoration", "line-through")]);
    text.kind = Kind::Text {
        content: vec![piece("aa"), template::Piece::Span(1)],
    };
    text.children = vec![1];
    let mut span = view(&[("fontSize", "32px"), ("textDecoration", "overline")], &[]);
    span.kind = Kind::Span {
        content: vec![piece("bb")],
    };
    let l = lay(vec![text, span], &[0]).unwrap();
    let lines = l.lines[0].as_ref().unwrap();
    let b = lines.baselines[0];
    let on = |node: u32| -> Vec<[f32; 2]> {
        (lines.decorations.iter())
            .filter(|d| lines.runs[d.run as usize].node == node)
            .map(|d| [d.rect[1], d.rect[3]])
            .collect()
    };
    // The Text's line-through, 1.6 px drawn as 1, is two thirds of the Text's ascent, 17 px,
    // under each run's ascent: the Text's 17 px, and the big Span's 34.
    let through = |ascent: f32| {
        let top = (b - ascent + 17.0 * 2.0 / 3.0 - 0.8).round();
        [top, top + 1.0]
    };
    assert_eq!(on(0), [through(17.0)]);
    // The Span's overline, 3.2 px drawn as 3, is on its ascent.
    let over = (b - 34.0 - 3.0).round();
    assert_eq!(on(1), [through(34.0), [over, over + 3.0]]);
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
