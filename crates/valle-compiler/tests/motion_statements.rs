#![cfg(feature = "motion")]
//! `let`, `if`/`else`, early `return` and loops in component, helper and callback bodies. Each
//! body is compared with the expressions it stands for, frame by frame.

use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_motion::{
    Fonts, LayoutOptions, Viewport, build_tree, default_font_naming, emit, layout_boxes,
    motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

/// Frames on both sides of progress 0.25, 0.5 and 0.75 of a three-second scene.
const FRAMES: [u32; 10] = [0, 15, 22, 23, 30, 44, 45, 46, 67, 89];

/// The layout boxes and the draw program of `source` at each of `FRAMES`.
fn frames(source: &str) -> Vec<(BTreeMap<String, [f32; 4]>, Vec<u8>)> {
    let artifact = compile_motion(source)
        .unwrap_or_else(|diagnostics| panic!("{source}: {diagnostics:#?}"))
        .artifact;
    artifact.validate().unwrap();
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    FRAMES
        .iter()
        .map(|&frame| {
            let ctx = motion_context_at_frame(frame, 90, FrameRate::new(30, 1).unwrap()).unwrap();
            let tree = build_tree(
                &prepared,
                &ctx,
                &props,
                &LayoutOptions {
                    viewport: Viewport::new((320, 180)),
                    fonts: &fonts,
                    styles: None,
                },
            )
            .unwrap();
            let report = emit(&tree, &default_font_naming).unwrap();
            (
                layout_boxes(&artifact, &tree).unwrap(),
                report.program.packed_bytes().unwrap(),
            )
        })
        .collect()
}

fn messages(source: &str) -> Vec<String> {
    compile_motion(source)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

/// Assert that one diagnostic of `source` contains every needle.
fn expect_message(source: &str, needles: &[&str]) {
    let messages = messages(source);
    assert!(
        messages
            .iter()
            .any(|message| needles.iter().all(|needle| message.contains(needle))),
        "{needles:?} in {messages:#?}"
    );
}

const BAR: &str = r##"MODULE
export default function P() {
  const { progress } = useCurrentTime();
  BODY
  return (<Scene className="h-full w-full">
    <View key="bar" style={{ position: "absolute", left: 0, top: 0, height: 10, width: WIDTH, backgroundColor: "#f00" }} />
  </Scene>);
}
"##;

/// A scene of one bar `width` wide, after module code `module` and component statements `body`.
fn bar(module: &str, body: &str, width: &str) -> String {
    BAR.replace("MODULE", module)
        .replace("BODY", body)
        .replace("WIDTH", width)
}

#[test]
fn let_if_and_compound_assignment_in_a_helper_choose_its_value_per_frame() {
    let helper = r#"
function width(t) {
  let w = 10;
  if (t > 0.5) {
    w = 40;
  } else if (t > 0.25) {
    w += 10;
  }
  return w * 2;
}"#;
    assert_eq!(
        frames(&bar(helper, "", "width(progress)")),
        frames(&bar(
            "",
            "",
            "(progress > 0.5 ? 40 : progress > 0.25 ? 20 : 10) * 2"
        ))
    );
}

#[test]
fn early_returns_in_a_helper_lower_like_nested_conditionals() {
    let helper = r#"
function level(t) {
  if (t < 0.25) return 10;
  if (t < 0.5) {
    return 20;
  }
  return 30;
}"#;
    assert_eq!(
        frames(&bar(helper, "", "level(progress)")),
        frames(&bar(
            "",
            "",
            "progress < 0.25 ? 10 : progress < 0.5 ? 20 : 30"
        ))
    );
}

#[test]
fn loops_unroll_while_the_scene_is_prepared() {
    let helper = r#"
function total(t) {
  let sum = 0;
  for (let i = 1; i <= 4; i++) {
    if (i === 3) continue;
    sum += i * t;
  }
  let n = 1;
  while (n < 50) n *= 2;
  for (const step of [1, 2]) {
    if (step > 1) break;
    sum += step;
  }
  return sum * 10 + n;
}"#;
    assert_eq!(
        frames(&bar(helper, "", "total(progress)")),
        frames(&bar(
            "",
            "",
            "(0 + 1 * progress + 2 * progress + 4 * progress + 1) * 10 + 64"
        ))
    );
}

#[test]
fn a_let_without_an_initializer_takes_what_each_branch_assigns() {
    let body = r#"
  let w;
  if (progress > 0.5) w = 40;
  else w = 10;"#;
    assert_eq!(
        frames(&bar("", body, "w")),
        frames(&bar("", "", "progress > 0.5 ? 40 : 10"))
    );
}

#[test]
fn a_condition_known_while_preparing_takes_one_branch() {
    let body = r#"
  let w = 10;
  if (WIDE) {
    w = 100;
  } else {
    w = progress;
  }"#;
    assert_eq!(
        frames(&bar("const WIDE = true;", body, "w")),
        frames(&bar("", "", "100"))
    );
}

#[test]
fn block_scoped_names_end_with_their_block() {
    let body = r#"
  function grow(x) { return x * 2; }
  let w = 5;
  {
    const w = 1000;
  }
  if (progress > 2) {
    const w = 1;
  }
  w = grow(w);"#;
    assert_eq!(frames(&bar("", body, "w")), frames(&bar("", "", "10")));
}

#[test]
fn an_early_return_of_jsx_is_the_conditional_it_stands_for() {
    let scene = |badge: &str| {
        format!(
            "{badge}\nexport default function P() {{\n  return (<Scene className=\"h-full w-full\"><Badge key=\"badge\" /></Scene>);\n}}\n"
        )
    };
    let on = r##"<View key="on" style={{ position: "absolute", left: 0, top: 0, width: 40, height: 10, backgroundColor: "#0f0" }} />"##;
    let off = r##"<View key="off" style={{ position: "absolute", left: 0, top: 20, width: 20, height: 10, backgroundColor: "#00f" }} />"##;
    assert_eq!(
        frames(&scene(&format!(
            "function Badge() {{\n  const {{ progress }} = useCurrentTime();\n  if (progress > 0.5) {{\n    return {on};\n  }}\n  return {off};\n}}"
        ))),
        frames(&scene(&format!(
            "function Badge() {{\n  const {{ progress }} = useCurrentTime();\n  return progress > 0.5 ? {on} : {off};\n}}"
        )))
    );
    assert_eq!(
        frames(&scene(&format!(
            "function Badge() {{\n  const {{ progress }} = useCurrentTime();\n  if (progress < 0.5) return null;\n  return {on};\n}}"
        ))),
        frames(&scene(&format!(
            "function Badge() {{\n  const {{ progress }} = useCurrentTime();\n  return progress < 0.5 ? null : {on};\n}}"
        )))
    );
}

#[test]
fn jsx_pushed_under_a_frame_dependent_condition_shows_when_it_holds() {
    let dot = |i: &str, left: &str| {
        format!(
            r##"<View key={{`dot${{{i}}}`}} style={{{{ position: "absolute", left: {left}, top: 0, width: 10, height: 10, backgroundColor: "#f00" }}}} />"##
        )
    };
    let pushed = format!(
        r##"export default function P() {{
  const {{ progress }} = useCurrentTime();
  const dots = [];
  for (const i of [0, 1, 2, 3]) {{
    if (progress > i / 4) {{
      dots.push({});
    }}
  }}
  return (<Scene className="h-full w-full">{{dots}}</Scene>);
}}
"##,
        dot("i", "i * 20")
    );
    let unrolled = format!(
        r##"export default function P() {{
  const {{ progress }} = useCurrentTime();
  return (<Scene className="h-full w-full">
    {{progress > 0 && {}}}
    {{progress > 0.25 && {}}}
    {{progress > 0.5 && {}}}
    {{progress > 0.75 && {}}}
  </Scene>);
}}
"##,
        dot("0", "0"),
        dot("1", "20"),
        dot("2", "40"),
        dot("3", "60")
    );
    assert_eq!(frames(&pushed), frames(&unrolled));
}

#[test]
fn a_map_callback_takes_statements_for_each_item() {
    let mapped = r##"export default function P() {
  const { progress } = useCurrentTime();
  return (<Scene className="h-full w-full">
    {[1, 2, 3].map((n) => {
      let w = n * 10;
      if (n === 2) w += 5;
      return <View key={`b${n}`} style={{ position: "absolute", left: n * 40, top: 0, width: w, height: 10, backgroundColor: "#f00" }} />;
    })}
  </Scene>);
}
"##;
    let unrolled = r##"export default function P() {
  const { progress } = useCurrentTime();
  return (<Scene className="h-full w-full">
    <View key="b1" style={{ position: "absolute", left: 40, top: 0, width: 10, height: 10, backgroundColor: "#f00" }} />
    <View key="b2" style={{ position: "absolute", left: 80, top: 0, width: 25, height: 10, backgroundColor: "#f00" }} />
    <View key="b3" style={{ position: "absolute", left: 120, top: 0, width: 30, height: 10, backgroundColor: "#f00" }} />
  </Scene>);
}
"##;
    assert_eq!(frames(mapped), frames(unrolled));
}

#[test]
fn statements_the_frame_cannot_choose_point_at_what_can() {
    expect_message(
        &bar(
            "",
            r#"
  let w = 0;
  for (let i = 0; i < 4; i++) {
    if (progress > 0.5) break;
    w += 10;
  }"#,
            "w",
        ),
        &["`break` under a frame-dependent condition"],
    );
    expect_message(
        &bar(
            "",
            r#"
  let label = 10;
  if (progress > 0.5) label = "wide";"#,
            "10",
        ),
        &["leaves `label` as", "after the other"],
    );
    expect_message(
        &bar(
            "function f(t) { if (t > 0.5) return 1; }",
            "",
            "f(progress)",
        ),
        &["does not return on every path"],
    );
    expect_message(
        &bar(
            "",
            r#"
  const w = 1;
  w = 2;"#,
            "w",
        ),
        &["`w` is a `const`"],
    );
    expect_message(
        &bar(
            "",
            r#"
  let w = 1;
  const bump = () => { w = 2; return w; };"#,
            "bump()",
        ),
        &["helper `bump` assigns only its own `let` bindings"],
    );
    expect_message(&bar("", "var w = 1;", "w"), &["`const`", "`let`"]);
    expect_message(
        &bar("", "switch (1) { default: break; }", "10"),
        &["`switch` is not supported"],
    );
    expect_message(
        r##"export default function P() {
  const { progress } = useCurrentTime();
  return (<Scene className="h-full w-full">
    {[0, 1].map((i) => {
      if (progress > 0.5) return <View key={`k${i}`} style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10 }} />;
      return <View key={`k${i}`} style={{ position: "absolute", left: 0, top: 0, width: 20, height: 10 }} />;
    })}
  </Scene>);
}
"##,
        &["duplicate node key", "frame-dependent condition"],
    );
}
