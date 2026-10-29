use valle_compiler::motion::compile_motion;
use valle_motion::{
    NodeKind,
    scene3d::{AlphaMode, MaterialKind},
};
const PARTICLES: &str = r##"<Particles3D key="dust"
 positions={particles3d(time.seconds, {seed:7,count:12,emitter:{min:[-1,-1,-1],max:[1,1,1]},
 birth:{interval:0},lifetime:2,velocity:{x:[-0.2,0.2],y:[0.1,0.3],z:[0,0]},gravity:[0,-0.1,0],drag:0.1,loop:true})}
 sizes={[0.2,0.05]} fills={["#ffaa44","#4488ff"]} opacities={[1,0]} rotations={[0,90]} />"##;
fn source(standalone: bool) -> String {
    let camera = "camera={{position:[0,0,5],target:[0,0,0]}} style={{width:128,height:128}}";
    let node = if standalone {
        PARTICLES.replace("key=\"dust\"", &format!("key=\"dust\" {camera}"))
    } else {
        format!("<Scene3D key=\"scene\" {camera}>{PARTICLES}</Scene3D>")
    };
    format!(
        "export const composition={{width:128,height:128,duration:2}}; export default function Main(){{const time=useCurrentTime();return <Scene>{node}</Scene>;}}"
    )
}
#[test]
fn both_entry_points_lower_to_the_same_scene_and_clock() {
    let a = compile_motion(&source(true)).unwrap().artifact;
    let b = compile_motion(&source(false)).unwrap().artifact;
    for artifact in [&a, &b] {
        artifact.validate().unwrap();
        assert!(artifact.resource_refs.is_empty());
    }
    let find = |a: &valle_motion::SceneArtifact| {
        a.nodes
            .iter()
            .find_map(|n| match &n.kind {
                NodeKind::Scene3D { scene, frame } => Some((scene.clone(), frame.clone())),
                _ => None,
            })
            .unwrap()
    };
    let (sa, fa) = find(&a);
    let (sb, fb) = find(&b);
    assert_eq!(sa, sb);
    assert_eq!(fa, fb);
    assert_eq!(sa.meshes[0].material.kind, Some(MaterialKind::Unlit));
    assert_eq!(sa.meshes[0].material.alpha_mode, Some(AlphaMode::Blend));
    assert!(fa.meshes[0].particle_time.is_some());
}
#[test]
fn groups_and_dynamic_materials_are_shared() {
    let src = source(false).replace(
        PARTICLES,
        &format!(
            "<Group3D key=\"g\" rotateY={{time.seconds*90}}>{}</Group3D>",
            PARTICLES.replace(
                "sizes=",
                "material={{type:\"pbr\",roughness:time.progress}} sizes="
            )
        ),
    );
    let a = compile_motion(&src).unwrap().artifact;
    a.validate().unwrap();
    let NodeKind::Scene3D { frame, .. } = &a
        .nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Scene3D { .. }))
        .unwrap()
        .kind
    else {
        panic!()
    };
    assert_eq!(frame.meshes[0].parents.len(), 1);
}
#[test]
fn invalid_particle_options_and_cross_space_props_are_diagnosed() {
    for (before, after) in [
        ("count:12", "count:0"),
        ("count:12", "count:9000"),
        ("seed:7", "seed:-1"),
        ("lifetime:2", "lifetime:0"),
        ("drag:0.1", "drag:-1"),
        ("count:12", "count:time.seconds"),
        ("drag:0.1", "drag:0.1,typo:1"),
        ("interval:0", "interval:0,typo:1"),
        ("max:[1,1,1]", "max:[-2,1,1]"),
        ("sizes={[0.2,0.05]}", "sizes={[0.1,0.2,0.3]}"),
        ("opacities={[1,0]}", "opacities={[2,0]}"),
        ("rotations={[0,90]}", "rotations={time.seconds}"),
        (
            "key=\"dust\"",
            "key=\"dust\" camera={{position:[0,0,3],target:[0,0,0]}}",
        ),
        ("particles3d(time.seconds,", "particles(time.seconds,"),
    ] {
        let src = source(false).replace(before, after);
        assert!(compile_motion(&src).is_err(), "accepted {after}");
    }
    assert!(
        compile_motion(&source(true).replace("camera={{position:[0,0,5],target:[0,0,0]}}", ""))
            .is_err()
    );
}
