//! Deterministic external signals for Motion JSX. Timeline resolves cues to half-open frame
//! windows; sample the complete CueState from the window, local frame, and frame rate without
//! mutable state or a clock.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use valle_timeline::FrameRate;

use crate::{ControlsSchema, PhaseSpec, motion_context_at, phase_windows};

/// A resolved right-open cue window in clip-local frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CueWindow {
    pub start_frame: u32,
    pub end_frame: u32,
    #[serde(default)]
    pub enter_frames: u32,
    #[serde(default)]
    pub exit_frames: u32,
}

impl CueWindow {
    pub fn duration_frames(self) -> u32 {
        self.end_frame.saturating_sub(self.start_frame)
    }

    fn validate(self, name: &str) -> Result<(), CueError> {
        if self.end_frame <= self.start_frame {
            return Err(CueError::InvalidWindow {
                name: name.to_owned(),
                start_frame: self.start_frame,
                end_frame: self.end_frame,
            });
        }
        Ok(())
    }
}

/// Frame-pure state exposed as `signals.<name>.*`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CueState {
    pub active: bool,
    pub progress: f64,
    pub enter: f64,
    pub hold: f64,
    pub exit: f64,
    pub local_frame: u32,
}

impl CueState {
    pub const INACTIVE: CueState = CueState {
        active: false,
        progress: 0.0,
        enter: 0.0,
        hold: 0.0,
        exit: 0.0,
        local_frame: 0,
    };
}

/// Validated cue bindings for one Motion clip.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CueSchedule(BTreeMap<String, Option<CueWindow>>);

impl CueSchedule {
    /// Match Timeline bindings against the component schema once, before rendering any frame.
    pub fn resolve(
        controls: &ControlsSchema,
        bindings: &BTreeMap<String, CueWindow>,
    ) -> Result<Self, CueError> {
        for name in bindings.keys() {
            if !controls.cues.contains_key(name) {
                return Err(CueError::UnknownCue { name: name.clone() });
            }
        }

        let mut schedule = BTreeMap::new();
        for (name, control) in &controls.cues {
            let binding = bindings.get(name).copied();
            if control.required && binding.is_none() {
                return Err(CueError::MissingCue { name: name.clone() });
            }
            if let Some(window) = binding {
                window.validate(name)?;
            }
            schedule.insert(name.clone(), binding);
        }
        Ok(CueSchedule(schedule))
    }

    /// Sample all declared cues at one clip-local frame.
    pub fn sample(&self, local_frame: u32, fps: FrameRate) -> ResolvedSignals {
        let cues = self
            .0
            .iter()
            .map(|(name, window)| {
                let state = window.map_or(CueState::INACTIVE, |window| {
                    cue_state_at(window, local_frame, fps)
                });
                (name.clone(), state)
            })
            .collect();
        ResolvedSignals { cues }
    }
}

/// All external scalar signals for one frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedSignals {
    cues: BTreeMap<String, CueState>,
}

impl ResolvedSignals {
    pub fn cue(&self, name: &str) -> Option<&CueState> {
        self.cues.get(name)
    }

    pub fn cues(&self) -> &BTreeMap<String, CueState> {
        &self.cues
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CueError {
    UnknownCue {
        name: String,
    },
    MissingCue {
        name: String,
    },
    InvalidWindow {
        name: String,
        start_frame: u32,
        end_frame: u32,
    },
}

impl core::fmt::Display for CueError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CueError::UnknownCue { name } => write!(f, "unknown cue `{name}`"),
            CueError::MissingCue { name } => write!(f, "required cue `{name}` is not bound"),
            CueError::InvalidWindow {
                name,
                start_frame,
                end_frame,
            } => write!(
                f,
                "cue `{name}` has invalid right-open window [{start_frame}, {end_frame})"
            ),
        }
    }
}

impl std::error::Error for CueError {}

/// Sample a cue using the same proportional enter/hold/exit compression as the clip phases.
fn cue_state_at(window: CueWindow, frame: u32, fps: FrameRate) -> CueState {
    let duration = window.duration_frames();
    if frame < window.start_frame {
        return CueState::INACTIVE;
    }
    if frame >= window.end_frame {
        return CueState {
            active: false,
            progress: 1.0,
            enter: 1.0,
            hold: 1.0,
            exit: 1.0,
            local_frame: duration,
        };
    }

    let local_frame = frame - window.start_frame;
    let layout = phase_windows(
        &PhaseSpec {
            enter_frames: window.enter_frames,
            exit_frames: window.exit_frames,
            hold_cycle_frames: None,
        },
        duration,
    );
    let context = motion_context_at(local_frame, &layout, fps)
        .expect("validated non-empty cue and an in-range local frame");
    CueState {
        active: true,
        progress: f64::from(local_frame) / f64::from(duration),
        enter: context.enter.progress,
        hold: context.hold.progress,
        exit: context.exit.progress,
        local_frame,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CameraControls, CueControl, CueKind, FrameControl, OptionalFrameControl, TimingControls,
    };

    fn controls(required: bool) -> ControlsSchema {
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
            cues: BTreeMap::from([(
                "filter".into(),
                CueControl {
                    kind: CueKind::Span,
                    required,
                },
            )]),
            assets: BTreeMap::new(),
            camera: CameraControls::default(),
        }
    }

    #[test]
    fn cue_state_is_right_open_and_clamps_outside() {
        let window = CueWindow {
            start_frame: 10,
            end_frame: 20,
            enter_frames: 2,
            exit_frames: 2,
        };
        let fps = FrameRate::new(30, 1).unwrap();
        assert_eq!(cue_state_at(window, 9, fps), CueState::INACTIVE);
        let first = cue_state_at(window, 10, fps);
        assert!(first.active);
        assert_eq!(first.progress, 0.0);
        assert_eq!(first.enter, 0.0);
        let last = cue_state_at(window, 19, fps);
        assert!(last.active);
        assert_eq!(last.progress, 0.9);
        assert_eq!(last.exit, 0.5);
        let after = cue_state_at(window, 20, fps);
        assert!(!after.active);
        assert_eq!(after.progress, 1.0);
        assert_eq!(after.local_frame, 10);
    }

    #[test]
    fn schedule_rejects_unknown_missing_and_empty_bindings() {
        let mut bindings = BTreeMap::new();
        assert!(matches!(
            CueSchedule::resolve(&controls(true), &bindings),
            Err(CueError::MissingCue { .. })
        ));
        bindings.insert(
            "other".into(),
            CueWindow {
                start_frame: 0,
                end_frame: 1,
                enter_frames: 0,
                exit_frames: 0,
            },
        );
        assert!(matches!(
            CueSchedule::resolve(&controls(false), &bindings),
            Err(CueError::UnknownCue { .. })
        ));
        bindings.clear();
        bindings.insert(
            "filter".into(),
            CueWindow {
                start_frame: 4,
                end_frame: 4,
                enter_frames: 0,
                exit_frames: 0,
            },
        );
        assert!(matches!(
            CueSchedule::resolve(&controls(false), &bindings),
            Err(CueError::InvalidWindow { .. })
        ));
    }
}
