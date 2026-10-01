#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_draw::program::{
    AuthorColor, GradientInterpolation, HueDirection, Node, Paint, interpolate_author_colors,
};
use valle_motion::{
    BatchFieldProgress, Expr, Fonts, LayoutOptions, MotionValue, NodeKind, Viewport, build_tree,
    default_font_naming, emit, motion_context_at_frame, prepare_scene,
    register_default_motion_fonts, resolve_geometry_batch, resolve_props,
};
use valle_timeline::FrameRate;

const OKLCH_COLOR_INTERPOLATION: &str =
    include_str!("fixtures/motion/composition/oklch-color-interpolation.motion.tsx");

#[test]
fn color_reaches_draw_program_without_byte_rounding() {
    let artifact = compile_motion(OKLCH_COLOR_INTERPOLATION).unwrap().artifact;
    let interpolate = artifact
        .exprs
        .iter()
        .position(|expr| matches!(expr, Expr::Interpolate { .. }))
        .unwrap();
    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let MotionValue::Color(color) = tree.values[interpolate] else {
        panic!("interpolate must produce a float color")
    };
    let actual = color.to_srgb8();
    for (actual, expected) in [(actual.r, 0), (actual.g, 196), (actual.b, 159)] {
        assert!(
            (i16::from(actual) - expected).abs() <= 6,
            "{actual} != {expected}"
        );
    }
    assert_ne!(
        color,
        AuthorColor::from_srgb8(color.to_srgb8()),
        "the evaluator rounded color to bytes"
    );
    let program = emit(&tree, &default_font_naming).unwrap().program;
    assert!(
        program
            .paints()
            .iter()
            .any(|paint| { matches!(paint, Paint::Solid(value) if *value == color.to_working()) }),
        "the evaluated float color did not reach DrawProgram"
    );
}

#[test]
fn interpolate_rejects_invalid_color_space_and_hue_options() {
    assert!(compile_motion(&OKLCH_COLOR_INTERPOLATION.replace("\"oklch\"", "\"hsl\"")).is_err());
    assert!(
        compile_motion(&OKLCH_COLOR_INTERPOLATION.replace("\"shorter\"", "\"sideways\"")).is_err()
    );
    assert!(
        compile_motion(
            &OKLCH_COLOR_INTERPOLATION.replace("colorSpace: \"oklch\"", "colorSpace: \"srgb\"")
        )
        .is_err()
    );
}

#[test]
fn animated_gradient_stop_reaches_draw_program_without_byte_rounding() {
    let source = r##"
export const composition = { width: 160, height: 80, fps: 30, duration: 2 };
export default function FloatGradient(ctx) {
  return <Scene className="relative h-full w-full">
    <Path key="gradient" d="M 0 0 L 160 0 L 160 80 L 0 80 Z"
      fill={linearGradient(point(0, 0), point(160, 0), [
        gradientStop(0, interpolate(ctx.progress, [0, 1], ["#2140ff", "#ffd000"],
          { colorSpace: "oklch" })),
        gradientStop(1, "#ffd000"),
      ])} />
  </Scene>;
}
"##;
    let artifact = compile_motion(source).unwrap().artifact;
    let interpolate = artifact
        .exprs
        .iter()
        .position(|expr| matches!(expr, Expr::Interpolate { .. }))
        .unwrap();
    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((160, 80)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let MotionValue::Color(color) = tree.values[interpolate] else {
        panic!("gradient stop must evaluate to an author color")
    };
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let first = program
        .paints()
        .iter()
        .find_map(|paint| match paint {
            Paint::LinearGradient { stops, .. } => stops.first().map(|stop| stop.color),
            _ => None,
        })
        .expect("Path gradient must emit a DrawProgram paint");
    assert_eq!(first, color.to_working());
    assert_ne!(
        first,
        AuthorColor::from_srgb8(color.to_srgb8()).to_working(),
        "the gradient stop was rounded to bytes"
    );
}

#[test]
fn animated_per_unit_text_color_reaches_draw_program_without_byte_rounding() {
    let source = r##"
export const composition = { width: 160, height: 80, fps: 30, duration: 2 };
export default function FloatLetters(ctx) {
  return <Scene className="relative h-full w-full">
    <Text key="letters" split="char" style={{ fontSize: 56, color: "#ffffff" }}
      perUnit={{ color: interpolate(ctx.progress, [0, 1], ["#2140ff", "#ffd000"],
        { colorSpace: "oklch" }) }}>AB</Text>
  </Scene>;
}

"##;
    let artifact = compile_motion(source).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let mut fonts = Fonts::default();
    register_default_motion_fonts(&mut fonts).unwrap();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((160, 80)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let expected = interpolate_author_colors(
        AuthorColor::from_srgb8(valle_draw::Rgba::rgb(0x21, 0x40, 0xff)),
        AuthorColor::from_srgb8(valle_draw::Rgba::rgb(0xff, 0xd0, 0)),
        0.5,
        GradientInterpolation::Oklch {
            hue: HueDirection::Shorter,
        },
    );
    let colors = tree
        .units
        .values()
        .flat_map(|units| units.iter())
        .filter_map(|unit| unit.color)
        .collect::<Vec<_>>();
    assert_eq!(colors, vec![expected, expected]);
    assert_ne!(expected, AuthorColor::from_srgb8(expected.to_srgb8()));
    let program = emit(&tree, &default_font_naming).unwrap().program;
    assert!(
        program
            .paints()
            .iter()
            .any(|paint| matches!(paint, Paint::Solid(color) if *color == expected.to_working())),
        "per-unit text paint was rounded to bytes"
    );
}

#[test]
fn animated_text_fill_reaches_draw_program_without_byte_rounding() {
    let source = r##"
export const composition = { width: 160, height: 80, fps: 30, duration: 2 };
export default function FloatText(ctx) {
  return <Scene className="relative h-full w-full">
    <Text key="letters" style={{ fontSize: 56,
      color: interpolate(ctx.progress, [0, 1], ["#2140ff", "#ffd000"],
        { colorSpace: "oklch" }) }}>AB</Text>
  </Scene>;
}
"##;
    let artifact = compile_motion(source).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let mut fonts = Fonts::default();
    register_default_motion_fonts(&mut fonts).unwrap();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((160, 80)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let expected = interpolate_author_colors(
        AuthorColor::from_srgb8(valle_draw::Rgba::rgb(0x21, 0x40, 0xff)),
        AuthorColor::from_srgb8(valle_draw::Rgba::rgb(0xff, 0xd0, 0)),
        0.5,
        GradientInterpolation::Oklch {
            hue: HueDirection::Shorter,
        },
    );
    let program = emit(&tree, &default_font_naming).unwrap().program;
    assert!(
        program
            .paints()
            .iter()
            .any(|paint| matches!(paint, Paint::Solid(color) if *color == expected.to_working())),
        "text fill was rounded to bytes"
    );
}

#[test]
fn geometry_batch_fill_field_reaches_float_instance_column() {
    let source = r##"
export const composition = { width: 160, height: 80, fps: 30, duration: 2 };
export default function FloatBatch(ctx) {
  return <Scene className="relative h-full w-full">
    <GeometryBatch key="tiles" geometry="rect" positions={[point(80, 40)]}
      sizes={point(80, 40)}
      fills={field({ from: "#2140ff", to: "#ffd000", progress: ctx.progress })}
      style={{ position: "absolute", left: 0, top: 0, width: 160, height: 80 }} />
  </Scene>;
}
"##;
    let artifact = compile_motion(source).unwrap().artifact;
    let batch = artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::GeometryBatch { batch } => Some(batch),
            _ => None,
        })
        .unwrap();
    let author = interpolate_author_colors(
        AuthorColor::from_srgb8(valle_draw::Rgba::rgb(0x21, 0x40, 0xff)),
        AuthorColor::from_srgb8(valle_draw::Rgba::rgb(0xff, 0xd0, 0)),
        0.5,
        GradientInterpolation::Srgb,
    );
    let color = resolve_geometry_batch(
        batch,
        30.0,
        30.0,
        BatchFieldProgress {
            fill: 0.5,
            ..Default::default()
        },
    )[0]
    .color;
    assert_eq!(author.to_srgb8(), valle_draw::Rgba::rgb(144, 136, 128));
    assert_ne!(author, AuthorColor::from_srgb8(author.to_srgb8()));
    assert_eq!(color, author.to_working());

    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((160, 80)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let actual = program
        .nodes()
        .iter()
        .find_map(|node| match node {
            Node::InstanceBatch(batch) => batch.instances.colors.first().copied(),
            _ => None,
        })
        .expect("GeometryBatch must emit an instance color column");
    assert_eq!(actual, color);
    assert_ne!(
        actual,
        AuthorColor::from_srgb8(author.to_srgb8()).to_working()
    );
}

#[test]
fn instance_template_color_reaches_float_instance_columns() {
    let source = include_str!("fixtures/motion/composition/float-color-instances.motion.tsx");
    let artifact = compile_motion(source).unwrap().artifact;
    assert_eq!(artifact.instance_groups.len(), 1);
    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let expected = interpolate_author_colors(
        AuthorColor::from_srgb8(valle_draw::Rgba::rgb(0x21, 0x40, 0xff)),
        AuthorColor::from_srgb8(valle_draw::Rgba::rgb(0xff, 0xd0, 0)),
        0.5,
        GradientInterpolation::Oklch {
            hue: HueDirection::Shorter,
        },
    );
    assert_ne!(expected, AuthorColor::from_srgb8(expected.to_srgb8()));
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let colors = program
        .nodes()
        .iter()
        .find_map(|node| match node {
            Node::InstanceBatch(batch) => Some(&batch.instances.colors),
            _ => None,
        })
        .expect("template must lower to an instance batch");
    assert_eq!(colors.len(), 64);
    assert!(colors.iter().all(|color| *color == expected.to_working()));
}

#[test]
fn glass_tint_remains_float_through_layout() {
    let source = include_str!("fixtures/motion/composition/float-color-glass.motion.tsx");
    let artifact = compile_motion(source).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let expected = interpolate_author_colors(
        AuthorColor::from_srgb8(valle_draw::Rgba::rgb(0x21, 0x40, 0xff)),
        AuthorColor::from_srgb8(valle_draw::Rgba::rgb(0xff, 0xd0, 0)),
        0.5,
        GradientInterpolation::Oklch {
            hue: HueDirection::Shorter,
        },
    );
    assert_ne!(expected, AuthorColor::from_srgb8(expected.to_srgb8()));
    assert_eq!(tree.glass.surfaces.len(), 1);
    assert_eq!(tree.glass.surfaces[0].material.unwrap().tint, expected);
}

#[test]
fn text_stroke_color_reaches_draw_program_without_byte_rounding() {
    let direct = include_str!("fixtures/motion/composition/float-color-text-stroke.motion.tsx");
    let inherited = direct.replace(
        ">VALLE</Text>",
        "><Span style={{ color: \"#00000000\" }}>VALLE</Span></Text>",
    );
    let mut fonts = Fonts::default();
    register_default_motion_fonts(&mut fonts).unwrap();
    let expected = interpolate_author_colors(
        AuthorColor::from_srgb8(valle_draw::Rgba::rgb(0x21, 0x40, 0xff)),
        AuthorColor::from_srgb8(valle_draw::Rgba::rgb(0xff, 0xd0, 0)),
        0.5,
        GradientInterpolation::Oklch {
            hue: HueDirection::Shorter,
        },
    );
    assert_ne!(expected, AuthorColor::from_srgb8(expected.to_srgb8()));
    for (label, source) in [("direct", direct), ("inherited Span", inherited.as_str())] {
        let artifact = compile_motion(source).unwrap().artifact;
        let prepared = prepare_scene(&artifact).unwrap();
        let context = motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
        let tree = build_tree(
            &prepared,
            &context,
            &props,
            &LayoutOptions {
                viewport: Viewport::new((640, 360)),
                fonts: &fonts,
                styles: None,
            },
        )
        .unwrap();
        let program = emit(&tree, &default_font_naming).unwrap().program;
        assert!(
            program.paints().iter().any(
                |paint| matches!(paint, Paint::Solid(color) if *color == expected.to_working())
            ),
            "the {label} glyph stroke paint was rounded to bytes"
        );
    }
}
