use super::*;

fn attributes(tag: Tag, list: &[(&str, V)]) -> Result<Computed, String> {
    let list: Vec<(&str, &V)> = list.iter().map(|(n, v)| (*n, v)).collect();
    computed(tag, &list)
}

fn text(s: &str) -> V {
    V::Str(s.into())
}

/// The smallest box around a path's points, control points too: left, top, right, bottom.
fn bounds(p: Path) -> [f32; 4] {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for s in &p.0 {
        let points: &[Point] = match s {
            Segment::Move(a) | Segment::Line(a) => std::slice::from_ref(a),
            Segment::Cubic(a, b, c) => &[*a, *b, *c],
            Segment::Quad(a, b) => &[*a, *b],
            Segment::Close => &[],
        };
        for p in points {
            b = [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ];
        }
    }
    b
}

#[test]
fn transform_lists_apply_right_to_left() {
    let m = |s: &str| {
        let c = attributes(Tag::G, &[("transform", text(s))]).unwrap();
        let m = c.transform.unwrap();
        [[0.0, 0.0], [1.0, 0.0]].map(|p| m.apply(p).map(|v| (v * 1000.0).round() / 1000.0))
    };
    assert_eq!(m("translate(10 20) scale(2)"), [[10.0, 20.0], [12.0, 20.0]]);
    assert_eq!(m("scale(2),translate(10,20)"), [[20.0, 40.0], [22.0, 40.0]]);
    assert_eq!(m("rotate(90 1 1)"), [[2.0, 0.0], [2.0, 1.0]]);
    assert_eq!(m("matrix(1 0 0 1 3 4) skewX(45)"), [[3.0, 4.0], [4.0, 4.0]]);
    let e = attributes(Tag::G, &[("transform", text("rotate(45deg)"))]).unwrap_err();
    assert!(e.contains("rotate() takes numbers without units"), "{e}");
}

#[test]
fn view_boxes_fit_their_viewport() {
    let c = attributes(
        Tag::Svg,
        &[
            ("viewBox", text("0 0 10 20")),
            ("preserveAspectRatio", text("xMaxYMin slice")),
        ],
    )
    .unwrap();
    let Element::Svg {
        view_box: Some(v),
        fit,
    } = c.element
    else {
        panic!("{c:?}")
    };
    assert_eq!(
        fit,
        Fit {
            align: Some([1.0, 0.0]),
            slice: true
        }
    );
    // 100 × 100: covered at 10 times, the bottom cut off.
    let m = viewport(v, fit, [100.0, 100.0]);
    assert_eq!(m.apply([10.0, 20.0]), [100.0, 200.0]);
    // Met at 5 times, in the middle.
    let m = viewport(v, MEET, [100.0, 100.0]);
    assert_eq!(m.apply([0.0, 0.0]), [25.0, 0.0]);
    let none = Fit {
        align: None,
        slice: false,
    };
    assert_eq!(
        viewport(v, none, [100.0, 100.0]).apply([10.0, 20.0]),
        [100.0, 100.0]
    );
    for (bad, e) in [("0 0 10", "is not 4 numbers"), ("0 0 -1 1", "negative")] {
        let err = attributes(Tag::Svg, &[("viewBox", text(bad))]).unwrap_err();
        assert!(err.starts_with("viewBox: ") && err.contains(e), "{err}");
    }
    let err = attributes(Tag::Svg, &[("preserveAspectRatio", text("xMidYmid"))]).unwrap_err();
    assert!(err.contains("is not an alignment"), "{err}");
}

#[test]
fn shapes_are_paths_in_their_viewport() {
    let shape = |tag, list: &[(&str, V)]| match attributes(tag, list).unwrap().element {
        Element::Shape(s) => s,
        e => panic!("{e:?}"),
    };
    let size = [200.0, 100.0];
    // r of a percentage of the normalized diagonal, √((200² + 100²) / 2) ≈ 158.1.
    let circle = shape(
        Tag::Circle,
        &[
            ("cx", text("50%")),
            ("cy", V::Num(50.0)),
            ("r", text("10%")),
        ],
    );
    let b = bounds(path(&circle, size).unwrap());
    assert!(
        (b[0] - 84.19).abs() < 0.01 && (b[3] - 65.81).abs() < 0.01,
        "{b:?}"
    );
    // A rect's radii: one given is both, and no more than half a side.
    let rect = shape(
        Tag::Rect,
        &[
            ("width", V::Num(20.0)),
            ("height", text("10px")),
            ("rx", V::Num(8.0)),
        ],
    );
    let p = path(&rect, size).unwrap();
    assert_eq!(p.0[0], Segment::Move([8.0, 0.0]));
    assert_eq!(p.0[3], Segment::Line([20.0, 5.0]));
    assert_eq!(
        path(&shape(Tag::Rect, &[("width", V::Num(5.0))]), size),
        None
    );
    let ellipse = shape(Tag::Ellipse, &[("ry", V::Num(4.0))]);
    assert_eq!(
        bounds(path(&ellipse, size).unwrap()),
        [-4.0, -4.0, 4.0, 4.0]
    );
    let polygon = shape(Tag::Polygon, &[("points", text("0,0 10,0 10 10"))]);
    assert_eq!(
        path(&polygon, size).unwrap().0.last(),
        Some(&Segment::Close)
    );
    for (tag, name, value, e) in [
        (Tag::Circle, "r", V::Num(-1.0), "r is negative"),
        (Tag::Rect, "x", text("1em"), "x: 1em is not a length"),
        (
            Tag::Polyline,
            "points",
            text("0 0 1"),
            "is not pairs of numbers",
        ),
        (Tag::Path, "d", text("L 1 1"), "it starts with a move"),
    ] {
        let err = attributes(tag, &[(name, value)]).unwrap_err();
        assert!(err.contains(e), "{err}");
    }
}

#[test]
fn strokes_are_outlines_with_caps_joins_and_dashes() {
    let line = path::parse("M0 0 H10").unwrap();
    let pen = |width: f64, cap: Cap| Pen {
        width,
        cap,
        join: Join::Miter,
        miter_limit: 4.0,
        dashes: Vec::new(),
        dash_offset: 0.0,
        path_length: None,
    };
    let near =
        |b: [f32; 4], want: [f32; 4]| (b.iter().zip(want)).all(|(a, w)| (a - w).abs() < 0.01);
    let outline = |p: &Path, pen: &Pen| stroke(p, pen, 1.0).unwrap();
    let b = bounds(outline(&line, &pen(2.0, Cap::Butt)));
    assert!(near(b, [0.0, -1.0, 10.0, 1.0]), "{b:?}");
    let b = bounds(outline(&line, &pen(2.0, Cap::Square)));
    assert!(near(b, [-1.0, -1.0, 11.0, 1.0]), "{b:?}");
    // The same in units 100 times smaller, 100 px each.
    let small = path::parse("M0 0 H0.1").unwrap();
    let b = bounds(stroke(&small, &pen(0.02, Cap::Square), 100.0).unwrap());
    assert!(near(b.map(|v| v * 100.0), [-1.0, -1.0, 11.0, 1.0]), "{b:?}");
    // A miter 1 / sin(θ / 2) widths long: past the limit at a sharp turn, a bevel.
    let sharp = path::parse("M0 0 L10 1 L0 2").unwrap();
    let b = bounds(outline(&sharp, &pen(2.0, Cap::Butt)));
    assert!(b[2] < 11.1, "{b:?}");
    let mitered = Pen {
        miter_limit: 30.0,
        ..pen(2.0, Cap::Butt)
    };
    assert!(bounds(outline(&sharp, &mitered))[2] > 19.0);
    // A subpath of no length: a dot with round or square caps, nothing with butt ones.
    let dot = path::parse("M5 5 Z").unwrap();
    let b = bounds(outline(&dot, &pen(4.0, Cap::Square)));
    assert!(near(b, [3.0, 3.0, 7.0, 7.0]), "{b:?}");
    // A round one's curves' control points are a little outside it.
    let b = bounds(outline(&dot, &pen(4.0, Cap::Round)));
    assert!(
        b[0] < 3.01 && b[0] > 2.7 && b[3] > 6.99 && b[3] < 7.3,
        "{b:?}"
    );
    assert_eq!(stroke(&dot, &pen(4.0, Cap::Butt), 1.0), None);
    // Dashes, each a subpath; one repeated; of no length, dots; by the path's own length.
    let moves = |p: Path| p.0.iter().filter(|s| matches!(s, Segment::Move(_))).count();
    let dashed = |dashes: &[f64], cap, path_length| Pen {
        dashes: dashes.to_vec(),
        path_length,
        ..pen(1.0, cap)
    };
    assert_eq!(
        moves(outline(&line, &dashed(&[2.0, 2.0], Cap::Butt, None))),
        3
    );
    assert_eq!(moves(outline(&line, &dashed(&[2.0], Cap::Butt, None))), 3);
    assert_eq!(
        moves(outline(&line, &dashed(&[0.0, 4.0], Cap::Round, None))),
        3
    );
    let by_length = dashed(&[4.0, 4.0], Cap::Butt, Some(20.0));
    assert_eq!(moves(outline(&line, &by_length)), 3);
    let solid = moves(outline(&line, &dashed(&[0.0, 0.0], Cap::Butt, None)));
    assert_eq!(solid, 1);
}

#[test]
fn path_lengths_are_above_0() {
    let c = attributes(Tag::Circle, &[("pathLength", V::Num(100.0))]).unwrap();
    assert_eq!(c.path_length, Some(100.0));
    let e = attributes(Tag::Circle, &[("pathLength", text("0"))]).unwrap_err();
    assert_eq!(e, "pathLength 0 is not a number above 0");
}
