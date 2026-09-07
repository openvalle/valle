use std::{cmp::Ordering, fmt, num::NonZeroU32};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

/// The single exact rational algebra used by Timeline wire values.
///
/// Values constructed in Rust are reduced eagerly. Canonical wire decoding is stricter: it only
/// accepts an already-reduced `"num/den"` string, so aliases can never enter a canonical domain
/// value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExactRational {
    numerator: i64,
    denominator: NonZeroU32,
}

/// Exact composition seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RationalTime(ExactRational);

/// Exact source seconds per composition second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RationalRate(ExactRational);

/// Exact frames per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameRate(ExactRational);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TimeError {
    #[error("exact rational denominator must be non-zero")]
    ZeroDenominator,
    #[error("exact rational arithmetic overflow")]
    Overflow,
    #[error("cannot divide an exact rational by zero")]
    DivisionByZero,
    #[error("exact rational wire must be a canonical num/den string")]
    InvalidCanonicalRational,
    #[error("exact rational wire must already be reduced")]
    NonCanonicalRational,
    #[error("rational rate must be positive")]
    InvalidRate,
    #[error("frame rate must be positive")]
    InvalidFrameRate,
    #[error("sample rate must be positive")]
    InvalidSampleRate,
    #[error("identity string must be a non-empty string of at most 128 bytes")]
    InvalidIdentity,
}

impl ExactRational {
    pub const ZERO: Self = Self {
        numerator: 0,
        denominator: NonZeroU32::MIN,
    };

    pub const ONE: Self = Self {
        numerator: 1,
        denominator: NonZeroU32::MIN,
    };

    /// Constructs a value and reduces it to its unique representation.
    pub fn new(numerator: i64, denominator: u32) -> Result<Self, TimeError> {
        if denominator == 0 {
            return Err(TimeError::ZeroDenominator);
        }
        Self::from_wide(numerator as i128, denominator as u128)
    }

    /// Parses an already-canonical wire value.
    ///
    /// This intentionally does not normalize input. Leading signs/zeroes, whitespace, `-0`, and
    /// reducible aliases are rejected rather than silently acquiring a different identity.
    pub fn parse_canonical(wire: &str) -> Result<Self, TimeError> {
        let (numerator_wire, denominator_wire) = wire
            .split_once('/')
            .ok_or(TimeError::InvalidCanonicalRational)?;
        if denominator_wire.contains('/')
            || !is_canonical_numerator(numerator_wire)
            || !is_canonical_denominator(denominator_wire)
        {
            return Err(TimeError::InvalidCanonicalRational);
        }

        let numerator = numerator_wire
            .parse::<i64>()
            .map_err(|_| TimeError::InvalidCanonicalRational)?;
        let denominator = denominator_wire
            .parse::<u32>()
            .map_err(|_| TimeError::InvalidCanonicalRational)?;
        let denominator =
            NonZeroU32::new(denominator).ok_or(TimeError::InvalidCanonicalRational)?;

        if gcd(numerator.unsigned_abs() as u128, denominator.get() as u128) != 1 {
            return Err(TimeError::NonCanonicalRational);
        }

        Ok(Self {
            numerator,
            denominator,
        })
    }

    pub const fn numerator(self) -> i64 {
        self.numerator
    }

    pub const fn denominator(self) -> u32 {
        self.denominator.get()
    }

    /// Lossy convenience for timeline/UI boundaries. Canonical arithmetic and quantization must
    /// use the checked integer operations instead.
    pub fn as_f64(self) -> f64 {
        self.numerator as f64 / self.denominator.get() as f64
    }

    pub const fn is_negative(self) -> bool {
        self.numerator < 0
    }

    pub const fn is_non_positive(self) -> bool {
        self.numerator <= 0
    }

    pub const fn is_positive(self) -> bool {
        self.numerator > 0
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, TimeError> {
        let left = (self.numerator as i128)
            .checked_mul(rhs.denominator() as i128)
            .ok_or(TimeError::Overflow)?;
        let right = (rhs.numerator as i128)
            .checked_mul(self.denominator() as i128)
            .ok_or(TimeError::Overflow)?;
        let numerator = left.checked_add(right).ok_or(TimeError::Overflow)?;
        let denominator = (self.denominator() as u128)
            .checked_mul(rhs.denominator() as u128)
            .ok_or(TimeError::Overflow)?;
        Self::from_wide(numerator, denominator)
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, TimeError> {
        let left = (self.numerator as i128)
            .checked_mul(rhs.denominator() as i128)
            .ok_or(TimeError::Overflow)?;
        let right = (rhs.numerator as i128)
            .checked_mul(self.denominator() as i128)
            .ok_or(TimeError::Overflow)?;
        let numerator = left.checked_sub(right).ok_or(TimeError::Overflow)?;
        let denominator = (self.denominator() as u128)
            .checked_mul(rhs.denominator() as u128)
            .ok_or(TimeError::Overflow)?;
        Self::from_wide(numerator, denominator)
    }

    pub fn checked_mul(self, rhs: Self) -> Result<Self, TimeError> {
        let numerator = (self.numerator as i128)
            .checked_mul(rhs.numerator as i128)
            .ok_or(TimeError::Overflow)?;
        let denominator = (self.denominator() as u128)
            .checked_mul(rhs.denominator() as u128)
            .ok_or(TimeError::Overflow)?;
        Self::from_wide(numerator, denominator)
    }

    pub fn checked_div(self, rhs: Self) -> Result<Self, TimeError> {
        if rhs.numerator == 0 {
            return Err(TimeError::DivisionByZero);
        }
        let numerator = (self.numerator as i128)
            .checked_mul(rhs.denominator() as i128)
            .ok_or(TimeError::Overflow)?;
        let denominator = (self.denominator() as u128)
            .checked_mul(rhs.numerator.unsigned_abs() as u128)
            .ok_or(TimeError::Overflow)?;
        let signed_numerator = if rhs.numerator < 0 {
            numerator.checked_neg().ok_or(TimeError::Overflow)?
        } else {
            numerator
        };
        Self::from_wide(signed_numerator, denominator)
    }

    pub fn checked_neg(self) -> Result<Self, TimeError> {
        Self::from_wide(-(self.numerator as i128), self.denominator() as u128)
    }

    fn from_wide(numerator: i128, denominator: u128) -> Result<Self, TimeError> {
        if denominator == 0 {
            return Err(TimeError::ZeroDenominator);
        }
        let divisor = gcd(numerator.unsigned_abs(), denominator);
        let divisor_i128 = i128::try_from(divisor).map_err(|_| TimeError::Overflow)?;
        let reduced_numerator = numerator / divisor_i128;
        let reduced_denominator = denominator / divisor;
        let numerator = i64::try_from(reduced_numerator).map_err(|_| TimeError::Overflow)?;
        let denominator = u32::try_from(reduced_denominator).map_err(|_| TimeError::Overflow)?;
        Ok(Self {
            numerator,
            denominator: NonZeroU32::new(denominator).ok_or(TimeError::ZeroDenominator)?,
        })
    }
}

impl RationalTime {
    pub const ZERO: Self = Self(ExactRational::ZERO);
    pub const ONE: Self = Self(ExactRational::ONE);

    pub fn new(numerator: i64, denominator: u32) -> Result<Self, TimeError> {
        ExactRational::new(numerator, denominator).map(Self)
    }

    pub const fn from_exact(exact: ExactRational) -> Self {
        Self(exact)
    }

    pub const fn into_exact(self) -> ExactRational {
        self.0
    }

    pub const fn numerator(self) -> i64 {
        self.0.numerator()
    }

    pub const fn denominator(self) -> u32 {
        self.0.denominator()
    }

    pub fn as_f64(self) -> f64 {
        self.0.as_f64()
    }

    pub const fn is_negative(self) -> bool {
        self.0.is_negative()
    }

    pub const fn is_non_positive(self) -> bool {
        self.0.is_non_positive()
    }

    pub const fn is_positive(self) -> bool {
        self.0.is_positive()
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, TimeError> {
        self.0.checked_add(rhs.0).map(Self)
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, TimeError> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    pub fn checked_mul(self, rhs: Self) -> Result<Self, TimeError> {
        self.0.checked_mul(rhs.0).map(Self)
    }

    /// Scales composition/source time by a dimensionless playback rate.
    ///
    /// Keeping the rate type distinct prevents accidentally multiplying two
    /// time values when mapping between clock domains.
    pub fn checked_scale(self, rate: RationalRate) -> Result<Self, TimeError> {
        self.0.checked_mul(rate.0).map(Self)
    }

    pub fn checked_div(self, rhs: Self) -> Result<Self, TimeError> {
        self.0.checked_div(rhs.0).map(Self)
    }

    pub fn checked_neg(self) -> Result<Self, TimeError> {
        self.0.checked_neg().map(Self)
    }
}

impl RationalRate {
    pub const ONE: Self = Self(ExactRational::ONE);

    pub fn new(numerator: i64, denominator: u32) -> Result<Self, TimeError> {
        Self::from_exact(ExactRational::new(numerator, denominator)?)
    }

    pub fn from_exact(exact: ExactRational) -> Result<Self, TimeError> {
        if !exact.is_positive() {
            return Err(TimeError::InvalidRate);
        }
        Ok(Self(exact))
    }

    pub const fn into_exact(self) -> ExactRational {
        self.0
    }

    pub const fn numerator(self) -> i64 {
        self.0.numerator()
    }

    pub const fn denominator(self) -> u32 {
        self.0.denominator()
    }
}

impl FrameRate {
    pub fn new(numerator: i64, denominator: u32) -> Result<Self, TimeError> {
        Self::from_exact(ExactRational::new(numerator, denominator)?)
    }

    pub fn from_exact(exact: ExactRational) -> Result<Self, TimeError> {
        if !exact.is_positive() {
            return Err(TimeError::InvalidFrameRate);
        }
        Ok(Self(exact))
    }

    pub const fn into_exact(self) -> ExactRational {
        self.0
    }

    pub const fn numerator(self) -> i64 {
        self.0.numerator()
    }

    pub const fn denominator(self) -> u32 {
        self.0.denominator()
    }
}

fn is_canonical_numerator(value: &str) -> bool {
    if value == "0" {
        return true;
    }
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty()
        && !digits.starts_with('0')
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && !value.starts_with('+')
}

fn is_canonical_denominator(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('0') && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a
}

impl Ord for ExactRational {
    fn cmp(&self, other: &Self) -> Ordering {
        let left = self.numerator as i128 * other.denominator() as i128;
        let right = other.numerator as i128 * self.denominator() as i128;
        left.cmp(&right)
    }
}

impl PartialOrd for ExactRational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

macro_rules! impl_wrapper_traits {
    ($type:ty) => {
        impl Ord for $type {
            fn cmp(&self, other: &Self) -> Ordering {
                self.0.cmp(&other.0)
            }
        }

        impl PartialOrd for $type {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        impl fmt::Display for $type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl Serialize for $type {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.0.serialize(serializer)
            }
        }
    };
}

impl_wrapper_traits!(RationalTime);
impl_wrapper_traits!(RationalRate);
impl_wrapper_traits!(FrameRate);

impl fmt::Display for ExactRational {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.numerator, self.denominator())
    }
}

impl Serialize for ExactRational {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ExactRational {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = String::deserialize(deserializer)?;
        Self::parse_canonical(&wire).map_err(de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for RationalTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ExactRational::deserialize(deserializer).map(Self)
    }
}

impl<'de> Deserialize<'de> for RationalRate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let exact = ExactRational::deserialize(deserializer)?;
        Self::from_exact(exact).map_err(de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for FrameRate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let exact = ExactRational::deserialize(deserializer)?;
        Self::from_exact(exact).map_err(de::Error::custom)
    }
}

impl From<RationalTime> for ExactRational {
    fn from(value: RationalTime) -> Self {
        value.0
    }
}

impl From<RationalRate> for ExactRational {
    fn from(value: RationalRate) -> Self {
        value.0
    }
}

impl From<FrameRate> for ExactRational {
    fn from(value: FrameRate) -> Self {
        value.0
    }
}
