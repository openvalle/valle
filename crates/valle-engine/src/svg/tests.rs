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

#[test]
fn gradients_take_their_attributes_along_their_hrefs() {
    let gradient = |tag, list: &[(&str, V)]| match attributes(tag, list).unwrap().element {
        Element::Gradient(g) => g,
        e => panic!("{e:?}"),
    };
    let own = gradient(
        Tag::LinearGradient,
        &[
            ("href", text("#base")),
            ("x2", text("50%")),
            ("spreadMethod", text("reflect")),
        ],
    );
    let base = gradient(
        Tag::LinearGradient,
        &[
            ("x1", V::Num(0.25)),
            ("x2", V::Num(1.0)),
            ("gradientUnits", text("objectBoundingBox")),
            ("gradientTransform", text("translate(1 0)")),
        ],
    );
    assert_eq!(own.href.as_deref(), Some("base"));
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];
    // Offsets never go back; the ends' colors reach 0 and 1.
    let stops = [(0.5, red), (0.25, blue)];
    let bbox = [10.0, 20.0, 100.0, 50.0];
    let (ramp, m) = gradient_of(&[&own, &base], &stops, bbox);
    assert_eq!(ramp.geometry, Geometry::Line([[0.25, 0.0], [0.5, 0.0]]));
    assert_eq!(ramp.spread, Spread::Reflect);
    assert!(!ramp.premultiplied);
    let spans: Vec<[f32; 2]> = ramp.segments.iter().map(|s| s.at).collect();
    assert_eq!(spans, [[0.0, 0.5], [0.5, 0.5], [0.5, 1.0]]);
    // Fractions of the bounding box, after the gradient's transform.
    assert_eq!(m.apply([0.0, 1.0]), [110.0, 70.0]);
    // In user units, percentages of the viewport; a radius of its normalized diagonal.
    let radial = gradient(
        Tag::RadialGradient,
        &[
            ("gradientUnits", text("userSpaceOnUse")),
            ("r", text("10%")),
            ("fx", V::Num(10.0)),
        ],
    );
    let (ramp, _) = super::gradient(&[&radial], &stops, bbox, [200.0, 100.0]).unwrap();
    let Geometry::Focal {
        focus,
        center,
        radius,
        ..
    } = ramp.geometry
    else {
        panic!("{:?}", ramp.geometry)
    };
    assert_eq!((focus, center), ([10.0, 50.0], [100.0, 50.0]));
    assert!((radius - 15.81).abs() < 0.01, "{radius}");
    // No stops, or a bounding box without area for its units: nothing.
    assert_eq!(super::gradient(&[&base], &[], bbox, [1.0; 2]), None);
    let flat = [10.0, 20.0, 100.0, 0.0];
    assert_eq!(super::gradient(&[&base], &stops, flat, [1.0; 2]), None);
    let e = attributes(Tag::RadialGradient, &[("r", V::Num(-1.0))]).unwrap_err();
    assert_eq!(e, "r is negative");
    let e = attributes(Tag::LinearGradient, &[("href", text("other.svg#g"))]).unwrap_err();
    assert!(e.contains("is not #id"), "{e}");
    let e = attributes(Tag::LinearGradient, &[("transform", text("scale(2)"))]).unwrap_err();
    assert!(e.contains("takes gradientTransform"), "{e}");
    let stop = |v: V| attributes(Tag::Stop, &[("offset", v)]).map(|c| c.element);
    assert_eq!(stop(text("150%")), Ok(Element::Stop(1.0)));
    assert_eq!(stop(V::Num(0.3)), Ok(Element::Stop(0.3)));
    assert!(stop(text("a")).is_err());
}

#[test]
fn clip_paths_and_masks_take_their_units_and_regions() {
    let element = |tag, list: &[(&str, V)]| attributes(tag, list).map(|c| c.element);
    assert_eq!(
        element(Tag::ClipPath, &[]),
        Ok(Element::ClipPath { user_space: true })
    );
    let units = [("clipPathUnits", text("objectBoundingBox"))];
    assert_eq!(
        element(Tag::ClipPath, &units),
        Ok(Element::ClipPath { user_space: false })
    );
    let clip = attributes(Tag::ClipPath, &[("transform", text("scale(2)"))]).unwrap();
    assert!(clip.transform.is_some());
    let percent = |percent| Length {
        units: 0.0,
        percent,
    };
    let Ok(Element::Mask(m)) = element(Tag::Mask, &[]) else {
        panic!()
    };
    assert!(!m.user_space && m.content_user_space);
    assert_eq!(
        m.region,
        [
            percent(-10.0),
            percent(-10.0),
            percent(120.0),
            percent(120.0)
        ]
    );
    let list = [
        ("maskUnits", text("userSpaceOnUse")),
        ("maskContentUnits", text("objectBoundingBox")),
        ("x", V::Num(5.0)),
        ("height", text("50%")),
    ];
    let Ok(Element::Mask(m)) = element(Tag::Mask, &list) else {
        panic!()
    };
    assert!(m.user_space && !m.content_user_space);
    assert_eq!(
        m.region[0],
        Length {
            units: 5.0,
            percent: 0.0
        }
    );
    assert_eq!(m.region[3], percent(50.0));
    let e = element(Tag::Mask, &[("width", V::Num(-1.0))]).unwrap_err();
    assert_eq!(e, "width is negative");
    let e = element(Tag::Mask, &[("transform", text("scale(2)"))]).unwrap_err();
    assert!(e.contains("<mask> takes none"), "{e}");
    let e = element(Tag::ClipPath, &[("clipPathUnits", text("px"))]).unwrap_err();
    assert_eq!(
        e,
        "clipPathUnits: px is not userSpaceOnUse or objectBoundingBox"
    );
    assert!(presentational(Tag::Rect, "clipPath") && presentational(Tag::Mask, "maskType"));
}

#[test]
fn uses_name_what_they_copy_and_symbols_their_view_box() {
    let element = |tag, list: &[(&str, V)]| attributes(tag, list).map(|c| c.element);
    let list = [
        ("href", text("#icon")),
        ("x", V::Num(5.0)),
        ("width", text("50%")),
    ];
    let Ok(Element::Use { href, at, size }) = element(Tag::Use, &list) else {
        panic!()
    };
    assert_eq!(&*href, "icon");
    assert_eq!(at[0].units, 5.0);
    assert_eq!(size[0].map(|l| l.percent), Some(50.0));
    assert_eq!(size[1], None);
    let e = element(Tag::Use, &[]).unwrap_err();
    assert!(e.contains("write href=\"#id\""), "{e}");
    let e = element(Tag::Use, &[("href", text("#a")), ("height", V::Num(-1.0))]).unwrap_err();
    assert_eq!(e, "height is negative");
    let list = [
        ("viewBox", text("0 0 10 20")),
        ("preserveAspectRatio", text("none")),
    ];
    let Ok(Element::Symbol { view_box, fit }) = element(Tag::Symbol, &list) else {
        panic!()
    };
    assert_eq!(view_box, Some([0.0, 0.0, 10.0, 20.0]));
    assert_eq!(fit.align, None);
    let e = element(Tag::Symbol, &[("transform", text("scale(2)"))]).unwrap_err();
    assert!(e.contains("<symbol> takes none"), "{e}");
}

#[test]
fn markers_go_at_vertices_turned_along_the_path() {
    let at = |tag, list: &[(&str, V)]| {
        let Element::Shape(shape) = attributes(tag, list).unwrap().element else {
            panic!()
        };
        let p = path(&shape, [100.0; 2]).unwrap();
        vertices(&shape, &p)
    };
    let turns = |v: Vec<(Point, f64)>| -> Vec<f64> {
        v.iter().map(|&(_, a)| (a + 360.0) % 360.0).collect()
    };
    // Open: the ends along their segments, a corner between the two.
    let open = at(Tag::Path, &[("d", text("M0 0 L10 0 L10 10"))]);
    let points: Vec<Point> = open.iter().map(|&(p, _)| p).collect();
    assert_eq!(points, [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
    assert_eq!(turns(open), [0.0, 45.0, 90.0]);
    // An arc is one command, so one vertex, though it is curves.
    let arc = turns(at(Tag::Path, &[("d", text("M0 0 A10 10 0 0 1 0 20"))]));
    assert_eq!(arc.len(), 2);
    let near = |a: f64, b: f64| ((a - b + 540.0) % 360.0 - 180.0).abs() < 1e-3;
    assert!(near(arc[0], 0.0) && near(arc[1], 180.0), "{arc:?}");
    // Closed: the close is a vertex at the start, and the start comes in from it.
    let closed = at(Tag::Polygon, &[("points", text("0,0 10,0 10,10"))]);
    assert_eq!(closed.len(), 4);
    assert_eq!(turns(closed), [292.5, 45.0, 157.5, 292.5]);
    let none = at(
        Tag::Rect,
        &[("width", V::Num(4.0)), ("height", V::Num(4.0))],
    );
    assert!(none.is_empty());
    let marker = |list: &[(&str, V)]| match attributes(Tag::Marker, list).map(|c| c.element) {
        Ok(Element::Marker(m)) => Ok(m),
        Ok(e) => panic!("{e:?}"),
        Err(e) => Err(e),
    };
    let m = marker(&[]).unwrap();
    assert_eq!(
        (m.size[0].units, m.orient, m.user_space),
        (3.0, Orient::Angle(0.0), false)
    );
    for (orient, want) in [
        ("auto", Orient::Auto),
        ("auto-start-reverse", Orient::AutoStartReverse),
        ("0.25turn", Orient::Angle(90.0)),
        ("-30", Orient::Angle(-30.0)),
    ] {
        assert_eq!(marker(&[("orient", text(orient))]).unwrap().orient, want);
    }
    let e = marker(&[("orient", text("up"))]).unwrap_err();
    assert_eq!(e, "orient: up is not auto, auto-start-reverse or an angle");
    let e = marker(&[("markerUnits", text("px"))]).unwrap_err();
    assert!(e.contains("is not strokeWidth or userSpaceOnUse"), "{e}");
}

/// A gradient over `bbox` in a viewport 100 units a side.
fn gradient_of(chain: &[&Gradient], stops: &[(f64, Rgba)], bbox: [f64; 4]) -> (Ramp, Matrix) {
    super::gradient(chain, stops, bbox, [100.0; 2]).unwrap()
}
