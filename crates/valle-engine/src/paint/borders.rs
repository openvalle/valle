//! Rounded borders, outlines and shadows.
use super::{Item, Radii, SQUARE, work};

use crate::layout::Rect;
use crate::style::color::{self, Rgba};
use crate::style::{BorderStyle, Shadow};

/// A corner's radii: square if either is 0.
pub(super) fn corner(r: [f32; 2]) -> [f32; 2] {
    if r.contains(&0.0) { [0.0; 2] } else { r }
}

/// `radii` grown by `by` px, or shrunk where it is negative; square corners stay square.
pub(super) fn grown(radii: Radii, by: f32) -> Radii {
    radii.map(|r| corner(r.map(|v| if v > 0.0 { (v + by).max(0.0) } else { 0.0 })))
}

/// `radii` grown by a shadow's spread, or shrunk where it is negative. A radius smaller than the
/// spread grows less, as CSS has it, so a small one stays nearly square.
fn spread(radii: Radii, by: f32) -> Radii {
    if by <= 0.0 {
        return grown(radii, by);
    }
    radii.map(|r| {
        corner(r.map(|v| match v {
            v if v <= 0.0 => 0.0,
            v if v >= by => v + by,
            v => v + by * (1.0 + (v / by - 1.0).powi(3)),
        }))
    })
}

/// `rect` grown by `by` px on each side.
pub(super) fn outset(rect: Rect, by: f32) -> Rect {
    Rect {
        x: rect.x - by,
        y: rect.y - by,
        width: rect.width + 2.0 * by,
        height: rect.height + 2.0 * by,
    }
}

/// An outer shadow of a box whose border box is `rect` with its corners rounded by `radii`: its
/// shape moved by the offset and grown by the spread, blurred, outside the box. Under an
/// `opaque` background it goes a px into the box, as in Chrome, so that no gap shows at the
/// box's antialiased edge: the box shrinks by a px, its radii by as much as its shorter side.
pub(super) fn outer_shadow(s: &Shadow, rect: Rect, radii: Radii, opaque: bool) -> Option<Item> {
    let [x, y] = s.offset.map(|v| v as f32);
    let by = s.spread as f32;
    let shape = outset(
        Rect {
            x: rect.x + x,
            y: rect.y + y,
            ..rect
        },
        by,
    );
    let color = work(s.color);
    if color[3] <= 0.0 || shape.width <= 0.0 || shape.height <= 0.0 {
        return None;
    }
    let radii_out = fit(spread(radii, by), shape);
    let (rect, radii) = match opaque {
        true => {
            let less = outset(rect, -1.0);
            let k = match less.width < less.height {
                true => less.width / rect.width,
                false => less.height / rect.height,
            };
            (less, radii.map(|r| r.map(|v| v * k.max(0.0))))
        }
        false => (rect, radii),
    };
    Some(Item::Shadow {
        sigma: s.blur as f32 / 2.0,
        rect,
        radii,
        inside: false,
        items: vec![Item::Rect {
            rect: shape,
            radii: radii_out,
            color,
        }],
    })
}

/// An inner shadow of a box whose padding box is `rect` with its corners rounded by `radii`: all
/// but the padding box's shape moved by the offset and shrunk by the spread, blurred, inside the
/// padding box.
pub(super) fn inner_shadow(s: &Shadow, rect: Rect, radii: Radii) -> Option<Item> {
    let [x, y] = s.offset.map(|v| v as f32);
    let by = s.spread as f32;
    let hole = outset(
        Rect {
            x: rect.x + x,
            y: rect.y + y,
            ..rect
        },
        -by,
    );
    let color = work(s.color);
    if color[3] <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
        return None;
    }
    let sigma = s.blur as f32 / 2.0;
    // Out past what the blur reaches from the padding box, which sees only shadow there.
    let around = outset(rect, (3.0 * sigma).ceil() + 1.0);
    Some(Item::Shadow {
        sigma,
        rect,
        radii,
        inside: true,
        items: vec![Item::Ring {
            rect: around,
            radii: SQUARE,
            inner: hole,
            inner_radii: fit(spread(radii, -by), hole),
            colors: [color; 4],
        }],
    })
}

/// `radii` all shrunk by one factor until they fit in `rect`, as CSS has them. None fit in a
/// rectangle of no size.
pub(super) fn fit(radii: Radii, rect: Rect) -> Radii {
    let [tl, tr, br, bl] = radii;
    // NaN where a side and its radii are 0, which `min` passes over.
    let fit = [
        rect.width / (tl[0] + tr[0]),
        rect.width / (bl[0] + br[0]),
        rect.height / (tl[1] + bl[1]),
        rect.height / (tr[1] + br[1]),
    ];
    let k = fit.into_iter().fold(1.0, f32::min).max(0.0);
    radii.map(|r| r.map(|v| v * k))
}

/// A line across a side of a border or an outline: from and to how deep in it, and its color.
pub(super) struct Line {
    pub(super) depth: [f32; 2],
    pub(super) color: [f32; 4],
}

/// The lines across each side of a border or an outline `widths` wide in `styles` and `colors`, as
/// Chrome draws them. A double line is two lines and a gap, about a third of its width each (the
/// gap from (w + 1) / 3 to (2w + 1) / 3 deep, rounded down), or one under 3 px. An inset line is
/// in the darker shade of its color at the top and left and the lighter at the bottom and right,
/// an outset line the other way round; a groove is an inset line outside an outset one, a ridge
/// the other way round, the side split with the odd px at its top or left, or solid under 2 px.
/// An opaque outer half goes on under the inner one, which leaves no seam where their antialiased
/// edges meet; Chrome's translucent ones meet in a layer, which is left out. Dots and dashes are
/// no lines across a side, but marks along it.
pub(super) fn lines(
    widths: [f32; 4],
    styles: [BorderStyle; 4],
    colors: [Rgba; 4],
) -> [Vec<Line>; 4] {
    std::array::from_fn(|k| {
        let (w, c) = (widths[k], colors[k]);
        let top_left = k == 0 || k == 3;
        let shade = |inset: bool| work(color::shades(c)[usize::from(inset != top_left)]);
        let half = if top_left {
            (w / 2.0).ceil()
        } else {
            (w / 2.0).floor()
        };
        let line = |from, to, color| Line {
            depth: [from, to],
            color,
        };
        let under = if c[3] >= 1.0 { w } else { half };
        match styles[k] {
            BorderStyle::Double if w >= 3.0 => vec![
                line(0.0, ((w + 1.0) / 3.0).floor(), work(c)),
                line(((2.0 * w + 1.0) / 3.0).floor(), w, work(c)),
            ],
            BorderStyle::Groove if w >= 2.0 => {
                vec![line(0.0, under, shade(true)), line(half, w, shade(false))]
            }
            BorderStyle::Ridge if w >= 2.0 => {
                vec![line(0.0, under, shade(false)), line(half, w, shade(true))]
            }
            BorderStyle::Inset => vec![line(0.0, w, shade(true))],
            BorderStyle::Outset => vec![line(0.0, w, shade(false))],
            BorderStyle::Dotted | BorderStyle::Dashed => Vec::new(),
            _ => vec![line(0.0, w, work(c))],
        }
    })
}

/// The rings a border or an outline `widths` deep inside `rect` paints: its sides' `lines`.
pub(super) fn rings(
    rect: Rect,
    radii: Radii,
    widths: [f32; 4],
    lines: &[Vec<Line>; 4],
) -> Vec<Item> {
    let count = lines.iter().map(Vec::len).max().unwrap_or(0);
    (0..count)
        .map(|n| {
            // A side with fewer lines has none here. One of dots or dashes has none at all, but
            // keeps its part of the border, where the next sides' corners stop.
            let line = |k: usize| lines[k].get(n);
            let none = |k: usize| match lines[k].is_empty() {
                true => [0.0, widths[k]],
                false => [widths[k]; 2],
            };
            let depth = |k: usize| line(k).map_or(none(k), |l| l.depth);
            let (outer, outer_radii) = inner(rect, radii, std::array::from_fn(|k| depth(k)[0]));
            let (inside, inner_radii) = inner(rect, radii, std::array::from_fn(|k| depth(k)[1]));
            Item::Ring {
                rect: outer,
                radii: outer_radii,
                inner: inside,
                inner_radii,
                colors: std::array::from_fn(|k| line(k).map_or([0.0; 4], |l| l.color)),
            }
        })
        .collect()
}

/// The inside of a border `widths` wide (top, right, bottom and left) on `rect` with its corners
/// rounded by `radii`: a rectangle, of a negative size where the border fills `rect`, and its
/// corners' radii, the outer ones less the widths.
pub(super) fn inner(rect: Rect, radii: Radii, widths: [f32; 4]) -> (Rect, Radii) {
    let [t, r, b, l] = widths;
    let inside = Rect {
        x: rect.x + l,
        y: rect.y + t,
        width: rect.width - l - r,
        height: rect.height - t - b,
    };
    let less = [[l, t], [r, t], [r, b], [l, b]];
    let radii =
        std::array::from_fn(|c| corner([0, 1].map(|k| (radii[c][k] - less[c][k]).max(0.0))));
    (inside, fit(radii, inside))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::style::color;

    use crate::testing::view;

    use crate::paint::*;
    use crate::testing::paint::{names, paint_of};

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
}
