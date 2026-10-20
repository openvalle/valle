#![cfg(feature = "motion")]
//! Supported math, operand types and static topology constraints.
use valle_compiler::motion::compile_motion;
use valle_motion::Expr;

fn diagnostics_of(source: &str) -> Vec<String> {
    compile_motion(source)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

/// Assert that one diagnostic of `source` contains every needle.
fn expect_message(source: &str, needles: &[&str]) {
    let messages = diagnostics_of(source);
    assert!(
        messages
            .iter()
            .any(|message| needles.iter().all(|needle| message.contains(needle))),
        "{needles:?} in {messages:#?}"
    );
}

#[test]
fn exactly_specified_math_members_work_at_frame_time_without_new_ir() {
    let compiled = compile_motion(
        r##"
export default function P() { const ctx = useTime();
  const t = ctx.progress;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10,
                           opacity: clamp(Math.abs(t - 0.5) * Math.min(2, 3) + Math.max(0, t), 0, 1) }} />
  </Scene>);
}
"##,
    )
    .expect("min/max/abs/clamp compose at frame time with context-dependent inputs");
    // Compose expressions exclusively from existing IR variants.
    for expr in &compiled.artifact.exprs {
        assert!(
            matches!(
                expr,
                Expr::Const { .. }
                    | Expr::Context { .. }
                    | Expr::Add { .. }
                    | Expr::Sub { .. }
                    | Expr::Mul { .. }
                    | Expr::Neg { .. }
                    | Expr::Compare { .. }
                    | Expr::Select { .. }
            ),
            "min/max/abs/clamp must be composed from existing primitives, found {expr:?}"
        );
    }
    compiled.artifact.validate().expect("artifact validates");
}

#[test]
fn approximated_math_members_stay_rejected() {
    let messages = diagnostics_of(
        r##"
export default function P() { const ctx = useTime();
  const t = ctx.progress;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10,
                           opacity: Math.log(t) }} />
  </Scene>);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("implementation-approximated")),
        "got {messages:#?}"
    );
}

#[test]
fn comparing_a_length_against_a_number_names_the_call_the_author_wrote() {
    let messages = diagnostics_of(
        r##"
export default function P() { const ctx = useTime();
  const t = ctx.progress;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: Math.min(t, "10px"), top: 0, width: 10, height: 10 }} />
  </Scene>);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("min") && message.contains("plain number")),
        "got {messages:#?}"
    );
}

#[test]
fn exponentiation_assignment_points_at_the_plain_operator() {
    let messages = diagnostics_of(
        r##"
function grow(x) { let y = x; y **= 2; return y; }
export default function P() { const ctx = useTime();
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: grow(3), top: 0, width: 10, height: 10 }} />
  </Scene>);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("**=") && message.contains("x = x ** y")),
        "got {messages:#?}"
    );
}

#[test]
fn a_literal_double_star_in_authored_text_is_not_mistaken_for_the_operator() {
    compile_motion(
        r##"
export default function P() { const ctx = useTime();
  return (<Scene className="h-full w-full">
    <Text key="t" style={{ position: "absolute", left: 0, top: 0, fontSize: 20, color: "#fff" }}>a ** b</Text>
  </Scene>);
}
"##,
    )
    .expect("`**` inside authored text is just two characters");
}

#[test]
fn a_frame_dependent_loop_points_at_the_prepare_time_map_that_replaces_it() {
    let messages = diagnostics_of(
        r##"
export default function P() { const ctx = useTime();
  for (let i = 0; i < ctx.progress * 3; i++) {}
  return (<Scene className="h-full w-full" />);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("`.map`") || message.contains(".map")),
        "got {messages:#?}"
    );
}

#[test]
fn reversed_clamp_bounds_are_rejected_on_every_path() {
    // Constant arguments inside a component.
    let diagnostics = diagnostics_of(
        r##"
export default function P() { const ctx = useTime();
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10,
                           width: clamp(5, 10, 3) }} />
  </Scene>);
}
"##,
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("low <= high")),
        "all-constant clamp must be rejected too: {diagnostics:?}"
    );

    // Constant arguments at module scope.
    let diagnostics = diagnostics_of(
        r##"
const x = clamp(5, 10, 3);
export default function P() { const ctx = useTime();
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10, width: x }} />
  </Scene>);
}
"##,
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("low <= high")),
        "module-scope clamp must be rejected too: {diagnostics:?}"
    );

    // Frame-time value with static bounds.
    let diagnostics = diagnostics_of(
        r##"
export default function P() { const ctx = useTime();
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10,
                           width: clamp(ctx.progress, 10, 3) }} />
  </Scene>);
}
"##,
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("low <= high")),
        "{diagnostics:?}"
    );
}

#[test]
fn well_formed_clamp_still_works_on_both_paths() {
    let folded = compile_motion(
        r##"
const x = clamp(5, 0, 3);
export default function P() { const ctx = useTime();
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10, width: x,
                           opacity: clamp(ctx.progress, 0, 1) }} />
  </Scene>);
}
"##,
    )
    .expect("well-formed clamp compiles on both paths");
    let width = folded.artifact.nodes.iter().find_map(|node| {
        node.styles
            .iter()
            .find(|entry| entry.property == "width")
            .map(|entry| entry.value.clone())
    });
    assert_eq!(
        width,
        Some(valle_motion::StyleValue::Static {
            value: valle_motion::MotionValue::Number(3.0)
        }),
        "clamp(5, 0, 3) folds to 3 at compile time"
    );
}

#[test]
fn kept_restrictions_name_what_to_write_instead() {
    let expect = expect_message;
    let scene =
        r#"<Scene><View key="a" style={{ width: 10, height: 10, opacity: VALUE }} /></Scene>"#;
    let component = |value: &str| {
        format!(
            "export default function P() {{ const ctx = useTime(); return {}; }}",
            scene.replace("VALUE", value)
        )
    };
    expect(&component("Math.random()"), &["seededRandom", "wiggle"]);
    expect(&component("Date.now()"), &["pure function of time"]);
    expect(
        &format!(
            "async function load() {{ return 1; }}\n{}",
            component("load()")
        ),
        &["no async code", "asset()", "--data"],
    );
    expect(
        &component("1").replace("export default function", "export default async function"),
        &["no async code", "controls.assets"],
    );
    expect(
        &format!(
            "const LOADED = await Promise.resolve(1);\n{}",
            component("LOADED")
        ),
        &["no async code", "asset://name"],
    );
    for control in [
        r#"{ type: "array", default: [1, 2, 3] }"#,
        "array(number(), { maxItems: 3 })",
        "[1, 2, 3]",
    ] {
        expect(
            &format!(
                "export const controls = {{ props: {{ items: {control} }} }};\n{}",
                component("1")
            ),
            &["controls.data", "--data"],
        );
    }
    expect(
        &format!(
            "export const controls = {{ props: {{ size: {{ type: \"number\" }} }} }};\n{}",
            component("1")
        ),
        &["number()", "select()"],
    );
}

#[test]
fn components_take_props_and_read_the_time_and_data_through_hooks() {
    let scene = r#"<Scene><View key="a" style={{ width: 10, height: 10 }} /></Scene>"#;
    let root = |params: &str| format!("export default function P({params}) {{ return {scene}; }}");
    expect_message(&root("ctx"), &["takes `(props)`", "const ctx = useTime()"]);
    expect_message(
        &root("props, data"),
        &["takes `(props)`", "const data = useData()"],
    );
    expect_message(&root("options"), &["name the component parameter `props`"]);
    expect_message(
        "function Card(ctx, props) { return <View key=\"c\" />; }\n\
         export default function P() { return <Scene><Card key=\"c\" /></Scene>; }",
        &["takes `(props)`"],
    );
    let body = |statement: &str, opacity: &str| {
        format!(
            "export default function P() {{ {statement} return <Scene><View key=\"a\" \
             style={{{{ width: 10, height: 10, opacity: {opacity} }}}} /></Scene>; }}"
        )
    };
    expect_message(
        &body("", "ctx.progress"),
        &["`ctx` is not defined", "const ctx = useTime()"],
    );
    expect_message(&body("", "useTime()"), &["holds several numbers"]);
    expect_message(
        &body("const ctx = useTime();", "ctx.nope"),
        &["is not a number in `useTime()`"],
    );
    expect_message(
        &body("const t = useTime(1);", "1"),
        &["useTime() takes no arguments"],
    );
    expect_message(
        &format!("const NOW = useTime();\n{}", root("")),
        &["inside a component or a helper"],
    );
    expect_message(
        &format!("const useTime = () => 1;\n{}", root("")),
        &["reserved Motion intrinsics"],
    );
    expect_message(
        &body("const [count] = useState(0);", "1"),
        &["`useState` is not available", "pure function of time"],
    );
    expect_message(
        &body("const frame = useCurrentFrame();", "1"),
        &["useTime().localFrame"],
    );
    expect_message(
        &body("const config = useVideoConfig();", "1"),
        &["composition", "durationFrames"],
    );
    // A module's own function of that name is not React's.
    compile_motion(&format!(
        "const useMemo = (make) => make();\n{}",
        body("const half = useMemo(() => 0.5);", "half")
    ))
    .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
}
