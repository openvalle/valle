#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{
    EvalInputs, MotionValue, SceneArtifact, StyleValue, eval_all, motion_context_at_frame,
    resolve_props,
};
use valle_timeline::FrameRate;

fn binding(artifact: &SceneArtifact, key: &str, property: &str) -> valle_motion::ExprId {
    let node = artifact.nodes.iter().find(|node| node.key == key).unwrap();
    match node
        .styles
        .iter()
        .find(|style| style.property == property)
        .unwrap()
        .value
    {
        StyleValue::Expr { expr } => expr,
        ref other => panic!("expected runtime binding: {other:?}"),
    }
}

#[test]
fn identical_lowered_helpers_share_one_expression_and_keep_random_access_values() {
    let source = r#"
    const repeat = (t) => sin(t * 2) * 10 + 20;
    export default function Main() { const ctx = useTime(); return <Scene>
      <View key="a" style={{width:repeat(ctx.seconds),height:10}} />
      <View key="b" style={{width:repeat(ctx.seconds),height:10}} />
    </Scene>; }"#;
    let compiled = compile_motion(source).unwrap();
    let artifact = &compiled.artifact;
    let a = binding(artifact, "a", "width");
    assert_eq!(a, binding(artifact, "b", "width"));
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    for frame in [59, 0, 31, 31, 2, 59, 0] {
        let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let values = eval_all(
            artifact,
            EvalInputs {
                ctx: &ctx,
                props: &props,
                unit: None,
                viewport: None,
            },
        )
        .unwrap();
        let expected = valle_draw::math::sin(ctx.sample.composition().as_f64() * 2.0) * 10.0 + 20.0;
        assert_eq!(values[a.0 as usize], MotionValue::Number(expected));
    }
    assert_eq!(compiled.source_map.exprs.len(), artifact.exprs.len());
}

#[test]
fn host_source_and_time_scopes_remain_distinct_after_reuse() {
    let source = r#"
    function Bar() { const ctx = useTime(); return <View key="bar" style={{width:ctx.seconds*10,height:10}} />; }
    export default function Main() { const ctx = useTime(); return <Scene>
      <View key="source" style={{width:ctx.seconds*10,height:10}} />
      <View key="host" style={{width:ctx.host.seconds*10,height:10}} />
      <TimeScope key="scope" offset={0.5} speed={2}><Bar /></TimeScope>
    </Scene>; }"#;
    let artifact = compile_motion(source).unwrap().artifact;
    assert_ne!(
        binding(&artifact, "source", "width"),
        binding(&artifact, "host", "width")
    );
    let scoped = artifact
        .nodes
        .iter()
        .find(|node| node.key.ends_with("/bar"))
        .unwrap();
    let scoped = binding(&artifact, &scoped.key, "width");
    assert_ne!(scoped, binding(&artifact, "source", "width"));
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let mut ctx = motion_context_at_frame(45, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    ctx.host = valle_motion::MotionHostContext::new(
        valle_timeline::internal::SampleTime::new(valle_timeline::RationalTime::new(7, 1).unwrap()),
        valle_timeline::RationalTime::new(10, 1).unwrap(),
        45,
        60,
    )
    .unwrap();
    let values = eval_all(
        &artifact,
        EvalInputs {
            ctx: &ctx,
            props: &props,
            unit: None,
            viewport: None,
        },
    )
    .unwrap();
    assert_eq!(
        values[binding(&artifact, "source", "width").0 as usize],
        MotionValue::Number(15.0)
    );
    assert_eq!(
        values[binding(&artifact, "host", "width").0 as usize],
        MotionValue::Number(70.0)
    );
    assert_eq!(values[scoped.0 as usize], MotionValue::Number(20.0));
}

#[test]
fn instance_rollback_folding_and_later_nodes_do_not_share_discarded_ids() {
    let source = r#"
    const A = Array.from({length:80}, (_,i)=>({id:`a${i}`,x:i*2}));
    const B = Array.from({length:80}, (_,i)=>({id:`b${i}`,x:i*3}));
    export default function Main() { const ctx = useTime(); return <Scene>
      {A.map(row=><View key={row.id} style={{width:row.x+ctx.seconds*2,height:10}} />)}
      {B.map(row=><View key={row.id} style={{width:row.x+ctx.seconds*3,height:10}} />)}
      <View key="later" style={{width:ctx.seconds*2+Math.sqrt(81),height:10}} />
    </Scene>; }"#;
    for _ in 0..2 {
        let artifact = compile_motion(source).unwrap().artifact;
        assert_eq!(artifact.instance_groups.len(), 2);
        artifact.validate().unwrap();
        let ctx = motion_context_at_frame(15, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
        let inputs = EvalInputs {
            ctx: &ctx,
            props: &props,
            unit: None,
            viewport: None,
        };
        let values = eval_all(&artifact, inputs).unwrap();
        assert_eq!(
            values[binding(&artifact, "later", "width").0 as usize],
            MotionValue::Number(10.0)
        );
        for (group, factor) in artifact.instance_groups.iter().zip([2.0, 3.0]) {
            let width = match group
                .template
                .styles
                .iter()
                .find(|style| style.property == "width")
                .unwrap()
                .value
            {
                StyleValue::Expr { expr } => expr,
                _ => panic!("instance width is dynamic"),
            };
            let rows = valle_motion::eval_instances(group, inputs).unwrap();
            for (row, values) in rows.iter().enumerate() {
                assert_eq!(
                    values[width.0 as usize],
                    MotionValue::Number((row as f64 + ctx.sample.composition().as_f64()) * factor)
                );
            }
        }
    }
}
