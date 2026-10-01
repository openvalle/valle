use std::collections::BTreeMap;

use valle_draw::{Point, Rect};
use valle_timeline::internal::{MotionInstanceId, SampleTime, TrackEpoch};
use valle_timeline::{FrameRate, TimeError};

use crate::NodeKind;
use crate::artifact::{NumberValue, PointValue, SceneArtifact, StyleValue};
use crate::eval::{EvalError, EvalInputs, EvalPlan, ResolvedProps, eval_roots_planned_into};
use crate::expr::ExprId;
use crate::glass::ids::GlassSurfaceId;
use crate::glass::intent::GlassNode;
use crate::glass::validate::{glass_track_expr_roots, validate_glass_schema};
use crate::motion_context_at_sample;

#[derive(Debug, Clone, PartialEq)]
pub struct GlassLocalShape {
    pub rect: Rect,
    pub presence: f64,
    pub intensity: f64,
    pub drive_translation: Point,
    pub drive_pressure: f64,
    pub drive_twist: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GlassSurfaceTrackSample {
    pub surface_id: GlassSurfaceId,
    pub instance: MotionInstanceId,
    pub epoch: TrackEpoch,
    pub time: SampleTime,
    pub local: GlassLocalShape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlassTrackError {
    Schema(String),
    Time(TimeError),
    Eval(EvalError),
    Unbound { node: String, property: String },
    IncompleteSlice { node: String },
}

impl std::fmt::Display for GlassTrackError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Schema(message) => write!(formatter, "{message}"),
            Self::Time(error) => write!(formatter, "{error}"),
            Self::Eval(error) => write!(formatter, "{error}"),
            Self::Unbound { node, property } => {
                write!(formatter, "Glass '{node}' is missing a finite `{property}`")
            }
            Self::IncompleteSlice { node } => write!(
                formatter,
                "Glass '{node}' has dynamic layout that cannot prove a complete dependency slice"
            ),
        }
    }
}

impl std::error::Error for GlassTrackError {}

/// Sample every Glass surface at the requested composition times using the compiled
/// dependency slice. Closing the cache and evaluating the full arena must match.
pub fn sample_tracks(
    artifact: &SceneArtifact,
    instance: &MotionInstanceId,
    times: &[SampleTime],
    duration_frames: u32,
    fps: FrameRate,
    epoch: &TrackEpoch,
) -> Result<Vec<GlassSurfaceTrackSample>, GlassTrackError> {
    validate_glass_schema(artifact).map_err(|errors| {
        GlassTrackError::Schema(
            errors
                .first()
                .map(|error| error.message.clone())
                .unwrap_or_else(|| "invalid Glass schema".into()),
        )
    })?;
    let mut samples = Vec::new();
    let props = ResolvedProps::default();
    let plan = EvalPlan::new(&artifact.exprs);
    let tracks: Vec<_> = artifact
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(at, node)| match &node.kind {
            NodeKind::Glass(glass) => Some((
                at,
                glass,
                plan.schedule(glass_track_expr_roots(artifact, at)),
            )),
            _ => None,
        })
        .collect();
    let mut shared = BTreeMap::new();
    let cache_across_times = times.len() > 1;
    for time in times {
        let ctx = motion_context_at_sample(*time, duration_frames, fps)
            .ok_or(GlassTrackError::Time(TimeError::Overflow))?;
        let inputs = EvalInputs {
            ctx: &ctx,
            props: &props,
            unit: None,
            viewport: None,
        };
        let mut values = BTreeMap::new();
        for (at, glass, order) in &tracks {
            eval_roots_planned_into(
                artifact,
                &plan,
                inputs,
                order,
                &mut values,
                cache_across_times.then_some(&mut shared),
            )
            .map_err(GlassTrackError::Eval)?;
            let local = local_shape(artifact, *at, glass, &values)?;
            samples.push(GlassSurfaceTrackSample {
                surface_id: glass.surface_id.clone(),
                instance: instance.clone(),
                epoch: epoch.clone(),
                time: *time,
                local,
            });
        }
    }
    Ok(samples)
}

fn local_shape(
    artifact: &SceneArtifact,
    at: usize,
    glass: &GlassNode,
    values: &BTreeMap<ExprId, crate::value::MotionValue>,
) -> Result<GlassLocalShape, GlassTrackError> {
    let node = &artifact.nodes[at];
    let number = |property: &str, fallback: Option<f64>| -> Result<f64, GlassTrackError> {
        for style in &node.styles {
            if style.property == property {
                return read_style_number(style, values, &glass.surface_id, property);
            }
        }
        fallback.ok_or(GlassTrackError::Unbound {
            node: glass.surface_id.as_str().into(),
            property: property.into(),
        })
    };
    let left = number("left", Some(0.0))?;
    let top = number("top", Some(0.0))?;
    let width = number("width", None)?;
    let height = number("height", None)?;
    Ok(GlassLocalShape {
        rect: Rect::new(left, top, width, height),
        presence: read_number(&glass.presence, values, &glass.surface_id, "presence")?,
        intensity: read_number(
            &glass.motion.intensity,
            values,
            &glass.surface_id,
            "intensity",
        )?,
        drive_translation: read_point(
            glass.motion.drive.translation.as_ref(),
            values,
            Point::new(0.0, 0.0),
        )?,
        drive_pressure: read_optional_number(
            glass.motion.drive.pressure.as_ref(),
            values,
            &glass.surface_id,
            "pressure",
        )?,
        drive_twist: read_optional_number(
            glass.motion.drive.twist.as_ref(),
            values,
            &glass.surface_id,
            "twist",
        )?,
    })
}

fn read_style_number(
    style: &crate::artifact::StyleBinding,
    values: &BTreeMap<ExprId, crate::value::MotionValue>,
    surface: &GlassSurfaceId,
    property: &str,
) -> Result<f64, GlassTrackError> {
    match &style.value {
        StyleValue::Static {
            value: crate::value::MotionValue::Number(value),
        }
        | StyleValue::Static {
            value: crate::value::MotionValue::Length(crate::value::Length { value, .. }),
        } => finite(*value, surface, property),
        StyleValue::Expr { expr } => read_expr_number(*expr, values, surface, property),
        _ => Err(GlassTrackError::Unbound {
            node: surface.as_str().into(),
            property: property.into(),
        }),
    }
}

fn read_number(
    value: &NumberValue,
    values: &BTreeMap<ExprId, crate::value::MotionValue>,
    surface: &GlassSurfaceId,
    property: &str,
) -> Result<f64, GlassTrackError> {
    match value {
        NumberValue::Static { value } => finite(*value, surface, property),
        NumberValue::Expr { expr } => read_expr_number(*expr, values, surface, property),
    }
}

fn read_optional_number(
    value: Option<&NumberValue>,
    values: &BTreeMap<ExprId, crate::value::MotionValue>,
    surface: &GlassSurfaceId,
    property: &str,
) -> Result<f64, GlassTrackError> {
    match value {
        None => Ok(0.0),
        Some(value) => read_number(value, values, surface, property),
    }
}

fn read_point(
    value: Option<&PointValue>,
    values: &BTreeMap<ExprId, crate::value::MotionValue>,
    fallback: Point,
) -> Result<Point, GlassTrackError> {
    match value {
        None => Ok(fallback),
        Some(PointValue::Static { value }) => Ok(*value),
        Some(PointValue::Expr { expr }) => match values.get(expr) {
            Some(crate::value::MotionValue::Point(point)) => Ok(*point),
            Some(crate::value::MotionValue::Vec2(vec)) => Ok(Point::new(vec.x, vec.y)),
            _ => Err(GlassTrackError::Unbound {
                node: "drive".into(),
                property: "translation".into(),
            }),
        },
    }
}

fn read_expr_number(
    expr: ExprId,
    values: &BTreeMap<ExprId, crate::value::MotionValue>,
    surface: &GlassSurfaceId,
    property: &str,
) -> Result<f64, GlassTrackError> {
    match values.get(&expr) {
        Some(crate::value::MotionValue::Number(value)) => finite(*value, surface, property),
        Some(crate::value::MotionValue::Length(length)) => finite(length.value, surface, property),
        _ => Err(GlassTrackError::IncompleteSlice {
            node: surface.as_str().into(),
        }),
    }
}

fn finite(value: f64, surface: &GlassSurfaceId, property: &str) -> Result<f64, GlassTrackError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(GlassTrackError::Unbound {
            node: surface.as_str().into(),
            property: property.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use valle_timeline::internal::{DiscontinuityIndex, TimeMapSegment};

    use crate::artifact::{
        ARTIFACT_FORMAT_VERSION, CapabilitySet, ChildRange, NumberValue, SceneArtifact, SceneNode,
        StyleBinding, StyleValue,
    };
    use crate::controls::ControlsSchema;
    use crate::eval::{EvalInputs, eval_all};
    use crate::expr::{ContextInput, Expr, ExprId};
    use crate::glass::ids::GlassSurfaceId;
    use crate::glass::intent::{
        DEFAULT_PRESENCE, GlassEnvironmentBinding, GlassForegroundIntent, GlassMaterialBinding,
        GlassNode, GlassShapeBinding, GlassSurfaceMotionBinding,
    };
    use crate::glass::validate_glass_schema;
    use crate::value::MotionValue;
    use crate::{NodeId, NodeKind, motion_context_at_frame};

    fn controls() -> ControlsSchema {
        ControlsSchema {
            props: BTreeMap::new(),
            data: BTreeMap::new(),
            assets: BTreeMap::new(),
        }
    }

    fn fixture(discrete_time: bool) -> SceneArtifact {
        let time = if discrete_time {
            Expr::Context {
                input: ContextInput::LocalFrame,
            }
        } else {
            Expr::Context {
                input: ContextInput::LocalProgress,
            }
        };
        SceneArtifact {
            role: valle_timeline::MotionRole::Clip,
            camera: None,
            format_version: ARTIFACT_FORMAT_VERSION,
            capability_set: CapabilitySet::base(),
            component: "glass-track".into(),
            composition: None,
            controls: controls(),
            resource_refs: vec![],
            exprs: vec![time],
            instance_groups: vec![],
            nodes: vec![
                SceneNode {
                    key: "root".into(),
                    kind: NodeKind::Group,
                    space: None,
                    class_names: vec![],
                    class_conditions: Default::default(),
                    styles: vec![],
                    visibility: None,
                    children: ChildRange { start: 0, end: 1 },
                    semantic: None,
                },
                SceneNode {
                    key: "lens".into(),
                    kind: NodeKind::Glass(GlassNode {
                        surface_id: GlassSurfaceId::new("hero-lens").unwrap(),
                        field_id: None,
                        shape: GlassShapeBinding::Circle,
                        material: Some(GlassMaterialBinding::defaults()),
                        environment: Some(GlassEnvironmentBinding::defaults()),
                        motion: GlassSurfaceMotionBinding::independent_defaults(),
                        presence: NumberValue::Expr { expr: ExprId(0) },
                        foreground: GlassForegroundIntent::defaults(),
                    }),
                    space: None,
                    class_names: vec![],
                    class_conditions: Default::default(),
                    styles: vec![
                        StyleBinding {
                            property: "left".into(),
                            value: StyleValue::Static {
                                value: MotionValue::Number(10.0),
                            },
                        },
                        StyleBinding {
                            property: "top".into(),
                            value: StyleValue::Static {
                                value: MotionValue::Number(20.0),
                            },
                        },
                        StyleBinding {
                            property: "width".into(),
                            value: StyleValue::Static {
                                value: MotionValue::Number(80.0),
                            },
                        },
                        StyleBinding {
                            property: "height".into(),
                            value: StyleValue::Static {
                                value: MotionValue::Number(80.0),
                            },
                        },
                    ],
                    visibility: None,
                    children: ChildRange::EMPTY,
                    semantic: None,
                },
            ],
            node_children: vec![NodeId(1)],
            root: NodeId(0),
        }
    }

    #[test]
    fn continuous_progress_binding_is_legal_and_slice_matches_full() {
        let artifact = fixture(false);
        validate_glass_schema(&artifact).unwrap();
        let fps = FrameRate::new(30, 1).unwrap();
        let time = crate::sample_time_at_frame(10, fps).unwrap();
        let instance = MotionInstanceId::new("clip-a").unwrap();
        let epoch = TrackEpoch::new(
            1,
            instance.clone(),
            TimeMapSegment::new(0),
            0,
            DiscontinuityIndex::NONE,
        );
        let samples = sample_tracks(&artifact, &instance, &[time], 30, fps, &epoch).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].local.rect.width, 80.0);
        let ctx = motion_context_at_frame(10, 30, fps).unwrap();
        let props = ResolvedProps::default();
        let inputs = EvalInputs {
            ctx: &ctx,
            props: &props,
            unit: None,
            viewport: None,
        };
        let full = eval_all(&artifact, inputs).unwrap();
        let slice_presence = samples[0].local.presence;
        let MotionValue::Number(full_presence) = &full[0] else {
            panic!("presence");
        };
        assert!((slice_presence - *full_presence).abs() < 1e-12);
        let _ = DEFAULT_PRESENCE;
    }

    #[test]
    fn shared_track_expressions_stay_correct_across_surfaces_and_reordered_samples() {
        let mut artifact = fixture(false);
        let mut second = artifact.nodes[1].clone();
        second.key = "lens-2".into();
        if let NodeKind::Glass(glass) = &mut second.kind {
            glass.surface_id = GlassSurfaceId::new("second-lens").unwrap();
        }
        artifact.nodes.push(second);
        artifact.node_children.push(NodeId(2));
        artifact.nodes[0].children.end = 2;
        let fps = FrameRate::new(30, 1).unwrap();
        let times = [2, 5, 2].map(|frame| crate::sample_time_at_frame(frame, fps).unwrap());
        let instance = MotionInstanceId::new("clip-a").unwrap();
        let epoch = TrackEpoch::new(
            1,
            instance.clone(),
            TimeMapSegment::new(0),
            0,
            DiscontinuityIndex::NONE,
        );
        let samples = sample_tracks(&artifact, &instance, &times, 30, fps, &epoch).unwrap();
        assert_eq!(samples.len(), 6);
        for pair in samples.chunks_exact(2) {
            assert_eq!(pair[0].local, pair[1].local);
        }
        assert_ne!(samples[0].local.presence, samples[2].local.presence);
        assert_eq!(samples[0].local.presence, samples[4].local.presence);
    }

    #[test]
    fn local_frame_binding_is_rejected_on_track_slots() {
        let artifact = fixture(true);
        let errors = validate_glass_schema(&artifact).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("discrete"))
        );
    }

    #[test]
    fn glass_artifact_validates_with_declared_capability() {
        // G4.7: production admission is open; a Glass artifact with the capability declared
        // validates fully (typed schema + capability registry).
        let mut artifact = fixture(false);
        let mut names = artifact.capability_set.names.clone();
        names.push(crate::glass::MOTION_GLASS_CAPABILITY.into());
        names.sort();
        names.dedup();
        artifact.capability_set = CapabilitySet::new(names);
        artifact.validate().expect("artifact validates");
    }
}
