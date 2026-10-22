#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;

#[test]
fn named_default_export_is_the_only_component_name_and_controls_are_optional() {
    let source = r#"export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export default function Simple() { const time = useCurrentTime();
  return <Scene><View style={{ width: 64, opacity: time.progress }} /></Scene>;
}"#;
    let compiled = compile_motion(source).unwrap();
    assert_eq!(compiled.artifact.component, "Simple");
    assert!(compiled.artifact.controls.props.is_empty());
}

#[test]
fn extra_component_export_and_a_second_root_parameter_are_rejected() {
    let source = r#"export const composition = { width: 64, height: 64, duration: 1 };
export const component = "alias";
export default function Named() { return <Scene />; }"#;
    assert!(compile_motion(source).is_err());

    let source = r#"export const composition = { width: 64, height: 64, duration: 1 };
export default function Named(props, data) { return <Scene />; }"#;
    assert!(compile_motion(source).is_err());
}
