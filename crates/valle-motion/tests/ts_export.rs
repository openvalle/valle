//! TypeScript export smoke tests for shared primitive types.

#![cfg(feature = "ts")]

use ts_rs::{Config, TS};
use valle_draw::program::recording::{Paint, ProgramRecording, RecordCmd};
use valle_motion::{
    ArrowKind, ArrowSpec, ColorValue, CueState, CueWindow, GeometryEvalPolicy, GradientStopValue,
    MaskValue, MotionContext, MotionDiagnostic, MotionValue, NarrationRetime, PaintValue,
    PathBooleanOp, PathData, PathStroke, PointValue, RectValue, SignalPlan,
};

fn decl<T: TS + ?Sized>() -> String {
    T::decl(&Config::default())
}

#[test]
fn motion_context_ts_matches_the_wire_shape() {
    let declaration = decl::<MotionContext>();
    assert!(declaration.contains("localFrame: number"), "{declaration}");
    assert!(declaration.contains("progress: number"), "{declaration}");
    assert!(declaration.contains("sample:"), "{declaration}");
    assert!(
        declaration.contains("durationFrames: number"),
        "{declaration}"
    );
    assert!(
        declaration.contains("currentPhase: PhaseKind"),
        "{declaration}"
    );
    assert!(declaration.contains("fps: string"), "{declaration}");
    assert!(
        declaration.contains("sample: { composition: string }"),
        "{declaration}"
    );
}

#[test]
fn cue_signal_ts_matches_the_author_and_host_wire_shapes() {
    let state = decl::<CueState>();
    for field in [
        "active: boolean",
        "progress: number",
        "enter: number",
        "hold: number",
        "exit: number",
        "localFrame: number",
    ] {
        assert!(state.contains(field), "{state}");
    }

    let window = decl::<CueWindow>();
    for field in [
        "startFrame: number",
        "endFrame: number",
        "enterFrames: number",
        "exitFrames: number",
    ] {
        assert!(window.contains(field), "{window}");
    }

    let retime = NarrationRetime::export_to_string(&Config::default()).expect("retime export");
    assert!(retime.contains("durationFrames: number"), "{retime}");
    assert!(retime.contains("phases: PhaseLayout"), "{retime}");
}

#[test]
fn signal_plan_ts_matches_the_prepare_wire() {
    let plan = SignalPlan::export_to_string(&Config::default()).expect("plan export");
    assert!(plan.contains("export type SignalPlan"), "{plan}");
    assert!(plan.contains("steps: Array<PlanStep>"), "{plan}");
}

#[test]
fn shared_tagged_unions_keep_their_discriminants() {
    let value = decl::<MotionValue>();
    assert!(value.contains(r#""kind": "number""#), "{value}");
    assert!(value.contains(r#""kind": "point""#), "{value}");
    assert!(value.contains(r#""kind": "rect""#), "{value}");
    assert!(value.contains(r#""kind": "pathData""#), "{value}");
    let command = decl::<RecordCmd>();
    assert!(command.contains(r#""op": "beginSaveLayer""#), "{command}");
    assert!(command.contains(r#""op": "beginMotionGlass""#), "{command}");
    assert!(command.contains("program: MotionGlassProgram"), "{command}");
    assert!(!command.contains("packedProgram"), "{command}");
    assert!(command.contains(r#""op": "end""#), "{command}");
}

#[test]
fn typed_geometry_and_eval_policy_are_shared_with_hosts() {
    let path = decl::<PathData>();
    assert!(path.contains("verbs: Array<PathVerb>"), "{path}");
    assert!(path.contains("points: Array<Point>"), "{path}");
    let policy = decl::<GeometryEvalPolicy>();
    for value in [
        r#""prepareCached""#,
        r#""frameExpr""#,
        r#""precomputedTrajectory""#,
    ] {
        assert!(policy.contains(value), "{policy}");
    }
}

#[test]
fn geometry_toolkit_stroke_arrow_and_boolean_types_are_shared_with_hosts() {
    let boolean = decl::<PathBooleanOp>();
    for value in [
        r#""union""#,
        r#""intersection""#,
        r#""difference""#,
        r#""xor""#,
    ] {
        assert!(boolean.contains(value), "{boolean}");
    }

    let arrow_kind = decl::<ArrowKind>();
    assert!(arrow_kind.contains(r#""triangle""#), "{arrow_kind}");
    assert!(arrow_kind.contains(r#""open""#), "{arrow_kind}");

    let arrow = decl::<ArrowSpec>();
    assert!(arrow.contains("kind: ArrowKind"), "{arrow}");
    assert!(arrow.contains("size: number"), "{arrow}");

    let stroke = decl::<PathStroke>();
    for field in [
        "paint: PaintValue",
        "width: NumberValue",
        "dash: Array<number> | null",
        "dashOffset: NumberValue",
        "cap: Cap",
        "join: Join",
        "miterLimit: number",
    ] {
        assert!(stroke.contains(field), "{stroke}");
    }
}

#[test]
fn glass_authoring_types_are_generated_for_tooling() {
    // Export Glass authoring types for editor validation and completion.
    let node = decl::<valle_motion::glass::GlassNode>();
    for field in [
        "surfaceId: GlassSurfaceId",
        "fieldId: GlassFieldId | null",
        "shape: GlassShapeBinding",
        "material: GlassMaterialBinding | null",
        "motion: GlassSurfaceMotionBinding",
        "presence: NumberValue",
        "foreground: GlassForegroundIntent",
    ] {
        assert!(node.contains(field), "{node}");
    }
    let shape = decl::<valle_motion::glass::GlassShapeBinding>();
    assert!(shape.contains(r#""kind": "circle""#), "{shape}");
    assert!(shape.contains(r#""kind": "continuousRect""#), "{shape}");
    let field = decl::<valle_motion::glass::GlassFieldNode>();
    assert!(field.contains("merge: GlassMergeIntent"), "{field}");
    assert!(
        field.contains("environment: GlassEnvironmentBinding"),
        "{field}"
    );
    // The id types are string aliases on the wire.
    let surface = decl::<valle_motion::glass::GlassSurfaceId>();
    assert_eq!(surface, "type GlassSurfaceId = string;");
    let field_id = decl::<valle_motion::glass::GlassFieldId>();
    assert_eq!(field_id, "type GlassFieldId = string;");
}

#[test]
fn typed_gradient_paints_are_shared_with_hosts() {
    let point = decl::<PointValue>();
    assert!(point.contains(r#""kind": "static""#), "{point}");
    assert!(point.contains("value: Point"), "{point}");
    assert!(point.contains(r#""kind": "expr""#), "{point}");
    assert!(point.contains("expr: ExprId"), "{point}");

    let color = decl::<ColorValue>();
    assert!(color.contains("value: string"), "{color}");
    assert!(color.contains("expr: ExprId"), "{color}");

    let stop = decl::<GradientStopValue>();
    assert!(stop.contains("offset: NumberValue"), "{stop}");
    assert!(stop.contains("color: ColorValue"), "{stop}");

    let number = decl::<valle_motion::NumberValue>();
    assert!(number.contains("expr: ExprId"), "{number}");

    let paint = PaintValue::export_to_string(&Config::default()).expect("paint export");
    for kind in ["solid", "linear", "radial", "conic"] {
        assert!(paint.contains(&format!(r#""kind": "{kind}""#)), "{paint}");
    }
    for dependency in [
        r#"import type { PointValue }"#,
        r#"import type { ColorValue }"#,
        r#"import type { NumberValue }"#,
        r#"import type { GradientStopValue }"#,
        r#"import type { SpreadMode }"#,
    ] {
        assert!(paint.contains(dependency), "{paint}");
    }
}

#[test]
fn typed_clip_and_mask_values_are_shared_with_hosts() {
    let rect = decl::<RectValue>();
    assert!(rect.contains(r#""kind": "static""#), "{rect}");
    assert!(rect.contains("value: Rect"), "{rect}");
    assert!(rect.contains(r#""kind": "expr""#), "{rect}");
    assert!(rect.contains("expr: ExprId"), "{rect}");

    let mask = MaskValue::export_to_string(&Config::default()).expect("mask export");
    assert!(mask.contains(r#""kind": "paint""#), "{mask}");
    assert!(mask.contains("paint: PaintValue"), "{mask}");
    assert!(mask.contains(r#""kind": "image""#), "{mask}");
    assert!(mask.contains("source: string"), "{mask}");
    assert!(mask.contains(r#"import type { PaintValue }"#), "{mask}");
}

#[test]
fn colors_stay_strings_on_the_wire() {
    let declaration = decl::<Paint>();
    assert!(declaration.contains("string"), "{declaration}");
    assert!(!declaration.contains("r: number"), "{declaration}");
}

#[test]
fn shared_dependency_closure_exports() {
    let display = ProgramRecording::export_to_string(&Config::default()).expect("display export");
    assert!(
        display.contains("export type ProgramRecording"),
        "{display}"
    );
    let diagnostic =
        MotionDiagnostic::export_to_string(&Config::default()).expect("diagnostic export");
    assert!(
        diagnostic.contains("export type MotionDiagnostic"),
        "{diagnostic}"
    );
}
