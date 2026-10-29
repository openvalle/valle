#![cfg(feature = "motion")]

use std::collections::BTreeMap;

use valle_compiler::motion::compile_motion;
use valle_motion::{
    Fonts, LayoutOptions, Viewport, build_tree, default_font_naming, emit, layout_boxes,
    motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

fn layout(source: &str, frame: u32) -> (valle_motion::LayoutTree, BTreeMap<String, [f32; 4]>) {
    let artifact = compile_motion(source).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let ctx = motion_context_at_frame(frame, 120, FrameRate::new(30, 1).unwrap()).unwrap();
    let mut fonts = Fonts::default();
    valle_motion::register_default_motion_fonts(&mut fonts).unwrap();
    let tree = build_tree(
        &prepared,
        &ctx,
        &resolve_props(&artifact.controls, &BTreeMap::new()).unwrap(),
        &LayoutOptions {
            viewport: Viewport::new((320, 180)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let boxes = layout_boxes(&artifact, &tree).unwrap();
    (tree, boxes)
}

fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
}

#[test]
fn slow_layout_motion_changes_every_frame_without_changing_box_size() {
    let source = r##"
export default function Motion() {
 const time = useCurrentTime();
 return <Scene><View key="moving" style={{position:"absolute",left:20.25+time.seconds*6,
   top:22.25+time.seconds*3,width:62.25,height:26.75,backgroundColor:"#63d8ab"}} /></Scene>;
}"##;
    let mut previous = None;
    for frame in 0..30 {
        let (tree, boxes) = layout(source, frame);
        let rect = boxes["moving"];
        near(rect[0], 20.25 + frame as f32 * 0.2);
        near(rect[1], 22.25 + frame as f32 * 0.1);
        assert_eq!(&rect[2..], &[62.25, 26.75]);
        let bytes = emit(&tree, &default_font_naming)
            .unwrap()
            .program
            .packed_bytes()
            .unwrap();
        assert_ne!(previous.as_ref(), Some(&bytes), "stalled frame {frame}");
        previous = Some(bytes);
    }
}

#[test]
fn fractional_nested_boxes_and_overflow_hit_testing_share_the_same_edges() {
    let (tree, boxes) = layout(
        r##"
export default function Motion() { return <Scene key="scene" className="w-full h-full">
 <View key="clip" style={{position:"absolute",left:10.25,top:20.25,width:40.5,height:30.5,overflow:"hidden"}}>
   <View key="child" style={{position:"absolute",left:2.25,top:1.5,width:80.5,height:50.5,backgroundColor:"red"}} />
 </View>
</Scene>; }
"##,
        0,
    );
    assert_eq!(boxes["child"], [12.5, 21.75, 80.5, 50.5]);
    let hit = |point| valle_motion::layout::layout_box_at_point(&tree, point).unwrap();
    assert_eq!(hit([12.6, 21.8]).as_deref(), Some("child"));
    assert_eq!(hit([50.7, 21.8]).as_deref(), Some("child"));
    assert_eq!(hit([50.8, 21.8]).as_deref(), Some("scene"));
    emit(&tree, &default_font_naming).unwrap();
}

#[test]
fn fractional_flex_and_grid_tracks_keep_shared_edges() {
    for style in [
        "display:'flex'",
        "display:'grid',gridTemplateColumns:'1fr 1fr 1fr'",
    ] {
        let source = format!(
            r#"
export default function Motion() {{ return <Scene>
 <View key="group" style={{{{position:"absolute",left:10.25,top:20.25,width:100.5,height:40.5,{style}}}}}>
   <View key="a" style={{{{flexGrow:1}}}} />
   <View key="b" style={{{{flexGrow:1}}}} />
   <View key="c" style={{{{flexGrow:1}}}} />
 </View>
</Scene>; }}"#
        );
        let (_, boxes) = layout(&source, 0);
        assert_eq!(boxes["a"], [10.25, 20.25, 33.5, 40.5], "{style}");
        near(boxes["a"][0] + boxes["a"][2], boxes["b"][0]);
        near(boxes["b"][0] + boxes["b"][2], boxes["c"][0]);
        near(boxes["c"][0] + boxes["c"][2], 110.75);
    }
}

#[test]
fn hoisted_absolute_children_and_anonymous_inline_boxes_receive_layout() {
    let (tree, boxes) = layout(
        r#"
export default function Motion() { return <Scene>
 <View key="container" style={{position:"absolute",left:10.25,top:20.25,width:140.5,height:90.5}}>
  <View key="flow" style={{padding:3.25}}>
   <Text key="label" style={{fontSize:14.5,lineHeight:1.25}}>Fractional <Span>inline</Span> text</Text>
   <View key="absolute" style={{position:"absolute",left:5.25,top:7.75,width:30.5,height:20.25}} />
  </View>
 </View>
</Scene>; }
"#,
        0,
    );
    assert_eq!(boxes["absolute"], [15.5, 28.0, 30.5, 20.25]);
    near(boxes["label"][0], 13.5);
    near(boxes["label"][1], 23.5);
    assert!(boxes["label"][2] > 20.0 && boxes["label"][3] > 10.0);
    emit(&tree, &default_font_naming).unwrap();
}

#[test]
fn native_rectangle_instances_keep_subpixel_positions_and_thin_sizes() {
    let source = r##"
const cells = Array.from({length:4}, (_, i) => ({id:`cell${i}`,x:10.25+i*24.5}));
export default function Motion() {
 const time = useCurrentTime();
 return <Scene className="relative h-full w-full">
  {cells.map(cell => <View key={cell.id} className="absolute" style={{
    left:cell.x+time.seconds*6,top:12.25+time.seconds*3,width:.25,height:20.75,backgroundColor:"#ffffff"}} />)}
 </Scene>;
}"##;
    let compact = compile_motion(source).unwrap().artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    let expanded = source.replace("className=\"absolute\"", "className=\"absolute \"");
    assert!(
        compile_motion(&expanded)
            .unwrap()
            .artifact
            .instance_groups
            .is_empty()
    );
    for frame in [0, 1, 2, 7, 29] {
        let (tree, _) = layout(source, frame);
        let (_, boxes) = layout(&expanded, frame);
        let program = emit(&tree, &default_font_naming).unwrap().program;
        let batch = program
            .nodes()
            .iter()
            .find_map(|node| match node {
                valle_draw::program::Node::InstanceBatch(batch) => Some(batch),
                _ => None,
            })
            .expect("must retain the native batch path");
        assert_eq!(
            batch.instances.len(),
            4,
            "thin rectangles must not disappear"
        );
        for (index, matrix) in batch.instances.transforms.iter().enumerate() {
            let rect = boxes[&format!("cell{index}")];
            let m = matrix.0.map(|value| value as f32);
            near(m[0], rect[2]);
            near(m[3], rect[3]);
            near(m[4], rect[0]);
            near(m[5], rect[1]);
        }
    }
}
