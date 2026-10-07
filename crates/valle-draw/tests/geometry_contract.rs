use valle_draw::{
    FontStyle, FontWeight, HAlign, Hit, Locatable, LocateError, PlacedText, Point, Rect,
    ResolvedTextStyle, Rgba, TextMetrics, TextStyle, VAlign, Vec2,
    program::{
        Affine2d, DrawProgramBuilder, FillRule, Node, PathData, PathNode, PathVerb, Transform2d,
    },
};

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
}

fn same_point(actual: Point, expected: Point) {
    near(actual.x, expected.x);
    near(actual.y, expected.y);
}

#[test]
fn affine_and_projective_composition_invert_at_arbitrary_points() {
    let affine = Affine2d::scale(2.0, 3.0)
        .then(Affine2d::rotate(30.0))
        .then(Affine2d::translate(17.0, -9.0));
    let projective = Transform2d::from_affine(affine);
    let inverse = Transform2d::from_affine(affine.inverse().unwrap());
    for point in [
        Point::new(0.0, 0.0),
        Point::new(12.0, -3.0),
        Point::new(-8.0, 27.0),
    ] {
        same_point(
            inverse
                .try_apply(projective.try_apply(point).unwrap())
                .unwrap(),
            point,
        );
        same_point(projective.then(inverse).try_apply(point).unwrap(), point);
        same_point(
            projective
                .inverse()
                .unwrap()
                .try_apply(projective.try_apply(point).unwrap())
                .unwrap(),
            point,
        );
        same_point(Transform2d::default().try_apply(point).unwrap(), point);
    }
    assert_eq!(Affine2d::default(), Affine2d::IDENTITY);
    assert!(Affine2d::scale(0.0, 1.0).inverse().is_none());
    assert!(Affine2d::scale(f64::INFINITY, 1.0).inverse().is_none());
    assert!(Transform2d([0.0; 9]).inverse().is_none());
    assert!(Transform2d([f64::NAN; 9]).inverse().is_none());
    assert!(
        Transform2d([0.0; 9])
            .try_apply(Point::new(1.0, 2.0))
            .is_none()
    );
}

#[test]
fn quadrilateral_mapping_preserves_corners_and_encloses_edges() {
    let source = Rect::new(10.0, 20.0, 100.0, 60.0);
    let corners = [
        Point::new(10.0, 20.0),
        Point::new(110.0, 20.0),
        Point::new(110.0, 80.0),
        Point::new(10.0, 80.0),
    ];
    let destinations = [
        [
            Point::new(20.0, 30.0),
            Point::new(170.0, 50.0),
            Point::new(140.0, 110.0),
            Point::new(40.0, 90.0),
        ],
        [
            Point::new(-10.0, 20.0),
            Point::new(90.0, 20.0),
            Point::new(90.0, 80.0),
            Point::new(-10.0, 80.0),
        ],
    ];
    for destination in destinations {
        let mapping = Transform2d::map_rect(source, destination).unwrap();
        let inverse = mapping.inverse().unwrap();
        let bounds = mapping.map_bounds(source).unwrap();
        for (input, output) in corners.into_iter().zip(destination) {
            same_point(mapping.try_apply(input).unwrap(), output);
            same_point(inverse.try_apply(output).unwrap(), input);
            assert!(output.x >= bounds.left() - 1e-8 && output.x <= bounds.right() + 1e-8);
            assert!(output.y >= bounds.top() - 1e-8 && output.y <= bounds.bottom() + 1e-8);
        }
    }
    assert!(Transform2d::map_rect(Rect::new(0.0, 0.0, 0.0, 1.0), corners).is_none());
    assert!(Transform2d::map_quad(corners, [Point::new(1.0, 1.0); 4]).is_none());
    let mut invalid = corners;
    invalid[2].x = f64::NAN;
    assert!(Transform2d::map_quad(corners, invalid).is_none());
    assert!(
        Transform2d([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, -50.0])
            .map_bounds(source)
            .is_none()
    );
    assert_eq!(
        Transform2d::IDENTITY.map_bounds(Rect::new(0.0, 0.0, 0.0, 0.0)),
        Some(Rect::new(0.0, 0.0, 0.0, 0.0))
    );
}

#[test]
fn perspective_layer_keeps_its_anchor_fixed_and_rejects_invalid_geometry() {
    let anchor = Point::new(50.0, 30.0);
    for (y, x) in [(20.0, 10.0), (-35.0, 25.0), (0.0, -45.0)] {
        let transform = Transform2d::layer(anchor.x, anchor.y, y, x, 800.0).unwrap();
        same_point(transform.try_apply(anchor).unwrap(), anchor);
        let point = Point::new(20.0, 10.0);
        same_point(
            transform
                .inverse()
                .unwrap()
                .try_apply(transform.try_apply(point).unwrap())
                .unwrap(),
            point,
        );
    }
    for (x, y, ry, rx, distance) in [
        (0.0, 0.0, 0.0, 0.0, 800.0),
        (0.0, 0.0, 20.0, 0.0, 0.0),
        (f64::NAN, 0.0, 20.0, 0.0, 800.0),
        (0.0, 0.0, 20.0, f64::INFINITY, 800.0),
    ] {
        assert!(Transform2d::layer(x, y, ry, rx, distance).is_none());
    }
}

#[test]
fn text_inheritance_and_rotated_footprint_preserve_layout_contract() {
    let base = TextStyle {
        color: Some(Rgba::rgb(10, 20, 30)),
        font_family: Some("Base".into()),
        font_size: Some(20.0),
        font_weight: Some(FontWeight::Bold),
        font_style: Some(FontStyle::Italic),
    };
    let leaf = TextStyle {
        font_size: Some(16.0),
        font_family: Some("Leaf".into()),
        ..Default::default()
    };
    let inherited = leaf.over(&base);
    assert_eq!(inherited.font_size, Some(16.0));
    assert_eq!(inherited.font_family.as_deref(), Some("Leaf"));
    assert_eq!(inherited.color, base.color);
    assert_eq!(inherited.font_weight, base.font_weight);
    assert_eq!(inherited.font_style, base.font_style);
    assert_eq!(TextStyle::default().over(&base), base);
    let mut text = PlacedText {
        text: "Layout".into(),
        anchor: Point::new(10.0, 15.0),
        h_align: HAlign::Center,
        v_align: VAlign::Middle,
        rotate: 0.0,
        metrics: TextMetrics {
            width: 80.0,
            ascent: 15.0,
            descent: 5.0,
        },
        style: inherited.resolve(&ResolvedTextStyle::root()),
    };
    assert_eq!(text.footprint(), (80.0, 20.0));
    for (degrees, width, height) in [(90.0, 20.0, 80.0), (-90.0, 20.0, 80.0), (180.0, 80.0, 20.0)] {
        text.rotate = degrees;
        let footprint = text.footprint();
        near(footprint.0, width);
        near(footprint.1, height);
    }
    text.rotate = 45.0;
    let footprint = text.footprint();
    near(footprint.0, 100.0 / 2.0_f64.sqrt());
    near(footprint.1, footprint.0);
}

struct AnimatedLocation;
impl Locatable for AnimatedLocation {
    fn locate_at(&self, target: &str, progress: f64) -> Result<Hit, LocateError> {
        if target != "bar" {
            return Err(LocateError {
                target: target.into(),
                message: "unknown element".into(),
                candidates: self.locatable(),
            });
        }
        Ok(Hit {
            rect: Rect::new(0.0, 0.0, 100.0 * progress, 20.0),
            anchor: Point::new(100.0 * progress, 10.0),
            outward: Vec2::new(1.0, 0.0),
            color: Some(Rgba::rgb(1, 2, 3)),
            value: Some(100.0),
        })
    }
    fn locatable(&self) -> Vec<String> {
        vec!["bar".into()]
    }
}

#[test]
fn location_uses_final_geometry_and_errors_offer_discovery_targets() {
    assert_eq!(
        AnimatedLocation.locate("bar").unwrap(),
        AnimatedLocation.locate_at("bar", 1.0).unwrap()
    );
    assert_eq!(
        AnimatedLocation.locate_at("bar", 0.5).unwrap().rect.width,
        50.0
    );
    let error = AnimatedLocation.locate("missing").unwrap_err();
    assert_eq!(
        error.to_string(),
        "cannot locate `missing`: unknown element; available targets: bar"
    );
    assert_eq!(
        LocateError {
            candidates: vec![],
            ..error
        }
        .to_string(),
        "cannot locate `missing`: unknown element"
    );
}

#[test]
fn empty_paths_survive_the_validation_boundary() {
    let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 100.0, 100.0));
    let empty = std::hint::black_box(PathData::empty as fn() -> PathData);
    let path = builder.push_path(empty());
    let node = builder.push_node(Node::Path(PathNode {
        path,
        fill_rule: FillRule::NonZero,
        fill: None,
        stroke: None,
    }));
    builder.add_root(node);
    builder.finish().unwrap().validate().unwrap();
}

#[test]
fn curved_path_point_counts_are_checked_at_the_program_boundary() {
    let path = PathData {
        verbs: vec![
            PathVerb::MoveTo,
            PathVerb::LineTo,
            PathVerb::QuadTo,
            PathVerb::CubicTo,
            PathVerb::Close,
        ],
        points: vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [15.0, 5.0],
            [10.0, 10.0],
            [8.0, 12.0],
            [2.0, 12.0],
            [0.0, 0.0],
        ],
    };
    for missing_point in [false, true] {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 20.0, 20.0));
        let mut data = path.clone();
        if missing_point {
            data.points.pop();
        }
        let path = builder.push_path(data);
        let node = builder.push_node(Node::Path(PathNode {
            path,
            fill_rule: FillRule::EvenOdd,
            fill: None,
            stroke: None,
        }));
        builder.add_root(node);
        assert_eq!(builder.finish().is_err(), missing_point);
    }
}
