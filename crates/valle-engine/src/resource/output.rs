use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    AuthorSrgbStraight, ColorPrimaries, LuminanceNits, OutputAlphaMode, SignalLuminance,
    TransferFunction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputColorEncoding {
    pub primaries: ColorPrimaries,
    pub transfer: TransferFunction,
}

impl OutputColorEncoding {
    pub const SRGB: Self = Self {
        primaries: ColorPrimaries::Rec709,
        transfer: TransferFunction::Srgb,
    };

    pub const DISPLAY_P3: Self = Self {
        primaries: ColorPrimaries::DisplayP3,
        transfer: TransferFunction::Srgb,
    };

    pub const REC709: Self = Self {
        primaries: ColorPrimaries::Rec709,
        transfer: TransferFunction::Rec709,
    };

    pub const REC2020_PQ: Self = Self {
        primaries: ColorPrimaries::Rec2020,
        transfer: TransferFunction::Pq,
    };

    pub const REC2020_HLG: Self = Self {
        primaries: ColorPrimaries::Rec2020,
        transfer: TransferFunction::Hlg,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToneMap {
    None,
    ReinhardLuminance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GamutMap {
    Clip,
    ChromaCompress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Dither {
    None,
    Triangular { seed: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputBitDepth {
    Eight,
    Ten,
    Twelve,
    Sixteen,
    Float16,
}

impl OutputBitDepth {
    const fn supports_hdr_transfer(self) -> bool {
        !matches!(self, Self::Eight)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OutputBackground {
    Transparent,
    AuthorSrgbStraight { color: AuthorSrgbStraight },
}

impl OutputBackground {
    pub const fn opaque_srgb(rgb: [u8; 3]) -> Self {
        Self::AuthorSrgbStraight {
            color: AuthorSrgbStraight([rgb[0], rgb[1], rgb[2], 255]),
        }
    }

    pub const fn author_color(self) -> AuthorSrgbStraight {
        match self {
            Self::Transparent => AuthorSrgbStraight([0, 0, 0, 0]),
            Self::AuthorSrgbStraight { color } => color,
        }
    }
}

/// Complete terminal output contract. No backend is allowed to fill in omitted color or alpha
/// behavior from a platform default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "OutputSpecWire", into = "OutputSpecWire")]
pub struct OutputSpec {
    target: OutputColorEncoding,
    alpha: OutputAlphaMode,
    background: OutputBackground,
    tone_map: ToneMap,
    gamut_map: GamutMap,
    dither: Dither,
    bit_depth: OutputBitDepth,
    luminance: SignalLuminance,
}

impl OutputSpec {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        target: OutputColorEncoding,
        alpha: OutputAlphaMode,
        background: OutputBackground,
        tone_map: ToneMap,
        gamut_map: GamutMap,
        dither: Dither,
        bit_depth: OutputBitDepth,
        luminance: SignalLuminance,
    ) -> Result<Self, OutputSpecError> {
        if alpha == OutputAlphaMode::Opaque {
            match background {
                OutputBackground::Transparent => {
                    return Err(OutputSpecError::OpaqueOutputNeedsBackground);
                }
                OutputBackground::AuthorSrgbStraight { color } if color.0[3] != u8::MAX => {
                    return Err(OutputSpecError::OpaqueOutputNeedsBackground);
                }
                OutputBackground::AuthorSrgbStraight { .. } => {}
            }
        }
        if matches!(
            target.transfer,
            TransferFunction::Pq | TransferFunction::Hlg
        ) && !bit_depth.supports_hdr_transfer()
        {
            return Err(OutputSpecError::HdrNeedsHighBitDepth);
        }
        if matches!(
            target.transfer,
            TransferFunction::Pq | TransferFunction::Hlg
        ) && target.primaries != ColorPrimaries::Rec2020
        {
            return Err(OutputSpecError::HdrNeedsRec2020);
        }
        if bit_depth == OutputBitDepth::Float16 && dither != Dither::None {
            return Err(OutputSpecError::FloatOutputCannotDither);
        }
        Ok(Self {
            target,
            alpha,
            background,
            tone_map,
            gamut_map,
            dither,
            bit_depth,
            luminance,
        })
    }

    pub fn srgb_preview(background: OutputBackground) -> Result<Self, OutputSpecError> {
        let alpha = match background {
            OutputBackground::Transparent => OutputAlphaMode::StraightCoverage,
            OutputBackground::AuthorSrgbStraight { color } if color.0[3] == u8::MAX => {
                OutputAlphaMode::Opaque
            }
            OutputBackground::AuthorSrgbStraight { .. } => OutputAlphaMode::StraightCoverage,
        };
        Self::new(
            OutputColorEncoding::SRGB,
            alpha,
            background,
            ToneMap::None,
            GamutMap::ChromaCompress,
            // A live preview targets a display surface, not a frozen integer delivery stream.
            // Keeping dither disabled gives Native and GPU/Web previews one reproducible contract;
            // PNG/video exporters request their delivery dither explicitly.
            Dither::None,
            OutputBitDepth::Eight,
            SignalLuminance::SDR_100,
        )
    }

    pub const fn target(self) -> OutputColorEncoding {
        self.target
    }

    pub const fn alpha(self) -> OutputAlphaMode {
        self.alpha
    }

    pub const fn background(self) -> OutputBackground {
        self.background
    }

    pub const fn tone_map(self) -> ToneMap {
        self.tone_map
    }

    pub const fn gamut_map(self) -> GamutMap {
        self.gamut_map
    }

    pub const fn dither(self) -> Dither {
        self.dither
    }

    pub const fn bit_depth(self) -> OutputBitDepth {
        self.bit_depth
    }

    pub const fn luminance(self) -> SignalLuminance {
        self.luminance
    }

    pub const fn reference_white(self) -> LuminanceNits {
        self.luminance.reference_white()
    }

    pub const fn peak_luminance(self) -> LuminanceNits {
        self.luminance.peak()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OutputSpecWire {
    target: OutputColorEncoding,
    alpha: OutputAlphaMode,
    background: OutputBackground,
    tone_map: ToneMap,
    gamut_map: GamutMap,
    dither: Dither,
    bit_depth: OutputBitDepth,
    luminance: SignalLuminance,
}

impl TryFrom<OutputSpecWire> for OutputSpec {
    type Error = OutputSpecError;

    fn try_from(value: OutputSpecWire) -> Result<Self, Self::Error> {
        Self::new(
            value.target,
            value.alpha,
            value.background,
            value.tone_map,
            value.gamut_map,
            value.dither,
            value.bit_depth,
            value.luminance,
        )
    }
}

impl From<OutputSpec> for OutputSpecWire {
    fn from(value: OutputSpec) -> Self {
        Self {
            target: value.target,
            alpha: value.alpha,
            background: value.background,
            tone_map: value.tone_map,
            gamut_map: value.gamut_map,
            dither: value.dither,
            bit_depth: value.bit_depth,
            luminance: value.luminance,
        }
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum OutputSpecError {
    #[error("opaque output requires an explicit opaque background")]
    OpaqueOutputNeedsBackground,
    #[error("PQ/HLG output requires at least 10-bit or floating-point output")]
    HdrNeedsHighBitDepth,
    #[error("PQ/HLG delivery currently requires Rec.2020 primaries")]
    HdrNeedsRec2020,
    #[error("floating-point output must not request integer-domain dither")]
    FloatOutputCannotDither,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_contract_rejects_implicit_transparency_and_low_bit_hdr() {
        let luminance = SignalLuminance::SDR_100;
        assert!(matches!(
            OutputSpec::new(
                OutputColorEncoding::SRGB,
                OutputAlphaMode::Opaque,
                OutputBackground::Transparent,
                ToneMap::None,
                GamutMap::Clip,
                Dither::None,
                OutputBitDepth::Eight,
                luminance,
            ),
            Err(OutputSpecError::OpaqueOutputNeedsBackground)
        ));
        assert!(matches!(
            OutputSpec::new(
                OutputColorEncoding::REC2020_PQ,
                OutputAlphaMode::Opaque,
                OutputBackground::opaque_srgb([0, 0, 0]),
                ToneMap::ReinhardLuminance,
                GamutMap::ChromaCompress,
                Dither::Triangular { seed: 7 },
                OutputBitDepth::Eight,
                luminance,
            ),
            Err(OutputSpecError::HdrNeedsHighBitDepth)
        ));
        assert!(matches!(
            OutputSpec::new(
                OutputColorEncoding::SRGB,
                OutputAlphaMode::Opaque,
                OutputBackground::opaque_srgb([0, 0, 0]),
                ToneMap::None,
                GamutMap::Clip,
                Dither::Triangular { seed: 1 },
                OutputBitDepth::Float16,
                luminance,
            ),
            Err(OutputSpecError::FloatOutputCannotDither)
        ));
    }

    #[test]
    fn live_preview_does_not_claim_delivery_dither() {
        let preview = OutputSpec::srgb_preview(OutputBackground::Transparent).unwrap();
        assert_eq!(preview.dither(), Dither::None);
    }
}
