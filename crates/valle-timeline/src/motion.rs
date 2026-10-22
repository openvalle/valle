//! Closed Motion roles and the template timing every host validates; `ClipClock` maps it.
use crate::RationalTime;
use serde::{Deserialize, Serialize};
mod caption;
pub use caption::*;

pub const MAX_PREPARE_DATA_DEPTH: usize = 16;
pub const MAX_PREPARE_DATA_ARRAY_ITEMS: usize = 10_000;
pub const MAX_PREPARE_DATA_TOTAL_ITEMS: usize = 50_000;
pub const MAX_PREPARE_DATA_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum OverlayHold {
    Once,
    Loop,
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum MotionRole {
    Clip,
    CaptionPresenter {
        intro: RationalTime,
        outro: RationalTime,
    },
    Overlay {
        intro: RationalTime,
        outro: RationalTime,
        hold: OverlayHold,
    },
    /// A track transition: the template stretches over the window and reads the outgoing and
    /// incoming clips as `input://from` and `input://to`.
    Transition,
    /// An effect on a clip or an adjustment: the template stretches over its host's window,
    /// reads the picture it applies to as `input://source` and replaces it.
    Effect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionSourceMetadata {
    pub duration: RationalTime,
    pub role: MotionRole,
}

impl MotionRole {
    pub fn validate(self, source: RationalTime, host: Option<RationalTime>) -> Result<(), String> {
        if !source.is_positive() {
            return Err("template duration must be positive".into());
        }
        if let Some((intro, outro, _)) = self.template_timing() {
            if intro.is_negative() || outro.is_negative() {
                return Err("template intro and outro must be non-negative".into());
            }
            let minimum = intro
                .checked_add(outro)
                .map_err(|_| "template timing overflows")?;
            if minimum > source {
                return Err(format!(
                    "template intro {} s + outro {} s = {} s exceeds template duration {} s",
                    intro.as_f64(),
                    outro.as_f64(),
                    minimum.as_f64(),
                    source.as_f64()
                ));
            }
            if let Some(host) = host.filter(|host| *host < minimum) {
                return Err(format!(
                    "template host duration {} s is shorter than intro {} s + outro {} s = {} s (template duration {} s)",
                    host.as_f64(),
                    intro.as_f64(),
                    outro.as_f64(),
                    minimum.as_f64(),
                    source.as_f64()
                ));
            }
        }
        Ok(())
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Clip => "clip",
            Self::Overlay { .. } => "overlay",
            Self::CaptionPresenter { .. } => "captionPresenter",
            Self::Transition => "transition",
            Self::Effect => "effect",
        }
    }

    pub(crate) fn template_timing(self) -> Option<(RationalTime, RationalTime, OverlayHold)> {
        match self {
            Self::Clip => None,
            Self::Overlay { intro, outro, hold } => Some((intro, outro, hold)),
            Self::CaptionPresenter { intro, outro } => Some((intro, outro, OverlayHold::Once)),
            Self::Transition | Self::Effect => {
                Some((RationalTime::ZERO, RationalTime::ZERO, OverlayHold::Stretch))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(n: i64, d: u32) -> RationalTime {
        RationalTime::new(n, d).unwrap()
    }
    fn role(hold: OverlayHold) -> MotionRole {
        MotionRole::Overlay {
            intro: t(1, 2),
            outro: t(1, 2),
            hold,
        }
    }
    #[test]
    fn a_template_fits_its_intro_and_outro_into_the_source_and_the_host() {
        for hold in [OverlayHold::Once, OverlayHold::Loop, OverlayHold::Stretch] {
            for host in [t(1, 1), t(3, 2), t(2, 1), t(4, 1)] {
                role(hold).validate(t(2, 1), Some(host)).unwrap();
            }
        }
        MotionRole::Transition
            .validate(t(1, 1), Some(t(1, 2)))
            .unwrap();
        assert!(
            role(OverlayHold::Once)
                .validate(t(2, 1), Some(t(999, 1000)))
                .is_err()
        );
        assert!(
            role(OverlayHold::Once)
                .validate(t(999, 1000), None)
                .is_err()
        );
        assert!(
            MotionRole::Overlay {
                intro: t(-1, 2),
                outro: t(0, 1),
                hold: OverlayHold::Once
            }
            .validate(t(2, 1), None)
            .is_err()
        );
    }
}
