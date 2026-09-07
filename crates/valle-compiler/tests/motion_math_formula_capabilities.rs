#![cfg(feature = "motion")]
//! Supported MathFormula syntax lowers to the declared formula capability.
use valle_compiler::motion::compile_motion;
use valle_motion::NodeKind;

fn compile_one(latex: &str) -> String {
    let src = format!(
        r#"
export const component = "mj12-cap";
export default function F() {{
  return <Scene><MathFormula latex={{String.raw`{latex}`}} displayMode="display" /></Scene>;
}}
"#
    );
    let compiled = compile_motion(&src).unwrap_or_else(|err| panic!("{latex}: {err:?}"));
    let NodeKind::MathFormula { latex: stored, .. } = &compiled
        .artifact
        .nodes
        .iter()
        .find(|node| matches!(node.kind, NodeKind::MathFormula { .. }))
        .unwrap()
        .kind
    else {
        panic!("missing node");
    };
    stored.clone()
}

#[test]
fn main_capability_cluster_compiles() {
    for latex in [
        r"x_i^2 + \frac{1}{2}",
        r"\sqrt{1+x}",
        r"\sum_{n=1}^{N} a_n",
        r"\left(\frac{a}{b}\right)",
        r"\hat{x}+\bar{y}",
        r"\begin{pmatrix} 1 & 0 \\ 0 & 1 \end{pmatrix}",
        r"\begin{aligned} a &= b \\ c &= d \end{aligned}",
        r"\text{Hello}",
        r"\sin x + \operatorname{tr} A",
        r"\def\q#1{#1^2}\q{z}",
        r"\tag{*} x=1",
    ] {
        let stored = compile_one(latex);
        assert_eq!(stored, latex);
    }
}
