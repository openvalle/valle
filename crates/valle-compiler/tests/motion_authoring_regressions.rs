#![cfg(feature = "motion")]
use valle_compiler::motion::compile_motion;

#[test]
fn only_list_roots_need_keys_and_static_descendants_survive_reordering() {
    let source = |items: &str| {
        format!(
            r#"const items = {items};
      export default function Scene1() {{return <Scene>{{items.map(x => <View key={{x}}><Text>{{x}}</Text></View>)}}</Scene>;}}"#
        )
    };
    let first = compile_motion(&source("[\"A\",\"B\"]")).unwrap();
    let second = compile_motion(&source("[\"B\",\"A\"]")).unwrap();
    let keys = |compiled: &valle_compiler::motion::CompiledMotion| {
        compiled
            .artifact
            .nodes
            .iter()
            .map(|node| node.key.clone())
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(keys(&first), keys(&second));
    let missing = source("[\"A\"]").replace(" key={x}", "");
    assert!(
        compile_motion(&missing)
            .unwrap_err()
            .iter()
            .any(|d| d.message.contains("stable `key`"))
    );
}

#[test]
fn unknown_style_properties_are_authoring_errors() {
    let error = compile_motion(
        r##"export default function T(){return <View style={{backgroundColour:"#f00"}}/>;}"##,
    )
    .unwrap_err();
    assert!(error.iter().any(|d| d.message.contains("backgroundColour")));
}

#[test]
fn authored_motion_style_extensions_remain_available() {
    compile_motion(r#"export default function T(){return <View style={{width:64,height:64,paperGrain:0.1,contactShadow:0.2,perspective:500,rotateX:15,rotateY:20}}/>;}"#).unwrap();
}
