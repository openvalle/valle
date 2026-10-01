#![cfg(feature = "motion")]

use valle_compiler::motion::{MeasureEnv, compile_motion, compile_motion_with_env};
use valle_motion::{NodeKind, scene3d::ProceduralGeometry};

const EXTRUDED_TEXT: &str = include_str!("fixtures/motion/composition/extruded-text.motion.tsx");

const SOURCE: &str = r##"
export const composition={width:128,height:128,duration:1};
export default function T(ctx){return <Scene style={{width:128,height:128}}>
  <Scene3D key="s" camera={{position:[0,0,5],target:[0,0,0]}} style={{width:128,height:128}}>
    <Mesh key="shape" geometry={extrude(path("M -1 -1 L 1 -1 L 1 1 L -1 1 Z"),{depth:0.8,bevel:0.1})}
      material={{type:"lambert",color:"#ffffff"}} rotateY={ctx.seconds*30}/>
    <AmbientLight intensity={0.3}/>
  </Scene3D>
</Scene>}
"##;

#[test]
fn extrude_is_prepared_geometry_with_no_model_asset() {
    let artifact = compile_motion(SOURCE).unwrap().artifact;
    artifact.validate().unwrap();
    let (scene, camera) = artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Scene3D { scene, frame } => Some((scene, &frame.camera)),
            _ => None,
        })
        .unwrap();
    assert_eq!(camera.constant().unwrap().unwrap().far, 1_000.0);
    assert_eq!(scene.meshes.len(), 1);
    assert!(scene.meshes[0].model_control.is_none());
    let Some(ProceduralGeometry::Extrude { path, depth, bevel }) = &scene.meshes[0].geometry else {
        panic!("expected an extruded path")
    };
    assert_eq!((*depth, *bevel), (0.8, 0.1));
    assert_eq!(path.points.len(), 4);
    assert!(
        scene.meshes[0]
            .geometry
            .as_ref()
            .unwrap()
            .admit_model()
            .unwrap()
            .triangle_count()
            > 12
    );
    assert!(artifact.resource_refs.is_empty());
    for bad in [
        SOURCE.replace("depth:0.8,bevel:0.1", "depth:0,bevel:0.1"),
        SOURCE.replace("depth:0.8,bevel:0.1", "depth:0.8,bevel:0.5"),
        SOURCE.replace("depth:0.8,bevel:0.1", "depth:0.8,bevel:\"bad\""),
        SOURCE.replace("depth:0.8,bevel:0.1", "depth:0.8,bevel:0.1,unknown:1"),
        SOURCE.replace("M -1 -1 L 1 -1 L 1 1 L -1 1 Z", "M -1 -1 L 1 -1 L 1 1"),
        SOURCE.replace("depth:0.8,bevel:0.1", "depth:ctx.seconds,bevel:0.1"),
    ] {
        assert!(compile_motion(&bad).is_err(), "{bad}");
    }
}

#[test]
fn text_outline_flows_into_real_extruded_scene_geometry() {
    let env = MeasureEnv::new(&[], (640, 360)).unwrap();
    let artifact = compile_motion_with_env(EXTRUDED_TEXT, &[], Some(&env))
        .unwrap()
        .artifact;
    artifact.validate().unwrap();
    let (scene, camera) = artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Scene3D { scene, frame } => Some((scene, &frame.camera)),
            _ => None,
        })
        .unwrap();
    assert_eq!(camera.constant().unwrap().unwrap().far, 1_000.0);
    let geometry = scene.meshes[0].geometry.as_ref().unwrap();
    let model = geometry.admit_model().unwrap();
    assert!(model.vertex_count() > 100);
    assert!(model.triangle_count() > 40);
    assert_eq!(model.nodes()[0].id, 0);
}

#[test]
fn lathe_and_tube_are_prepared_without_model_assets() {
    let cases = [
        (
            "lathe(path(\"M 0.5 1 L 0.6 0 L 0.4 -1\"),{segments:24})",
            "lathe",
            24,
        ),
        ("lathe(path(\"M 0.5 1 L 0.6 0 L 0.4 -1\"))", "lathe", 64),
        (
            "tube(path(\"M -1 0 L 0 0.4 L 1 0\"),{radius:0.2,sides:12})",
            "tube",
            12,
        ),
        (
            "tube(path(\"M -1 0 L 0 0.4 L 1 0\"),{radius:0.2})",
            "tube",
            16,
        ),
    ];
    for (source, kind, subdivision) in cases {
        let source = SOURCE.replace(
            "extrude(path(\"M -1 -1 L 1 -1 L 1 1 L -1 1 Z\"),{depth:0.8,bevel:0.1})",
            source,
        );
        let artifact = compile_motion(&source).unwrap().artifact;
        artifact.validate().unwrap();
        assert!(artifact.resource_refs.is_empty());
        let scene = artifact
            .nodes
            .iter()
            .find_map(|node| match &node.kind {
                NodeKind::Scene3D { scene, .. } => Some(scene),
                _ => None,
            })
            .unwrap();
        assert!(scene.meshes[0].model_control.is_none());
        assert!(
            scene.meshes[0]
                .geometry
                .as_ref()
                .unwrap()
                .admit_model()
                .unwrap()
                .triangle_count()
                > 20
        );
        match (kind, scene.meshes[0].geometry.as_ref().unwrap()) {
            ("lathe", ProceduralGeometry::Lathe { segments, .. }) if *segments == subdivision => {}
            ("tube", ProceduralGeometry::Tube { radius, sides, .. })
                if *radius == 0.2 && *sides == subdivision => {}
            _ => panic!("unexpected prepared geometry"),
        }
    }
}

#[test]
fn lathe_and_tube_reject_dynamic_or_invalid_geometry() {
    for geometry in [
        "lathe(path(\"M 1 1 L 1 -1\"),{segments:2})",
        "lathe(path(\"M 1 1 L 1 -1\"),{segments:8.5})",
        "lathe(path(\"M 1 1 L 1 -1\"),{segments:ctx.seconds})",
        "lathe(path(\"M 1 1 L 1 -1\"),{radius:1})",
        "lathe(path(\"M -1 1 L 1 -1\"))",
        "tube(path(\"M -1 0 L 1 0\"),{radius:0})",
        "tube(path(\"M -1 0 L 1 0\"),{radius:0.2,sides:2})",
        "tube(path(\"M -1 0 L 1 0\"),{radius:0.2,sides:ctx.seconds})",
        "tube(path(\"M -1 0 L 1 0\"),{radius:0.2,unknown:1})",
    ] {
        let source = SOURCE.replace(
            "extrude(path(\"M -1 -1 L 1 -1 L 1 1 L -1 1 Z\"),{depth:0.8,bevel:0.1})",
            geometry,
        );
        assert!(compile_motion(&source).is_err(), "{geometry}");
    }
}
