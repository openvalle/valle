use serde::{Deserialize, Serialize};

use super::{FrameRate, RationalTime, TimeError};

/// Discrete output-frame request. It is an anchor, not a temporal cache identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameKey {
    index: i64,
}

impl FrameKey {
    pub const fn new(index: i64) -> Self {
        Self { index }
    }

    pub const fn index(self) -> i64 {
        self.index
    }
}

/// Unique absolute composition time used by temporal dependencies and caches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleTime {
    composition: RationalTime,
}

impl SampleTime {
    pub const ZERO: Self = Self {
        composition: RationalTime::ZERO,
    };

    pub const fn new(composition: RationalTime) -> Self {
        Self { composition }
    }

    pub fn from_frame(frame: FrameKey, frame_rate: FrameRate) -> Result<Self, TimeError> {
        let numerator = (frame.index as i128)
            .checked_mul(frame_rate.denominator() as i128)
            .ok_or(TimeError::Overflow)?;
        Ok(Self::new(rational_time_from_wide(
            numerator,
            frame_rate.numerator() as u128,
        )?))
    }

    pub const fn composition(self) -> RationalTime {
        self.composition
    }

    pub fn checked_offset(self, offset: RationalTime) -> Result<Self, TimeError> {
        Ok(Self::new(self.composition.checked_add(offset)?))
    }
}

fn rational_time_from_wide(numerator: i128, denominator: u128) -> Result<RationalTime, TimeError> {
    if denominator == 0 {
        return Err(TimeError::ZeroDenominator);
    }
    let divisor = gcd(numerator.unsigned_abs(), denominator);
    let divisor_i128 = i128::try_from(divisor).map_err(|_| TimeError::Overflow)?;
    let numerator = i64::try_from(numerator / divisor_i128).map_err(|_| TimeError::Overflow)?;
    let denominator = u32::try_from(denominator / divisor).map_err(|_| TimeError::Overflow)?;
    RationalTime::new(numerator, denominator)
}

fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}
