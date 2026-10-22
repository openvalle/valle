#![cfg(feature = "motion")]
use valle_compiler::motion::{compile_motion, compile_motion_with_full_env};

fn scene(style: &str) -> String {
    format!(
        "export const composition = {{ width:64,height:64,fps:30,duration:1 }}; export default function Contract() {{ const time = useCurrentTime(); return <Scene><View style={{{{width:32,height:32,{style}}}}}/></Scene>; }}"
    )
}

#[test]
fn ordered_css_3d_transforms_accept_static_and_typed_template_arguments() {
    for transform in [
        "'translate3d(10px,20%,0) scale3d(1,2,3) rotate3d(1,0,0,45deg)'",
        "'rotateX(20deg) translate(10%,20px) translateX(10px) translateY(20%) translateZ(0)'",
        "'rotateY(0.5rad) rotateZ(0.25turn) rotate(30deg) scale(2) scale(1,2) scaleX(1) scaleY(2) scaleZ(3)'",
        "`translate3d(${time.progress}px,${time.progress}%,${time.progress}px) scale3d(${time.progress},2,3) rotate3d(1,0,0,${time.progress}turn)`",
        "`rotateX(${time.progress}rad) rotateY(${time.progress}deg) scale(${time.progress},2) translate(${time.progress}%,0)`",
        "'perspective(400px) rotateY(30deg) skew(10deg, 5deg) skewX(1deg) skewY(2deg) matrix(1,0,0,1,4,5)'",
        "'matrix3d(1,0,0,0,0,1,0,0,0,0,1,0,10,20,0,1)'",
        "'perspective(none) rotateX(10deg)'",
        "`perspective(${200 + time.progress * 100}px) rotateX(20deg) skew(${time.progress}deg, 0deg) matrix3d(1,0,0,0,0,1,0,0,0,0,1,0,${time.progress},0,0,1)`",
    ] {
        let compiled = compile_motion(&scene(&format!("transform:{transform}")))
            .unwrap_or_else(|diagnostics| panic!("{transform}: {diagnostics:#?}"));
        compiled.artifact.validate().unwrap();
        let wire = serde_json::to_string(&compiled.artifact).unwrap();
        assert!(wire.contains("motion-transform-3d"), "{transform}");
        assert_eq!(compiled.artifact.component, "Contract");
    }
    for style in [
        "transformStyle:'flat'",
        "transformStyle:time.progress<0.5?'flat':'preserve-3d'",
        "backfaceVisibility:time.progress<0.5?'hidden':'visible'",
    ] {
        compile_motion(&scene(style))
            .unwrap_or_else(|diagnostics| panic!("{style}: {diagnostics:#?}"))
            .artifact
            .validate()
            .unwrap();
    }
    for origin in [
        "'left top'",
        "'right bottom'",
        "'center center'",
        "'12px 30%'",
        "`${time.progress}px ${time.progress}%`",
    ] {
        let compiled = compile_motion(&scene(&format!(
            "perspectiveOrigin:{origin},perspective:'200px'"
        )))
        .unwrap_or_else(|d| panic!("{origin}: {d:#?}"));
        assert!(
            serde_json::to_string(&compiled.artifact)
                .unwrap()
                .contains("motion-perspective-origin")
        );
    }
}

#[test]
fn malformed_css_3d_arguments_produce_diagnostics_instead_of_partial_transforms() {
    for style in [
        "perspectiveOrigin:12",
        "perspectiveOrigin:'center'",
        "perspectiveOrigin:'left top extra'",
        "perspectiveOrigin:'bad top'",
        "perspectiveOrigin:`${time.progress} ${time.progress}px`",
        "transform:'translate3d(1px,2px)'",
        "transform:'scale3d(1,2)'",
        "transform:'rotate3d(1,0,45deg)'",
        "transform:'rotateX(20deg) translate(1px,2px,3px)'",
        "transform:'rotateX(20deg) scale(1,2,3)'",
        "transform:'translateZ(10%)'",
        "transform:'translateZ(10em)'",
        "transform:'rotateX(10px)'",
        "transform:`translateZ(${time.progress}%)`",
        "transform:`rotateX(${time.progress}px)`",
        "transform:`rotateX(20deg) scaleX(${time.progress}px)`",
        "transform:`rotateX(2${time.progress}deg)`",
        "transform:`rotateX(20deg) translateX(2${time.progress}px)`",
        "transform:'rotateX(20deg) scaleX(Infinity)'",
        "transform:'matrix3d(1,2,3)'",
        "transform:'rotateX(20deg) matrix(1,0,0,1)'",
        "transform:'perspective(-10px) rotateX(1deg)'",
        "transform:'perspective(10em) rotateX(1deg)'",
        "transform:'rotateX(20deg) skew(1deg,2deg,3deg)'",
        "transform:'rotateX(20deg) skewX(1px)'",
        "transform:`perspective(${time.progress}) rotateX(1deg)`",
        "transformStyle:'none'",
        "transformStyle:time.progress",
        "backfaceVisibility:time.progress<0.5?'hidden':'nope'",
    ] {
        let diagnostics = compile_motion(&scene(style)).expect_err(style);
        assert!(!diagnostics.is_empty(), "{style}");
    }
}

fn body(value: &str) -> String {
    format!(
        "export default function Contract() {{ const time = useCurrentTime(); return <Scene>{value}</Scene>; }}"
    )
}

#[test]
fn spans_reject_unsupported_attributes_children_and_layout_styles() {
    for span in [
        "<Span bad:name='x'>x</Span>",
        "<Span key='x'>x</Span>",
        "<Span style='red'>x</Span>",
        "<Span style={{width:10}}>x</Span>",
        "<Span><View/></Span>",
    ] {
        assert!(
            compile_motion(&body(&format!("<Text>{span}</Text>"))).is_err(),
            "{span}"
        );
    }
    let source = body(
        "<Text> First\n  <Span style={{color:'red'}}>Second\n\t third</Span><Span>{true}{12}</Span><Span>{/*empty*/}</Span> </Text>",
    );
    let compiled = compile_motion(&source).unwrap();
    let wire = serde_json::to_string(&compiled.artifact).unwrap();
    assert!(wire.contains("Second third"));
    assert!(wire.contains("true"));
    assert!(wire.contains("12"));
}

#[test]
fn geometry_helpers_admit_fixed_topology_and_reject_invalid_arity_or_svg_commands() {
    for expression in [
        "pathTemplate`M 0 0 L 10 0 Q 15 5 10 10 C 5 15 0 15 0 10 Z`",
        "pathTemplate`M ${time.progress} 0 L 10 0 L 10 10 Z`",
        "area(line([point(0,0),point(10,10)]),0)",
    ] {
        let source = body(&format!("<Path d={{{expression}}} fill='red'/>"));
        let compiled = compile_motion(&source).unwrap_or_else(|d| panic!("{expression}: {d:#?}"));
        compiled.artifact.validate().unwrap();
    }
    for expression in [
        "point(1)",
        "rect(1,2,3)",
        "line()",
        "area(line([point(0,0),point(10,10)]))",
        "pointAt()",
        "tangentAt()",
        "pathLength()",
        "pathTrajectory()",
        "pathTemplate`m 0 0 l 10 0 z`",
        "pathTemplate`M 0 0 A 5 5 0 0 0 10 10`",
        "pathTemplate`M 0 0 L 10`",
        "pathTemplate`M 0 0 L nope 10`",
        "pathTemplate`M 0 0 L 1e999 10`",
        "pathTemplate`M 0 0 Z 10 10`",
    ] {
        assert!(
            compile_motion(&body(&format!("<Path d={{{expression}}} fill='red'/>"))).is_err(),
            "{expression}"
        );
    }
    let source = body("<View style={{clip:rect(1,2,3,4)}}/>");
    assert!(
        compile_motion(&source).is_err(),
        "non-CSS clip is rejected at style admission"
    );
}

#[test]
fn shader_manifest_defaults_become_typed_artifact_bindings() {
    use valle_motion::shader::*;
    let defaults = [
        (UniformType::Float, UniformValue::Float(0.5)),
        (UniformType::Float2, UniformValue::Float2([0.25, 0.75])),
        (UniformType::Float3, UniformValue::Float3([0.1, 0.2, 0.3])),
        (
            UniformType::Float4,
            UniformValue::Float4([0.1, 0.2, 0.3, 0.4]),
        ),
        (
            UniformType::Float2x2,
            UniformValue::Float2x2([1.0, 0.0, 0.0, 1.0]),
        ),
        (
            UniformType::Float3x3,
            UniformValue::Float3x3([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
        ),
        (
            UniformType::Float4x4,
            UniformValue::Float4x4([
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ]),
        ),
        (
            UniformType::Color,
            UniformValue::Color([0.25, 0.5, 0.75, 1.0]),
        ),
        (UniformType::Bool, UniformValue::Bool(true)),
    ];
    let manifest = ShaderManifest {
        name: "defaults".into(),
        entry: "shader.vsksl".into(),
        role: valle_motion::shader::ShaderRole::Layer,
        inputs: vec![],
        uniforms: defaults
            .into_iter()
            .enumerate()
            .map(|(index, (uniform_type, default))| ShaderUniform {
                name: format!("u{index}"),
                uniform_type,
                required: false,
                default: Some(default),
                min: if matches!(uniform_type, UniformType::Color | UniformType::Bool) {
                    None
                } else {
                    Some(0.0)
                },
                max: if matches!(uniform_type, UniformType::Color | UniformType::Bool) {
                    None
                } else {
                    Some(2.0)
                },
            })
            .collect(),
        output: OutputContract::default(),
        budget: BudgetClass::Local,
    };
    let package = ShaderPackage::compile(
        manifest,
        b"float4 valle_main(float2 uv) { return float4(uv,0.0,1.0); }",
    )
    .unwrap();
    let mut registry = ShaderRegistry::new();
    let resource = valle_motion::ResourceRef {
        control: "effect".into(),
        content_hash: package.content_hash,
    };
    registry.register_asset("effect", package).unwrap();
    let source = "export const controls={assets:{effect:asset({kind:'shader'})}};export default function Contract(){return <ShaderLayer source='asset://effect' style={{width:32,height:32}}/>;}";
    let compiled = compile_motion_with_full_env(
        source,
        std::slice::from_ref(&resource),
        None,
        Some(&registry),
    )
    .unwrap_or_else(|d| panic!("{d:#?}"));
    compiled.artifact.validate().unwrap();
    let node = compiled
        .artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            valle_motion::NodeKind::ShaderLayer { uniforms, .. } => Some(uniforms),
            _ => None,
        })
        .unwrap();
    assert_eq!(node.len(), 9);
    let authored = "{u0:time.progress,u1:point(0.2,0.4),u2:[time.progress,0.2,0.3],u3:[0.1,0.2,0.3,0.4],u4:[1,0,0,1],u5:[1,0,0,0,1,0,0,0,1],u6:[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1],u7:'red',u8:time.progress>0}";
    let explicit = source
        .replace("Contract(){", "Contract(){const time = useCurrentTime();")
        .replace("style={{", &format!("uniforms={{{authored}}} style={{{{"));
    let compiled = compile_motion_with_full_env(
        &explicit,
        &[valle_motion::ResourceRef {
            control: "effect".into(),
            content_hash: resource.content_hash,
        }],
        None,
        Some(&registry),
    )
    .unwrap_or_else(|d| panic!("{d:#?}"));
    compiled.artifact.validate().unwrap();
    for uniforms in [
        "{u0:3}",
        "{u1:point(3,0)}",
        "{u2:[0,0]}",
        "{u3:[0,0,0,3]}",
        "{u4:[time.progress]}",
        "{u8:2}",
        "{unknown:1}",
    ] {
        let invalid = source
            .replace("Contract(){", "Contract(){const time = useCurrentTime();")
            .replace("style={{", &format!("uniforms={{{uniforms}}} style={{{{"));
        assert!(
            compile_motion_with_full_env(
                &invalid,
                &[valle_motion::ResourceRef {
                    control: "effect".into(),
                    content_hash: resource.content_hash
                }],
                None,
                Some(&registry)
            )
            .is_err(),
            "{uniforms}"
        );
    }
}

const SHADER_CODE_SCENE: &str = r##"const shade = `
uniform float progress;
uniform color tint;
uniform shader field;

float4 valle_main(float2 uv) {
  return mix(sampleContent(uv), tint, progress * sample_field(uv).r);
}`;
export const controls = { assets: { field: asset({ kind: "image" }) } };
export default function Contract() { const time = useCurrentTime();
  return <Scene>LAYERS</Scene>;
}"##;

const SHADER_CODE_LAYER: &str = r##"code={shade} inputs={{ field: "asset://field" }} uniforms={{ progress: time.progress, tint: "#38bdf8" }}"##;

fn shader_code_scene(layers: &[&str]) -> String {
    let layers = layers
        .iter()
        .map(|attributes| {
            format!("<ShaderLayer {attributes} style={{{{ width: 32, height: 32 }}}} />")
        })
        .collect::<String>();
    SHADER_CODE_SCENE.replace("LAYERS", &layers)
}

fn compile_shader_code(
    source: &str,
) -> Result<valle_compiler::motion::CompiledMotion, Vec<String>> {
    let field = valle_motion::ResourceRef {
        control: "field".into(),
        content_hash: valle_motion::ContentDigest::of_bytes(b"field"),
    };
    compile_motion_with_full_env(source, std::slice::from_ref(&field), None, None).map_err(
        |diagnostics| {
            diagnostics
                .into_iter()
                .map(|diagnostic| diagnostic.message)
                .collect()
        },
    )
}

#[test]
fn shader_layer_code_declares_its_uniforms_and_inputs() {
    use valle_motion::shader::{InputKind, InputSampling, InputWrap};

    let programs = |artifact: &valle_motion::SceneArtifact| {
        artifact
            .nodes
            .iter()
            .filter_map(|node| match &node.kind {
                valle_motion::NodeKind::ShaderLayer { program, .. } => Some(program.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    // Two layers with the same code and input options share one package.
    let data = SHADER_CODE_LAYER.replace(
        r#"field: "asset://field""#,
        r#"field: { src: "asset://field", kind: "data" }"#,
    ) + " outset={[2, 4]}";
    let compiled = compile_shader_code(&shader_code_scene(&[&data, &data]))
        .unwrap_or_else(|messages| panic!("{messages:#?}"));
    let artifact = &compiled.artifact;
    artifact.validate().unwrap();
    assert_eq!(artifact.shaders.len(), 1);
    let shader = &artifact.shaders[0];
    let input = &shader.manifest.inputs[0];
    assert_eq!(
        (input.name.as_str(), input.kind, input.sampling, input.wrap),
        (
            "field",
            InputKind::Data,
            InputSampling::Nearest,
            InputWrap::Clamp
        )
    );
    assert_eq!(
        shader
            .manifest
            .uniforms
            .iter()
            .map(|uniform| uniform.name.as_str())
            .collect::<Vec<_>>(),
        ["progress", "tint"]
    );
    assert!(!shader.source.contains("uniform"), "{}", shader.source);
    // CSS order, top and bottom 2 and the sides 4, as [left, top, right, bottom].
    assert_eq!(shader.manifest.output.padding, [4, 2, 4, 2]);
    let uri = shader.uri().unwrap().to_string();
    let layers = programs(artifact);
    assert_eq!(layers.len(), 2);
    assert!(
        layers
            .iter()
            .all(|program| program.uri == uri && program.padding == [4, 2, 4, 2])
    );
    let scene = valle_motion::prepare_scene(artifact).unwrap();
    assert_eq!(scene.inline_shaders().len(), 1);
    assert_eq!(scene.inline_shaders()[0].uri().to_string(), uri);

    // A color input filters linearly by default; other options make another package.
    let compiled = compile_shader_code(&shader_code_scene(&[SHADER_CODE_LAYER, &data]))
        .unwrap_or_else(|messages| panic!("{messages:#?}"));
    let artifact = &compiled.artifact;
    assert_eq!(artifact.shaders.len(), 2);
    let color = artifact
        .shaders
        .iter()
        .map(|shader| &shader.manifest)
        .find(|manifest| manifest.inputs[0].kind == InputKind::Color)
        .expect("the color layer has its own package");
    assert_eq!(color.inputs[0].sampling, InputSampling::Linear);
    assert_eq!(color.output.padding, [0; 4]);
    let layers = programs(artifact);
    assert_ne!(layers[0].uri, layers[1].uri);

    let edited = |from: &str, to: &str| shader_code_scene(&[&SHADER_CODE_LAYER.replace(from, to)]);
    for (source, expected) in [
        (
            edited(r#""asset://field""#, r#"{ src: "input://backdrop" }"#),
            "reads the backdrop, which takes no options",
        ),
        (
            shader_code_scene(&[&format!(r#"{SHADER_CODE_LAYER} source="asset://field""#)]),
            "either source=",
        ),
        (
            edited(r##", tint: "#38bdf8""##, ""),
            "ShaderLayer code requires uniform `tint`",
        ),
        (
            edited(
                "progress: time.progress",
                "progress: time.progress, glow: 1",
            ),
            "ShaderLayer code has no uniform `glow`",
        ),
        (
            edited(r#"inputs={{ field: "asset://field" }}"#, ""),
            "ShaderLayer code requires texture input `field`",
        ),
        (
            edited(
                r#""asset://field""#,
                r#"{ src: "asset://field", filter: "linear" }"#,
            ),
            "not `filter`",
        ),
        (
            edited(r#""asset://field""#, r#"{ kind: "data" }"#),
            "needs `src`",
        ),
        (
            shader_code_scene(&[&format!("{SHADER_CODE_LAYER} outset={{-1}}")]),
            "ShaderLayer outset must be a whole number of pixels",
        ),
        (
            shader_code_scene(&[&format!("{SHADER_CODE_LAYER} outset={{[1, 2, 3, 4, 5]}}")]),
            "ShaderLayer outset must be a whole number of pixels",
        ),
        (
            edited("code={shade}", "code={time.progress > 0.5 ? shade : shade}"),
            "ShaderLayer code must be a static string",
        ),
        (
            shader_code_scene(&[SHADER_CODE_LAYER])
                .replace("uniform float progress;", "uniform float progress = 0.5;"),
            "not an initializer",
        ),
        (
            shader_code_scene(&[SHADER_CODE_LAYER]).replace("return mix(", "return blend("),
            "ShaderLayer code, line 7, column ",
        ),
    ] {
        let messages = compile_shader_code(&source).expect_err(expected);
        assert!(
            messages.iter().any(|message| message.contains(expected)),
            "{expected}: {messages:#?}"
        );
        assert_eq!(messages.len(), 1, "{expected}: {messages:#?}");
    }
    let messages = compile_shader_code(&shader_code_scene(&[
        r#"source="asset://field" outset={2}"#,
    ]))
    .expect_err("outset with a descriptor");
    assert!(
        messages
            .iter()
            .any(|message| message.contains("outset is for ShaderLayer code")),
        "{messages:#?}"
    );
}

#[test]
fn shader_layer_code_reads_the_backdrop_in_place() {
    use valle_motion::shader::{InputKind, InputSampling, InputWrap};

    let layer = SHADER_CODE_LAYER.replace(r#""asset://field""#, r#""input://backdrop""#);
    let compiled = compile_shader_code(&shader_code_scene(&[&layer]))
        .unwrap_or_else(|messages| panic!("{messages:#?}"));
    let artifact = &compiled.artifact;
    artifact.validate().unwrap();
    let input = &artifact.shaders[0].manifest.inputs[0];
    assert_eq!(
        (input.name.as_str(), input.kind, input.sampling, input.wrap),
        (
            "field",
            InputKind::Backdrop,
            InputSampling::Linear,
            InputWrap::Clamp
        )
    );
    let inputs = artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            valle_motion::NodeKind::ShaderLayer { inputs, .. } => Some(inputs),
            _ => None,
        })
        .unwrap();
    assert_eq!(inputs[0].source.as_deref(), Some("input://backdrop"));
    let mut tampered = artifact.clone();
    for node in &mut tampered.nodes {
        if let valle_motion::NodeKind::ShaderLayer { inputs, .. } = &mut node.kind {
            inputs[0].source = Some("asset://field".into());
        }
    }
    assert!(tampered.validate().is_err());
}

#[test]
fn dynamic_gradients_preserve_spread_and_geometry_batches_keep_semantic_keys() {
    for (kind, geometry) in [
        ("linearGradient", "point(time.progress,0),point(10,10)"),
        ("radialGradient", "point(5,5),2+time.progress"),
        ("conicGradient", "point(5,5),time.progress"),
    ] {
        for spread in ["pad", "repeat", "reflect"] {
            let paint = format!(
                "{kind}({geometry},[gradientStop(0,'red'),gradientStop(1,'blue')],'{spread}')"
            );
            let compiled = compile_motion(&body(&format!(
                "<Path d='M0 0 L10 0 L10 10 Z' fill={{{paint}}}/>"
            )))
            .unwrap_or_else(|d| panic!("{paint}: {d:#?}"));
            let wire = serde_json::to_value(&compiled.artifact).unwrap();
            assert!(wire.to_string().contains(spread));
            compiled.artifact.validate().unwrap();
        }
    }
    let batch = "<GeometryBatch geometry='circle' positions={[point(1,2),point(3,4)]} sizes={2} fills='red' semanticKeys={['first','second']}/>";
    let compiled = compile_motion(&body(batch)).unwrap();
    assert!(
        serde_json::to_string(&compiled.artifact)
            .unwrap()
            .contains("second")
    );
    for keys in [
        "semanticKeys='one'",
        "semanticKeys",
        "semanticKeys={time.progress}",
        "semanticKeys={[1,2]}",
    ] {
        let source = body(&format!(
            "<GeometryBatch geometry='circle' positions={{[point(1,2),point(3,4)]}} sizes={{2}} fills='red' {keys}/>"
        ));
        assert!(compile_motion(&source).is_err(), "{keys}");
    }
    for shape in [
        "<polygon points='0,0 10,0 10,10' fill='red'/>",
        "<polyline points={[point(0,0),point(10,10)]} fill='none' stroke='red'/>",
    ] {
        compile_motion(&body(shape))
            .unwrap()
            .artifact
            .validate()
            .unwrap();
    }
    for path in [
        "<Path d='M0 0 L10 10' fill/>",
        "<Path d='M0 0 L10 10' fill='invalid'/>",
        "<Path d='M0 0 L10 10' fill={linearGradient(point(0,0),point(10,10),[gradientStop(0,'red'),gradientStop(1,'blue')],'bad')}/>",
        "<Path d='M0 0 L10 10' fill={radialGradient(point(0,0),0,[gradientStop(0,'red'),gradientStop(1,'blue')])}/>",
        "<Path d='M0 0 L10 10' stroke='red' strokeWidth={-1}/>",
        "<polygon points='0,0 bad,10'/>",
        "<polygon points={true}/>",
        "<line x1='bad' x2={3}/>",
        "<Path d='M0 0 L10 10' trimStart={2}/>",
        "<Path d='M0 0 L10 10' arrowEnd='invalid'/>",
    ] {
        assert!(compile_motion(&body(path)).is_err(), "{path}");
    }
}

#[test]
fn fit_text_and_per_unit_styles_admit_closed_contracts_and_report_invalid_inputs() {
    let source = body("<Text style={{fitText:fitText({minFontSize:8,maxFontSize:20})}}>fit</Text>");
    let compiled = compile_motion(&source).unwrap_or_else(|d| panic!("{d:#?}"));
    let wire = serde_json::to_string(&compiled.artifact).unwrap();
    assert!(wire.contains("text-fit"));
    assert!(wire.contains("shrink 40%"));
    for value in [
        "time.progress",
        "3",
        "fitText({minFontSize:0,maxFontSize:20})",
        "fitText({minFontSize:20,maxFontSize:8})",
    ] {
        assert!(
            compile_motion(&body(&format!(
                "<Text style={{{{fitText:{value}}}}}>bad</Text>"
            )))
            .is_err(),
            "{value}"
        );
    }
    for text in [
        "<Text split='char'>x</Text>",
        "<Text perUnit={() => ({opacity:1})}>x</Text>",
        "<Text split='char' perUnit={() => ({})}>x</Text>",
        "<Text split='char' perUnit={() => ({bad:1})}>x</Text>",
        "<Text split='char' perUnit={() => ({...{opacity:1}})}>x</Text>",
        "<Text split='char' perUnit={() => ({[time.progress]:1})}>x</Text>",
        "<Text split='char' perUnit={() => ({opacity:2})}>x</Text>",
    ] {
        assert!(compile_motion(&body(text)).is_err(), "{text}");
    }
}

#[test]
fn interpolation_options_reject_ambiguous_and_malformed_ranges() {
    for options in [
        "{easing:['linear','easeIn']}",
        "{colorSpace:'oklch',hue:'longer'}",
        "{colorSpace:'oklch',hue:'increasing'}",
        "{colorSpace:'oklch',hue:'decreasing'}",
    ] {
        let source = body(&format!(
            "<View style={{{{backgroundColor:interpolate(time.progress,[0,0.5,1],['red','blue','green'],{options})}}}}/>"
        ));
        compile_motion(&source).unwrap_or_else(|d| panic!("{options}: {d:#?}"));
    }
    for options in [
        "false",
        "{...{easing:'linear'}}",
        "{[time.progress]:'linear'}",
        "{unknown:1}",
        "{colorSpace:1}",
        "{colorSpace:'bad'}",
        "{hue:1}",
        "{hue:'bad'}",
        "{hue:'longer'}",
        "{easing:['linear']}",
        "{easing:[1,2]}",
    ] {
        let source = body(&format!(
            "<View style={{{{backgroundColor:interpolate(time.progress,[0,0.5,1],['red','blue','green'],{options})}}}}/>"
        ));
        assert!(compile_motion(&source).is_err(), "{options}");
    }
    for ranges in [
        "[0,'bad'],[0,1]",
        "[0,1],[0,{}]",
        "[0,,1],[0,1,2]",
        "[0,time.progress],[0,1]",
    ] {
        assert!(
            compile_motion(&body(&format!(
                "<View style={{{{opacity:interpolate(time.progress,{ranges})}}}}/>"
            )))
            .is_err(),
            "{ranges}"
        );
    }
}

#[test]
fn interpolation_resolves_constant_object_arrays_and_validates_every_stop() {
    let source = "const ranges={input:[0,0.5,1],output:[0,0.5,1],easing:['linear','easeIn']}; export default function Contract(){ const time = useCurrentTime();return <Scene><View style={{opacity:interpolate(time.progress,ranges.input,ranges.output,{easing:ranges.easing})}}/></Scene>; }";
    compile_motion(source).unwrap_or_else(|d| panic!("{d:#?}"));
    for (from, to) in [
        ("input:[0,0.5,1]", "input:[0,'bad',1]"),
        ("output:[0,0.5,1]", "output:[0,{},1]"),
        ("easing:['linear','easeIn']", "easing:['linear',false]"),
    ] {
        let diagnostics = compile_motion(&source.replace(from, to)).expect_err(to);
        assert!(!diagnostics.is_empty());
    }
}

#[test]
fn bundle_compatibility_returns_the_same_canonical_timeline_and_rejects_duplicates() {
    let source = "{\"canvas\":{\"width\":32,\"height\":32,\"fps\":30},\"tracks\":{\"visual\":[{\"clips\":[{\"kind\":\"solid\",\"color\":\"#ff0000\",\"start\":0,\"duration\":1}]}]}}";
    let entries = valle_compiler::parse_bundle(&format!("-- timeline.json --\r\n{source}\r\n"));
    let project = valle_compiler::compile_project(&entries).unwrap();
    let canonical = valle_compiler::compile_bundle(&entries).unwrap();
    assert_eq!(
        serde_json::to_value(&project).unwrap()["timeline"],
        serde_json::to_value(canonical.to_wire()).unwrap()
    );
    let mut duplicate = entries.clone();
    duplicate.extend(entries);
    assert!(
        valle_compiler::compile_bundle(&duplicate)
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
}
