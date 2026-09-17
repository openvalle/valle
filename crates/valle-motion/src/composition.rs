//! Motion's authored canvas and source duration. Output frame rate and pixel extent are host inputs.

use serde::{Deserialize, Serialize};
use valle_timeline::internal::quantize::quantize_frame_boundary;
use valle_timeline::time::{ExactRational, FrameRate, RationalTime, TimeError};

use crate::artifact::ValidationError;
use crate::domain::MotionViewport;

pub const DEFAULT_DPR: f32 = 1.0;
pub const ROOT_FONT_SIZE: f32 = 16.0;
pub const MAX_DURATION_FRAMES: u32 = 10_000_000;
pub const COMPOSITION_TEMPLATE: &str =
    "export const composition = { width: 1920, height: 1080, duration: 5 };";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Composition {
    pub width: u32,
    pub height: u32,
    /// Optional default for standalone preview and export. Timeline supplies its own frame rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<String>,
    /// Positive seconds stored as a canonical exact rational after public q6 normalization.
    pub duration: String,
}

impl Composition {
    pub fn new(width: u32, height: u32, fps: Option<FrameRate>, duration: RationalTime) -> Self {
        Self {
            width,
            height,
            fps: fps.map(|value| value.into_exact().to_string()),
            duration: duration.into_exact().to_string(),
        }
    }

    pub fn frame_rate(&self) -> Result<Option<FrameRate>, TimeError> {
        self.fps
            .as_deref()
            .map(|value| ExactRational::parse_canonical(value).and_then(FrameRate::from_exact))
            .transpose()
    }

    pub const fn viewport(&self) -> MotionViewport {
        MotionViewport::new(self.width, self.height)
    }

    pub fn duration(&self) -> Result<RationalTime, TimeError> {
        ExactRational::parse_canonical(&self.duration).map(RationalTime::from_exact)
    }

    pub fn duration_frames(&self, fps: FrameRate) -> Result<u32, TimeError> {
        let frames = quantize_frame_boundary(self.duration()?, fps)?;
        u32::try_from(frames).map_err(|_| TimeError::Overflow)
    }

    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        if !self.viewport().is_valid_pixel_extent() {
            errors.push(ValidationError::new(
                "/composition/width",
                "canvas dimensions must be positive and inside the renderer's pixel domain",
            ));
        }
        if let Err(error) = self.frame_rate() {
            errors.push(ValidationError::new(
                "/composition/fps",
                format!("frame rate must be a positive exact value: {error}"),
            ));
        }
        match self.duration() {
            Ok(duration) if duration > RationalTime::ZERO => {}
            _ => errors.push(ValidationError::new(
                "/composition/duration",
                "duration must be a positive exact number of seconds",
            )),
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl Default for Composition {
    /// Fixture-only value; authoring and rendering never fill missing metadata from it.
    fn default() -> Self {
        Self::new(
            1920,
            1080,
            Some(FrameRate::new(30, 1).expect("valid fps")),
            RationalTime::new(5, 1).expect("valid duration"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_quantizes_at_actual_output_rate() {
        let composition = Composition::new(1920, 1080, None, RationalTime::new(23, 5).unwrap());
        composition.validate().unwrap();
        assert_eq!(
            composition
                .duration_frames(FrameRate::new(24, 1).unwrap())
                .unwrap(),
            110
        );
        assert_eq!(
            composition
                .duration_frames(FrameRate::new(25, 1).unwrap())
                .unwrap(),
            115
        );
    }

    #[test]
    fn removed_author_fields_are_rejected() {
        let json = r#"{"width":320,"height":180,"duration":"5/1","durationInFrames":150}"#;
        assert!(serde_json::from_str::<Composition>(json).is_err());
    }
}
