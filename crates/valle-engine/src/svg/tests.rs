use super::*;

fn attributes(tag: Tag, list: &[(&str, V)]) -> Result<Computed, String> {
    let list: Vec<(&str, &V)> = list.iter().map(|(n, v)| (*n, v)).collect();
    computed(tag, &list)
}

fn text(s: &str) -> V {
    V::Str(s.into())
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
    let bounds = |p: Path| {
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
