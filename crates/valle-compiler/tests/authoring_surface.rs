#![cfg(feature = "motion")]
//! Every permanent rejection must name a replacement. A diagnostic that points at unbuilt or
//! removed syntax is worse than none.
//!
//! The test compiles one trigger per rejection and checks the message an author would see.

use std::collections::BTreeMap;

use valle_compiler::motion::{
    MeasureEnv, MotionModuleGraph, compile_motion_modules_with_full_env, compile_motion_with_env,
};

const COMPOSITION: &str =
    "export const composition = { width: 320, height: 180, fps: 30, duration: 1 };\n";
const BODY_OPEN: &str = "export default function D(){return <Scene style={{width:320,height:180}}>";

fn env() -> MeasureEnv {
    MeasureEnv::new_with_aliases(&[], &[], (320, 180)).expect("measure env")
}

/// Compile an entry body and return every diagnostic message it produced.
fn messages(entry: &str, body: &str) -> String {
    let source = format!("{COMPOSITION}{body}");
    let graph =
        MotionModuleGraph::new(entry, BTreeMap::from([(entry.to_owned(), source)])).expect("graph");
    match compile_motion_modules_with_full_env(&graph, &[], Some(&env()), None) {
        Ok(_) => panic!("the trigger must be rejected: {body}"),
        Err(diagnostics) => diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn rejection(body: &str, phrase: &str) {
    let text = messages("entry.motion.tsx", &format!("{BODY_OPEN}{body}</Scene>;}}"));
    assert!(
        text.contains(phrase),
        "`{body}` must name `{phrase}`, got: {text}"
    );
}

#[test]
fn every_rejection_names_a_documented_replacement() {
    for (body, phrase) in [
        (
            r#"<View className="md:w-4" />"#,
            "responsive and state variants are not supported",
        ),
        (
            r#"<View className="md:w-4" />"#,
            "Keep one file per deliverable shape, or branch on props or data",
        ),
        (
            r#"<View className="w-(--size)" />"#,
            "the custom-property shorthand has no variable environment",
        ),
        (
            r#"<View className="w-(--size)" />"#,
            "a literal value, or an arbitrary value with a `const` interpolated",
        ),
        (
            r#"<View style={{ width: "var(--x)" }} />"#,
            "CSS variable references are not supported",
        ),
        (
            r#"<View style={{ width: "var(--x)" }} />"#,
            "a `const` at the top of the file, or a literal value",
        ),
        (
            r##"<View style={{ "--tone": "#fff" }} />"##,
            "author CSS custom properties are not supported",
        ),
        (
            r##"<View style={{ "--tone": "#fff" }} />"##,
            "a `const` at the top of the file, or a style object literal",
        ),
        (
            r#"<View className="bg-brand-500" />"#,
            "no built-in color matches this name; use a literal color such as `bg-[#2563eb]`, or `style={{ backgroundColor: accent }}`",
        ),
        (
            r#"<View className="text-huge" />"#,
            "no implemented utility expansion; use an admitted CSS property through style or [property:value]",
        ),
        (
            r#"<View className="animate-wiggle" />"#,
            "`animate-*` is not a Motion loop; use `interpolate` over local time, for example `interpolate(ctx.seconds % 1, [0, 1], [0, 360])`",
        ),
        (
            r#"<View style={{ animation: "spin 1s linear infinite" }} />"#,
            "CSS keyframes are permanently unsupported",
        ),
        (
            r#"<View style={{ animation: "spin 1s linear infinite" }} />"#,
            "use `interpolate` over local time, for example `interpolate(ctx.seconds % 1, [0, 1], [0, 360])`",
        ),
        (
            r#"<View style={{ transition: "opacity 0.2s" }} />"#,
            "an explicit range: `interpolate(ctx.seconds, [start, end], [a, b])`",
        ),
        (
            r#"<View style={{ width: "fit-content" }} />"#,
            "an explicit length or percentage, or `measureText` for text-sized boxes",
        ),
        (
            r#"<View style={{ width: "min(10px, 5%)" }} />"#,
            "`calc()`, or a Motion expression for the numeric part",
        ),
        (
            r#"<View style={{ display: "grid", gridTemplateColumns: "subgrid" }} />"#,
            "explicit tracks such as `grid-cols-[1fr_2fr]`",
        ),
        (
            r#"<View style={{ overflow: "auto" }} />"#,
            "`overflow-hidden`, `overflow-clip`, or `overflow-visible`",
        ),
        (
            r#"<View style={{ clipPath: "circle(40%)" }} />"#,
            "`<Clip path={...} />`",
        ),
        (
            r#"<View style={{ maskImage: "url(a.png)" }} />"#,
            "`<Mask paint={...} />` or a bound image source",
        ),
        (
            r#"<View style={{ offsetPath: "path('M0 0')" }} />"#,
            "the `motionPath` helper",
        ),
        (
            r#"<View style={{ borderImage: "url(a.png) 30" }} />"#,
            "a border, a background gradient, or an explicit `<Image>`",
        ),
        (
            r##"<Text split="word" perUnit={{ opacity: 1 }}>one <Span style={{ color: "#fff" }}>two</Span></Text>"##,
            "keep the split text in its own single-run Text, or drop split/perUnit",
        ),
        (
            r##"<Text path={path("M0 0L100 0")}>one <Span style={{ color: "#fff" }}>two</Span></Text>"##,
            "keep the path text in its own single-run Text, or drop the path",
        ),
    ] {
        rejection(body, phrase);
    }
}

#[test]
fn module_and_text_shape_rejections_are_documented() {
    for (body, phrase) in [
        (
            r#"function Inner(ctx, { card: { title } }) { return <View style={{ opacity: title }} />; }
export default function D(){return <Scene style={{width:320,height:180}}><Inner card={{ title: 1 }} /></Scene>;}"#,
            "nested props destructuring is not supported; bind one scalar field at a time",
        ),
        (
            r#"export const helper = 1;
export default function D(){return <Scene style={{width:320,height:180}}/>;}"#,
            "is not part of the module contract",
        ),
    ] {
        let source = format!("{COMPOSITION}{body}");
        let graph = MotionModuleGraph::new(
            "entry.motion.tsx",
            BTreeMap::from([("entry.motion.tsx".to_owned(), source)]),
        )
        .expect("graph");
        let diagnostics = compile_motion_modules_with_full_env(&graph, &[], Some(&env()), None)
            .expect_err("the trigger must be rejected");
        let text = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains(phrase), "got: {text}");
    }
}

#[test]
fn measure_text_rejections_are_documented() {
    let source = format!(
        "{COMPOSITION}const M = measureText('x', {{ className: 'text-sm' }});\n\
         {BODY_OPEN}<View style={{{{ width: M.width }}}} /></Scene>;}}\n"
    );
    let diagnostics = compile_motion_with_env(&source, &[], Some(&env()))
        .expect_err("className is not part of the measureText signature");
    let text = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let phrase = "measureText does not accept `className`; pass explicit typography instead: { fontSize, fontFamily, fontWeight, letterSpacing, lineHeight }";
    assert!(text.contains(phrase), "got: {text}");
}

#[test]
fn delivery_contract_is_required_by_rendering_not_compilation() {
    // An unbound component can compile, even when its path ends in `.motion.tsx`.
    let graph = MotionModuleGraph::new(
        "entry.motion.tsx",
        BTreeMap::from([(
            "entry.motion.tsx".to_owned(),
            "export default function D(){return <Scene style={{width:320,height:180}}/>;}\n"
                .to_owned(),
        )]),
    )
    .expect("graph");
    let compiled = compile_motion_modules_with_full_env(&graph, &[], Some(&env()), None)
        .expect("unbound component compiles");
    assert!(compiled.artifact.composition.is_none());

    // A component module must not declare the contract.
    let graph = MotionModuleGraph::new(
        "entry.motion.tsx",
        BTreeMap::from([
            (
                "entry.motion.tsx".to_owned(),
                format!("{COMPOSITION}import {{ helper }} from './lib.motion';\n{BODY_OPEN}<View style={{{{ opacity: helper }}}} /></Scene>;}}\n"),
            ),
            (
                "lib.motion.tsx".to_owned(),
                format!("{COMPOSITION}export const helper = 1;\n"),
            ),
        ]),
    )
    .expect("graph");
    let diagnostics = compile_motion_modules_with_full_env(&graph, &[], Some(&env()), None)
        .expect_err("only the entry may declare the delivery contract");
    let text = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let phrase = "belongs to the entry `.motion.tsx` only; a component module must not declare the delivery contract";
    assert!(text.contains(phrase), "got: {text}");
}
