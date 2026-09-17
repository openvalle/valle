#![cfg(feature = "motion")]
//! Measured values are artifact constants with font bytes as explicit inputs. Missing measurement environments and frame-dependent inputs must fail.

#![cfg(feature = "motion")]

use valle_compiler::motion::{MeasureEnv, compile_motion, compile_motion_with_env};
use valle_motion::{Expr, MotionValue};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/noto/NotoSansCJKsc-Regular.otf");

fn env() -> MeasureEnv {
    MeasureEnv::new(&[FONT.to_vec()], (1920, 1080)).expect("font bundle builds a measure env")
}

/// Place a measured static text width in an opacity slot to isolate measurement behavior.
fn source(expr: &str) -> String {
    format!(
        "const M = measureText(\"全球业务网络\", {{ fontSize: 48 }});\n\
         export default function Card(ctx) {{\n\
         \x20 return <Scene key=\"scene\" style={{{{ opacity: {expr} }}}} />;\n\
         }}\n"
    )
}

/// Collect both static styles and constant expressions when checking baked measurement values.
fn folded_numbers(artifact: &valle_motion::SceneArtifact) -> Vec<f64> {
    let from_exprs = artifact.exprs.iter().filter_map(|expr| match expr {
        Expr::Const {
            value: MotionValue::Number(value),
        } => Some(*value),
        _ => None,
    });
    let from_styles = artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .filter_map(|binding| match &binding.value {
            valle_motion::StyleValue::Static {
                value: MotionValue::Number(value),
            } => Some(*value),
            _ => None,
        });
    from_exprs.chain(from_styles).collect()
}

#[test]
fn measured_width_folds_into_the_artifact_as_a_constant() {
    let compiled = compile_motion_with_env(&source("M.width / 1000"), &[], Some(&env()))
        .expect("measure-backed source compiles when fonts are supplied");
    let numbers = folded_numbers(&compiled.artifact);
    // Six CJK characters at 48px should measure within 200-400px, allowing font-metric variation.
    let width = numbers
        .iter()
        .map(|value| value * 1000.0)
        .find(|value| (200.0..400.0).contains(value));
    assert!(
        width.is_some(),
        "no plausibly-measured width folded into the artifact: {numbers:?}"
    );
    compiled.artifact.validate().expect("artifact validates");
}

#[test]
fn measuring_without_a_font_bundle_fails_closed_and_points_at_the_flag() {
    // Missing fonts must prevent artifact production.
    let diagnostics =
        compile_motion(&source("M.width / 1000")).expect_err("no fonts must not compile");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("--font")),
        "diagnostic must point at the fix, got: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn an_empty_font_list_uses_the_canonical_default_font() {
    let fallback = MeasureEnv::new(&[], (1920, 1080)).expect("canonical fallback");
    let explicit = MeasureEnv::new(&[FONT.to_vec()], (1920, 1080)).expect("explicit default");
    let fallback_artifact =
        compile_motion_with_env(&source("M.width / 1000"), &[], Some(&fallback))
            .expect("fallback compile")
            .artifact;
    let explicit_artifact =
        compile_motion_with_env(&source("M.width / 1000"), &[], Some(&explicit))
            .expect("explicit compile")
            .artifact;
    assert_eq!(
        valle_motion::canonical_bytes(&fallback_artifact).unwrap(),
        valle_motion::canonical_bytes(&explicit_artifact).unwrap(),
        "implicit fallback and the canonical explicit font must measure identically"
    );
}

#[test]
fn frame_varying_measure_inputs_fail_closed() {
    // A ctx-dependent measureText argument must produce a preparation-time capability diagnostic.
    let source = "export default function Card(ctx) {\n  \
                  const m = measureText(\"x\", { maxWidth: ctx.enter.progress * 100 });\n  \
                  return <Scene key=\"scene\" style={{ opacity: m.width }} />;\n}\n";
    let diagnostics =
        compile_motion_with_env(source, &[], Some(&env())).expect_err("frame-varying measure");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("prepare-time capability")),
        "got: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn measurement_is_reproducible_across_compiles() {
    // Identical source and font bytes must produce identical artifacts.
    let first = compile_motion_with_env(&source("M.width / 1000"), &[], Some(&env()))
        .expect("first compile");
    let second = compile_motion_with_env(&source("M.width / 1000"), &[], Some(&env()))
        .expect("second compile");
    assert_eq!(
        valle_motion::canonical_bytes(&first.artifact).unwrap(),
        valle_motion::canonical_bytes(&second.artifact).unwrap(),
        "measure must not introduce compile-to-compile drift"
    );
}

#[test]
fn bad_measure_options_surface_as_authored_diagnostics_not_panics() {
    let source = "const M = measureText(\"x\", { fontSize: 16, maxWidth: -5 });\n\
                  export default function Card(ctx) {\n  \
                  return <Scene key=\"scene\" style={{ opacity: M.width }} />;\n}\n";
    let diagnostics =
        compile_motion_with_env(source, &[], Some(&env())).expect_err("negative maxWidth");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("maxWidth")),
        "got: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn measure_cannot_be_used_to_smuggle_nondeterminism_into_prepare() {
    // Installing measurement capabilities must preserve sandbox determinism restrictions.
    let source = "const M = measureText(String(Date.now()));\n\
                  export default function Card(ctx) {\n  \
                  return <Scene key=\"scene\" style={{ opacity: M.width }} />;\n}\n";
    let diagnostics = compile_motion_with_env(source, &[], Some(&env()))
        .expect_err("Date must stay forbidden with measure installed");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("Date")),
        "got: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}

fn theme_graph(source: &str) -> valle_compiler::motion::MotionModuleGraph {
    valle_compiler::motion::MotionModuleGraph::new(
        "main.tsx",
        std::collections::BTreeMap::from([("main.tsx".into(), source.to_owned())]),
    )
    .unwrap()
}

fn assert_measured_box_matches_text(
    artifact: &valle_motion::SceneArtifact,
    viewport: (u32, u32),
    aliases: &[(String, &[u8])],
) {
    use std::collections::BTreeMap;
    use valle_motion::{Fonts, LayoutOptions, ResolvedSignals, Viewport};
    let mut fonts = Fonts::default();
    valle_motion::register_default_motion_fonts(&mut fonts).unwrap();
    for (alias, bytes) in aliases {
        fonts
            .register(
                valle_motion::FontResource::new(bytes.to_vec()).override_info(
                    valle_motion::FontOverride {
                        family_name: Some(alias.as_str().into()),
                        ..Default::default()
                    },
                ),
            )
            .unwrap();
    }
    let prepared = valle_motion::prepare_scene(artifact).unwrap();
    let props = valle_motion::resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let windows = valle_motion::phase_windows(&artifact.controls.phase_spec(), 90);
    let mut programs = Vec::new();
    for frame in [60, 0, 60] {
        let ctx = valle_motion::motion_context_at(
            frame,
            &windows,
            valle_timeline::FrameRate::new(30, 1).unwrap(),
        )
        .unwrap();
        let tree = valle_motion::build_tree(
            &prepared,
            &ctx,
            &props,
            &ResolvedSignals::default(),
            &LayoutOptions {
                viewport: Viewport::new(viewport),
                fonts: &fonts,
                styles: None,
            },
        )
        .unwrap();
        let boxes = valle_motion::layout_boxes(artifact, &tree).unwrap();
        assert_eq!(
            &boxes["measured"][2..],
            &boxes["actual"][2..],
            "frame {frame}: {boxes:?}"
        );
        let report = valle_motion::emit(&tree, &valle_motion::default_font_naming).unwrap();
        assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
        programs.push(report.program.packed_bytes().unwrap());
    }
    assert_eq!(programs[0], programs[2]);
    assert_ne!(
        programs[0], programs[1],
        "authored frame motion must still run"
    );
}

#[test]
fn constrained_measurement_matches_wrapped_rendered_text() {
    use valle_compiler::motion::compile_motion_modules_with_full_env;
    // Measure and render receive the same numbers, so the measured box must equal the laid-out
    // Text box once the text wraps inside the same width.
    let source = r#"
const label='全球业务网络 Overview: shared measurement';
const M=measureText(label,{fontFamily:'monospace',fontSize:28,lineHeight:1.25,maxWidth:200});
export default function Card(ctx){return <Scene style={{width:'100%',height:480}}>
  <View key='measured' style={{position:'absolute',width:M.width,height:M.height,backgroundColor:'#334455',opacity:ctx.localFrame/100}}/>
  <Text key='actual' style={{position:'absolute',top:100,fontFamily:'monospace',fontSize:28,lineHeight:1.25,width:200}}>{label}</Text>
</Scene>}
"#;
    let graph = theme_graph(source);
    let compiled = compile_motion_modules_with_full_env(
        &graph,
        &[],
        Some(&MeasureEnv::new(&[], (640, 480)).unwrap()),
        None,
    )
    .unwrap();
    assert_measured_box_matches_text(&compiled.artifact, (640, 480), &[]);
}

#[test]
fn entry_measurement_uses_composition_canvas_even_with_a_small_host_viewport() {
    let source = r#"
export const composition = { width: 320, height: 180, duration: 1 };
const label = 'iiii WWWW iiii WWWW';
const M = measureText(label, { fontFamily: 'monospace, sans-serif', fontSize: 32, fontWeight: 700, letterSpacing: 1.5, lineHeight: 1.25, maxWidth: 120 });
export default function Card(ctx) { return <Scene style={{ width: 320, height: 180 }}>
  <View key='measured' style={{ position: 'absolute', width: M.width, height: M.height, opacity: ctx.localFrame / 100 }} />
  <Text key='actual' style={{ position: 'absolute', top: 0, fontFamily: 'monospace, sans-serif', fontSize: 32, fontWeight: 700, letterSpacing: 1.5, lineHeight: 1.25, width: 120 }}>{label}</Text>
</Scene>; }
"#;
    let compiled = valle_compiler::motion::compile_motion_with_env(
        source,
        &[],
        Some(&MeasureEnv::new(&[], (1, 1)).unwrap()),
    )
    .unwrap();
    assert_measured_box_matches_text(&compiled.artifact, (320, 180), &[]);
}

#[test]
fn font_lists_measure_like_text_with_and_without_wrapping() {
    use valle_compiler::motion::compile_motion_modules_with_full_env;
    for family in [
        "monospace, sans-serif",
        "Noto Sans Mono, sans-serif",
        "\"Noto Sans Mono\", monospace",
        "'Noto Sans Mono', monospace",
        "\"Missing, Family\", monospace",
    ] {
        for width in [None, Some(120)] {
            let constraint = width.map_or(String::new(), |width| format!(",maxWidth:{width}"));
            let text_width = width.map_or(String::new(), |width| format!(",width:{width}"));
            let source = format!(
                r#"
const label='iiii WWWW iiii WWWW';
const family={family:?};
const M=measureText(label,{{fontFamily:family,fontSize:32{constraint}}});
export default function Card(ctx){{return <Scene style={{{{width:640,height:480}}}}>
  <View key='measured' style={{{{position:'absolute',width:M.width,height:M.height,opacity:ctx.localFrame/100}}}}/>
  <Text key='actual' style={{{{position:'absolute',top:100,fontFamily:family,fontSize:32{text_width}}}}}>{{label}}</Text>
</Scene>}}
"#
            );
            let compiled = compile_motion_modules_with_full_env(
                &theme_graph(&source),
                &[],
                Some(&MeasureEnv::new(&[], (640, 480)).unwrap()),
                None,
            )
            .unwrap_or_else(|diagnostics| panic!("{family:?}: {diagnostics:#?}"));
            assert_measured_box_matches_text(&compiled.artifact, (640, 480), &[]);
        }
    }
}

#[test]
fn invalid_measure_options_name_the_option_and_the_replacement() {
    let env = env();
    for (options, needle) in [
        ("{className:'font-mono'}", "className"),
        ("{style:'font-size:24px'}", "style"),
        ("{size:12}", "size"),
        ("{fontSize:0}", "fontSize"),
        ("{fontSize:16,fontWeight:2000}", "fontWeight"),
        ("{fontSize:16,maxWidth:1e300}", "maxWidth"),
        ("{fontSize:16,maxWidth:Infinity}", "maxWidth"),
        ("{fontSize:16,maxWidth:NaN}", "maxWidth"),
    ] {
        let source = format!(
            "const M=measureText('x',{options}); export default function Card(){{return <View style={{{{width:M.width}}}}/>}}"
        );
        let diagnostics = compile_motion_with_env(&source, &[], Some(&env)).unwrap_err();
        assert!(
            diagnostics.iter().any(|d| d.message.contains(needle)),
            "`{options}` must name `{needle}`: {diagnostics:#?}"
        );
    }
}

#[test]
fn literal_font_alias_matches_rendering() {
    use valle_compiler::motion::compile_motion_modules_with_full_env;
    const MONO: &[u8] = include_bytes!("../../../assets/fonts/noto/NotoSansMono-Regular.ttf");
    let hash = valle_motion::ContentDigest::of_bytes(MONO);
    let env = MeasureEnv::new_with_aliases(
        &[],
        &[("asset://brandFont".into(), MONO.to_vec())],
        (960, 480),
    )
    .unwrap();
    let source = r#"
export const controls=defineControls({assets:{brandFont:asset({kind:'font',required:true})}});
const label='iiii WWWW';
const M=measureText(label,{fontFamily:'asset://brandFont',fontSize:32,lineHeight:1.5});
export default function Card(ctx){return <Scene style={{width:960,height:480}}>
  <View key='measured' style={{position:'absolute',width:M.width,height:M.height,backgroundColor:'#334455',opacity:ctx.localFrame/100}}/>
  <Text key='actual' style={{position:'absolute',top:100,fontFamily:'asset://brandFont',fontSize:32,lineHeight:1.5}}>{label}</Text>
</Scene>}
"#;
    let graph = theme_graph(source);
    let compiled = compile_motion_modules_with_full_env(
        &graph,
        &[valle_motion::ResourceRef {
            control: "brandFont".into(),
            content_hash: hash.clone(),
        }],
        Some(&env),
        None,
    )
    .unwrap();
    assert_measured_box_matches_text(
        &compiled.artifact,
        (960, 480),
        &[(valle_motion::font_family_alias(&hash), MONO)],
    );
}

#[test]
fn explicit_typography_measurement_matches_rendered_text() {
    // Every numeric option is the same value an authored style takes, so the measured box and the
    // laid-out Text box must agree without a second style language in between.
    let source = r#"
const label='iiii 全球 Overview';
const M=measureText(label,{fontFamily:'monospace',fontSize:24,lineHeight:1.5,letterSpacing:2,fontWeight:700});
export default function Card(ctx){return <Scene style={{width:640,height:480}}>
  <View key='measured' style={{position:'absolute',width:M.width,height:M.height,backgroundColor:'#334455',opacity:ctx.localFrame/100}}/>
  <Text key='actual' style={{position:'absolute',top:100,fontFamily:'monospace',fontSize:24,lineHeight:1.5,letterSpacing:2,fontWeight:700}}>{label}</Text>
</Scene>}
"#;
    let compiled = compile_motion_with_env(
        source,
        &[],
        Some(&MeasureEnv::new(&[], (640, 480)).unwrap()),
    )
    .unwrap();
    assert_measured_box_matches_text(&compiled.artifact, (640, 480), &[]);
}
