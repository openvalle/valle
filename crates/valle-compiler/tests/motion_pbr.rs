#![cfg(feature = "motion")]
use valle_compiler::motion::{CompiledMotion, CompilerDiagnostic, compile_motion_with_resources};
use valle_motion::{
    NodeKind,
    scene3d::{EnvironmentSpec, LightKind, MaterialKind, ToneMapping},
};

const SOURCE: &str = r##"
export const controls=defineControls({assets:{model:asset({kind:"model3d"}),sky:asset({kind:"environment"})}});
export default function Pbr(ctx) {
 return <Scene><Scene3D key="scene" camera={{position:[0,0,4],target:[0,0,0]}}
 pbr={{environment:{src:"asset://sky",intensity:1,rotation:ctx.seconds*30,background:false},toneMapping:"aces",exposure:1.1}} style={{width:64,height:64}}>
 <Mesh key="model" src="asset://model" rotateY={ctx.seconds*60} material={{type:"pbr",color:"#ffffff"}} />
 <HemisphereLight skyColor="#bad6ff" groundColor="#1a1a22" intensity={1+ctx.seconds*0} />
 </Scene3D></Scene>;
}
"##;

fn compile_motion(source: &str) -> Result<CompiledMotion, Vec<CompilerDiagnostic>> {
    compile_motion_with_resources(
        source,
        &["model", "sky"].map(|name| valle_motion::ResourceRef {
            control: name.into(),
            content_hash: valle_motion::ContentDigest::of_bytes(name.as_bytes()),
        }),
    )
}

#[test]
fn pbr_configuration_is_frozen_and_hemisphere_intensity_uses_frame_binding() {
    let artifact = compile_motion(SOURCE).unwrap().artifact;
    artifact.validate().unwrap();
    let (scene, frame) = artifact
        .nodes
        .iter()
        .find_map(|n| match &n.kind {
            NodeKind::Scene3D { scene, frame } => Some((scene, frame)),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        scene.pbr.environment,
        Some(EnvironmentSpec {
            control: "sky".into(),
            background: false
        })
    );
    assert_eq!(scene.pbr.tone_mapping, ToneMapping::Aces);
    assert_eq!(
        frame.exposure,
        valle_motion::NumberValue::Static { value: 1.1 }
    );
    assert_eq!(scene.meshes[0].material.kind, Some(MaterialKind::Pbr));
    assert!(matches!(scene.lights[0], LightKind::Hemisphere));
    assert!(matches!(
        frame.lights[0],
        valle_motion::Scene3DLightBinding::Hemisphere {
            intensity: valle_motion::NumberValue::Expr { .. },
            ..
        }
    ));
}

#[test]
fn pbr_unknown_options_dynamic_configuration_and_transparency_fail_closed() {
    for (from, to) in [
        ("src:\"asset://sky\"", "src:\"asset://missing\""),
        ("toneMapping:\"aces\"", "toneMapping:\"unknown\""),
        ("exposure:1.1", "exposure:17"),
        ("color:\"#ffffff\"", "roughness:2"),
        ("skyColor=\"#bad6ff\"", "skyColor=\"#bad6ff80\""),
    ] {
        assert!(
            compile_motion(&SOURCE.replace(from, to)).is_err(),
            "must reject {to}"
        );
    }
}

#[test]
fn camera_vectors_light_colors_and_exposure_use_frame_expressions() {
    let source=SOURCE.replace("position:[0,0,4],target:[0,0,0]","position:[ctx.seconds*0.1,0,4],target:[0,ctx.seconds*0.05,0],near:0.1+ctx.seconds*0.01,far:10+ctx.seconds,fov:45-ctx.seconds,orbitYaw:ctx.seconds*20")
        .replace("exposure:1.1","exposure:0.5+ctx.seconds*0.1")
        .replace("skyColor=\"#bad6ff\"","skyColor={interpolate(ctx.seconds,[0,2],[\"#ff0000\",\"#0000ff\"])} direction={[ctx.seconds,1,0]}")
        .replace("</Scene3D>","<DirectionalLight color={interpolate(ctx.seconds,[0,2],[\"#00ff00\",\"#ff0000\"])} direction={[ctx.seconds,0,1]} intensity={ctx.seconds*0.1}/></Scene3D>");
    let artifact = compile_motion(&source).unwrap().artifact;
    artifact.validate().unwrap();
    let (scene, frame) = artifact
        .nodes
        .iter()
        .find_map(|n| {
            if let NodeKind::Scene3D { scene, frame } = &n.kind {
                Some((scene, frame))
            } else {
                None
            }
        })
        .unwrap();
    assert!(frame.camera.constant().is_none());
    assert!(matches!(
        frame.camera.position[0],
        valle_motion::NumberValue::Expr { .. }
    ));
    assert!(matches!(
        frame.camera.near,
        valle_motion::NumberValue::Expr { .. }
    ));
    assert!(matches!(
        frame.exposure,
        valle_motion::NumberValue::Expr { .. }
    ));
    assert!(matches!(
        frame.lights[0],
        valle_motion::Scene3DLightBinding::Hemisphere {
            sky_color: valle_motion::ColorValue::Expr { .. },
            ..
        }
    ));
    assert_eq!(
        scene.lights,
        [LightKind::Hemisphere, LightKind::Directional]
    );
    assert_eq!(scene.frame_scalar_count(), 55);
    assert!(serde_json::to_value(scene).unwrap().get("camera").is_none());
}

#[test]
fn mesh_vectors_and_model_node_replacements_are_typed_complete_transforms() {
    let bindings = r#"position={[ctx.seconds,0,0]} translateX={2} rotation={[0,10,0]} scale={[2,1,1]} scaleX={ctx.seconds+1} nodes={[{id:2,position:[ctx.seconds,0,0],rotation:[0,ctx.seconds*10,0],scale:[-1,2,1]},{id:0}]}"#;
    let source = SOURCE.replace(
        "rotateY={ctx.seconds*60}",
        &format!("rotateY={{ctx.seconds*60}} {bindings}"),
    );
    let artifact = compile_motion(&source).unwrap().artifact;
    artifact.validate().unwrap();
    let (scene, frame) = artifact
        .nodes
        .iter()
        .find_map(|n| match &n.kind {
            NodeKind::Scene3D { scene, frame } => Some((scene, frame)),
            _ => None,
        })
        .unwrap();
    assert_eq!(scene.meshes[0].node_ids, [2, 0]);
    assert!(
        serde_json::to_value(&scene.meshes[0])
            .unwrap()
            .get("transform")
            .is_none()
    );
    assert!(frame.meshes[0].transform.constant().is_none());
    assert!(frame.meshes[0].nodes[0].transform.constant().is_none());
    assert_eq!(
        frame.meshes[0].nodes[1].transform.constant().unwrap(),
        valle_motion::scene3d::Transform3D::default()
    );
    for bad in [
        "nodes={[{id:2},{id:2}]}",
        "nodes={[{id:256}]}",
        "nodes={[{id:ctx.seconds}]}",
        "nodes={[{id:0,scale:[0,1,1]}]}",
        "nodes={[{id:0,position:[true,0,0]}]}",
        "nodes={[{id:0,unknown:1}]}",
        "scale={[1001,1,1]}",
    ] {
        assert!(
            compile_motion(&SOURCE.replace("rotateY={ctx.seconds*60}", bad)).is_err(),
            "must reject {bad}"
        );
    }
}

#[test]
fn material_values_and_texture_roles_lower_through_global_and_indexed_overrides() {
    let source = SOURCE.replace("sky:asset", r#"map:asset({kind:"image"}),sky:asset"#)
        .replace(r##"material={{type:"pbr",color:"#ffffff"}}"##,r##"material={{type:"pbr",color:interpolate(ctx.seconds,[0,2],["#ff0000","#0000ff"]),metallic:ctx.seconds/3,roughness:0.5,textures:{baseColor:"asset://map",normal:{src:"asset://map",wrapU:"mirror",wrapV:"clamp",minFilter:"nearest",mipmap:"none"}}}} materials={[{id:0,alphaMode:"mask",alphaCutoff:ctx.seconds/3,emissive:"#ff0000",emissiveIntensity:ctx.seconds,normalScale:ctx.seconds,occlusionStrength:ctx.seconds/3,doubleSided:true,textures:{normal:null}}]}"##);
    let compile = |source: &str| {
        compile_motion_with_resources(
            source,
            &["model", "sky", "map"].map(|name| valle_motion::ResourceRef {
                control: name.into(),
                content_hash: valle_motion::ContentDigest::of_bytes(name.as_bytes()),
            }),
        )
    };
    let artifact = compile(&source).unwrap().artifact;
    artifact.validate().unwrap();
    let (scene, frame) = artifact
        .nodes
        .iter()
        .find_map(|n| match &n.kind {
            NodeKind::Scene3D { scene, frame } => Some((scene, frame)),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        scene.meshes[0].texture_controls().collect::<Vec<_>>(),
        [
            ("map", valle_motion::scene3d::TextureRole::Color),
            ("map", valle_motion::scene3d::TextureRole::Data)
        ]
    );
    assert_eq!(scene.meshes[0].material_overrides[0].id, 0);
    assert_eq!(
        scene.meshes[0].material_overrides[0].material.alpha_mode,
        Some(valle_motion::scene3d::AlphaMode::Mask)
    );
    assert!(matches!(
        frame.meshes[0].material.color,
        Some(valle_motion::ColorValue::Expr { .. })
    ));
    assert!(
        frame.meshes[0].material_overrides[0]
            .material
            .emissive_intensity
            .is_some()
    );
    for (from, to) in [
        ("roughness:0.5", "roughness:2"),
        ("occlusionStrength:ctx.seconds/3", "occlusionStrength:true"),
        (r#"wrapU:"mirror""#, r#"wrapU:"bad""#),
        ("normal:null", "unknown:null"),
        ("id:0", "id:ctx.seconds"),
        (r#"alphaMode:"mask""#, r#"alphaMode:"blend""#),
        (r##"emissive:"#ff0000""##, r##"emissive:"#ff000080""##),
        ("asset://map", "asset://missing"),
    ] {
        assert!(
            compile(&source.replace(from, to)).is_err(),
            "must reject {to}"
        );
    }
}
