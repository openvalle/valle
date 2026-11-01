use super::*;

use Segment::{Close, Cubic, Line, Move, Quad};

#[test]
fn path_data_takes_absolute_and_relative_commands() {
    let p = parse("M10 20 l5 0 H30 v-10 h-5 V0 Z").unwrap();
    assert_eq!(
        p.0,
        [
            Move([10.0, 20.0]),
            Line([15.0, 20.0]),
            Line([30.0, 20.0]),
            Line([30.0, 10.0]),
            Line([25.0, 10.0]),
            Line([25.0, 0.0]),
            Close,
        ]
    );
    // Numbers repeat the command, lines after a move; they need no separator where one is
    // clear.
    let p = parse("m1-2.5.5,3e1 1 1 L1,1 2,2").unwrap();
    assert_eq!(
        p.0,
        [
            Move([1.0, -2.5]),
            Line([1.5, 27.5]),
            Line([2.5, 28.5]),
            Line([1.0, 1.0]),
            Line([2.0, 2.0]),
        ]
    );
    // After a close, what follows starts where the subpath did.
    let p = parse("M1 1 L5 1 z l0 4").unwrap();
    assert_eq!(
        p.0,
        [
            Move([1.0, 1.0]),
            Line([5.0, 1.0]),
            Close,
            Move([1.0, 1.0]),
            Line([1.0, 5.0])
        ]
    );
}

#[test]
fn smooth_curves_reflect_the_control_point_before() {
    let p = parse("M0 0 C0 10 10 10 10 0 s10 -10 10 0 Q25 10 30 0 T40 0 S50 5 50 0").unwrap();
    assert_eq!(
        p.0,
        [
            Move([0.0, 0.0]),
            Cubic([0.0, 10.0], [10.0, 10.0], [10.0, 0.0]),
            Cubic([10.0, -10.0], [20.0, -10.0], [20.0, 0.0]),
            Quad([25.0, 10.0], [30.0, 0.0]),
            Quad([35.0, -10.0], [40.0, 0.0]),
            // Not after a cubic curve: its first control point is where it starts.
            Cubic([40.0, 0.0], [50.0, 5.0], [50.0, 0.0]),
        ]
    );
}

#[test]
fn arcs_are_cubic_curves_a_quarter_turn_at_most() {
    // Half a circle of radius 10 around (10, 0), counterclockwise on screen: through (10, 10).
    let p = parse("M0 0 A10 10 0 0 0 20 0").unwrap();
    let [Move(_), Cubic(a1, a2, mid), Cubic(_, _, end)] = p.0[..] else {
        panic!("{p:?}");
    };
    let near = |a: Point, b: Point| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
    assert!(near(mid, [10.0, 10.0]), "{mid:?}");
    let k = 4.0 / 3.0 * (std::f32::consts::FRAC_PI_8).tan() * 10.0;
    assert!(
        near(a1, [0.0, k]) && near(a2, [10.0 - k, 10.0]),
        "{a1:?} {a2:?}"
    );
    assert_eq!(end, [20.0, 0.0]);
    // Flags need no separator; radii too small grow to reach; a radius of 0 is a line.
    let p = parse("M0 0a1 1 0 1120 0").unwrap();
    let Cubic(_, _, mid) = p.0[1] else {
        panic!("{p:?}")
    };
    assert!(near(mid, [10.0, -10.0]), "{mid:?}");
    assert_eq!(parse("M0 0 A0 5 0 0 1 9 9").unwrap().0[1], Line([9.0, 9.0]));
}

#[test]
fn path_data_refuses_what_is_not_a_path() {
    for (d, e) in [
        ("", "it is empty"),
        ("L1 1", "it starts with a move"),
        ("M0 0 X", "X is not a command"),
        ("M0 0 L1", "L needs numbers"),
        ("M0 0 A1 1 0 2 0 1 1", "A needs flags"),
        ("1 2", "numbers need a command"),
        ("M0 0 Z 1 1", "numbers need a command"),
    ] {
        let got = parse(d).unwrap_err();
        assert!(got.contains(e), "{d}: {got}");
    }
}

#[test]
fn filled_paths_close_their_subpaths_as_quadratic_curves() {
    let p = parse("M0 0 H10 V10 M20 0 Q30 0 30 10 Z M5 5").unwrap();
    assert_eq!(
        p.quads(0.1),
        [
            vec![
                [[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]],
                [[10.0, 0.0], [10.0, 5.0], [10.0, 10.0]],
                [[10.0, 10.0], [5.0, 5.0], [0.0, 0.0]],
            ],
            vec![
                [[20.0, 0.0], [30.0, 0.0], [30.0, 10.0]],
                [[30.0, 10.0], [25.0, 5.0], [20.0, 0.0]],
            ],
        ]
    );
    assert_eq!(p.moved([1.0, 2.0]).0[0], Move([1.0, 2.0]));
}

#[test]
fn cubic_curves_split_into_quadratic_ones_within_the_tolerance() {
    let points = [[0.0, 0.0], [0.0, 100.0], [100.0, -50.0], [100.0, 50.0]];
    let on = |t: f32| -> Point {
        let s = 1.0 - t;
        std::array::from_fn(|k| {
            s * s * s * points[0][k]
                + 3.0 * s * s * t * points[1][k]
                + 3.0 * s * t * t * points[2][k]
                + t * t * t * points[3][k]
        })
    };
    for tolerance in [1.0, 0.1, 0.01] {
        let mut out = Vec::new();
        cubic(points, tolerance, &mut out);
        assert_eq!(out[0][0], points[0]);
        assert_eq!(out[out.len() - 1][2], points[3]);
        // Each piece ends where the next starts, and the curve's points are near them.
        assert!(out.windows(2).all(|w| w[0][2] == w[1][0]));
        let n = out.len() as f32;
        for (i, [a, c, b]) in out.iter().enumerate() {
            for j in 0..=8 {
                let u = j as f32 / 8.0;
                let q: Point = std::array::from_fn(|k| {
                    (1.0 - u) * (1.0 - u) * a[k] + 2.0 * (1.0 - u) * u * c[k] + u * u * b[k]
                });
                let p = on((i as f32 + u) / n);
                let d = (q[0] - p[0]).hypot(q[1] - p[1]);
                assert!(d <= tolerance, "{tolerance}: {d} at {i} {u}");
            }
        }
    }
}

#[test]
fn across_finds_where_curves_are_in_a_band() {
    // A diamond of lines from (0, 0) to (10, 10) and back, and a bowl from (0, 10) down to y 0
    // at x 5 and up to (10, 10).
    let line = |a: Point, b: Point| [a, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], b];
    let diamond = [
        line([5.0, 0.0], [10.0, 5.0]),
        line([10.0, 5.0], [5.0, 10.0]),
        line([5.0, 10.0], [0.0, 5.0]),
        line([0.0, 5.0], [5.0, 0.0]),
    ];
    assert_eq!(across(&diamond, 1.0, 2.0), Some([3.0, 7.0]));
    assert_eq!(across(&diamond, 4.0, 6.0), Some([0.0, 10.0]));
    assert_eq!(across(&diamond, 11.0, 12.0), None);
    let bowl = [[[0.0, 10.0], [5.0, -10.0], [10.0, 10.0]]];
    let [x0, x1] = across(&bowl, 0.0, 1.0).unwrap();
    // x = 10 t and y = 10 (1 - 2t)²: it touches 0 at x 5, and is at 1 where t is ½ ∓ √0.1 / 2.
    assert!(
        (x0 - 3.419).abs() < 0.001 && (x1 - 6.581).abs() < 0.001,
        "{x0} {x1}"
    );
}

/// The ends of a contour's curves.
fn ends(contour: &[Curve]) -> Vec<Point> {
    contour.iter().map(|c| c[2]).collect()
}

#[test]
fn strokes_miter_outer_corners_and_turn_inner_ones_through_the_corner() {
    let square = parse("M0 0 H10 V10 H0 Z").unwrap().quads(0.1);
    let sides = stroke(&square, 2.0, 0.01);
    assert_eq!(sides.len(), 2);
    // The square turns left: its left side is inside, and goes through its corners.
    let inside = ends(&sides[0]);
    for corner in [[10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]] {
        assert!(inside.contains(&corner), "{corner:?} in {inside:?}");
    }
    // Its right side, reversed, is mitered out to the square around it.
    let outside = ends(&sides[1]);
    for tip in [[-1.0, -1.0], [11.0, -1.0], [11.0, 11.0], [-1.0, 11.0]] {
        assert!(outside.contains(&tip), "{tip:?} in {outside:?}");
    }
    assert!(outside.iter().flatten().all(|v| (-1.0..=11.0).contains(v)));
    assert_eq!(sides[1][0][0], sides[1].last().unwrap()[2]);
    // A corner sharper than the miter limit (29°) is beveled: the miter would reach 20 px out.
    let sharp = parse("M0 0 L20 0 L0 2 Z").unwrap().quads(0.1);
    let reach = (stroke(&sharp, 2.0, 0.01).iter().flatten())
        .flat_map(|c| c.map(|p| p[0]))
        .fold(f32::MIN, f32::max);
    assert!(reach > 20.0 && reach < 21.0, "{reach}");
}

#[test]
fn strokes_move_curves_by_half_their_width_within_the_tolerance() {
    // A circle of radius 10 as 8 curves.
    let (r, k) = (10.0f32, std::f32::consts::FRAC_PI_4);
    let at = |a: f32, r: f32| [r * a.cos(), r * a.sin()];
    let circle: Vec<Curve> = (0..8)
        .map(|i| {
            let a = i as f32 * k;
            [at(a, r), at(a + k / 2.0, r / (k / 2.0).cos()), at(a + k, r)]
        })
        .collect();
    let sides = stroke(std::slice::from_ref(&circle), 4.0, 0.01);
    let samples = |curves: &[Curve], n: usize| -> Vec<Point> {
        (curves.iter())
            .flat_map(|&c| (0..=n).map(move |i| point(c, i as f32 / n as f32)))
            .collect()
    };
    let on = samples(&circle, 200);
    for side in &sides {
        assert!(side.len() <= 8 * 4, "{}", side.len());
        for p in samples(side, 8) {
            let d = (on.iter())
                .map(|q| (p[0] - q[0]).hypot(p[1] - q[1]))
                .fold(f32::MAX, f32::min);
            assert!((d - 2.0).abs() < 0.02, "{p:?} is {d} away");
        }
    }
}
