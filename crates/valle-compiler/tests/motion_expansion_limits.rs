#![cfg(feature = "motion")]
//! Recursion, helper fan-out and expression nesting obey bounded expansion budgets.
use valle_compiler::motion::compile_motion;

fn diagnostics_of(source: &str) -> Vec<String> {
    compile_motion(source)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

#[test]
fn a_recursive_jsx_helper_is_rejected_rather_than_expanded_forever() {
    let messages = diagnostics_of(
        r##"
const bar = (i) => (<View key={`g-${i}`} style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10 }}>{bar(i)}</View>);
export default function P(ctx) {
  return (<Scene className="h-full w-full">{bar(0)}</Scene>);
}
"##,
    );
    assert!(
        messages.iter().any(|message| message.contains("recursive")),
        "got {messages:#?}"
    );
}

#[test]
fn exponential_helper_fan_out_hits_the_expansion_budget_instead_of_hanging() {
    let mut chain = String::from("  const h14 = (x) => x + 1;\n");
    for i in (1..14).rev() {
        chain.push_str(&format!(
            "  const h{i} = (x) => h{}(x) + h{}(x);\n",
            i + 1,
            i + 1
        ));
    }
    let source = format!(
        r##"
export default function P(ctx) {{
{chain}
  return (<Scene className="h-full w-full">
    <View key="a" style={{{{ position: "absolute", left: 0, top: 0, height: 10,
                           width: h1(ctx.hold.progress) }}}} />
  </Scene>);
}}
"##
    );
    let diagnostics = diagnostics_of(&source);
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("expansion exceeded the module budget")),
        "{diagnostics:?}"
    );
}

#[test]
fn deep_expression_chain_hits_the_nesting_budget_instead_of_the_stack() {
    let chain = " + 0.001".repeat(300);
    let source = format!(
        r##"export default function Deep(ctx) {{
  return (<Scene key="s" className="relative" style={{{{ width: "64px", height: "64px" }}}}>
    <View key="b" className="absolute" style={{{{ left: "0px", top: "0px", width: "8px",
      height: "8px", backgroundColor: "#22c55e",
      opacity: interpolate(ctx.enter.progress{chain}, [0, 300], [0, 1]) }}}} />
  </Scene>);
}}
"##
    );
    let diagnostics = diagnostics_of(&source);
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("expression nesting exceeds the compiler budget")),
        "{diagnostics:?}"
    );
    assert_eq!(
        diagnostics.len(),
        1,
        "expect one diagnostic per chain: {diagnostics:?}"
    );
}

#[test]
fn chains_inside_the_nesting_budget_still_compile() {
    let chain = " + 0.001".repeat(200);
    let source = format!(
        r##"export default function Deep(ctx) {{
  return (<Scene key="s" className="relative" style={{{{ width: "64px", height: "64px" }}}}>
    <View key="b" className="absolute" style={{{{ left: "0px", top: "0px", width: "8px",
      height: "8px", backgroundColor: "#22c55e",
      opacity: interpolate(ctx.enter.progress{chain}, [0, 200], [0, 1]) }}}} />
  </Scene>);
}}
"##
    );
    compile_motion(&source).expect("a 400-level chain must compile within the 512-level budget");
}
