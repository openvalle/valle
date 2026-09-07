use serde::{Deserialize, Serialize};
use thiserror::Error;

/// RGB chromaticities. Every currently supported set uses the D65 white point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorPrimaries {
    Rec709,
    DisplayP3,
    Rec2020,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferFunction {
    Linear,
    Srgb,
    Rec709,
    Pq,
    Hlg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MatrixCoefficients {
    Identity,
    Bt601,
    Bt709,
    Bt2020Ncl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorRange {
    Full,
    Limited,
}

/// A complete interpretation of encoded color samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColorDescription {
    pub primaries: ColorPrimaries,
    pub transfer: TransferFunction,
    pub matrix: MatrixCoefficients,
    pub range: ColorRange,
}

impl ColorDescription {
    pub const SRGB: Self = Self {
        primaries: ColorPrimaries::Rec709,
        transfer: TransferFunction::Srgb,
        matrix: MatrixCoefficients::Identity,
        range: ColorRange::Full,
    };

    pub const LINEAR_REC2020: Self = Self {
        primaries: ColorPrimaries::Rec2020,
        transfer: TransferFunction::Linear,
        matrix: MatrixCoefficients::Identity,
        range: ColorRange::Full,
    };
}

/// Alpha meaning at an external/source boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InputAlphaMode {
    Opaque,
    StraightCoverage,
    PremultipliedCoverage,
}

/// Engine's single legal intermediate alpha representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkingAlphaMode {
    PremultipliedCoverage,
}

/// Alpha representation after the terminal output pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputAlphaMode {
    Opaque,
    StraightCoverage,
    PremultipliedCoverage,
}

/// Domain in which an operator's RGB math is defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "transfer", rename_all = "camelCase")]
pub enum OperatorColorDomain {
    WorkingLinear,
    PerceptualSrgb,
    AssetEncoded(TransferFunction),
}

/// How a runtime operator is allowed to transform coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperatorAlphaBehavior {
    PreservesCoverage,
    RewritesCoverage,
    FiltersPremultiplied,
}

/// Tagged author color. It is decoded from sRGB and converted exactly once during prepare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthorSrgbStraight(pub [u8; 4]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct LuminanceNits(u16);

impl LuminanceNits {
    pub fn new(nits: u16) -> Result<Self, LuminanceError> {
        if nits == 0 {
            Err(LuminanceError::Zero)
        } else {
            Ok(Self(nits))
        }
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for LuminanceNits {
    type Error = LuminanceError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<LuminanceNits> for u16 {
    fn from(value: LuminanceNits) -> Self {
        value.0
    }
}

/// Absolute signal anchors frozen with input metadata or an output delivery spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "SignalLuminanceWire", into = "SignalLuminanceWire")]
pub struct SignalLuminance {
    reference_white: LuminanceNits,
    peak: LuminanceNits,
}

impl SignalLuminance {
    pub const SDR_100: Self = Self {
        reference_white: LuminanceNits(100),
        peak: LuminanceNits(100),
    };

    pub fn new(
        reference_white: LuminanceNits,
        peak: LuminanceNits,
    ) -> Result<Self, LuminanceError> {
        if peak.get() < reference_white.get() {
            return Err(LuminanceError::PeakBelowReferenceWhite);
        }
        Ok(Self {
            reference_white,
            peak,
        })
    }

    pub const fn reference_white(self) -> LuminanceNits {
        self.reference_white
    }

    pub const fn peak(self) -> LuminanceNits {
        self.peak
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignalLuminanceWire {
    reference_white: LuminanceNits,
    peak: LuminanceNits,
}

impl TryFrom<SignalLuminanceWire> for SignalLuminance {
    type Error = LuminanceError;

    fn try_from(value: SignalLuminanceWire) -> Result<Self, Self::Error> {
        Self::new(value.reference_white, value.peak)
    }
}

impl From<SignalLuminance> for SignalLuminanceWire {
    fn from(value: SignalLuminance) -> Self {
        Self {
            reference_white: value.reference_white,
            peak: value.peak,
        }
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum LuminanceError {
    #[error("luminance must be positive")]
    Zero,
    #[error("peak luminance must be greater than or equal to reference white")]
    PeakBelowReferenceWhite,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_luminance_wire_cannot_invert_white_and_peak() {
        assert!(
            SignalLuminance::new(
                LuminanceNits::new(203).unwrap(),
                LuminanceNits::new(100).unwrap()
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<SignalLuminance>(r#"{"referenceWhite":203,"peak":100}"#)
                .is_err()
        );
    }
}
