#![cfg(feature = "motion")]
//! Compile one source and verify viewport-dependent layout and geometry at two resolutions.

use valle_compiler::motion::compile_motion;
use valle_motion::{FontResource, Fonts, LayoutOptions, Viewport, build_tree, prepare_scene};
use valle_motion::{ResolvedSignals, motion_context_at, phase_windows, resolve_props};
use valle_timeline::FrameRate;

const FONT: &[u8] =
    include_bytes!("../../valle-motion/assets/fonts/noto/NotoSansCJKsc-Regular.otf");

fn fonts() -> Fonts {
    let mut fonts = Fonts::default();
    fonts
        .register(FontResource::new(FONT.to_vec()))
        .expect("register font");
    fonts
}

struct Laid {
    boxes: std::collections::BTreeMap<String, [f32; 4]>,
    /// Read the baseline path's endpoint x coordinates.
    baseline_x: (f64, f64),
}

fn lay_out(source: &str, width: u32, height: u32) -> Laid {
    let compiled = compile_motion(source).expect("compiles");
    let prepared = prepare_scene(&compiled.artifact).expect("prepare");
    let props = resolve_props(&compiled.artifact.controls, &Default::default()).expect("props");
    let phases = phase_windows(&compiled.artifact.controls.phase_spec(), 30);
    let ctx = motion_context_at(0, &phases, FrameRate::new(30, 1).unwrap()).expect("frame 0");
    let fonts = fonts();
    let tree = build_tree(
        &prepared,
        &ctx,
        &props,
        &ResolvedSignals::default(),
        &LayoutOptions {
            viewport: Viewport::new((width, height)),
            fonts: &fonts,
            styles: None,
        },
    )
    .expect("layout");
    let path = tree.paths.get("baseline").expect("baseline path resolved");
    Laid {
        boxes: valle_motion::layout_boxes(&compiled.artifact, &tree).expect("boxes"),
        baseline_x: (
            path.points.first().expect("path has points").x,
            path.points.last().expect("path has points").x,
        ),
    }
}

/// Compute both boxes and geometry from ctx.viewport.
const RESOLUTION_INDEPENDENT: &str = r##"
const DATA = [12, 45, 78, 33];
const maxV = DATA.reduce((a, b) => (a > b ? a : b), 0);

export default function Chart(ctx) {
  const w = ctx.viewport.width;
  const h = ctx.viewport.height;
  const plotHeight = h * 0.6;
  const barWidth = w / (DATA.length * 2);
  const barX = (i) => w * 0.1 + i * barWidth * 2;
  const barHeight = (v) => (v / maxV) * plotHeight;
  return (
    <Scene className="h-full w-full">
      {DATA.map((v, i) => (
        <View
          key={`bar-${i}`}
          style={{
            position: "absolute",
            left: barX(i),
            top: h * 0.8 - barHeight(v),
            width: barWidth,
            height: barHeight(v),
          }}
        />
      ))}
      <Path
        key="baseline"
        style={{ position: "absolute", left: 0, top: 0 }}
        fill="none"
        stroke="#fff"
        strokeWidth="2"
        d={line([point(w * 0.1, h * 0.8), point(w * 0.9, h * 0.8)])}
      />
    </Scene>
  );
}
"##;

/// Doubling the viewport must double both layout boxes and geometry.
#[test]
fn one_source_lays_out_proportionally_at_two_resolutions() {
    let small = lay_out(RESOLUTION_INDEPENDENT, 1920, 1080);
    let large = lay_out(RESOLUTION_INDEPENDENT, 3840, 2160);

    for key in ["bar-0", "bar-1", "bar-2", "bar-3"] {
        let a = small
            .boxes
            .get(key)
            .unwrap_or_else(|| panic!("{key} laid out"));
        let b = large
            .boxes
            .get(key)
            .unwrap_or_else(|| panic!("{key} laid out"));
        for axis in 0..4 {
            // Allow two pixels of rounding tolerance when comparing scaled boxes.
            assert!(
                (b[axis] - a[axis] * 2.0).abs() <= 2.0,
                "{key}[{axis}] must double with the viewport: {} vs {}",
                a[axis],
                b[axis]
            );
        }
    }

    // Apply the same proportionality check to geometry primitives.
    let (a0, a1) = small.baseline_x;
    let (b0, b1) = large.baseline_x;
    assert!(
        (a0 - 192.0).abs() < 1.0 && (a1 - 1728.0).abs() < 1.0,
        "1920 wide: expected x ~(192, 1728), got ({a0}, {a1})"
    );
    assert!(
        (b0 - 384.0).abs() < 1.0 && (b1 - 3456.0).abs() < 1.0,
        "3840 wide: expected x ~(384, 3456), got ({b0}, {b1})"
    );
}

/// Declare the viewport capability exactly when its inputs are used.
#[test]
fn reading_the_viewport_declares_the_capability_and_not_otherwise() {
    let with = compile_motion(RESOLUTION_INDEPENDENT).expect("compiles");
    assert!(
        with.artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == valle_motion::VIEWPORT_CAPABILITY),
        "reading ctx.viewport must declare the capability"
    );

    let without = compile_motion(
        r##"
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: "10%", top: 0, width: "50%", height: 10 }} />
  </Scene>);
}
"##,
    )
    .expect("compiles");
    assert!(
        !without
            .artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == valle_motion::VIEWPORT_CAPABILITY),
        "a scene that never reads ctx.viewport must not declare the capability"
    );
}

/// Reject viewport fields other than width and height.
#[test]
fn unknown_viewport_fields_fail_closed() {
    let diagnostics = compile_motion(
        r##"
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: ctx.viewport.dpr, top: 0, width: 10, height: 10 }} />
  </Scene>);
}
"##,
    )
    .expect_err("unknown viewport field must be rejected");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("ctx.viewport.dpr")),
        "got {diagnostics:#?}"
    );
}

/// A missing host viewport is an evaluation error; it must not silently collapse geometry to zero.
#[test]
fn evaluating_without_a_viewport_fails_closed_with_its_own_error() {
    let compiled = compile_motion(RESOLUTION_INDEPENDENT).expect("compiles");
    let props = resolve_props(&compiled.artifact.controls, &Default::default()).expect("props");
    let phases = phase_windows(&compiled.artifact.controls.phase_spec(), 30);
    let ctx = motion_context_at(0, &phases, FrameRate::new(30, 1).unwrap()).expect("frame 0");
    let error = valle_motion::eval_all(
        &compiled.artifact,
        valle_motion::EvalInputs {
            ctx: &ctx,
            props: &props,
            signals: &ResolvedSignals::default(),
            unit: None,
            viewport: None,
        },
    )
    .expect_err("no viewport must fail closed");
    assert!(
        matches!(error, valle_motion::EvalError::ViewportUnavailable { .. }),
        "got {error:?}"
    );
}
