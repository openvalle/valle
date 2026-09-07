//! Frame schedules derived only from an admitted fixed render canvas.

use thiserror::Error;
use valle_engine::render::{CanvasClockError, CompiledCanvas, FrameKey};

#[derive(Debug, Clone, PartialEq)]
pub enum FrameSchedule {
    All,
    Single { at_seconds: f64 },
    Range { from_seconds: f64, to_seconds: f64 },
    Sparse { times_seconds: Vec<f64> },
}

impl FrameSchedule {
    pub fn frames(&self, canvas: CompiledCanvas) -> Result<Vec<FrameKey>, FrameScheduleError> {
        let total = canvas.frame_count();
        if total <= 0 {
            return Ok(Vec::new());
        }
        let mut frames = match self {
            Self::All => (0..total).collect(),
            Self::Single { at_seconds } => vec![canvas.frame_at_seconds(*at_seconds)?.index()],
            Self::Range {
                from_seconds,
                to_seconds,
            } => {
                if to_seconds <= from_seconds {
                    return Err(FrameScheduleError::InvalidRange {
                        from_seconds: *from_seconds,
                        to_seconds: *to_seconds,
                    });
                }
                let first = canvas.frame_boundary_at_seconds(*from_seconds)?;
                let end = canvas.frame_boundary_at_seconds(*to_seconds)?;
                if first >= total || end <= first {
                    return Err(FrameScheduleError::EmptyRange {
                        start_frame: first,
                        end_frame: end,
                    });
                }
                (first..end).collect()
            }
            Self::Sparse { times_seconds } => times_seconds
                .iter()
                .map(|time| canvas.frame_at_seconds(*time).map(FrameKey::index))
                .collect::<Result<Vec<_>, _>>()?,
        };
        frames.sort_unstable();
        frames.dedup();
        Ok(frames.into_iter().map(FrameKey::new).collect())
    }
}

#[derive(Debug, Error)]
pub enum FrameScheduleError {
    #[error(transparent)]
    Clock(#[from] CanvasClockError),
    #[error("frame range must be finite and increasing, got [{from_seconds}, {to_seconds})")]
    InvalidRange { from_seconds: f64, to_seconds: f64 },
    #[error("frame range quantized to an empty interval [{start_frame}, {end_frame})")]
    EmptyRange { start_frame: i64, end_frame: i64 },
}
