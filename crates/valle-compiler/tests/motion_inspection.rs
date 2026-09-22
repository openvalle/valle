#![cfg(feature = "motion")]
use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_motion::inspect::{PropertySampleRequest, sample_properties};
use valle_motion::{EvalInputs, MotionValue, StyleValue};

const SOURCE: &str = r##"
export const controls=({props:{distance:number({default:100,min:0,max:1000})}});
export default function Scene(ctx, props) {
 return <Scene className="w-full h-full">
  <View key="track" style={{width:40,height:40,translate:point(props.distance*interpolate(ctx.seconds,[0,.25,.5],[0,1,1.1]),0),scale:point(1,1),rotate:interpolate(ctx.seconds,[0,1],["0deg","90deg"]),opacity:ctx.seconds<.5 ? .2 : 1}}/>
  <View key="bounce" style={{translate:point(100*spring({elapsedFrames:ctx.localFrame,fps:ctx.fps,initialVelocity:2,preset:"wobbly"}),0)}}/>
  <View key="relative" style={{translate:"10% 0px"}}/>
 </Scene>;
}
"##;
fn request() -> PropertySampleRequest {
    serde_json::from_value(serde_json::json!({"node":"track","startFrame":0,"endFrame":59,"maxPoints":240,"durationFrames":60,
        "fps":"60/1","props":{},"viewport":[400,300]})).unwrap()
}

#[test]
fn property_points_match_render_evaluator_and_suppress_boundary_velocity() {
    let artifact = compile_motion(SOURCE).unwrap().artifact;
    let mut request = request();
    request.node = artifact
        .nodes
        .iter()
        .find(|n| n.key == "track")
        .unwrap()
        .key
        .clone();
    for fps in ["24/1", "30/1", "60/1", "30000/1001"] {
        request.fps = serde_json::from_value(serde_json::json!(fps)).unwrap();
        let samples = sample_properties(&artifact, &request).unwrap();
        let props = valle_motion::resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
        let node = artifact
            .nodes
            .iter()
            .find(|n| n.key == request.node)
            .unwrap();
        for frame in [0, 1, 29, 15, 59, 1] {
            let ctx =
                valle_motion::motion_context_at_frame(frame, request.duration_frames, request.fps)
                    .unwrap();
            let values = valle_motion::eval_all(
                &artifact,
                EvalInputs {
                    ctx: &ctx,
                    props: &props,
                    unit: None,
                    viewport: Some((400.0, 300.0)),
                },
            )
            .unwrap();
            let opacity = node
                .styles
                .iter()
                .find(|s| s.property == "opacity")
                .unwrap();
            let StyleValue::Expr { expr } = opacity.value else {
                panic!()
            };
            let point = &samples
                .channels
                .iter()
                .find(|c| c.property == "opacity")
                .unwrap()
                .samples[frame as usize];
            assert_eq!(MotionValue::Number(point.value[0]), values[expr.0 as usize]);
        }
        assert_eq!(
            serde_json::to_vec(&samples).unwrap(),
            serde_json::to_vec(&sample_properties(&artifact, &request).unwrap()).unwrap()
        );
    }
    request.fps = serde_json::from_value(serde_json::json!("60/1")).unwrap();
    let samples = sample_properties(&artifact, &request).unwrap();
    let translate = samples
        .channels
        .iter()
        .find(|c| c.property == "translate")
        .unwrap();
    assert!((translate.samples[6].velocity.as_ref().unwrap()[0] - 400.0).abs() < 1e-5);
    assert!((translate.samples[21].velocity.as_ref().unwrap()[0] - 40.0).abs() < 1e-5);
    assert!(translate.samples[15].velocity.is_none());
    let opacity = samples
        .channels
        .iter()
        .find(|c| c.property == "opacity")
        .unwrap();
    assert!(opacity.samples[29].velocity.is_none());
    assert!(opacity.samples[30].velocity.is_none());
    assert_eq!(opacity.samples[40].velocity, Some(vec![0.0]));
}

#[test]
fn sampling_bounds_props_spring_and_relative_units_are_explicit() {
    let artifact = compile_motion(SOURCE).unwrap().artifact;
    let mut request = request();
    request.node = artifact
        .nodes
        .iter()
        .find(|n| n.key == "bounce")
        .unwrap()
        .key
        .clone();
    let samples = sample_properties(&artifact, &request).unwrap();
    assert!(
        samples.channels[0]
            .samples
            .iter()
            .any(|p| p.value[0] > 100.0)
    );
    request.duration_frames = 100_000;
    request.end_frame = 99_999;
    assert_eq!(
        sample_properties(&artifact, &request).unwrap().frames.len(),
        240
    );
    request.max_points = 241;
    assert!(sample_properties(&artifact, &request).is_err());
    request.max_points = 240;
    request.node = artifact
        .nodes
        .iter()
        .find(|n| n.key == "relative")
        .unwrap()
        .key
        .clone();
    assert!(
        sample_properties(&artifact, &request).unwrap().channels[0]
            .unavailable
            .is_some()
    );
    request.node = artifact
        .nodes
        .iter()
        .find(|n| n.key == "track")
        .unwrap()
        .key
        .clone();
    request.end_frame = 59;
    request.duration_frames = 60;
    request
        .props
        .insert("distance".into(), MotionValue::Number(200.0));
    let samples = sample_properties(&artifact, &request).unwrap();
    let translate = samples
        .channels
        .iter()
        .find(|c| c.property == "translate")
        .unwrap();
    assert!((translate.samples[6].velocity.as_ref().unwrap()[0] - 800.0).abs() < 1e-5);
}
