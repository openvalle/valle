use valle_timeline::RationalTime;

use crate::expr::{ContextInput, Expr, ExprId, Extrapolation};
use crate::value::MotionEasing;

/// Proven temporal smoothness of an expression used as a Glass track binding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum TemporalContinuity {
    C2,
    C1,
    Piecewise { boundaries: Vec<RationalTime> },
    Discrete,
}

impl TemporalContinuity {
    pub const fn rank(&self) -> u8 {
        match self {
            Self::C2 => 0,
            Self::C1 => 1,
            Self::Piecewise { .. } => 2,
            Self::Discrete => 3,
        }
    }

    pub fn is_track_legal(&self) -> bool {
        !matches!(self, Self::Discrete)
    }

    fn meet(self, other: Self) -> Self {
        if self.rank() >= other.rank() {
            self.merge_boundaries(other)
        } else {
            other.merge_boundaries(self)
        }
    }

    fn merge_boundaries(self, other: Self) -> Self {
        match (self, other) {
            (
                Self::Piecewise {
                    boundaries: mut left,
                },
                Self::Piecewise { boundaries: right },
            ) => {
                left.extend(right);
                left.sort();
                left.dedup();
                Self::Piecewise { boundaries: left }
            }
            (left, _) => left,
        }
    }
}

use serde::{Deserialize, Serialize};

/// Infer per-expression continuity. Unknown or frame-index time bases are Discrete.
pub fn infer_continuity(exprs: &[Expr]) -> Vec<TemporalContinuity> {
    let mut out = vec![TemporalContinuity::Discrete; exprs.len()];
    for (at, expr) in exprs.iter().enumerate() {
        out[at] = continuity_of(expr, &out);
    }
    out
}

fn continuity_of(expr: &Expr, prior: &[TemporalContinuity]) -> TemporalContinuity {
    let child = |id: ExprId| {
        prior
            .get(id.0 as usize)
            .cloned()
            .unwrap_or(TemporalContinuity::Discrete)
    };
    let fold = |ids: Vec<ExprId>| {
        ids.into_iter()
            .map(child)
            .reduce(TemporalContinuity::meet)
            .unwrap_or(TemporalContinuity::C2)
    };
    match expr {
        Expr::Const { .. } => TemporalContinuity::C2,
        Expr::Context { input } => context_continuity(*input),
        Expr::Prop { .. } => TemporalContinuity::C2,
        Expr::Cue { .. } => TemporalContinuity::Piecewise {
            boundaries: Vec::new(),
        },
        Expr::Spring { elapsed_frames, .. } => child(*elapsed_frames).meet(TemporalContinuity::C2),
        Expr::Interpolate {
            input,
            extrapolate_left,
            extrapolate_right,
            easings,
            ..
        } => interpolate_continuity(
            child(*input),
            *extrapolate_left,
            *extrapolate_right,
            easings,
        ),
        Expr::Select { .. }
        | Expr::Compare { .. }
        | Expr::Template { .. }
        | Expr::Noise1D { .. }
        | Expr::Noise2D { .. }
        | Expr::FormatNumber { .. }
        | Expr::PathTrajectory { .. } => TemporalContinuity::Discrete,
        Expr::NodeBounds { .. } | Expr::Project3D { .. } => TemporalContinuity::C1,
        other => fold(other.children()),
    }
}

fn context_continuity(input: ContextInput) -> TemporalContinuity {
    match input {
        ContextInput::LocalProgress | ContextInput::CompositionSeconds => TemporalContinuity::C2,
        ContextInput::DurationFrames
        | ContextInput::FpsNum
        | ContextInput::FpsDen
        | ContextInput::ViewportWidth
        | ContextInput::ViewportHeight => TemporalContinuity::C2,
        ContextInput::PhaseProgress { .. } | ContextInput::HoldCycleProgress => {
            TemporalContinuity::Piecewise {
                boundaries: Vec::new(),
            }
        }
        ContextInput::LocalFrame
        | ContextInput::PhaseFrame { .. }
        | ContextInput::PhaseDurationFrames { .. }
        | ContextInput::PhaseElapsedFrames { .. }
        | ContextInput::PhaseActive { .. }
        | ContextInput::HoldIteration
        | ContextInput::HoldCycleFrame
        | ContextInput::UnitIndex
        | ContextInput::UnitCount
        | ContextInput::UnitStart
        | ContextInput::UnitEnd => TemporalContinuity::Discrete,
    }
}

fn interpolate_continuity(
    input: TemporalContinuity,
    left: Extrapolation,
    right: Extrapolation,
    easings: &[MotionEasing],
) -> TemporalContinuity {
    if matches!(input, TemporalContinuity::Discrete) {
        return TemporalContinuity::Discrete;
    }
    let smooth = easings.iter().all(|easing| {
        matches!(
            easing,
            MotionEasing::Linear
                | MotionEasing::Ease
                | MotionEasing::EaseIn
                | MotionEasing::EaseOut
                | MotionEasing::EaseInOut
                | MotionEasing::CubicBezier { .. }
        )
    });
    if !smooth {
        return TemporalContinuity::Discrete;
    }
    let c1 = easings.iter().all(|easing| {
        matches!(
            easing,
            MotionEasing::Linear | MotionEasing::CubicBezier { .. }
        )
    });
    let piecewise = TemporalContinuity::Piecewise {
        boundaries: Vec::new(),
    };
    let joint = if c1 && left == Extrapolation::Extend && right == Extrapolation::Extend {
        TemporalContinuity::C1
    } else {
        piecewise
    };
    input.meet(joint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::{Expr, ExprId};
    use crate::value::MotionValue;

    #[test]
    fn constants_and_progress_are_c2_local_frame_is_discrete() {
        let exprs = vec![
            Expr::Const {
                value: MotionValue::Number(1.0),
            },
            Expr::Context {
                input: ContextInput::LocalProgress,
            },
            Expr::Context {
                input: ContextInput::LocalFrame,
            },
            Expr::Add {
                lhs: ExprId(0),
                rhs: ExprId(1),
            },
            Expr::Add {
                lhs: ExprId(1),
                rhs: ExprId(2),
            },
        ];
        let continuity = infer_continuity(&exprs);
        assert!(matches!(continuity[0], TemporalContinuity::C2));
        assert!(matches!(continuity[1], TemporalContinuity::C2));
        assert!(matches!(continuity[2], TemporalContinuity::Discrete));
        assert!(matches!(continuity[3], TemporalContinuity::C2));
        assert!(matches!(continuity[4], TemporalContinuity::Discrete));
        assert!(continuity[3].is_track_legal());
        assert!(!continuity[4].is_track_legal());
    }
}
