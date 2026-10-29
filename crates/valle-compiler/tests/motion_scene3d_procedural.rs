#![cfg(feature = "motion")]

use valle_compiler::motion::{MeasureEnv, compile_motion, compile_motion_with_env};
use valle_motion::{NodeKind, scene3d::ProceduralGeometry};

const EXTRUDED_TEXT: &str = include_str!("fixtures/motion/composition/extruded-text.motion.tsx");

const SOURCE: &str = r##"
export const composition={width:128,height:128,duration:1};
export default function T(){ const time = useCurrentTime();return <Scene style={{width:128,height:128}}>
  <Scene3D key="s" camera={{position:[0,0,5],target:[0,0,0]}} style={{width:128,height:128}}>
    <Mesh key="shape" geometry={extrude(path("M -1 -1 L 1 -1 L 1 1 L -1 1 Z"),{depth:0.8,bevel:0.1})}
      material={{type:"lambert",color:"#ffffff"}} rotateY={time.seconds*30}/>
    <AmbientLight intensity={0.3}/>
  </Scene3D>
</Scene>}
"##;

#[test]
fn nested_groups_preserve_local_transforms_and_time_bindings() {
    let source = SOURCE.replace("<Mesh key=", "<Group3D key=\"outer\" translateX={time.seconds} scale={[2,1,1]}><Group3D key=\"inner\" rotateZ={30}><Mesh key=")
        .replace("rotateY={time.seconds*30}/>", "rotateY={time.seconds*30}/></Group3D></Group3D>");
    let artifact = compile_motion(&source).unwrap().artifact;
    artifact.validate().unwrap();
    let mesh = artifact
        .nodes
        .iter()
        .find_map(|n| match &n.kind {
            NodeKind::Scene3D { frame, .. } => Some(&frame.meshes[0]),
            _ => None,
        })
        .unwrap();
    assert_eq!(mesh.parents.len(), 2);
    assert!(mesh.parents[0].constant().is_none());
    assert_eq!(
        mesh.parents[1].constant().unwrap().rotation_degrees.0[2],
        30.0
    );
    for bad in [
        source.replace("scale={[2,1,1]}", "scale={[0,1,1]}"),
        source.replace("key=\"inner\"", "key=\"outer\""),
        source.replace(
            "</Group3D></Group3D>",
            "<AmbientLight/></Group3D></Group3D>",
        ),
    ] {
        assert!(compile_motion(&bad).is_err(), "{bad}");
    }
}

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
        SOURCE.replace("depth:0.8,bevel:0.1", "depth:time.seconds,bevel:0.1"),
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
        "lathe(path(\"M 1 1 L 1 -1\"),{segments:time.seconds})",
        "lathe(path(\"M 1 1 L 1 -1\"),{radius:1})",
        "lathe(path(\"M -1 1 L 1 -1\"))",
        "tube(path(\"M -1 0 L 1 0\"),{radius:0})",
        "tube(path(\"M -1 0 L 1 0\"),{radius:0.2,sides:2})",
        "tube(path(\"M -1 0 L 1 0\"),{radius:0.2,sides:time.seconds})",
        "tube(path(\"M -1 0 L 1 0\"),{radius:0.2,unknown:1})",
    ] {
        let source = SOURCE.replace(
            "extrude(path(\"M -1 -1 L 1 -1 L 1 1 L -1 1 Z\"),{depth:0.8,bevel:0.1})",
            geometry,
        );
        assert!(compile_motion(&source).is_err(), "{geometry}");
    }
}

#[test]
fn dielectric_material_parameters_are_typed_and_animated() {
    let source=SOURCE.replace("type:\"lambert\",color:\"#ffffff\"", "type:\"pbr\",color:\"#ffffff\",transmission:0.9,thickness:0.2+time.seconds,attenuationColor:\"#ffaa88\",attenuationDistance:2");
    let artifact = compile_motion(&source).unwrap().artifact;
    artifact.validate().unwrap();
    let frame = artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Scene3D { frame, .. } => Some(frame),
            _ => None,
        })
        .unwrap();
    let material = &frame.meshes[0].material;
    assert_eq!(material.static_values().transmission, Some(0.9));
    assert!(material.static_values().thickness.is_none());
    assert!(
        material
            .numbers()
            .iter()
            .any(|(key, _)| *key == "thickness")
    );
    assert_eq!(material.static_values().attenuation_distance, Some(2.0));
    assert!(
        material
            .colors()
            .iter()
            .any(|(key, _)| *key == "attenuationColor")
    );
    for invalid in [
        source.replace("transmission:0.9", "transmission:1.1"),
        source.replace("attenuationDistance:2", "attenuationDistance:0"),
        source.replace("thickness:0.2+time.seconds", "thickness:-1"),
    ] {
        assert!(compile_motion(&invalid).is_err());
    }
}

#[test]
fn physical_material_parameters_support_animation_and_validate_ranges() {
    let source = SOURCE.replace("type:\"lambert\",color:\"#ffffff\"",
        "type:\"pbr\",color:\"#ffffff\",ior:1.33,specularIntensity:0.4,specularColor:\"#ffeecc\",clearcoat:time.seconds,clearcoatRoughness:0.15");
    let artifact = compile_motion(&source).unwrap().artifact;
    artifact.validate().unwrap();
    let material = artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Scene3D { frame, .. } => Some(&frame.meshes[0].material),
            _ => None,
        })
        .unwrap();
    assert_eq!(material.static_values().ior, Some(1.33));
    assert_eq!(material.static_values().specular_intensity, Some(0.4));
    assert!(material.static_values().clearcoat.is_none());
    assert!(
        material
            .numbers()
            .iter()
            .any(|(name, _)| *name == "clearcoat")
    );
    for invalid in [
        source.replace("ior:1.33", "ior:0.5"),
        source.replace("specularIntensity:0.4", "specularIntensity:2"),
        source.replace("clearcoatRoughness:0.15", "clearcoatRoughness:-1"),
        source.replace("#ffeecc", "#ffeecc55"),
    ] {
        assert!(compile_motion(&invalid).is_err(), "{invalid}");
    }
}

#[test]
fn scene3d_material_host_frames_require_the_corresponding_template_and_color_slot() {
    let source = include_str!("fixtures/motion/composition/ui-material.motion.tsx");
    let artifact = compile_motion(source).unwrap().artifact;
    artifact.validate().unwrap();
    for bad in [
        source.replace("export const role = effect();", ""),
        source.replace("input://source", "input://from"),
        source.replace("baseColor:", "normal:"),
        source.replace("input://source", "input://backdrop"),
    ] {
        assert!(compile_motion(&bad).is_err(), "{bad}");
    }
    let transition = source
        .replace("effect()", "transition()")
        .replace("input://source", "input://from");
    compile_motion(&transition)
        .unwrap()
        .artifact
        .validate()
        .unwrap();
}
