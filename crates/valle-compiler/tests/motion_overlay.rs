#![cfg(feature = "motion")]
use serde_json::{Value, json};
use std::collections::BTreeMap;
use valle_compiler::{compile_timeline_with_motion_sources, motion::compile_motion};
use valle_timeline::{
    MotionRole, MotionSourceMetadata, OverlayHold, RationalTime, decode_timeline,
};
fn source(role: &str) -> String {
    format!(
        r#"export const composition={{width:64,height:32,duration:2}};
{role}
export default function Main(ctx) {{return <Scene><View style={{{{width:ctx.viewport.width,height:ctx.viewport.height}}}}/></Scene>;}}"#
    )
}
#[test]
fn overlay_declaration_is_closed_exact_and_validated_with_composition() {
    let a = compile_motion(&source(
        "export const role=overlay({intro:seconds(0.6),outro:seconds(0.4),hold:'once'});",
    ))
    .unwrap()
    .artifact;
    assert_eq!(
        a.role,
        MotionRole::Overlay {
            intro: RationalTime::new(3, 5).unwrap(),
            outro: RationalTime::new(2, 5).unwrap(),
            hold: OverlayHold::Once
        }
    );
    assert_eq!(
        compile_motion(&source("")).unwrap().artifact.role,
        MotionRole::Clip
    );
    for declaration in [
        "overlay({intro:0.6,outro:seconds(0.4),hold:'once'})",
        "overlay({intro:frames(1),outro:seconds(0),hold:'once'})",
        "overlay({intro:seconds(-0.1),outro:seconds(0),hold:'loop'})",
        "overlay({intro:seconds(1.6),outro:seconds(0.5),hold:'stretch'})",
        "overlay({intro:seconds(0.5),outro:seconds(0.5),hold:'repeat'})",
        "overlay({intro:seconds(0.5),outro:seconds(0.5),hold:'once',extra:1})",
        "overlay({intro:seconds(0.5),outro:seconds(0.5)})",
        "{type:'overlay',intro:0.5,outro:0.5,hold:'once'}",
        "overlay({intro:seconds(Infinity),outro:seconds(0),hold:'once'})",
    ] {
        assert!(
            compile_motion(&source(&format!("export const role={declaration};"))).is_err(),
            "{declaration}"
        );
    }
    assert!(compile_motion("export const role=overlay({intro:seconds(0),outro:seconds(0),hold:'once'}); export default function Main(){return <Scene/>;}").is_err());
}
fn author(duration: f64) -> Value {
    json!({"canvas":{"width":64,"height":32,"fps":30},"resources":{"card":"card.motion.tsx"},"tracks":{"visual":[{"clips":[{"kind":"motion","component":"card","start":0,"duration":duration}]}]}})
}
#[test]
fn timeline_admits_host_placement_and_rejects_media_fields_and_short_hosts() {
    let metadata = BTreeMap::from([(
        "card".into(),
        MotionSourceMetadata {
            duration: RationalTime::new(2, 1).unwrap(),
            role: MotionRole::Overlay {
                intro: RationalTime::new(1, 2).unwrap(),
                outro: RationalTime::new(1, 2).unwrap(),
                hold: OverlayHold::Once,
            },
        },
    )]);
    for duration in [1.0, 1.5, 4.0] {
        let canonical = compile_timeline_with_motion_sources(
            decode_timeline(&author(duration).to_string()).unwrap(),
            &metadata,
        )
        .unwrap();
        assert!(
            valle_timeline::internal::encode_canonical(&canonical)
                .unwrap()
                .contains("overlay")
        );
    }
    let error = compile_timeline_with_motion_sources(
        decode_timeline(&author(0.8).to_string()).unwrap(),
        &metadata,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("overlay") && error.contains("/tracks/visual/0/clips/0"));
    assert!(error.contains("component `card`"), "{error}");
    assert!(error.contains("host duration 0.8 s"), "{error}");
    assert!(error.contains("intro 0.5 s + outro 0.5 s = 1 s"), "{error}");
    assert!(error.contains("template duration 2 s"), "{error}");
    assert!(
        !error.contains("motion-") && !error.contains("4/5"),
        "{error}"
    );
    for (field, value) in [
        ("fit", json!("contain")),
        ("trimStart", json!(0)),
        ("rate", json!(1)),
        ("end", json!("hold")),
    ] {
        let mut input = author(4.0);
        input["tracks"]["visual"][0]["clips"][0][field] = value;
        let error = compile_timeline_with_motion_sources(
            decode_timeline(&input.to_string()).unwrap(),
            &metadata,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains(field)
                && error.contains("overlay")
                && error.contains("/tracks/visual/0/clips/0"),
            "{error}"
        );
    }
}

#[test]
fn overlay_temporal_samples_hold_at_the_authored_exclusive_boundary() {
    use valle_motion::{
        Fonts, LayoutOptions, Viewport, build_tree, motion_context_at_sample, prepare_scene,
        resolve_props,
    };
    use valle_timeline::{FrameRate, internal::SampleTime};
    let compiled = compile_motion(
        r#"export const composition={width:64,height:32,fps:4,duration:1.14};
export const role=overlay({intro:seconds(0),outro:seconds(0),hold:"once"});
export default function Main(ctx){return <Scene><Shutter key="exposure" samples={4} angle={360}>
<View style={{width:ctx.seconds*10,height:10,backgroundColor:"white"}}/></Shutter></Scene>;}
"#,
    )
    .unwrap();
    let prepared = prepare_scene(&compiled.artifact).unwrap();
    let props = resolve_props(&compiled.artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let ctx = motion_context_at_sample(
        SampleTime::new(RationalTime::new(28, 25).unwrap()),
        5,
        FrameRate::new(4, 1).unwrap(),
    )
    .unwrap();
    let tree = build_tree(
        &prepared,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((64, 32)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let times = tree.shutter_sample_times("exposure").unwrap();
    assert_eq!(
        times
            .iter()
            .map(|time| time.composition())
            .collect::<Vec<_>>(),
        vec![
            RationalTime::new(821, 800).unwrap(),
            RationalTime::new(871, 800).unwrap(),
            RationalTime::ONE,
            RationalTime::ONE,
        ]
    );
}

#[test]
fn only_the_entry_module_may_declare_a_role() {
    use valle_compiler::motion::{MotionModuleGraph, compile_motion_modules};
    let graph=MotionModuleGraph::new("entry.motion.tsx",BTreeMap::from([
        ("entry.motion.tsx".into(),"export const composition={width:64,height:32,duration:2}; import {Card} from './card'; export default function Main(){return <Scene><Card/></Scene>; }".into()),
        ("card.tsx".into(),"export const role=overlay({intro:seconds(0),outro:seconds(0),hold:'once'}); export function Card(){return <View/>;}".into()),
    ])).unwrap();
    let diagnostics = compile_motion_modules(&graph).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("role")
                && diagnostic.message.contains("entry"))
    );
}
