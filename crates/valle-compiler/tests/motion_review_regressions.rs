#![cfg(feature = "motion")]
use valle_compiler::motion::compile_motion;
use valle_motion::{
    Fonts, LayoutOptions, Viewport, build_tree, default_font_naming, emit, motion_context_at_frame,
    prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

const GRID: &str = r##"
const ROWS = Array.from({length: 4}, (_, i) => ({id:`row${i}`, i}));
export default function Main(ctx) {
  const k = ctx.progress * 2;
  const active = ctx.localFrame >= 0;
  return <Scene style={{width:640,height:360}}>{ROWS.map(row => <View key={row.id}
    visible={active} className="absolute" style={{left:20 + row.i*60 + k*150,
    top:20, width:ctx.localFrame, height:10, backgroundColor:"#ffffff"}} />)}</Scene>;
}
"##;

fn report(source: &str) -> valle_motion::layout::MotionReview {
    let artifact = compile_motion(source).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let mut fonts = Fonts::default();
    valle_motion::register_default_motion_fonts(&mut fonts).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    valle_motion::review_motion(
        &prepared,
        &props,
        &LayoutOptions {
            fonts: &fonts,
            viewport: Viewport::new((640, 360)),
            styles: None,
        },
        FrameRate::new(30, 1).unwrap(),
        60,
        valle_timeline::RationalTime::new(2, 1).unwrap(),
        60,
        true,
    )
    .unwrap()
}

#[test]
fn captured_animation_and_zero_size_match_expansion_in_review() {
    assert_eq!(
        compile_motion(GRID).unwrap().artifact.instance_groups.len(),
        1
    );
    let expanded = GRID.replace("key={row.id}", "key={`${row.id}`}");
    let a = report(GRID);
    let b = report(&expanded);
    for row in 0..4 {
        let key = format!("row{row}");
        let a = a
            .trajectories
            .iter()
            .find(|t| t.node == key)
            .expect("batch row is reviewed");
        let b = b
            .trajectories
            .iter()
            .find(|t| t.node == key)
            .expect("expanded row is reviewed");
        for frame in [1, 15, 30, 59] {
            let a = a.points.iter().find(|p| p.frame == frame).unwrap();
            let b = b.points.iter().find(|p| p.frame == frame).unwrap();
            assert_eq!(a.bounds, b.bounds, "{key}, frame {frame}");
        }
    }
    // Frame zero emits successfully with no invertible/visible rectangle instances.
    let artifact = compile_motion(GRID).unwrap().artifact;
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let ctx = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let tree = build_tree(
        &prepare_scene(&artifact).unwrap(),
        &ctx,
        &props,
        &LayoutOptions {
            fonts: &fonts,
            viewport: Viewport::new((640, 360)),
            styles: None,
        },
    )
    .unwrap();
    emit(&tree, &default_font_naming).unwrap();
}

#[test]
fn class_paint_and_inline_paint_have_identical_review_trajectories() {
    let inline = "export default function Main(ctx) { return <Scene><View key=\"box\" className=\"absolute\" style={{left:ctx.localFrame*8,top:20,width:10,height:10,backgroundColor:'white'}} /></Scene>; }";
    let class = inline
        .replace("className=\"absolute\"", "className=\"absolute bg-white\"")
        .replace(",backgroundColor:'white'", "");
    let a = report(inline);
    let b = report(&class);
    assert_eq!(
        serde_json::to_value(a).unwrap(),
        serde_json::to_value(b).unwrap()
    );
}

#[test]
fn counters_do_not_generate_one_reading_warning_per_frame() {
    let source = "export default function Main(ctx) { return <Scene><Text key=\"counter\">{`${ctx.localFrame} USD`}</Text></Scene>; }";
    let review = report(source);
    assert!(
        review
            .issues
            .iter()
            .all(|issue| !issue.code.contains("reading")),
        "{:?}",
        review.issues
    );
}

#[test]
fn identity_time_scope_preserves_geometry_and_siblings() {
    let source = "export default function Main() { return <Scene><TimeScope key=\"scope\" offset={0} speed={1}><Circle key=\"dot\" cx={40} cy={40} r={20} fill=\"white\" /></TimeScope><Circle key=\"sibling\" cx={90} cy={40} r={10} fill=\"red\" /></Scene>; }";
    let plain = source
        .replace("<TimeScope key=\"scope\" offset={0} speed={1}>", "")
        .replace("</TimeScope>", "");
    let build = |source: &str| {
        let artifact = compile_motion(source).unwrap().artifact;
        let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
        let fonts = Fonts::default();
        let ctx = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let tree = build_tree(
            &prepare_scene(&artifact).unwrap(),
            &ctx,
            &props,
            &LayoutOptions {
                fonts: &fonts,
                viewport: Viewport::new((640, 360)),
                styles: None,
            },
        )
        .unwrap();
        emit(&tree, &default_font_naming).unwrap().program
    };
    assert_eq!(build(source), build(&plain));
}

fn particles(size: f64, alpha: f64, lifetime: f64) -> String {
    format!(
        r##"export default function Main(ctx) {{ return <Scene>
      <GeometryBatch key="dust" geometry="circle" positions={{particles(ctx.localFrame, ctx.fps, {{
        seed: 7, count: 4, emitter: rect(20, 100, 1, 1), birth: {{interval:0.25}}, lifetime:{lifetime},
        velocity: {{x:[300,300],y:[0,0]}}, gravity:point(0,0),loop:true
      }})}} sizes={{[{size},{size}]}} fills={{["#ffffff"]}} opacities={{[{alpha}]}}
      style={{{{position:"absolute",left:0,top:0,width:640,height:360}}}} />
    </Scene>; }}"##
    )
}

#[test]
fn small_faint_particles_do_not_strobe_and_large_rows_are_summarized() {
    assert!(
        report(&particles(3.0, 0.5, 1.0))
            .issues
            .iter()
            .all(|i| i.code != "strobe")
    );
    let reviewed = report(&particles(8.0, 1.0, 1.0));
    let strobes = reviewed
        .issues
        .iter()
        .filter(|i| i.code == "strobe")
        .collect::<Vec<_>>();
    assert_eq!(strobes.len(), 1, "{:?}", reviewed.issues);
    assert_eq!(strobes[0].node, "dust");
    assert_eq!(strobes[0].affected_rows, Some(4));
    assert_eq!(strobes[0].example_keys.len(), 3);
    let row = reviewed
        .trajectories
        .iter()
        .find(|t| t.node == "dust/0")
        .unwrap();
    assert!(
        row.points
            .iter()
            .find(|p| p.frame == 30)
            .unwrap()
            .velocity_px_per_second
            .is_none()
    );
    assert!(
        row.points
            .iter()
            .filter_map(|p| p.velocity_px_per_second)
            .all(|v| (v[0] - 300.0).abs() < 0.001)
    );
}

#[test]
fn particle_identity_survives_invisible_rows_and_birth_gaps() {
    let reviewed = report(&particles(8.0, 1.0, 0.25));
    for index in 0..4 {
        let row = reviewed
            .trajectories
            .iter()
            .find(|t| t.node == format!("dust/{index}"))
            .unwrap();
        assert!(
            row.points.iter().all(|p| p.position[0] < 97.0),
            "{:?}",
            row.points
        );
        assert!(
            row.points
                .iter()
                .filter_map(|p| p.velocity_px_per_second)
                .all(|v| (v[0] - 300.0).abs() < 0.001)
        );
    }
}

#[test]
fn text_checks_use_transformed_ink_and_preserve_true_collisions() {
    let source = r##"export default function Main() { return <Scene>
        <Text key="brand" style={{position:"absolute",left:10,top:30,fontSize:24}}>Brand</Text>
        <Text key="chapter" style={{position:"absolute",left:0,top:30,width:640,textAlign:"center",fontSize:24}}>Chapter</Text>
        <Text key="zoom" style={{position:"absolute",left:0,top:160,width:640,textAlign:"center",fontSize:24,transform:"scale(1.5)"}}>Zoom</Text>
    </Scene>; }"##;
    let reviewed = report(source);
    assert!(
        reviewed
            .issues
            .iter()
            .all(|i| !matches!(i.code, "text_overlap" | "text_out_of_frame")),
        "{:?}",
        reviewed.issues
    );
    let collision = source.replace("textAlign:\"center\"", "textAlign:\"left\"");
    let reviewed = report(&collision);
    assert!(reviewed.issues.iter().any(|i| i.code == "text_overlap"));
    assert!(
        reviewed
            .issues
            .iter()
            .any(|i| i.code == "text_out_of_frame")
    );
}

#[test]
fn reading_time_counts_words_and_cjk_characters() {
    for (text, minimum) in [
        ("one two three four five six seven eight", 2.7),
        ("这是一段需要时间阅读的中文", 3.0),
    ] {
        let source = format!(
            "export default function Main() {{ return <Scene><Text key=\"caption\" style={{{{fontSize:24,color:\"white\"}}}}>{text}</Text></Scene>; }}"
        );
        let reviewed = report(&source);
        let issue = reviewed
            .issues
            .iter()
            .find(|i| i.code == "text_readability")
            .unwrap_or_else(|| panic!("two seconds is too short for {text}: {reviewed:?}"));
        assert!(issue.recommended_seconds.unwrap() >= minimum - 1e-9);
    }
}
