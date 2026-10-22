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
            "a `const` at the top of the file, or an expression for a value that changes per frame",
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
            "`animate-*` is not a Motion loop; use `interpolate` over local time, for example `interpolate(seconds % 1, [0, 1], [0, 360])`",
        ),
        (
            r#"<View style={{ animation: "spin 1s linear infinite" }} />"#,
            "CSS keyframes are permanently unsupported",
        ),
        (
            r#"<View style={{ animation: "spin 1s linear infinite" }} />"#,
            "use `interpolate` over local time, for example `interpolate(seconds % 1, [0, 1], [0, 360])`",
        ),
        (
            r#"<View style={{ transition: "opacity 0.2s" }} />"#,
            "an explicit range: `interpolate(seconds, [start, end], [a, b])`",
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
            r#"function Inner({ card: { title } }) { return <View style={{ opacity: title }} />; }
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

/// A module whose component reads the frame as `ctx`: `helpers` at the top, then the scene with
/// `body` among its children.
fn scene(helpers: &str, body: &str) -> String {
    format!(
        "{helpers}\nexport default function D(){{const time = useCurrentTime(); const frame = Math.floor(time.seconds * 30);\n\
         return <Scene style={{{{width:320,height:180}}}}>{body}</Scene>;}}\n"
    )
}

/// A module whose component runs `statements` before it returns the scene.
fn body(statements: &str) -> String {
    format!(
        "export default function D(){{const time = useCurrentTime(); const frame = Math.floor(time.seconds * 30);\n{statements}\n\
         return <Scene style={{{{width:320,height:180}}}}><View style={{{{ width: 10, height: 10 }}}} /></Scene>;}}\n"
    )
}

#[test]
fn language_rejections_name_a_replacement() {
    let recursion = "which the compiler cannot expand; flatten the nested data into a prepared \
                     array and draw it with `.map`";
    for (module, phrase) in [
        (
            scene(
                "function Nest({ depth }) { return depth <= 0 ? <View /> : <View style={{ padding: 2 }}><Nest depth={depth - 1} /></View>; }",
                "<Nest depth={3} />",
            ),
            format!("component `Nest` is recursive, {recursion}"),
        ),
        (
            scene(
                "function Boxes(n) { return n <= 0 ? null : <View style={{ width: n, height: 4 }}>{Boxes(n - 1)}</View>; }",
                "{Boxes(3)}",
            ),
            format!("helper `Boxes` is recursive, {recursion}"),
        ),
        (
            scene(
                "function fact(n) { return n <= 1 ? 1 : n * fact(n - 1); }",
                "<View style={{ width: fact(frame), height: 10 }} />",
            ),
            "write the repetition as a `for` loop whose bound is known while the scene is prepared"
                .into(),
        ),
        (
            scene(
                r#"function* boxes() { yield <View key="a" style={{ width: 5, height: 5 }} />; }"#,
                "{boxes()}",
            ),
            "helper `boxes` is a generator; return an array instead, built with `.map` or `Array.from`"
                .into(),
        ),
        (
            scene(
                "function Row(...widths) { return <View>{widths.map((w, i) => <View key={`r${i}`} style={{ width: w, height: 4 }} />)}</View>; }",
                "{Row(1, 2, 3)}",
            ),
            "helper `Row` takes `...rest`; take one array parameter instead and call it as `Row([a, b, c])`"
                .into(),
        ),
        (
            scene(
                "const xs = [1, 2, 3];",
                "{xs.filter((x) => x > time.seconds).map((x) => <View key={`f${x}`} style={{ width: x, height: 4 }} />)}",
            ),
            "map the whole prepared array and decide each item per frame".into(),
        ),
        (
            scene(
                "",
                "{Array.from({ length: frame }, (_, i) => <View key={`a${i}`} style={{ width: 4, height: 4 }} />)}",
            ),
            "the array it repeats cannot change with the frame".into(),
        ),
        (
            scene(
                "",
                "<View onClick={() => {}} style={{ width: 10, height: 10 }} />",
            ),
            "remove it, and drive the change with `useCurrentTime()` or a prop".into(),
        ),
        (
            scene("class Box { constructor(w) { this.w = w; } }", ""),
            "a Motion module declares no classes; write a function that returns an object".into(),
        ),
        (
            scene("let size = 10;", "<View style={{ width: size, height: 10 }} />"),
            "module scope keeps no state; declare the value with `const`".into(),
        ),
        (
            body("const o = { a: 1, b: 2 };\ndelete o.a;"),
            "leave the field out of a new one with rest destructuring, as in `const { removed, ...rest } = object`"
                .into(),
        ),
        (
            body("let w = 10;\nswitch (2) { case 1: w = 5; break; default: w = 20; }"),
            "write `if`/`else if`, or a chain of `cond ? a : b`".into(),
        ),
        (
            scene(
                "function Vec(x) { this.x = x; }",
                "<View style={{ width: new Vec(time.seconds).x, height: 10 }} />",
            ),
            "`new` does not run per frame".into(),
        ),
        (
            scene(
                r#"function Card(props) { return <View style={{ width: typeof props.width === "number" ? props.width : 5, height: 10 }} />; }"#,
                "<Card width={20} />",
            ),
            "`typeof` does not run per frame".into(),
        ),
        (
            scene(
                "function Card(props) { return <View style={{ width: props.width ?? 100, height: 10 }} />; }",
                "<Card width={20} />",
            ),
            "give a prop its default in the parameter, as in `function Card({ width = 100 })`".into(),
        ),
        (
            scene("", "<View style={{ width: (frame | 0) + 1, height: 10 }} />"),
            "write `Math.trunc(x)` for `x | 0`".into(),
        ),
        (
            scene("", "<View style={{ width: ~frame, height: 10 }} />"),
            "write `-x - 1` for `~x`".into(),
        ),
        (
            scene("", "<View style={{ width: time.seconds > 1 ? 5 : null, height: 10 }} />"),
            "`null` is not a value of the frame".into(),
        ),
        (
            body("let w = 1;\nconst x = (w = time.seconds);"),
            "assign on a line of its own in the body".into(),
        ),
        (
            scene(
                "",
                "<View style={{ width: [1, 2].indexOf(frame), height: 10 }} />",
            ),
            "compute a method call such as `list.indexOf(x)` in a module-level `const`".into(),
        ),
    ] {
        let text = messages("entry.motion.tsx", &module);
        assert!(
            text.contains(&phrase),
            "`{module}` must name `{phrase}`, got: {text}"
        );
    }
}

#[test]
fn the_named_replacements_compile() {
    for module in [
        scene(
            "function Card({ width = 100 }) { return <View style={{ width, height: 10 }} />; }",
            "<Card width={20} /><Card />",
        ),
        scene(
            "function Row(widths) { return <View>{widths.map((w, i) => <View key={`r${i}`} style={{ width: w, height: 4 }} />)}</View>; }",
            "{Row([1, 2, 3])}",
        ),
        scene(
            "function boxes() { return Array.from({ length: 2 }, (_, i) => <View key={`b${i}`} style={{ width: i + 1, height: 5 }} />); }",
            "{boxes()}",
        ),
        scene(
            "",
            "{Array.from({ length: 3 }, (_, i) => <View key={`a${i}`} style={{ width: i + 1, height: 4 }} />)}",
        ),
        scene(
            r#"const items = [{ id: "a", at: 0.2 }, { id: "b", at: 0.6 }];"#,
            "{items.map((item) => (item.at < time.seconds ? <View key={item.id} style={{ width: 4, height: 4 }} /> : null))}",
        ),
        scene(
            "function fact(n) { let r = 1; for (let i = 2; i <= 6; i++) { r = i <= n ? r * i : r; } return r; }",
            "<View style={{ width: fact(frame % 4), height: 10 }} />",
        ),
        scene(
            r#"function flatten(node, depth) { return [{ id: node.id, depth }, ...node.children.flatMap((child) => flatten(child, depth + 1))]; }
const rows = flatten({ id: "a", children: [{ id: "b", children: [{ id: "c", children: [] }] }] }, 0);"#,
            "{rows.map((row) => <View key={row.id} style={{ marginLeft: row.depth * 8, width: 10, height: 4 }} />)}",
        ),
        scene(
            "function box(w) { return { w }; }\nconst B = box(3);",
            "<View style={{ width: B.w, height: 10 }} />",
        ),
        scene(
            "const W = new Map([[1, 2]]).get(1);",
            "<View style={{ width: W * 10, height: 10 }} />",
        ),
        scene(
            "",
            "<View style={{ width: Math.trunc(time.seconds * 10) + 1, height: 10 }} />",
        ),
        scene(
            "",
            "<View style={{ width: Math.floor(time.progress * 3) < 1 ? 10 : Math.floor(time.progress * 3) < 2 ? 20 : 30, height: 10 }} />",
        ),
        body("const o = { a: 1, b: 2 };\nconst { a, ...rest } = o;\nconst width = rest.b * 5;"),
        body(
            "const i = Math.floor(time.progress * 3);\nlet w = 30;\nif (i < 1) { w = 10; } else if (i < 2) { w = 20; }",
        ),
    ] {
        let source = format!("{COMPOSITION}{module}");
        let graph = MotionModuleGraph::new(
            "entry.motion.tsx",
            BTreeMap::from([("entry.motion.tsx".to_owned(), source)]),
        )
        .expect("graph");
        if let Err(diagnostics) =
            compile_motion_modules_with_full_env(&graph, &[], Some(&env()), None)
        {
            let text = diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect::<Vec<_>>()
                .join("\n");
            panic!("a replacement a diagnostic names must compile: {module}\n{text}");
        }
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
    let phrase = "`composition` and `role` belong to the entry `.motion.tsx` only; a component module must not declare the delivery contract";
    assert!(text.contains(phrase), "got: {text}");
}
