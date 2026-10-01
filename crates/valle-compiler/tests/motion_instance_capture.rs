#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::DiagCode;

fn source(prefix: &str, suffix: &str) -> String {
    format!(
        r##"
const ROWS = Array.from({{length:80}}, (_, i) => ({{id:`row${{i}}`, x:i*4}}));
export default function Main(ctx) {{
 const k = ctx.progress * 2;
 return <Scene style={{{{width:640,height:200}}}}>
 {prefix}
 {{ROWS.map(row => <View key={{row.id}} className="absolute"
   style={{{{left:row.x+k,top:40,width:3,height:10,opacity:ctx.progress,backgroundColor:"white"}}}} />)}}
 {suffix}
 </Scene>;
}}
"##
    )
}

#[test]
fn unrelated_prefixes_do_not_change_instance_admission_or_expression_closure() {
    let baseline = compile_motion(&source("", "")).unwrap().artifact;
    let template_exprs = &baseline.instance_groups[0].exprs;
    let animated = (0..400)
        .map(|i| {
            format!(
                "<View key=\"v{i}\" style={{{{width:10,height:10,opacity:ctx.progress*0.5}}}} />"
            )
        })
        .collect::<String>();
    for prefix in [
        r#"<Text key="title" split="char" perUnit={{opacity:clamp(ctx.progress*3-ctx.unit.index*0.1,0,1)}}>Hello</Text>"#,
        r#"<View key="target" style={{width:10,height:10}} /><View key="overlay" style={{opacity:bounds("target").width/100}} />"#,
        r##"<Scene3D key="three" camera={{position:[0,0,5],target:[0,0,0]}} style={{width:128,height:128}}>
          <Mesh key="mesh" geometry={extrude(path("M -1 -1 L 1 -1 L 1 1 L -1 1 Z"),{depth:0.8})} material={{type:"lambert",color:"#ffffff"}} />
          <Anchor3D key="a" parent="mesh" position={[0,0,0]} />
        </Scene3D><View key="projected" className="absolute" style={{width:5,height:5,translate:project3d("three","mesh::a")}} />"##,
        animated.as_str(),
    ] {
        for (before, after) in [(prefix, ""), ("", prefix)] {
            let compiled = compile_motion(&source(before, after)).unwrap();
            assert_eq!(compiled.artifact.instance_groups.len(), 1);
            assert!(
                !compiled
                    .warnings
                    .iter()
                    .any(|w| w.code == DiagCode::InstanceFallback)
            );
            assert_eq!(&compiled.artifact.instance_groups[0].exprs, template_exprs);
            assert_eq!(
                compiled.source_map.instance_exprs[0].len(),
                template_exprs.len()
            );
            assert!(
                compiled.source_map.instance_exprs[0]
                    .iter()
                    .all(|mapping| mapping.span.end > mapping.span.start)
            );
            compiled.artifact.validate().unwrap();
        }
    }
}
