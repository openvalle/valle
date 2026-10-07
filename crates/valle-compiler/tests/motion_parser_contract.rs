#![cfg(feature = "motion")]
use valle_compiler::motion::{compile_motion, compile_motion_with_full_env};

fn scene(style: &str) -> String {
    format!(
        "export const composition = {{ width:64,height:64,fps:30,duration:1 }}; export default function Contract(ctx) {{ return <Scene><View style={{{{width:32,height:32,{style}}}}}/></Scene>; }}"
    )
}

#[test]
fn ordered_css_3d_transforms_accept_static_and_typed_template_arguments() {
    for transform in [
        "'translate3d(10px,20%,0) scale3d(1,2,3) rotate3d(1,0,0,45deg)'",
        "'rotateX(20deg) translate(10%,20px) translateX(10px) translateY(20%) translateZ(0)'",
        "'rotateY(0.5rad) rotateZ(0.25turn) rotate(30deg) scale(2) scale(1,2) scaleX(1) scaleY(2) scaleZ(3)'",
        "`translate3d(${ctx.progress}px,${ctx.progress}%,${ctx.progress}px) scale3d(${ctx.progress},2,3) rotate3d(1,0,0,${ctx.progress}turn)`",
        "`rotateX(${ctx.progress}rad) rotateY(${ctx.progress}deg) scale(${ctx.progress},2) translate(${ctx.progress}%,0)`",
    ] {
        let compiled = compile_motion(&scene(&format!("transform:{transform}")))
            .unwrap_or_else(|diagnostics| panic!("{transform}: {diagnostics:#?}"));
        compiled.artifact.validate().unwrap();
        let wire = serde_json::to_string(&compiled.artifact).unwrap();
        assert!(wire.contains("motion-transform-3d"), "{transform}");
        assert_eq!(compiled.artifact.component, "Contract");
    }
    for origin in [
        "'left top'",
        "'right bottom'",
        "'center center'",
        "'12px 30%'",
        "`${ctx.progress}px ${ctx.progress}%`",
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
        "perspectiveOrigin:`${ctx.progress} ${ctx.progress}px`",
        "transform:'translate3d(1px,2px)'",
        "transform:'scale3d(1,2)'",
        "transform:'rotate3d(1,0,45deg)'",
        "transform:'rotateX(20deg) translate(1px,2px,3px)'",
        "transform:'rotateX(20deg) scale(1,2,3)'",
        "transform:'translateZ(10%)'",
        "transform:'translateZ(10em)'",
        "transform:'rotateX(10px)'",
        "transform:`translateZ(${ctx.progress}%)`",
        "transform:`rotateX(${ctx.progress}px)`",
        "transform:`rotateX(20deg) scaleX(${ctx.progress}px)`",
        "transform:`rotateX(2${ctx.progress}deg)`",
        "transform:`rotateX(20deg) translateX(2${ctx.progress}px)`",
        "transform:'rotateX(20deg) scaleX(Infinity)'",
    ] {
        let diagnostics = compile_motion(&scene(style)).expect_err(style);
        assert!(!diagnostics.is_empty(), "{style}");
    }
}

fn body(value: &str) -> String {
    format!("export default function Contract(ctx) {{ return <Scene>{value}</Scene>; }}")
}

#[test]
fn spans_reject_unsupported_attributes_children_and_layout_styles() {
    for span in [
        "<Span {...{style:{color:'red'}}}>x</Span>",
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
        "pathTemplate`M ${ctx.progress} 0 L 10 0 L 10 10 Z`",
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
    let authored = "{u0:ctx.progress,u1:point(0.2,0.4),u2:[ctx.progress,0.2,0.3],u3:[0.1,0.2,0.3,0.4],u4:[1,0,0,1],u5:[1,0,0,0,1,0,0,0,1],u6:[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1],u7:'red',u8:ctx.progress>0}";
    let explicit = source
        .replace("Contract()", "Contract(ctx)")
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
        "{u4:[ctx.progress]}",
        "{u8:2}",
        "{unknown:1}",
    ] {
        let invalid = source
            .replace("Contract()", "Contract(ctx)")
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

#[test]
fn dynamic_gradients_preserve_spread_and_geometry_batches_keep_semantic_keys() {
    for (kind, geometry) in [
        ("linearGradient", "point(ctx.progress,0),point(10,10)"),
        ("radialGradient", "point(5,5),2+ctx.progress"),
        ("conicGradient", "point(5,5),ctx.progress"),
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
        "semanticKeys={ctx.progress}",
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
        "ctx.progress",
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
        "<Text perUnit={{opacity:1}}>x</Text>",
        "<Text split='char' perUnit={{}}>x</Text>",
        "<Text split='char' perUnit={{bad:1}}>x</Text>",
        "<Text split='char' perUnit={{...{opacity:1}}}>x</Text>",
        "<Text split='char' perUnit={{[ctx.progress]:1}}>x</Text>",
        "<Text split='char' perUnit={{opacity:2}}>x</Text>",
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
            "<View style={{{{backgroundColor:interpolate(ctx.progress,[0,0.5,1],['red','blue','green'],{options})}}}}/>"
        ));
        compile_motion(&source).unwrap_or_else(|d| panic!("{options}: {d:#?}"));
    }
    for options in [
        "false",
        "{...{easing:'linear'}}",
        "{[ctx.progress]:'linear'}",
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
            "<View style={{{{backgroundColor:interpolate(ctx.progress,[0,0.5,1],['red','blue','green'],{options})}}}}/>"
        ));
        assert!(compile_motion(&source).is_err(), "{options}");
    }
    for ranges in [
        "[0,'bad'],[0,1]",
        "[0,1],[0,{}]",
        "[0,...[1]],[0,1]",
        "[0,,1],[0,1,2]",
        "[0,ctx.progress],[0,1]",
    ] {
        assert!(
            compile_motion(&body(&format!(
                "<View style={{{{opacity:interpolate(ctx.progress,{ranges})}}}}/>"
            )))
            .is_err(),
            "{ranges}"
        );
    }
}

#[test]
fn interpolation_resolves_constant_object_arrays_and_validates_every_stop() {
    let source = "const ranges={input:[0,0.5,1],output:[0,0.5,1],easing:['linear','easeIn']}; export default function Contract(ctx){return <Scene><View style={{opacity:interpolate(ctx.progress,ranges.input,ranges.output,{easing:ranges.easing})}}/></Scene>; }";
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
