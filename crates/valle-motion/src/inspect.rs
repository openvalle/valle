//! Bounded, read-only local property samples. Uses the render evaluator, never playback history.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use valle_timeline::FrameRate;

use crate::value::{Angle, Length2, LengthUnit};
use crate::{
    CueSchedule, CueWindow, EvalInputs, Expr, MotionValue, PhaseSpec, SceneArtifact, StyleValue,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropertySampleRequest {
    pub node: String,
    pub start_frame: u32,
    pub end_frame: u32,
    pub max_points: u32,
    pub duration_frames: u32,
    pub fps: FrameRate,
    pub phases: PhaseSpec,
    pub props: BTreeMap<String, MotionValue>,
    pub cues: BTreeMap<String, CueWindow>,
    pub viewport: [u32; 2],
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertySamples {
    pub node: String,
    pub channels: Vec<PropertyChannel>,
    pub frames: Vec<u32>,
    pub phase_boundaries: [u32; 2],
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyChannel {
    pub property: String,
    pub unit: &'static str,
    pub samples: Vec<PropertyPoint>,
    pub unavailable: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyPoint {
    pub frame: u32,
    pub value: Vec<f64>,
    /// Central difference over adjacent source frames; absent at endpoints or uncertain boundaries.
    pub velocity: Option<Vec<f64>>,
    pub boundary: Option<&'static str>,
}

pub fn sample_properties(
    artifact: &SceneArtifact,
    request: &PropertySampleRequest,
) -> Result<PropertySamples, String> {
    artifact
        .validate()
        .map_err(|errors| format!("invalid Motion artifact: {errors:?}"))?;
    if request.duration_frames == 0
        || request.start_frame > request.end_frame
        || request.end_frame >= request.duration_frames
        || !(2..=240).contains(&request.max_points)
        || request.viewport.contains(&0)
    {
        return Err("invalid sample range, viewport or point limit (2..=240)".into());
    }
    let node = artifact
        .nodes
        .iter()
        .find(|n| n.key == request.node)
        .ok_or("unknown Motion node")?;
    let props =
        crate::resolve_props(&artifact.controls, &request.props).map_err(|e| e.to_string())?;
    let cues =
        CueSchedule::resolve(&artifact.controls, &request.cues).map_err(|e| e.to_string())?;
    let phases = crate::phase_windows(&request.phases, request.duration_frames);
    let span = request.end_frame - request.start_frame;
    let count = request.max_points.min(span.saturating_add(1));
    let frames: Vec<_> = (0..count)
        .map(|i| {
            request.start_frame
                + (u64::from(i) * u64::from(span) / u64::from((count - 1).max(1))) as u32
        })
        .collect();
    let mut result = PropertySamples {
        node: node.key.clone(),
        channels: Vec::new(),
        frames: frames.clone(),
        phase_boundaries: [
            phases.enter_frames,
            phases.enter_frames + phases.hold_frames,
        ],
    };
    let post = crate::post_layout_dependent(&artifact.exprs);
    let unit = crate::expr::unit_dependent(&artifact.exprs);
    for property in ["opacity", "translate", "scale", "rotate"] {
        // Only explicit bindings are inspected. Absent/inherited CSS is not fabricated as a default.
        let Some(binding) = node.styles.iter().rev().find(|s| s.property == property) else {
            continue;
        };
        let mut channel = PropertyChannel {
            property: property.into(),
            unit: match property {
                "translate" => "px",
                "rotate" => "deg",
                _ => "unit",
            },
            samples: Vec::new(),
            unavailable: None,
        };
        let mut live = BTreeSet::new();
        if let StyleValue::Expr { expr } = binding.value {
            let mut todo = vec![expr];
            while let Some(id) = todo.pop() {
                if live.insert(id) {
                    todo.extend(artifact.exprs[id.0 as usize].children());
                }
            }
        }
        if live.len() > 4096
            || live
                .iter()
                .any(|id| post[id.0 as usize] || unit[id.0 as usize])
        {
            channel.unavailable =
                Some("Requires layout/per-unit context or exceeds the inspection budget".into());
            result.channels.push(channel);
            continue;
        }
        let at = |frame| -> Result<(Vec<f64>, Vec<MotionValue>), String> {
            let ctx = crate::motion_context_at(frame, &phases, request.fps)
                .ok_or("frame outside scene")?;
            let signals = cues.sample(frame, request.fps);
            let values = crate::eval::eval_slice(
                artifact,
                EvalInputs {
                    ctx: &ctx,
                    props: &props,
                    signals: &signals,
                    unit: None,
                    viewport: Some((request.viewport[0] as f64, request.viewport[1] as f64)),
                },
                &live,
            )
            .map_err(|e| e.to_string())?;
            let value = match &binding.value {
                StyleValue::Static { value } => value,
                StyleValue::Expr { expr } => values[expr.0 as usize]
                    .as_ref()
                    .ok_or("missing expression")?,
            };
            let numeric = numeric(property, value)
                .ok_or("Requires resolved CSS units or a nonnumeric value")?;
            let mut branches = Vec::new();
            for id in &live {
                match &artifact.exprs[id.0 as usize] {
                    Expr::Select { condition, .. } => {
                        branches.push(values[condition.0 as usize].clone().unwrap())
                    }
                    Expr::Interpolate { input, stops, .. } => {
                        if let Some(MotionValue::Number(x)) = &values[input.0 as usize] {
                            branches.push(MotionValue::Number(
                                stops.partition_point(|s| s.input <= *x) as f64,
                            ));
                        }
                    }
                    _ => {}
                }
            }
            Ok((numeric, branches))
        };
        // A conservative guard, not an assertion that all other expressions are continuous.
        let discrete = live.iter().any(|id| {
            matches!(
                &artifact.exprs[id.0 as usize],
                Expr::MathUnary {
                    op: crate::expr::MathUnaryOp::Floor
                        | crate::expr::MathUnaryOp::Ceil
                        | crate::expr::MathUnaryOp::Round
                        | crate::expr::MathUnaryOp::Trunc
                        | crate::expr::MathUnaryOp::Fract,
                    ..
                } | Expr::MathBinary {
                    op: crate::expr::MathBinaryOp::Mod | crate::expr::MathBinaryOp::Remainder,
                    ..
                } | Expr::PathTrajectory { .. }
                    | Expr::Template { .. }
            )
        });
        for &frame in &frames {
            let sample = (|| -> Result<PropertyPoint, String> {
                let (value, branch) = at(frame)?;
                let mut boundary = discrete.then_some("discrete expression");
                let velocity = if frame == 0 || frame + 1 == request.duration_frames || discrete {
                    None
                } else {
                    let (before, a) = at(frame - 1)?;
                    let (after, b) = at(frame + 1)?;
                    if a != branch || b != branch {
                        boundary = Some("branch or segment boundary");
                        None
                    } else {
                        let fps = crate::frame_rate_as_f64(request.fps);
                        Some(
                            before
                                .iter()
                                .zip(after)
                                .map(|(a, b)| (b - a) * fps / 2.0)
                                .collect(),
                        )
                    }
                };
                Ok(PropertyPoint {
                    frame,
                    value,
                    velocity,
                    boundary,
                })
            })();
            match sample {
                Ok(point) => channel.samples.push(point),
                Err(error) => {
                    channel.unavailable = Some(error);
                    channel.samples.clear();
                    break;
                }
            }
        }
        result.channels.push(channel);
    }
    Ok(result)
}

fn numeric(property: &str, value: &MotionValue) -> Option<Vec<f64>> {
    match (property, value) {
        ("opacity" | "scale", MotionValue::Number(v)) => Some(vec![*v]),
        ("scale", MotionValue::Vec2(v)) => Some(vec![v.x, v.y]),
        ("translate", MotionValue::Point(p)) => Some(vec![p.x, p.y]),
        ("translate", MotionValue::Length2(v))
            if v.x.unit == LengthUnit::Px && v.y.unit == LengthUnit::Px =>
        {
            Some(vec![v.x.value, v.y.value])
        }
        ("rotate", MotionValue::Angle(v)) => Some(vec![v.as_degrees()]),
        ("rotate", MotionValue::Str(v)) => Angle::parse(v).map(|v| vec![v.as_degrees()]),
        ("translate", MotionValue::Str(v)) => {
            Length2::parse(v).and_then(|v| numeric(property, &MotionValue::Length2(v)))
        }
        _ => None,
    }
}
