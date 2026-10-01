//! Closed Motion roles and exact template timing shared by every host.
use crate::{RationalTime, TimeError};
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionSourceMetadata {
    pub duration: RationalTime,
    pub role: MotionRole,
}

impl MotionRole {
    pub fn validate(
        self,
        source: RationalTime,
        host: Option<RationalTime>,
    ) -> Result<(), &'static str> {
        if !source.is_positive() {
            return Err("template duration must be positive");
        }
        if let Some((intro, outro, _)) = self.template_timing() {
            if intro.is_negative() || outro.is_negative() {
                return Err("template intro and outro must be non-negative");
            }
            let minimum = intro
                .checked_add(outro)
                .map_err(|_| "template timing overflows")?;
            if minimum > source {
                return Err("template intro + outro exceeds template duration");
            }
            if host.is_some_and(|host| host < minimum) {
                return Err("template host duration is shorter than intro + outro");
            }
        }
        Ok(())
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Clip => "clip",
            Self::Overlay { .. } => "overlay",
            Self::CaptionPresenter { .. } => "captionPresenter",
        }
    }

    fn template_timing(self) -> Option<(RationalTime, RationalTime, OverlayHold)> {
        match self {
            Self::Clip => None,
            Self::Overlay { intro, outro, hold } => Some((intro, outro, hold)),
            Self::CaptionPresenter { intro, outro } => Some((intro, outro, OverlayHold::Once)),
        }
    }

    /// Templates share one exact map. The logical right endpoint is handled by the source ABI.
    pub fn template_time(
        self,
        sample: RationalTime,
        host: RationalTime,
        source: RationalTime,
    ) -> Result<RationalTime, TimeError> {
        let Some((intro, outro, hold)) = self.template_timing() else {
            return Ok(sample);
        };
        let t = sample.max(RationalTime::ZERO).min(host);
        let exit = host.checked_sub(outro)?;
        if t < intro {
            return Ok(t);
        }
        if t >= exit {
            return source.checked_sub(outro)?.checked_add(t.checked_sub(exit)?);
        }
        let length = source.checked_sub(intro)?.checked_sub(outro)?;
        if length == RationalTime::ZERO {
            return Ok(intro);
        }
        let elapsed = t.checked_sub(intro)?;
        let offset = match hold {
            OverlayHold::Once => elapsed.min(length),
            OverlayHold::Loop => {
                let ratio = elapsed.checked_div(length)?.into_exact();
                let cycles = ratio.numerator().div_euclid(i64::from(ratio.denominator()));
                elapsed.checked_sub(length.checked_mul(RationalTime::new(cycles, 1)?)?)?
            }
            OverlayHold::Stretch => elapsed
                .checked_mul(length)?
                .checked_div(exit.checked_sub(intro)?)?,
        };
        intro.checked_add(offset)
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
    fn hold_modes_map_exactly_between_unchanged_intro_and_outro() {
        for hold in [OverlayHold::Once, OverlayHold::Loop, OverlayHold::Stretch] {
            let role = role(hold);
            for host in [t(1, 1), t(3, 2), t(2, 1), t(4, 1)] {
                role.validate(t(2, 1), Some(host)).unwrap();
                assert_eq!(
                    role.template_time(t(-1, 4), host, t(2, 1)).unwrap(),
                    t(0, 1)
                );
                assert_eq!(role.template_time(t(1, 4), host, t(2, 1)).unwrap(), t(1, 4));
                assert_eq!(
                    role.template_time(host.checked_sub(t(1, 4)).unwrap(), host, t(2, 1))
                        .unwrap(),
                    t(7, 4)
                );
                assert_eq!(role.template_time(host, host, t(2, 1)).unwrap(), t(2, 1));
            }
        }
        assert_eq!(
            role(OverlayHold::Once)
                .template_time(t(5, 2), t(4, 1), t(2, 1))
                .unwrap(),
            t(3, 2)
        );
        assert_eq!(
            role(OverlayHold::Loop)
                .template_time(t(5, 2), t(4, 1), t(2, 1))
                .unwrap(),
            t(1, 2)
        );
        assert_eq!(
            role(OverlayHold::Stretch)
                .template_time(t(5, 2), t(4, 1), t(2, 1))
                .unwrap(),
            t(7, 6)
        );
        assert_eq!(
            role(OverlayHold::Stretch)
                .template_time(t(3, 4), t(3, 2), t(2, 1))
                .unwrap(),
            t(1, 1)
        );
    }
    #[test]
    fn empty_segments_and_fractional_frames_need_no_epsilon() {
        for hold in [OverlayHold::Once, OverlayHold::Loop, OverlayHold::Stretch] {
            let no_hold = MotionRole::Overlay {
                intro: t(1, 2),
                outro: t(1, 2),
                hold,
            };
            assert_eq!(
                no_hold.template_time(t(3, 2), t(3, 1), t(1, 1)).unwrap(),
                t(1, 2)
            );
            assert_eq!(
                no_hold.template_time(t(1, 2), t(1, 1), t(1, 1)).unwrap(),
                t(1, 2)
            );
            let no_edges = MotionRole::Overlay {
                intro: t(0, 1),
                outro: t(0, 1),
                hold,
            };
            assert_eq!(
                no_edges.template_time(t(4, 1), t(4, 1), t(2, 1)).unwrap(),
                t(2, 1)
            );
            assert_eq!(
                role(hold)
                    .template_time(t(299 * 1001, 30000), t(10, 1), t(2, 1))
                    .unwrap(),
                t(59299, 30000)
            );
            let expected = if hold == OverlayHold::Stretch {
                t(139019, 270000)
            } else {
                t(19019, 30000)
            };
            assert_eq!(
                role(hold)
                    .template_time(t(19 * 1001, 30000), t(10, 1), t(2, 1))
                    .unwrap(),
                expected
            );
        }
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
