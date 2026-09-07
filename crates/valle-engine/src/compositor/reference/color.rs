use thiserror::Error;
use valle_draw::math;

use super::{PixelError, PremulRgba32, validate_unit};
use crate::resource::{
    AuthorSrgbStraight, ColorDescription, ColorPrimaries, ColorRange, InputAlphaMode,
    LuminanceNits, MatrixCoefficients, SignalLuminance, TransferFunction, VisualInterpretation,
};

/// Working value `1.0` represents this many display-referred nits for absolute PQ ingress.
pub const WORKING_REFERENCE_WHITE_NITS: f32 = 100.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodedLayout {
    Rgb,
    Yuv,
}

/// Integer source sample before input color and alpha interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodedPixel {
    layout: EncodedLayout,
    color: [u16; 3],
    alpha: u16,
    bit_depth: u8,
}

impl EncodedPixel {
    pub fn rgb(color: [u16; 3], alpha: u16, bit_depth: u8) -> Result<Self, ColorMathError> {
        Self::new(EncodedLayout::Rgb, color, alpha, bit_depth)
    }

    pub fn yuv(color: [u16; 3], alpha: u16, bit_depth: u8) -> Result<Self, ColorMathError> {
        Self::new(EncodedLayout::Yuv, color, alpha, bit_depth)
    }

    fn new(
        layout: EncodedLayout,
        color: [u16; 3],
        alpha: u16,
        bit_depth: u8,
    ) -> Result<Self, ColorMathError> {
        if !matches!(bit_depth, 8 | 10 | 12 | 16) {
            return Err(ColorMathError::UnsupportedBitDepth { bit_depth });
        }
        let maximum = code_max(bit_depth);
        if color.into_iter().any(|value| u32::from(value) > maximum) || u32::from(alpha) > maximum {
            return Err(ColorMathError::CodeOutOfRange { bit_depth });
        }
        Ok(Self {
            layout,
            color,
            alpha,
            bit_depth,
        })
    }

    pub const fn layout(self) -> EncodedLayout {
        self.layout
    }

    pub const fn color(self) -> [u16; 3] {
        self.color
    }

    pub const fn alpha(self) -> u16 {
        self.alpha
    }

    pub const fn bit_depth(self) -> u8 {
        self.bit_depth
    }
}

/// Decode RGB/YUV integer samples into the one working contract: Linear Rec.2020 D65,
/// premultiplied coverage alpha.
pub fn decode_input_pixel(
    pixel: EncodedPixel,
    interpretation: VisualInterpretation,
) -> Result<PremulRgba32, ColorMathError> {
    let maximum = code_max(pixel.bit_depth) as f32;
    let alpha = match interpretation.alpha {
        InputAlphaMode::Opaque => 1.0,
        InputAlphaMode::StraightCoverage | InputAlphaMode::PremultipliedCoverage => {
            f32::from(pixel.alpha) / maximum
        }
    };
    validate_unit(alpha, "input alpha")?;
    if alpha == 0.0 {
        return Ok(PremulRgba32::TRANSPARENT);
    }

    let mut encoded_rgb = match pixel.layout {
        EncodedLayout::Rgb => {
            if interpretation.color.matrix != MatrixCoefficients::Identity {
                return Err(ColorMathError::RgbNeedsIdentityMatrix);
            }
            decode_rgb_codes(pixel.color, pixel.bit_depth, interpretation.color.range)
        }
        EncodedLayout::Yuv => {
            if interpretation.alpha == InputAlphaMode::PremultipliedCoverage {
                return Err(ColorMathError::PremultipliedYuv);
            }
            decode_yuv_codes(pixel.color, pixel.bit_depth, interpretation.color)?
        }
    };

    if interpretation.alpha == InputAlphaMode::PremultipliedCoverage {
        for channel in &mut encoded_rgb {
            *channel /= alpha;
        }
    }

    let linear_source = decode_input_rgb(
        encoded_rgb,
        interpretation.color.transfer,
        interpretation.color.primaries,
        interpretation.luminance,
    )?;
    let working = primaries_to_working(linear_source, interpretation.color.primaries)?;
    PremulRgba32::from_straight(working, alpha).map_err(Into::into)
}

/// Decode a tagged author sRGB straight color into working premultiplied form.
pub fn decode_author_srgb(color: AuthorSrgbStraight) -> Result<PremulRgba32, ColorMathError> {
    decode_input_pixel(
        EncodedPixel::rgb(
            [
                u16::from(color.0[0]),
                u16::from(color.0[1]),
                u16::from(color.0[2]),
            ],
            u16::from(color.0[3]),
            8,
        )?,
        VisualInterpretation::new(
            ColorDescription::SRGB,
            SignalLuminance::SDR_100,
            InputAlphaMode::StraightCoverage,
        ),
    )
}

/// Convert straight Linear Rec.2020 working RGB to straight extended-sRGB encoded RGB.
pub fn working_to_extended_srgb(rgb: [f32; 3]) -> Result<[f32; 3], ColorMathError> {
    let linear_srgb = working_to_primaries(rgb, ColorPrimaries::Rec709)?;
    Ok(linear_srgb.map(|channel| encode_transfer(channel, TransferFunction::Srgb)))
}

/// Convert straight extended-sRGB encoded RGB back to straight Linear Rec.2020 working RGB.
pub fn extended_srgb_to_working(rgb: [f32; 3]) -> Result<[f32; 3], ColorMathError> {
    let linear_srgb = rgb.map(|channel| decode_transfer(channel, TransferFunction::Srgb));
    primaries_to_working(linear_srgb, ColorPrimaries::Rec709)
}

pub(crate) fn primaries_to_working(
    rgb: [f32; 3],
    primaries: ColorPrimaries,
) -> Result<[f32; 3], ColorMathError> {
    if primaries == ColorPrimaries::Rec2020 {
        return validate_rgb(rgb);
    }
    let xyz = multiply_matrix(primaries_to_xyz(primaries), rgb)?;
    multiply_matrix(XYZ_TO_REC2020, xyz)
}

pub(crate) fn working_to_primaries(
    rgb: [f32; 3],
    primaries: ColorPrimaries,
) -> Result<[f32; 3], ColorMathError> {
    if primaries == ColorPrimaries::Rec2020 {
        return validate_rgb(rgb);
    }
    let xyz = multiply_matrix(REC2020_TO_XYZ, rgb)?;
    multiply_matrix(xyz_to_primaries(primaries), xyz)
}

pub(crate) fn decode_transfer(value: f32, transfer: TransferFunction) -> f32 {
    decode_input_transfer(value, transfer, SignalLuminance::SDR_100)
}

pub(crate) fn decode_input_rgb(
    value: [f32; 3],
    transfer: TransferFunction,
    primaries: ColorPrimaries,
    luminance: SignalLuminance,
) -> Result<[f32; 3], ColorMathError> {
    if transfer != TransferFunction::Hlg {
        return validate_rgb(
            value.map(|channel| decode_input_transfer(channel, transfer, luminance)),
        );
    }
    let scene = value.map(signed_hlg_inverse_oetf);
    let scene_luminance = linear_luminance(scene, primaries);
    let scale = hlg_ootf_scale(scene_luminance, hlg_system_gamma(luminance.peak()));
    let absolute_scale = f32::from(luminance.peak().get()) / WORKING_REFERENCE_WHITE_NITS;
    validate_rgb(scene.map(|channel| channel * scale * absolute_scale))
}

pub(crate) fn decode_input_transfer(
    value: f32,
    transfer: TransferFunction,
    luminance: SignalLuminance,
) -> f32 {
    let relative_scale =
        f32::from(luminance.reference_white().get()) / WORKING_REFERENCE_WHITE_NITS;
    match transfer {
        TransferFunction::Linear => value * relative_scale,
        TransferFunction::Srgb => {
            signed_curve(
                value,
                0.04045,
                |magnitude| magnitude / 12.92,
                |magnitude| powf((magnitude + 0.055) / 1.055, 2.4),
            ) * relative_scale
        }
        TransferFunction::Rec709 => {
            signed_curve(
                value,
                0.081,
                |magnitude| magnitude / 4.5,
                |magnitude| powf((magnitude + 0.099) / 1.099, 1.0 / 0.45),
            ) * relative_scale
        }
        TransferFunction::Pq => {
            value.signum() * pq_eotf(value.abs()) * (10_000.0 / WORKING_REFERENCE_WHITE_NITS)
        }
        TransferFunction::Hlg => {
            decode_input_rgb([value; 3], transfer, ColorPrimaries::Rec2020, luminance)
                .map_or(f32::NAN, |rgb| rgb[0])
        }
    }
}

pub(crate) fn encode_transfer(value: f32, transfer: TransferFunction) -> f32 {
    encode_output_transfer(value, transfer, SignalLuminance::SDR_100)
}

pub(crate) fn encode_output_rgb(
    value: [f32; 3],
    transfer: TransferFunction,
    primaries: ColorPrimaries,
    luminance: SignalLuminance,
) -> Result<[f32; 3], ColorMathError> {
    if transfer != TransferFunction::Hlg {
        return validate_rgb(
            value.map(|channel| encode_output_transfer(channel, transfer, luminance)),
        );
    }
    let peak_scale = WORKING_REFERENCE_WHITE_NITS / f32::from(luminance.peak().get());
    let display = value.map(|channel| channel * peak_scale);
    let display_luminance = linear_luminance(display, primaries);
    let gamma = hlg_system_gamma(luminance.peak());
    let scale = hlg_inverse_ootf_scale(display_luminance, gamma);
    validate_rgb(display.map(|channel| signed_hlg_oetf(channel / scale)))
}

pub(crate) fn encode_output_transfer(
    value: f32,
    transfer: TransferFunction,
    luminance: SignalLuminance,
) -> f32 {
    let relative =
        value * WORKING_REFERENCE_WHITE_NITS / f32::from(luminance.reference_white().get());
    match transfer {
        TransferFunction::Linear => relative,
        TransferFunction::Srgb => signed_curve(
            relative,
            0.003_130_8,
            |magnitude| magnitude * 12.92,
            |magnitude| 1.055 * powf(magnitude, 1.0 / 2.4) - 0.055,
        ),
        TransferFunction::Rec709 => signed_curve(
            relative,
            0.018,
            |magnitude| magnitude * 4.5,
            |magnitude| 1.099 * powf(magnitude, 0.45) - 0.099,
        ),
        TransferFunction::Pq => {
            let absolute = value.abs() * (WORKING_REFERENCE_WHITE_NITS / 10_000.0);
            value.signum() * pq_inverse_eotf(absolute)
        }
        TransferFunction::Hlg => {
            encode_output_rgb([value; 3], transfer, ColorPrimaries::Rec2020, luminance)
                .map_or(f32::NAN, |rgb| rgb[0])
        }
    }
}

fn decode_rgb_codes(codes: [u16; 3], bit_depth: u8, range: ColorRange) -> [f32; 3] {
    match range {
        ColorRange::Full => {
            let maximum = code_max(bit_depth) as f32;
            codes.map(|code| f32::from(code) / maximum)
        }
        ColorRange::Limited => {
            let scale = 1u32 << (bit_depth - 8);
            let minimum = (16 * scale) as f32;
            let span = (219 * scale) as f32;
            codes.map(|code| (f32::from(code) - minimum) / span)
        }
    }
}

fn decode_yuv_codes(
    codes: [u16; 3],
    bit_depth: u8,
    description: ColorDescription,
) -> Result<[f32; 3], ColorMathError> {
    let (y, cb, cr) = match description.range {
        ColorRange::Full => {
            let maximum = code_max(bit_depth) as f32;
            (
                f32::from(codes[0]) / maximum,
                f32::from(codes[1]) / maximum - 0.5,
                f32::from(codes[2]) / maximum - 0.5,
            )
        }
        ColorRange::Limited => {
            let scale = 1u32 << (bit_depth - 8);
            let y_minimum = (16 * scale) as f32;
            let y_span = (219 * scale) as f32;
            let chroma_center = (128 * scale) as f32;
            let chroma_span = (224 * scale) as f32;
            (
                (f32::from(codes[0]) - y_minimum) / y_span,
                (f32::from(codes[1]) - chroma_center) / chroma_span,
                (f32::from(codes[2]) - chroma_center) / chroma_span,
            )
        }
    };
    let rgb = match description.matrix {
        MatrixCoefficients::Identity => return Err(ColorMathError::YuvNeedsMatrix),
        MatrixCoefficients::Bt601 => [
            y + 1.402 * cr,
            y - 0.344_136 * cb - 0.714_136 * cr,
            y + 1.772 * cb,
        ],
        MatrixCoefficients::Bt709 => [
            y + 1.5748 * cr,
            y - 0.187_324 * cb - 0.468_124 * cr,
            y + 1.8556 * cb,
        ],
        MatrixCoefficients::Bt2020Ncl => [
            y + 1.4746 * cr,
            y - 0.164_553 * cb - 0.571_353 * cr,
            y + 1.8814 * cb,
        ],
    };
    validate_rgb(rgb)
}

fn signed_curve(
    value: f32,
    threshold: f32,
    linear: impl FnOnce(f32) -> f32,
    curved: impl FnOnce(f32) -> f32,
) -> f32 {
    let magnitude = value.abs();
    value.signum()
        * if magnitude <= threshold {
            linear(magnitude)
        } else {
            curved(magnitude)
        }
}

fn pq_eotf(value: f32) -> f32 {
    const M1: f32 = 2610.0 / 16_384.0;
    const M2: f32 = 2523.0 / 32.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 128.0;
    const C3: f32 = 2392.0 / 128.0;
    let power = powf(value, 1.0 / M2);
    powf(
        (power - C1).max(0.0) / (C2 - C3 * power).max(f32::MIN_POSITIVE),
        1.0 / M1,
    )
}

fn pq_inverse_eotf(value: f32) -> f32 {
    const M1: f32 = 2610.0 / 16_384.0;
    const M2: f32 = 2523.0 / 32.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 128.0;
    const C3: f32 = 2392.0 / 128.0;
    let power = powf(value.max(0.0), M1);
    powf((C1 + C2 * power) / (1.0 + C3 * power), M2)
}

const HLG_A: f32 = 0.178_832_77;
const HLG_B: f32 = 0.284_668_92;
const HLG_C: f32 = 0.559_910_7;

fn hlg_oetf(value: f32) -> f32 {
    if value <= 1.0 / 12.0 {
        sqrtf(3.0 * value)
    } else {
        HLG_A * lnf(12.0 * value - HLG_B) + HLG_C
    }
}

fn hlg_inverse_oetf(value: f32) -> f32 {
    if value <= 0.5 {
        value * value / 3.0
    } else {
        (expf((value - HLG_C) / HLG_A) + HLG_B) / 12.0
    }
}

fn signed_hlg_oetf(value: f32) -> f32 {
    value.signum() * hlg_oetf(value.abs())
}

fn signed_hlg_inverse_oetf(value: f32) -> f32 {
    value.signum() * hlg_inverse_oetf(value.abs())
}

fn hlg_system_gamma(peak: LuminanceNits) -> f32 {
    (1.2 + 0.42 * log10f(f32::from(peak.get()) / 1000.0)).max(1.0)
}

fn hlg_ootf_scale(luminance: f32, gamma: f32) -> f32 {
    if luminance.abs() <= f32::EPSILON {
        1.0
    } else {
        powf(luminance.abs(), gamma - 1.0)
    }
}

fn hlg_inverse_ootf_scale(display_luminance: f32, gamma: f32) -> f32 {
    if display_luminance.abs() <= f32::EPSILON {
        1.0
    } else {
        powf(display_luminance.abs(), (gamma - 1.0) / gamma)
    }
}

fn linear_luminance(rgb: [f32; 3], primaries: ColorPrimaries) -> f32 {
    let weights = match primaries {
        ColorPrimaries::Rec709 => [0.212_639, 0.715_169, 0.072_192],
        ColorPrimaries::DisplayP3 => [0.228_975, 0.691_739, 0.079_287],
        ColorPrimaries::Rec2020 => [0.2627, 0.6780, 0.0593],
    };
    weights[0] * rgb[0] + weights[1] * rgb[1] + weights[2] * rgb[2]
}

// Reference results enter canonical Native/WASM parity evidence. Always call the repository's
// exact, pure-Rust libm implementation and round only once at the explicit f32 boundary.
fn powf(base: f32, exponent: f32) -> f32 {
    math::pow(f64::from(base), f64::from(exponent)) as f32
}

fn sqrtf(value: f32) -> f32 {
    math::sqrt(f64::from(value)) as f32
}

fn expf(value: f32) -> f32 {
    math::exp(f64::from(value)) as f32
}

fn lnf(value: f32) -> f32 {
    math::ln(f64::from(value)) as f32
}

fn log10f(value: f32) -> f32 {
    math::log10(f64::from(value)) as f32
}

fn primaries_to_xyz(primaries: ColorPrimaries) -> [[f64; 3]; 3] {
    match primaries {
        ColorPrimaries::Rec709 => REC709_TO_XYZ,
        ColorPrimaries::DisplayP3 => DISPLAY_P3_TO_XYZ,
        ColorPrimaries::Rec2020 => REC2020_TO_XYZ,
    }
}

fn xyz_to_primaries(primaries: ColorPrimaries) -> [[f64; 3]; 3] {
    match primaries {
        ColorPrimaries::Rec709 => XYZ_TO_REC709,
        ColorPrimaries::DisplayP3 => XYZ_TO_DISPLAY_P3,
        ColorPrimaries::Rec2020 => XYZ_TO_REC2020,
    }
}

fn multiply_matrix(matrix: [[f64; 3]; 3], value: [f32; 3]) -> Result<[f32; 3], ColorMathError> {
    let value = value.map(f64::from);
    let result = matrix.map(|row| row[0] * value[0] + row[1] * value[1] + row[2] * value[2]);
    validate_rgb(result.map(|channel| channel as f32))
}

fn validate_rgb(rgb: [f32; 3]) -> Result<[f32; 3], ColorMathError> {
    if rgb.iter().all(|channel| channel.is_finite()) {
        Ok(rgb)
    } else {
        Err(ColorMathError::NonFiniteTransform)
    }
}

const fn code_max(bit_depth: u8) -> u32 {
    (1u32 << bit_depth) - 1
}

const REC709_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.412_390_799_265_959_5,
        0.357_584_339_383_877_96,
        0.180_480_788_401_834_3,
    ],
    [
        0.212_639_005_871_510_36,
        0.715_168_678_767_755_9,
        0.072_192_315_360_733_71,
    ],
    [
        0.019_330_818_715_591_85,
        0.119_194_779_794_625_99,
        0.950_532_152_249_660_6,
    ],
];
const XYZ_TO_REC709: [[f64; 3]; 3] = [
    [
        3.240_969_941_904_522_6,
        -1.537_383_177_570_094,
        -0.498_610_760_293_003_4,
    ],
    [
        -0.969_243_636_280_879_6,
        1.875_967_501_507_720_2,
        0.041_555_057_407_175_59,
    ],
    [
        0.055_630_079_696_993_66,
        -0.203_976_958_888_976_52,
        1.056_971_514_242_878_6,
    ],
];
const DISPLAY_P3_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.486_570_948_648_216_2,
        0.265_667_693_169_093_06,
        0.198_217_285_234_362_5,
    ],
    [
        0.228_974_564_069_748_8,
        0.691_738_521_836_506_4,
        0.079_286_914_093_745,
    ],
    [0.0, 0.045_113_381_858_902_64, 1.043_944_368_900_976],
];
const XYZ_TO_DISPLAY_P3: [[f64; 3]; 3] = [
    [
        2.493_496_911_941_425,
        -0.931_383_617_919_123_9,
        -0.402_710_784_450_716_84,
    ],
    [
        -0.829_488_969_561_574_7,
        1.762_664_060_318_346_3,
        0.023_624_685_841_943_577,
    ],
    [
        0.035_845_830_243_784_47,
        -0.076_172_389_268_041_82,
        0.956_884_524_007_687_2,
    ],
];
const REC2020_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.636_958_048_301_291_4,
        0.144_616_903_586_208_32,
        0.168_880_975_164_172_1,
    ],
    [
        0.262_700_212_011_267_1,
        0.677_998_071_518_870_8,
        0.059_301_716_469_861_96,
    ],
    [0.0, 0.028_072_693_049_087_428, 1.060_985_057_710_791],
];
const XYZ_TO_REC2020: [[f64; 3]; 3] = [
    [
        1.716_651_187_971_267_4,
        -0.355_670_783_776_392_4,
        -0.253_366_281_373_659_74,
    ],
    [
        -0.666_684_351_832_489_2,
        1.616_481_236_634_939_5,
        0.015_768_545_813_911_13,
    ],
    [
        0.017_639_857_445_310_783,
        -0.042_770_613_257_808_524,
        0.942_103_121_235_473_8,
    ],
];

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ColorMathError {
    #[error(transparent)]
    Pixel(#[from] PixelError),
    #[error("unsupported input bit depth {bit_depth}; expected 8, 10, 12 or 16")]
    UnsupportedBitDepth { bit_depth: u8 },
    #[error("sample code exceeds the declared {bit_depth}-bit range")]
    CodeOutOfRange { bit_depth: u8 },
    #[error("RGB input requires identity matrix coefficients")]
    RgbNeedsIdentityMatrix,
    #[error("YUV input requires non-identity matrix coefficients")]
    YuvNeedsMatrix,
    #[error("premultiplied encoded YUV is not a supported source representation")]
    PremultipliedYuv,
    #[error("color transform produced a non-finite f32 channel")]
    NonFiniteTransform,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec709_video(alpha: InputAlphaMode) -> VisualInterpretation {
        VisualInterpretation::new(
            ColorDescription {
                primaries: ColorPrimaries::Rec709,
                transfer: TransferFunction::Rec709,
                matrix: MatrixCoefficients::Bt709,
                range: ColorRange::Limited,
            },
            SignalLuminance::SDR_100,
            alpha,
        )
    }

    #[test]
    fn author_srgb_red_maps_to_known_rec2020_primary_mix() {
        let red = decode_author_srgb(AuthorSrgbStraight([255, 0, 0, 255])).unwrap();
        let [r, g, b, a] = red.channels();
        assert!((r - 0.627_404).abs() < 2e-6);
        assert!((g - 0.069_097).abs() < 2e-6);
        assert!((b - 0.016_391).abs() < 2e-6);
        assert_eq!(a, 1.0);
    }

    #[test]
    fn limited_range_video_black_and_white_are_explicit() {
        let black = decode_input_pixel(
            EncodedPixel::yuv([16, 128, 128], 255, 8).unwrap(),
            rec709_video(InputAlphaMode::Opaque),
        )
        .unwrap();
        assert!(black.approx_eq(PremulRgba32::from_straight([0.0; 3], 1.0).unwrap(), 1e-6));

        let white = decode_input_pixel(
            EncodedPixel::yuv([235, 128, 128], 255, 8).unwrap(),
            rec709_video(InputAlphaMode::Opaque),
        )
        .unwrap();
        assert!(white.approx_eq(PremulRgba32::from_straight([1.0; 3], 1.0).unwrap(), 2e-6));
    }

    #[test]
    fn encoded_premul_is_unpremultiplied_before_nonlinear_decode() {
        let interpretation = VisualInterpretation::new(
            ColorDescription::SRGB,
            SignalLuminance::SDR_100,
            InputAlphaMode::PremultipliedCoverage,
        );
        let pixel = decode_input_pixel(
            EncodedPixel::rgb([128, 0, 0], 128, 8).unwrap(),
            interpretation,
        )
        .unwrap();
        let straight = pixel.straight_rgb().unwrap();
        assert!((straight[0] - 0.627_404).abs() < 2e-3);
        assert!((pixel.alpha() - 128.0 / 255.0).abs() < 1e-7);
    }

    #[test]
    fn transparent_external_rgb_is_canonicalized_at_import() {
        let pixel = decode_input_pixel(
            EncodedPixel::rgb([255, 12, 99], 0, 8).unwrap(),
            VisualInterpretation::new(
                ColorDescription::SRGB,
                SignalLuminance::SDR_100,
                InputAlphaMode::StraightCoverage,
            ),
        )
        .unwrap();
        assert_eq!(pixel, PremulRgba32::TRANSPARENT);
    }

    #[test]
    fn extended_srgb_round_trip_preserves_negative_and_hdr_values() {
        let input = [-0.1, 0.5, 2.0];
        let encoded = working_to_extended_srgb(input).unwrap();
        let output = extended_srgb_to_working(encoded).unwrap();
        for (actual, expected) in output.into_iter().zip(input) {
            assert!((actual - expected).abs() < 2e-5, "{actual} != {expected}");
        }
    }

    #[test]
    fn absolute_and_hybrid_log_transfers_round_trip_working_white() {
        let hdr = SignalLuminance::new(
            LuminanceNits::new(203).unwrap(),
            LuminanceNits::new(1000).unwrap(),
        )
        .unwrap();
        for transfer in [TransferFunction::Pq, TransferFunction::Hlg] {
            let encoded = encode_output_transfer(1.0, transfer, hdr);
            let decoded = decode_input_transfer(encoded, transfer, hdr);
            assert!((decoded - 1.0).abs() < 2e-4, "{transfer:?}: {decoded}");
        }

        let colored = [0.2, 0.7, 1.5];
        let encoded =
            encode_output_rgb(colored, TransferFunction::Hlg, ColorPrimaries::Rec2020, hdr)
                .unwrap();
        let decoded =
            decode_input_rgb(encoded, TransferFunction::Hlg, ColorPrimaries::Rec2020, hdr).unwrap();
        for (actual, expected) in decoded.into_iter().zip(colored) {
            assert!((actual - expected).abs() < 3e-5, "{actual} != {expected}");
        }
    }
}
