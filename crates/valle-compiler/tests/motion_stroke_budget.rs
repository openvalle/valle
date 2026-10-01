#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{EvalInputs, MotionValue, eval_all, motion_context_at_frame, resolve_props};

fn circle(segments: usize, radius: f64) -> String {
    let step = std::f64::consts::TAU / segments as f64;
    let handle = 4.0 / 3.0 * (step / 4.0).tan();
    let mut path = format!("M {} 100", 160.0 + radius);
    for i in 0..segments {
        let a = i as f64 * step;
        let b = (i + 1) as f64 * step;
        let (sa, ca) = a.sin_cos();
        let (sb, cb) = b.sin_cos();
        path.push_str(&format!(
            " C {} {} {} {} {} {}",
            160.0 + radius * (ca - handle * sa),
            100.0 + radius * (sa + handle * ca),
            160.0 + radius * (cb + handle * sb),
            100.0 + radius * (sb - handle * cb),
            160.0 + radius * cb,
            100.0 + radius * sb
        ));
    }
    path.push_str(" Z");
    path
}

#[test]
fn complex_strokes_are_admitted_and_stay_bounded_at_every_sampled_frame() {
    for segments in [100, 150] {
        let source = format!(
            r##"
const A = path("{}"); const B = path("{}");
export default function Main(ctx) {{ return <Scene>
 <Path key="static" d={{strokeToPath(A,6)}} fill="white" />
 <Path key="dynamic" d={{strokeToPath(morphPath(A,B,ctx.progress),2+ctx.progress*4)}} fill="white" />
</Scene>; }}"##,
            circle(segments, 70.0),
            circle(segments, 80.0)
        );
        let compiled = compile_motion(&source).unwrap();
        compiled.artifact.validate().unwrap();
        let props = resolve_props(&compiled.artifact.controls, &Default::default()).unwrap();
        for frame in [0, 8, 15, 29] {
            let ctx =
                motion_context_at_frame(frame, 30, valle_timeline::FrameRate::new(30, 1).unwrap())
                    .unwrap();
            let values = eval_all(
                &compiled.artifact,
                EvalInputs {
                    ctx: &ctx,
                    props: &props,
                    unit: None,
                    viewport: None,
                },
            )
            .unwrap();
            for node in &compiled.artifact.nodes {
                if let valle_motion::NodeKind::Path {
                    d: valle_motion::PathValue::Expr { expr, .. },
                    ..
                } = &node.kind
                {
                    let MotionValue::PathData(path) = &values[expr.0 as usize] else {
                        panic!("path result")
                    };
                    assert!(!path.points.is_empty());
                    assert!(path.points.len() <= valle_motion::geometry::MAX_FRAME_GEOMETRY_POINTS);
                }
            }
        }
    }
}
