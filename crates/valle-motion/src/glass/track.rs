use valle_draw::{Point, Rect};
use valle_timeline::internal::{MotionInstanceId, SampleTime, TrackEpoch};
use valle_timeline::{FrameRate, TimeError};

use crate::NodeKind;
use crate::artifact::{NumberValue, PointValue, SceneArtifact, StyleValue};
use crate::eval::{EvalError, EvalInputs, ResolvedProps, eval_slice};
use crate::expr::ExprId;
use crate::glass::ids::GlassSurfaceId;
use crate::glass::intent::GlassNode;
use crate::glass::validate::{glass_track_expr_roots, reachable_exprs, validate_glass_schema};
use crate::signals::ResolvedSignals;
use crate::{PhaseLayout, motion_context_at_sample};

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
    layout: &PhaseLayout,
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
    let signals = ResolvedSignals::default();
    for time in times {
        let ctx = motion_context_at_sample(*time, layout, fps)
            .ok_or(GlassTrackError::Time(TimeError::Overflow))?;
        let inputs = EvalInputs {
            ctx: &ctx,
            props: &props,
            signals: &signals,
            unit: None,
            viewport: None,
        };
        for (at, node) in artifact.nodes.iter().enumerate() {
            let NodeKind::Glass(glass) = &node.kind else {
                continue;
            };
            let roots = glass_track_expr_roots(artifact, at);
            let slice = reachable_exprs(artifact, &roots);
            let values = eval_slice(artifact, inputs, &slice).map_err(GlassTrackError::Eval)?;
            let local = local_shape(artifact, at, glass, &values)?;
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
    values: &[Option<crate::value::MotionValue>],
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
    values: &[Option<crate::value::MotionValue>],
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
    values: &[Option<crate::value::MotionValue>],
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
    values: &[Option<crate::value::MotionValue>],
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
    values: &[Option<crate::value::MotionValue>],
    fallback: Point,
) -> Result<Point, GlassTrackError> {
    match value {
        None => Ok(fallback),
        Some(PointValue::Static { value }) => Ok(*value),
        Some(PointValue::Expr { expr }) => {
            match values.get(expr.0 as usize).and_then(Option::as_ref) {
                Some(crate::value::MotionValue::Point(point)) => Ok(*point),
                Some(crate::value::MotionValue::Vec2(vec)) => Ok(Point::new(vec.x, vec.y)),
                _ => Err(GlassTrackError::Unbound {
                    node: "drive".into(),
                    property: "translation".into(),
                }),
            }
        }
    }
}

fn read_expr_number(
    expr: ExprId,
    values: &[Option<crate::value::MotionValue>],
    surface: &GlassSurfaceId,
    property: &str,
) -> Result<f64, GlassTrackError> {
    match values.get(expr.0 as usize).and_then(Option::as_ref) {
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
    use crate::controls::{ControlsSchema, FrameControl, OptionalFrameControl, TimingControls};
    use crate::eval::{EvalInputs, eval_all};
    use crate::expr::{ContextInput, Expr, ExprId};
    use crate::glass::ids::GlassSurfaceId;
    use crate::glass::intent::{
        DEFAULT_PRESENCE, GlassEnvironmentBinding, GlassForegroundIntent, GlassMaterialBinding,
        GlassNode, GlassShapeBinding, GlassSurfaceMotionBinding,
    };
    use crate::glass::validate_glass_schema;
    use crate::value::MotionValue;
    use crate::{NodeId, NodeKind, motion_context_at, phase_windows};

    fn controls() -> ControlsSchema {
        ControlsSchema {
            props: BTreeMap::new(),
            data: BTreeMap::new(),
            timing: TimingControls {
                enter_frames: FrameControl {
                    default: 0,
                    min: 0,
                    max: None,
                },
                hold_cycle_frames: OptionalFrameControl {
                    default: None,
                    min: 1,
                    max: None,
                },
                exit_frames: FrameControl {
                    default: 0,
                    min: 0,
                    max: None,
                },
            },
            cues: BTreeMap::new(),
            assets: BTreeMap::new(),
            camera: Default::default(),
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
            camera: None,
            format_version: ARTIFACT_FORMAT_VERSION,
            capability_set: CapabilitySet::base(),
            component: "glass-track".into(),
            controls: controls(),
            resource_refs: vec![],
            exprs: vec![time],
            nodes: vec![
                SceneNode {
                    key: "root".into(),
                    kind: NodeKind::Group,
                    space: None,
                    class_names: vec![],
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
        let layout = phase_windows(&artifact.controls.phase_spec(), 30);
        let time = crate::sample_time_at_frame(10, fps).unwrap();
        let instance = MotionInstanceId::new("clip-a").unwrap();
        let epoch = TrackEpoch::new(
            1,
            instance.clone(),
            TimeMapSegment::new(0),
            0,
            DiscontinuityIndex::NONE,
        );
        let samples = sample_tracks(&artifact, &instance, &[time], &layout, fps, &epoch).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].local.rect.width, 80.0);
        let ctx = motion_context_at(10, &layout, fps).unwrap();
        let props = ResolvedProps::default();
        let signals = ResolvedSignals::default();
        let inputs = EvalInputs {
            ctx: &ctx,
            props: &props,
            signals: &signals,
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
