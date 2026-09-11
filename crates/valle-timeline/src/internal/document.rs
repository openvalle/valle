//! Canonical Timeline domain model.
//!
//! The types in this module deliberately do not implement `Deserialize` and do
//! not own any protocol DTO. A [`crate::internal::CanonicalTimeline`] is the validated
//! aggregate; wire conversion is confined to its construction and encoding
//! boundaries.

use std::collections::BTreeMap;

use serde_json::Value as JsonValue;

use crate::internal::{
    time::{FrameRate, RationalRate, RationalTime},
    wire::document as wire,
};

pub type EntityId = String;
pub type ResourceId = String;
pub type JsonObject = BTreeMap<String, JsonValue>;
pub type Vec2 = [f64; 2];

/// A finite rectangle with a strictly positive extent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect([f64; 4]);

impl Rect {
    pub fn new(value: [f64; 4]) -> Result<Self, InvalidRect> {
        if value.iter().all(|part| part.is_finite()) && value[2] > 0.0 && value[3] > 0.0 {
            Ok(Self(value))
        } else {
            Err(InvalidRect)
        }
    }

    pub const fn as_array(&self) -> &[f64; 4] {
        &self.0
    }

    pub const fn into_array(self) -> [f64; 4] {
        self.0
    }

    pub const fn x(self) -> f64 {
        self.0[0]
    }

    pub const fn y(self) -> f64 {
        self.0[1]
    }

    pub const fn width(self) -> f64 {
        self.0[2]
    }

    pub const fn height(self) -> f64 {
        self.0[3]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("rectangle values must be finite and width/height must be positive")]
pub struct InvalidRect;

/// A positive rectangle wholly contained in normalized `[0, 1]` space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizedRect(Rect);

impl NormalizedRect {
    pub fn new(value: [f64; 4]) -> Result<Self, InvalidNormalizedRect> {
        let rect = Rect::new(value).map_err(|_| InvalidNormalizedRect)?;
        if rect.x() >= 0.0
            && rect.y() >= 0.0
            && rect.x() + rect.width() <= 1.0
            && rect.y() + rect.height() <= 1.0
        {
            Ok(Self(rect))
        } else {
            Err(InvalidNormalizedRect)
        }
    }

    pub const fn rect(self) -> Rect {
        self.0
    }

    pub const fn as_array(&self) -> &[f64; 4] {
        self.0.as_array()
    }

    pub const fn into_array(self) -> [f64; 4] {
        self.0.into_array()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("caption region must be a positive rectangle contained in normalized space")]
pub struct InvalidNormalizedRect;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamespacedKernelType(String);

impl NamespacedKernelType {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidNamespacedKernelType> {
        let value = value.into();
        let Some((namespace, name)) = value.split_once('/') else {
            return Err(InvalidNamespacedKernelType);
        };
        if namespace.is_empty()
            || name.is_empty()
            || name.contains('/')
            || value.chars().any(char::is_whitespace)
        {
            return Err(InvalidNamespacedKernelType);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("kernel type must use the namespaced `<namespace>/<name>` form")]
pub struct InvalidNamespacedKernelType;

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineDocument {
    pub canvas: Canvas,
    pub background: Background,
    pub visual: VisualComposition,
    pub audio: AudioComposition,
    pub adjustments: Vec<TimedAdjustment>,
    pub captions: CaptionComposition,
    pub camera: Option<CameraTrack>,
    pub metadata: JsonObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub sample_rate: u32,
    pub channel_layout: ChannelLayout,
    pub color_space: ColorSpace,
    pub duration: RationalTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelLayout {
    Mono,
    Stereo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    Srgb,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Background {
    pub color: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Param<T> {
    Constant(ConstantParam<T>),
    Curve(Curve<T>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConstantParam<T> {
    pub value: T,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Curve<T> {
    pub id: EntityId,
    pub interpolation: Interpolation,
    pub keyframes: Vec<Keyframe<T>>,
    pub extrapolation: Extrapolation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe<T> {
    pub id: EntityId,
    pub time: RationalTime,
    pub value: T,
    pub out_easing: Option<Easing>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Step,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extrapolation {
    Clamp,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Easing {
    Named(NamedEasing),
    CubicBezier(CubicBezierEasing),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedEasing {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CubicBezierEasing {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VisualComposition {
    pub tracks: Vec<VisualTrack>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VisualTrack {
    pub id: EntityId,
    pub items: Vec<VisualItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VisualItem {
    Clip(VisualClip),
    Gap(Gap),
    Transition(VisualTransition),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    pub id: EntityId,
    pub duration: RationalTime,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VisualClip {
    pub id: EntityId,
    pub duration: RationalTime,
    pub layer: VisualLayer,
    pub source: VisualSource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VisualTransition {
    pub id: EntityId,
    pub duration: RationalTime,
    pub kernel: TransitionKernel,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransitionKernel {
    CrossFade,
    Extension(TransitionKernelExtension),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionKernelExtension {
    pub kind: NamespacedKernelType,
    pub parameters: JsonObject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VisualLayer {
    pub transform: LayerTransform,
    pub opacity: Param<f64>,
    pub mask: Option<LayerMask>,
    pub filters: Vec<VisualFilter>,
    pub blend: BlendMode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerTransform {
    pub size: Option<Param<Vec2>>,
    pub position: Param<Vec2>,
    pub scale: Param<Vec2>,
    pub rotation: Param<f64>,
    pub anchor: Vec2,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayerMask {
    Rect {
        rect: Param<Rect>,
        feather: Param<f64>,
        invert: bool,
    },
    Ellipse {
        rect: Param<Rect>,
        feather: Param<f64>,
        invert: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct VisualFilter {
    pub id: EntityId,
    pub kind: NamespacedKernelType,
    pub parameters: JsonObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Screen,
    Lighten,
    ColorDodge,
    Multiply,
    Darken,
    ColorBurn,
    LinearBurn,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VisualSource {
    Video(VideoSource),
    Image(ImageSource),
    Lottie(LottieSource),
    Motion(MotionInstance),
    Solid(SolidSource),
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoSource {
    pub gain: Option<Param<f64>>,
    pub resource: ResourceId,
    pub source_start: RationalTime,
    pub rate: RationalRate,
    pub end_behavior: MediaEndBehavior,
    pub sampling: RasterSampling,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSource {
    pub resource: ResourceId,
    pub sampling: RasterSampling,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LottieSource {
    pub resource: ResourceId,
    pub source_start: RationalTime,
    pub rate: RationalRate,
    pub end_behavior: MediaEndBehavior,
    pub sampling: LottieSampling,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MotionInstance {
    pub component: ResourceId,
    pub source_start: RationalTime,
    pub source_duration: RationalTime,
    pub rate: RationalRate,
    pub end_behavior: MediaEndBehavior,
    pub props: BTreeMap<String, Param<JsonValue>>,
    pub cues: BTreeMap<String, MotionCueBinding>,
    pub resources: BTreeMap<String, ResourceId>,
    pub phases: MotionPhaseOverrides,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MotionCueBinding {
    SourceRange {
        start: RationalTime,
        end: RationalTime,
        enter_duration: RationalTime,
        exit_duration: RationalTime,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MotionPhaseOverrides {
    pub enter_duration: Option<RationalTime>,
    pub exit_duration: Option<RationalTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolidSource {
    pub color: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaEndBehavior {
    Error,
    Hold,
    Loop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterSampling {
    pub fit: RasterFit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LottieSampling {
    pub fit: RasterFit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterFit {
    Contain,
    Cover,
    Fill,
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioComposition {
    pub tracks: Vec<AudioTrack>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioTrack {
    pub id: EntityId,
    pub items: Vec<AudioItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AudioItem {
    Clip(AudioClip),
    Gap(Gap),
    Crossfade(AudioCrossfade),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioClip {
    pub id: EntityId,
    pub duration: RationalTime,
    pub source: AudioSource,
    pub gain: Param<f64>,
    pub pan: Param<f64>,
    pub effects: Vec<AudioEffect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioCrossfade {
    pub id: EntityId,
    pub duration: RationalTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioSource {
    Media(AudioMediaSource),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioMediaSource {
    pub resource: ResourceId,
    pub source_start: RationalTime,
    pub rate: RationalRate,
    pub end_behavior: MediaEndBehavior,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioEffect {
    pub id: EntityId,
    pub kind: NamespacedKernelType,
    pub parameters: JsonObject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimedAdjustment {
    pub id: EntityId,
    pub start: RationalTime,
    pub duration: RationalTime,
    pub effect: AdjustmentEffect,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AdjustmentEffect {
    ColorGrade(ColorGradeEffect),
    Extension(AdjustmentEffectExtension),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColorGradeEffect {
    pub temperature: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdjustmentEffectExtension {
    pub kind: NamespacedKernelType,
    pub parameters: JsonObject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptionComposition {
    pub tracks: Vec<CaptionTrack>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptionTrack {
    pub id: EntityId,
    pub items: Vec<CaptionItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CaptionItem {
    Clip(Caption),
    Gap(Gap),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Caption {
    pub id: EntityId,
    pub duration: RationalTime,
    pub runs: Vec<TextRun>,
    pub style: CaptionStyle,
    pub layout: CaptionLayout,
    pub presentation: CaptionPresentation,
    pub behavior: Option<CaptionBehavior>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub id: EntityId,
    pub text: String,
    pub timing: Option<TextRunTiming>,
    pub style: Option<TextRunStyle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRunTiming {
    pub start: RationalTime,
    pub end: RationalTime,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextRunStyle {
    pub font_size: Option<f64>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptionStyle {
    pub font: ResourceId,
    pub font_size: f64,
    pub color: String,
    pub shadow: Option<CaptionShadow>,
}

/// Closed timeline bound that keeps caption metrics inside the common f32 shaper/raster ABI.
pub const MAX_CAPTION_FONT_SIZE: f64 = 1_000_000.0;
/// Closed timeline bound shared by caption shadow and presentation Gaussian blur.
pub const MAX_CAPTION_BLUR_SIGMA: f64 = 1_000_000.0;

#[derive(Debug, Clone, PartialEq)]
pub struct CaptionShadow {
    pub color: String,
    pub offset: Vec2,
    pub blur_sigma: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptionLayout {
    pub region: NormalizedRect,
    pub align: CaptionAlign,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptionPresentation {
    pub opacity: Param<f64>,
    pub translation: Param<Vec2>,
    pub scale: Param<f64>,
    pub rotation: Param<f64>,
    pub clip_inset: Param<[f64; 4]>,
    pub blur_sigma: Param<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionAlign {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl CaptionAlign {
    /// Normalized anchor associated with this closed alignment value.
    pub const fn anchor(self) -> [f64; 2] {
        match self {
            Self::TopLeft => [0.0, 0.0],
            Self::TopCenter => [0.5, 0.0],
            Self::TopRight => [1.0, 0.0],
            Self::CenterLeft => [0.0, 0.5],
            Self::Center => [0.5, 0.5],
            Self::CenterRight => [1.0, 0.5],
            Self::BottomLeft => [0.0, 1.0],
            Self::BottomCenter => [0.5, 1.0],
            Self::BottomRight => [1.0, 1.0],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CaptionBehavior {
    Scroll { axis: ScrollAxis, speed: f64 },
    Karaoke { mode: KaraokeMode },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KaraokeMode {
    Word,
    Line,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CameraTrack {
    pub center_x: Param<f64>,
    pub center_y: Param<f64>,
    pub zoom: Param<f64>,
    pub rotation: Param<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DocumentConversionError {
    #[error("invalid frame rate in validated document")]
    FrameRate,
    #[error("invalid playback rate in validated document")]
    RationalRate,
    #[error("invalid mask rectangle in validated document")]
    Rect,
    #[error("invalid normalized caption region in validated document")]
    NormalizedRect,
    #[error("invalid namespaced kernel type in validated document")]
    KernelType,
}

pub(crate) fn from_wire(
    document: wire::TimelineDocumentWire,
) -> Result<TimelineDocument, DocumentConversionError> {
    Ok(TimelineDocument {
        canvas: Canvas {
            width: document.canvas.width,
            height: document.canvas.height,
            fps: FrameRate::from_exact(document.canvas.fps)
                .map_err(|_| DocumentConversionError::FrameRate)?,
            sample_rate: document.canvas.sample_rate,
            channel_layout: channel_layout_from_wire(document.canvas.channel_layout),
            color_space: color_space_from_wire(document.canvas.color_space),
            duration: RationalTime::from_exact(document.canvas.duration),
        },
        background: Background {
            color: document.background.color,
        },
        visual: visual_composition_from_wire(document.visual)?,
        audio: audio_composition_from_wire(document.audio)?,
        adjustments: document
            .adjustments
            .into_iter()
            .map(adjustment_from_wire)
            .collect::<Result<_, _>>()?,
        captions: caption_composition_from_wire(document.captions)?,
        camera: document.camera.map(camera_from_wire).transpose()?,
        metadata: document.metadata,
    })
}

pub(crate) fn to_wire(document: &TimelineDocument) -> wire::TimelineDocumentWire {
    into_wire(document.clone())
}

fn into_wire(document: TimelineDocument) -> wire::TimelineDocumentWire {
    wire::TimelineDocumentWire {
        canvas: wire::CanvasWire {
            width: document.canvas.width,
            height: document.canvas.height,
            fps: document.canvas.fps.into_exact(),
            sample_rate: document.canvas.sample_rate,
            channel_layout: channel_layout_into_wire(document.canvas.channel_layout),
            color_space: color_space_into_wire(document.canvas.color_space),
            duration: document.canvas.duration.into_exact(),
        },
        background: wire::BackgroundWire {
            color: document.background.color,
        },
        visual: visual_composition_into_wire(document.visual),
        audio: audio_composition_into_wire(document.audio),
        adjustments: document
            .adjustments
            .into_iter()
            .map(adjustment_into_wire)
            .collect(),
        captions: caption_composition_into_wire(document.captions),
        camera: document.camera.map(camera_into_wire),
        metadata: document.metadata,
    }
}

fn kernel_type_from_wire(
    kind: wire::NamespacedKernelTypeWire,
) -> Result<NamespacedKernelType, DocumentConversionError> {
    NamespacedKernelType::new(kind.into_string()).map_err(|_| DocumentConversionError::KernelType)
}

fn kernel_type_into_wire(kind: NamespacedKernelType) -> wire::NamespacedKernelTypeWire {
    wire::NamespacedKernelTypeWire::new(kind.into_string())
        .expect("canonical domain kernel names are namespaced")
}

fn param_from_wire<T, U>(
    param: wire::ParamWire<T>,
    mut convert: impl FnMut(T) -> Result<U, DocumentConversionError>,
) -> Result<Param<U>, DocumentConversionError> {
    match param {
        wire::ParamWire::Constant(constant) => Ok(Param::Constant(ConstantParam {
            value: convert(constant.value)?,
        })),
        wire::ParamWire::Curve(curve) => Ok(Param::Curve(Curve {
            id: curve.id,
            interpolation: interpolation_from_wire(curve.interpolation),
            keyframes: curve
                .keyframes
                .into_iter()
                .map(|keyframe| {
                    Ok(Keyframe {
                        id: keyframe.id,
                        time: RationalTime::from_exact(keyframe.time),
                        value: convert(keyframe.value)?,
                        out_easing: keyframe.out_easing.map(easing_from_wire),
                    })
                })
                .collect::<Result<_, DocumentConversionError>>()?,
            extrapolation: extrapolation_from_wire(curve.extrapolation),
        })),
    }
}

fn param_into_wire<T, U>(param: Param<T>, mut convert: impl FnMut(T) -> U) -> wire::ParamWire<U> {
    match param {
        Param::Constant(constant) => wire::ParamWire::Constant(wire::ConstantParamWire {
            value: convert(constant.value),
        }),
        Param::Curve(curve) => wire::ParamWire::Curve(wire::CurveWire {
            id: curve.id,
            interpolation: interpolation_into_wire(curve.interpolation),
            keyframes: curve
                .keyframes
                .into_iter()
                .map(|keyframe| wire::KeyframeWire {
                    id: keyframe.id,
                    time: keyframe.time.into_exact(),
                    value: convert(keyframe.value),
                    out_easing: keyframe.out_easing.map(easing_into_wire),
                })
                .collect(),
            extrapolation: extrapolation_into_wire(curve.extrapolation),
        }),
    }
}

fn identity<T>(value: T) -> Result<T, DocumentConversionError> {
    Ok(value)
}

fn identity_into<T>(value: T) -> T {
    value
}

fn channel_layout_from_wire(value: wire::ChannelLayoutWire) -> ChannelLayout {
    match value {
        wire::ChannelLayoutWire::Mono => ChannelLayout::Mono,
        wire::ChannelLayoutWire::Stereo => ChannelLayout::Stereo,
    }
}

fn channel_layout_into_wire(value: ChannelLayout) -> wire::ChannelLayoutWire {
    match value {
        ChannelLayout::Mono => wire::ChannelLayoutWire::Mono,
        ChannelLayout::Stereo => wire::ChannelLayoutWire::Stereo,
    }
}

fn color_space_from_wire(value: wire::ColorSpaceWire) -> ColorSpace {
    match value {
        wire::ColorSpaceWire::Srgb => ColorSpace::Srgb,
    }
}

fn color_space_into_wire(value: ColorSpace) -> wire::ColorSpaceWire {
    match value {
        ColorSpace::Srgb => wire::ColorSpaceWire::Srgb,
    }
}

fn interpolation_from_wire(value: wire::InterpolationWire) -> Interpolation {
    match value {
        wire::InterpolationWire::Step => Interpolation::Step,
        wire::InterpolationWire::Linear => Interpolation::Linear,
    }
}

fn interpolation_into_wire(value: Interpolation) -> wire::InterpolationWire {
    match value {
        Interpolation::Step => wire::InterpolationWire::Step,
        Interpolation::Linear => wire::InterpolationWire::Linear,
    }
}

fn extrapolation_from_wire(value: wire::ExtrapolationWire) -> Extrapolation {
    match value {
        wire::ExtrapolationWire::Clamp => Extrapolation::Clamp,
    }
}

fn extrapolation_into_wire(value: Extrapolation) -> wire::ExtrapolationWire {
    match value {
        Extrapolation::Clamp => wire::ExtrapolationWire::Clamp,
    }
}

fn easing_from_wire(value: wire::EasingWire) -> Easing {
    match value {
        wire::EasingWire::Named(named) => Easing::Named(named_easing_from_wire(named)),
        wire::EasingWire::CubicBezier(bezier) => Easing::CubicBezier(CubicBezierEasing {
            x1: bezier.x1,
            y1: bezier.y1,
            x2: bezier.x2,
            y2: bezier.y2,
        }),
    }
}

fn easing_into_wire(value: Easing) -> wire::EasingWire {
    match value {
        Easing::Named(named) => wire::EasingWire::Named(named_easing_into_wire(named)),
        Easing::CubicBezier(bezier) => wire::EasingWire::CubicBezier(wire::CubicBezierEasingWire {
            kind: wire::CubicBezierTag::CubicBezier,
            x1: bezier.x1,
            y1: bezier.y1,
            x2: bezier.x2,
            y2: bezier.y2,
        }),
    }
}

fn named_easing_from_wire(value: wire::NamedEasingWire) -> NamedEasing {
    match value {
        wire::NamedEasingWire::Linear => NamedEasing::Linear,
        wire::NamedEasingWire::Ease => NamedEasing::Ease,
        wire::NamedEasingWire::EaseIn => NamedEasing::EaseIn,
        wire::NamedEasingWire::EaseOut => NamedEasing::EaseOut,
        wire::NamedEasingWire::EaseInOut => NamedEasing::EaseInOut,
    }
}

fn named_easing_into_wire(value: NamedEasing) -> wire::NamedEasingWire {
    match value {
        NamedEasing::Linear => wire::NamedEasingWire::Linear,
        NamedEasing::Ease => wire::NamedEasingWire::Ease,
        NamedEasing::EaseIn => wire::NamedEasingWire::EaseIn,
        NamedEasing::EaseOut => wire::NamedEasingWire::EaseOut,
        NamedEasing::EaseInOut => wire::NamedEasingWire::EaseInOut,
    }
}

fn visual_composition_from_wire(
    visual: wire::VisualCompositionWire,
) -> Result<VisualComposition, DocumentConversionError> {
    Ok(VisualComposition {
        tracks: visual
            .tracks
            .into_iter()
            .map(|track| {
                Ok(VisualTrack {
                    id: track.id,
                    items: track
                        .items
                        .into_iter()
                        .map(visual_item_from_wire)
                        .collect::<Result<_, _>>()?,
                })
            })
            .collect::<Result<_, DocumentConversionError>>()?,
    })
}

fn visual_composition_into_wire(visual: VisualComposition) -> wire::VisualCompositionWire {
    wire::VisualCompositionWire {
        tracks: visual
            .tracks
            .into_iter()
            .map(|track| wire::VisualTrackWire {
                id: track.id,
                items: track.items.into_iter().map(visual_item_into_wire).collect(),
            })
            .collect(),
    }
}

fn visual_item_from_wire(
    item: wire::VisualItemWire,
) -> Result<VisualItem, DocumentConversionError> {
    match item {
        wire::VisualItemWire::Clip(clip) => Ok(VisualItem::Clip(VisualClip {
            id: clip.id,
            duration: RationalTime::from_exact(clip.duration),
            layer: visual_layer_from_wire(clip.layer)?,
            source: visual_source_from_wire(clip.source)?,
        })),
        wire::VisualItemWire::Gap(gap) => Ok(VisualItem::Gap(gap_from_wire(gap))),
        wire::VisualItemWire::Transition(transition) => {
            Ok(VisualItem::Transition(VisualTransition {
                id: transition.id,
                duration: RationalTime::from_exact(transition.duration),
                kernel: transition_kernel_from_wire(transition.kernel)?,
            }))
        }
    }
}

fn visual_item_into_wire(item: VisualItem) -> wire::VisualItemWire {
    match item {
        VisualItem::Clip(clip) => wire::VisualItemWire::Clip(wire::VisualClipWire {
            id: clip.id,
            duration: clip.duration.into_exact(),
            layer: visual_layer_into_wire(clip.layer),
            source: visual_source_into_wire(clip.source),
        }),
        VisualItem::Gap(gap) => wire::VisualItemWire::Gap(gap_into_wire(gap)),
        VisualItem::Transition(transition) => {
            wire::VisualItemWire::Transition(wire::VisualTransitionWire {
                id: transition.id,
                duration: transition.duration.into_exact(),
                kernel: transition_kernel_into_wire(transition.kernel),
            })
        }
    }
}

fn gap_from_wire(gap: wire::GapWire) -> Gap {
    Gap {
        id: gap.id,
        duration: RationalTime::from_exact(gap.duration),
    }
}

fn gap_into_wire(gap: Gap) -> wire::GapWire {
    wire::GapWire {
        id: gap.id,
        duration: gap.duration.into_exact(),
    }
}

fn transition_kernel_from_wire(
    kernel: wire::TransitionKernelWire,
) -> Result<TransitionKernel, DocumentConversionError> {
    match kernel {
        wire::TransitionKernelWire::CrossFade(_) => Ok(TransitionKernel::CrossFade),
        wire::TransitionKernelWire::Extension(extension) => {
            Ok(TransitionKernel::Extension(TransitionKernelExtension {
                kind: kernel_type_from_wire(extension.kind)?,
                parameters: extension.parameters,
            }))
        }
    }
}

fn transition_kernel_into_wire(kernel: TransitionKernel) -> wire::TransitionKernelWire {
    match kernel {
        TransitionKernel::CrossFade => {
            wire::TransitionKernelWire::CrossFade(wire::CrossFadeTransitionKernelWire {
                kind: wire::CrossFadeTransitionKernelTag::CrossFade,
            })
        }
        TransitionKernel::Extension(extension) => {
            wire::TransitionKernelWire::Extension(wire::TransitionKernelExtensionWire {
                kind: kernel_type_into_wire(extension.kind),
                parameters: extension.parameters,
            })
        }
    }
}

fn visual_layer_from_wire(
    layer: wire::VisualLayerWire,
) -> Result<VisualLayer, DocumentConversionError> {
    Ok(VisualLayer {
        transform: LayerTransform {
            size: layer
                .transform
                .size
                .map(|size| param_from_wire(size, identity))
                .transpose()?,
            position: param_from_wire(layer.transform.position, identity)?,
            scale: param_from_wire(layer.transform.scale, identity)?,
            rotation: param_from_wire(layer.transform.rotation, identity)?,
            anchor: layer.transform.anchor,
        },
        opacity: param_from_wire(layer.opacity, identity)?,
        mask: layer.mask.map(layer_mask_from_wire).transpose()?,
        filters: layer
            .filters
            .into_iter()
            .map(|filter| {
                Ok(VisualFilter {
                    id: filter.id,
                    kind: kernel_type_from_wire(filter.kind)?,
                    parameters: filter.parameters,
                })
            })
            .collect::<Result<_, DocumentConversionError>>()?,
        blend: blend_mode_from_wire(layer.blend),
    })
}

fn visual_layer_into_wire(layer: VisualLayer) -> wire::VisualLayerWire {
    wire::VisualLayerWire {
        transform: wire::LayerTransformWire {
            size: layer
                .transform
                .size
                .map(|size| param_into_wire(size, std::convert::identity)),
            position: param_into_wire(layer.transform.position, std::convert::identity),
            scale: param_into_wire(layer.transform.scale, std::convert::identity),
            rotation: param_into_wire(layer.transform.rotation, std::convert::identity),
            anchor: layer.transform.anchor,
        },
        opacity: param_into_wire(layer.opacity, std::convert::identity),
        mask: layer.mask.map(layer_mask_into_wire),
        filters: layer
            .filters
            .into_iter()
            .map(|filter| wire::VisualFilterWire {
                id: filter.id,
                kind: kernel_type_into_wire(filter.kind),
                parameters: filter.parameters,
            })
            .collect(),
        blend: blend_mode_into_wire(layer.blend),
    }
}

fn layer_mask_from_wire(mask: wire::LayerMaskWire) -> Result<LayerMask, DocumentConversionError> {
    match mask {
        wire::LayerMaskWire::Rect {
            rect,
            feather,
            invert,
        } => Ok(LayerMask::Rect {
            rect: param_from_wire(rect, |value| {
                Rect::new(value).map_err(|_| DocumentConversionError::Rect)
            })?,
            feather: param_from_wire(feather, identity)?,
            invert,
        }),
        wire::LayerMaskWire::Ellipse {
            rect,
            feather,
            invert,
        } => Ok(LayerMask::Ellipse {
            rect: param_from_wire(rect, |value| {
                Rect::new(value).map_err(|_| DocumentConversionError::Rect)
            })?,
            feather: param_from_wire(feather, identity)?,
            invert,
        }),
    }
}

fn layer_mask_into_wire(mask: LayerMask) -> wire::LayerMaskWire {
    match mask {
        LayerMask::Rect {
            rect,
            feather,
            invert,
        } => wire::LayerMaskWire::Rect {
            rect: param_into_wire(rect, Rect::into_array),
            feather: param_into_wire(feather, std::convert::identity),
            invert,
        },
        LayerMask::Ellipse {
            rect,
            feather,
            invert,
        } => wire::LayerMaskWire::Ellipse {
            rect: param_into_wire(rect, Rect::into_array),
            feather: param_into_wire(feather, std::convert::identity),
            invert,
        },
    }
}

fn blend_mode_from_wire(value: wire::BlendModeWire) -> BlendMode {
    match value {
        wire::BlendModeWire::Normal => BlendMode::Normal,
        wire::BlendModeWire::Screen => BlendMode::Screen,
        wire::BlendModeWire::Lighten => BlendMode::Lighten,
        wire::BlendModeWire::ColorDodge => BlendMode::ColorDodge,
        wire::BlendModeWire::Multiply => BlendMode::Multiply,
        wire::BlendModeWire::Darken => BlendMode::Darken,
        wire::BlendModeWire::ColorBurn => BlendMode::ColorBurn,
        wire::BlendModeWire::LinearBurn => BlendMode::LinearBurn,
        wire::BlendModeWire::Overlay => BlendMode::Overlay,
        wire::BlendModeWire::SoftLight => BlendMode::SoftLight,
        wire::BlendModeWire::HardLight => BlendMode::HardLight,
        wire::BlendModeWire::Difference => BlendMode::Difference,
        wire::BlendModeWire::Exclusion => BlendMode::Exclusion,
        wire::BlendModeWire::Hue => BlendMode::Hue,
        wire::BlendModeWire::Saturation => BlendMode::Saturation,
        wire::BlendModeWire::Color => BlendMode::Color,
        wire::BlendModeWire::Luminosity => BlendMode::Luminosity,
    }
}

fn blend_mode_into_wire(value: BlendMode) -> wire::BlendModeWire {
    match value {
        BlendMode::Normal => wire::BlendModeWire::Normal,
        BlendMode::Screen => wire::BlendModeWire::Screen,
        BlendMode::Lighten => wire::BlendModeWire::Lighten,
        BlendMode::ColorDodge => wire::BlendModeWire::ColorDodge,
        BlendMode::Multiply => wire::BlendModeWire::Multiply,
        BlendMode::Darken => wire::BlendModeWire::Darken,
        BlendMode::ColorBurn => wire::BlendModeWire::ColorBurn,
        BlendMode::LinearBurn => wire::BlendModeWire::LinearBurn,
        BlendMode::Overlay => wire::BlendModeWire::Overlay,
        BlendMode::SoftLight => wire::BlendModeWire::SoftLight,
        BlendMode::HardLight => wire::BlendModeWire::HardLight,
        BlendMode::Difference => wire::BlendModeWire::Difference,
        BlendMode::Exclusion => wire::BlendModeWire::Exclusion,
        BlendMode::Hue => wire::BlendModeWire::Hue,
        BlendMode::Saturation => wire::BlendModeWire::Saturation,
        BlendMode::Color => wire::BlendModeWire::Color,
        BlendMode::Luminosity => wire::BlendModeWire::Luminosity,
    }
}

fn visual_source_from_wire(
    source: wire::VisualSourceWire,
) -> Result<VisualSource, DocumentConversionError> {
    match source {
        wire::VisualSourceWire::Video(source) => Ok(VisualSource::Video(VideoSource {
            gain: source
                .gain
                .map(|gain| param_from_wire(gain, identity))
                .transpose()?,
            resource: source.resource,
            source_start: RationalTime::from_exact(source.source_start),
            rate: RationalRate::from_exact(source.rate)
                .map_err(|_| DocumentConversionError::RationalRate)?,
            end_behavior: media_end_behavior_from_wire(source.end_behavior),
            sampling: RasterSampling {
                fit: raster_fit_from_wire(source.sampling.fit),
            },
        })),
        wire::VisualSourceWire::Image(source) => Ok(VisualSource::Image(ImageSource {
            resource: source.resource,
            sampling: RasterSampling {
                fit: raster_fit_from_wire(source.sampling.fit),
            },
        })),
        wire::VisualSourceWire::Lottie(source) => Ok(VisualSource::Lottie(LottieSource {
            resource: source.resource,
            source_start: RationalTime::from_exact(source.source_start),
            rate: RationalRate::from_exact(source.rate)
                .map_err(|_| DocumentConversionError::RationalRate)?,
            end_behavior: media_end_behavior_from_wire(source.end_behavior),
            sampling: LottieSampling {
                fit: raster_fit_from_wire(source.sampling.fit),
            },
        })),
        wire::VisualSourceWire::Motion(source) => Ok(VisualSource::Motion(MotionInstance {
            component: source.component,
            source_start: RationalTime::from_exact(source.source_start),
            source_duration: RationalTime::from_exact(source.source_duration),
            rate: RationalRate::from_exact(source.rate)
                .map_err(|_| DocumentConversionError::RationalRate)?,
            end_behavior: media_end_behavior_from_wire(source.end_behavior),
            props: source
                .props
                .into_iter()
                .map(|(name, param)| Ok((name, param_from_wire(param, identity)?)))
                .collect::<Result<_, DocumentConversionError>>()?,
            cues: source
                .cues
                .into_iter()
                .map(|(name, cue)| (name, motion_cue_from_wire(cue)))
                .collect(),
            resources: source.resources,
            phases: MotionPhaseOverrides {
                enter_duration: source.phases.enter_duration.map(RationalTime::from_exact),
                exit_duration: source.phases.exit_duration.map(RationalTime::from_exact),
            },
        })),
        wire::VisualSourceWire::Solid(source) => Ok(VisualSource::Solid(SolidSource {
            color: source.color,
        })),
    }
}

fn visual_source_into_wire(source: VisualSource) -> wire::VisualSourceWire {
    match source {
        VisualSource::Video(source) => wire::VisualSourceWire::Video(wire::VideoSourceWire {
            gain: source
                .gain
                .map(|gain| param_into_wire(gain, std::convert::identity)),
            resource: source.resource,
            source_start: source.source_start.into_exact(),
            rate: source.rate.into_exact(),
            end_behavior: media_end_behavior_into_wire(source.end_behavior),
            sampling: wire::RasterSamplingWire {
                fit: raster_fit_into_wire(source.sampling.fit),
            },
        }),
        VisualSource::Image(source) => wire::VisualSourceWire::Image(wire::ImageSourceWire {
            resource: source.resource,
            sampling: wire::RasterSamplingWire {
                fit: raster_fit_into_wire(source.sampling.fit),
            },
        }),
        VisualSource::Lottie(source) => wire::VisualSourceWire::Lottie(wire::LottieSourceWire {
            resource: source.resource,
            source_start: source.source_start.into_exact(),
            rate: source.rate.into_exact(),
            end_behavior: media_end_behavior_into_wire(source.end_behavior),
            sampling: wire::LottieSamplingWire {
                fit: raster_fit_into_wire(source.sampling.fit),
            },
        }),
        VisualSource::Motion(source) => wire::VisualSourceWire::Motion(wire::MotionInstanceWire {
            component: source.component,
            source_start: source.source_start.into_exact(),
            source_duration: source.source_duration.into_exact(),
            rate: source.rate.into_exact(),
            end_behavior: media_end_behavior_into_wire(source.end_behavior),
            props: source
                .props
                .into_iter()
                .map(|(name, param)| (name, param_into_wire(param, std::convert::identity)))
                .collect(),
            cues: source
                .cues
                .into_iter()
                .map(|(name, cue)| (name, motion_cue_into_wire(cue)))
                .collect(),
            resources: source.resources,
            phases: wire::MotionPhaseOverridesWire {
                enter_duration: source.phases.enter_duration.map(RationalTime::into_exact),
                exit_duration: source.phases.exit_duration.map(RationalTime::into_exact),
            },
        }),
        VisualSource::Solid(source) => wire::VisualSourceWire::Solid(wire::SolidSourceWire {
            color: source.color,
        }),
    }
}

fn motion_cue_from_wire(cue: wire::MotionCueBindingWire) -> MotionCueBinding {
    match cue {
        wire::MotionCueBindingWire::SourceRange {
            start,
            end,
            enter_duration,
            exit_duration,
        } => MotionCueBinding::SourceRange {
            start: RationalTime::from_exact(start),
            end: RationalTime::from_exact(end),
            enter_duration: RationalTime::from_exact(enter_duration),
            exit_duration: RationalTime::from_exact(exit_duration),
        },
    }
}

fn motion_cue_into_wire(cue: MotionCueBinding) -> wire::MotionCueBindingWire {
    match cue {
        MotionCueBinding::SourceRange {
            start,
            end,
            enter_duration,
            exit_duration,
        } => wire::MotionCueBindingWire::SourceRange {
            start: start.into_exact(),
            end: end.into_exact(),
            enter_duration: enter_duration.into_exact(),
            exit_duration: exit_duration.into_exact(),
        },
    }
}

fn media_end_behavior_from_wire(value: wire::MediaEndBehaviorWire) -> MediaEndBehavior {
    match value {
        wire::MediaEndBehaviorWire::Error => MediaEndBehavior::Error,
        wire::MediaEndBehaviorWire::Hold => MediaEndBehavior::Hold,
        wire::MediaEndBehaviorWire::Loop => MediaEndBehavior::Loop,
    }
}

fn media_end_behavior_into_wire(value: MediaEndBehavior) -> wire::MediaEndBehaviorWire {
    match value {
        MediaEndBehavior::Error => wire::MediaEndBehaviorWire::Error,
        MediaEndBehavior::Hold => wire::MediaEndBehaviorWire::Hold,
        MediaEndBehavior::Loop => wire::MediaEndBehaviorWire::Loop,
    }
}

fn raster_fit_from_wire(value: wire::RasterFitWire) -> RasterFit {
    match value {
        wire::RasterFitWire::Contain => RasterFit::Contain,
        wire::RasterFitWire::Cover => RasterFit::Cover,
        wire::RasterFitWire::Fill => RasterFit::Fill,
        wire::RasterFitWire::None => RasterFit::None,
    }
}

fn raster_fit_into_wire(value: RasterFit) -> wire::RasterFitWire {
    match value {
        RasterFit::Contain => wire::RasterFitWire::Contain,
        RasterFit::Cover => wire::RasterFitWire::Cover,
        RasterFit::Fill => wire::RasterFitWire::Fill,
        RasterFit::None => wire::RasterFitWire::None,
    }
}

fn audio_composition_from_wire(
    audio: wire::AudioCompositionWire,
) -> Result<AudioComposition, DocumentConversionError> {
    Ok(AudioComposition {
        tracks: audio
            .tracks
            .into_iter()
            .map(|track| {
                Ok(AudioTrack {
                    id: track.id,
                    items: track
                        .items
                        .into_iter()
                        .map(audio_item_from_wire)
                        .collect::<Result<_, _>>()?,
                })
            })
            .collect::<Result<_, DocumentConversionError>>()?,
    })
}

fn audio_composition_into_wire(audio: AudioComposition) -> wire::AudioCompositionWire {
    wire::AudioCompositionWire {
        tracks: audio
            .tracks
            .into_iter()
            .map(|track| wire::AudioTrackWire {
                id: track.id,
                items: track.items.into_iter().map(audio_item_into_wire).collect(),
            })
            .collect(),
    }
}

fn audio_item_from_wire(item: wire::AudioItemWire) -> Result<AudioItem, DocumentConversionError> {
    match item {
        wire::AudioItemWire::Clip(clip) => Ok(AudioItem::Clip(AudioClip {
            id: clip.id,
            duration: RationalTime::from_exact(clip.duration),
            source: audio_source_from_wire(clip.source)?,
            gain: param_from_wire(clip.gain, identity)?,
            pan: param_from_wire(clip.pan, identity)?,
            effects: clip
                .effects
                .into_iter()
                .map(|effect| {
                    Ok(AudioEffect {
                        id: effect.id,
                        kind: kernel_type_from_wire(effect.kind)?,
                        parameters: effect.parameters,
                    })
                })
                .collect::<Result<_, DocumentConversionError>>()?,
        })),
        wire::AudioItemWire::Gap(gap) => Ok(AudioItem::Gap(gap_from_wire(gap))),
        wire::AudioItemWire::Crossfade(crossfade) => Ok(AudioItem::Crossfade(AudioCrossfade {
            id: crossfade.id,
            duration: RationalTime::from_exact(crossfade.duration),
        })),
    }
}

fn audio_item_into_wire(item: AudioItem) -> wire::AudioItemWire {
    match item {
        AudioItem::Clip(clip) => wire::AudioItemWire::Clip(wire::AudioClipWire {
            id: clip.id,
            duration: clip.duration.into_exact(),
            source: audio_source_into_wire(clip.source),
            gain: param_into_wire(clip.gain, std::convert::identity),
            pan: param_into_wire(clip.pan, std::convert::identity),
            effects: clip
                .effects
                .into_iter()
                .map(|effect| wire::AudioEffectWire {
                    id: effect.id,
                    kind: kernel_type_into_wire(effect.kind),
                    parameters: effect.parameters,
                })
                .collect(),
        }),
        AudioItem::Gap(gap) => wire::AudioItemWire::Gap(gap_into_wire(gap)),
        AudioItem::Crossfade(crossfade) => {
            wire::AudioItemWire::Crossfade(wire::AudioCrossfadeWire {
                id: crossfade.id,
                duration: crossfade.duration.into_exact(),
            })
        }
    }
}

fn audio_source_from_wire(
    source: wire::AudioSourceWire,
) -> Result<AudioSource, DocumentConversionError> {
    match source {
        wire::AudioSourceWire::Media(source) => Ok(AudioSource::Media(AudioMediaSource {
            resource: source.resource,
            source_start: RationalTime::from_exact(source.source_start),
            rate: RationalRate::from_exact(source.rate)
                .map_err(|_| DocumentConversionError::RationalRate)?,
            end_behavior: media_end_behavior_from_wire(source.end_behavior),
        })),
    }
}

fn audio_source_into_wire(source: AudioSource) -> wire::AudioSourceWire {
    match source {
        AudioSource::Media(source) => wire::AudioSourceWire::Media(wire::AudioMediaSourceWire {
            resource: source.resource,
            source_start: source.source_start.into_exact(),
            rate: source.rate.into_exact(),
            end_behavior: media_end_behavior_into_wire(source.end_behavior),
        }),
    }
}

fn adjustment_from_wire(
    effect: wire::TimedAdjustmentWire,
) -> Result<TimedAdjustment, DocumentConversionError> {
    Ok(TimedAdjustment {
        id: effect.id,
        start: RationalTime::from_exact(effect.start),
        duration: RationalTime::from_exact(effect.duration),
        effect: match effect.effect {
            wire::AdjustmentEffectWire::ColorGrade(grade) => {
                AdjustmentEffect::ColorGrade(ColorGradeEffect {
                    temperature: grade.temperature,
                })
            }
            wire::AdjustmentEffectWire::Extension(extension) => {
                AdjustmentEffect::Extension(AdjustmentEffectExtension {
                    kind: kernel_type_from_wire(extension.kind)?,
                    parameters: extension.parameters,
                })
            }
        },
    })
}

fn adjustment_into_wire(effect: TimedAdjustment) -> wire::TimedAdjustmentWire {
    wire::TimedAdjustmentWire {
        id: effect.id,
        start: effect.start.into_exact(),
        duration: effect.duration.into_exact(),
        effect: match effect.effect {
            AdjustmentEffect::ColorGrade(grade) => {
                wire::AdjustmentEffectWire::ColorGrade(wire::ColorGradeEffectWire {
                    kind: wire::ColorGradeEffectTag::ColorGrade,
                    temperature: grade.temperature,
                })
            }
            AdjustmentEffect::Extension(extension) => {
                wire::AdjustmentEffectWire::Extension(wire::AdjustmentEffectExtensionWire {
                    kind: kernel_type_into_wire(extension.kind),
                    parameters: extension.parameters,
                })
            }
        },
    }
}

fn caption_composition_from_wire(
    captions: wire::CaptionCompositionWire,
) -> Result<CaptionComposition, DocumentConversionError> {
    Ok(CaptionComposition {
        tracks: captions
            .tracks
            .into_iter()
            .map(|track| {
                Ok(CaptionTrack {
                    id: track.id,
                    items: track
                        .items
                        .into_iter()
                        .map(caption_item_from_wire)
                        .collect::<Result<_, _>>()?,
                })
            })
            .collect::<Result<_, DocumentConversionError>>()?,
    })
}

fn caption_composition_into_wire(captions: CaptionComposition) -> wire::CaptionCompositionWire {
    wire::CaptionCompositionWire {
        tracks: captions
            .tracks
            .into_iter()
            .map(|track| wire::CaptionTrackWire {
                id: track.id,
                items: track
                    .items
                    .into_iter()
                    .map(caption_item_into_wire)
                    .collect(),
            })
            .collect(),
    }
}

fn caption_item_from_wire(
    item: wire::CaptionItemWire,
) -> Result<CaptionItem, DocumentConversionError> {
    match item {
        wire::CaptionItemWire::Clip(caption) => Ok(CaptionItem::Clip(Caption {
            id: caption.id,
            duration: RationalTime::from_exact(caption.duration),
            runs: caption
                .runs
                .into_iter()
                .map(|run| TextRun {
                    id: run.id,
                    text: run.text,
                    timing: run.timing.map(|timing| TextRunTiming {
                        start: RationalTime::from_exact(timing.start),
                        end: RationalTime::from_exact(timing.end),
                    }),
                    style: run.style.map(|style| TextRunStyle {
                        font_size: style.font_size,
                        color: style.color,
                    }),
                })
                .collect(),
            style: CaptionStyle {
                font: caption.style.font,
                font_size: caption.style.font_size,
                color: caption.style.color,
                shadow: caption.style.shadow.map(|shadow| CaptionShadow {
                    color: shadow.color,
                    offset: shadow.offset,
                    blur_sigma: shadow.blur_sigma,
                }),
            },
            layout: CaptionLayout {
                region: NormalizedRect::new(caption.layout.region)
                    .map_err(|_| DocumentConversionError::NormalizedRect)?,
                align: caption_align_from_wire(caption.layout.align),
            },
            presentation: CaptionPresentation {
                opacity: param_from_wire(caption.presentation.opacity, identity)?,
                translation: param_from_wire(caption.presentation.translation, identity)?,
                scale: param_from_wire(caption.presentation.scale, identity)?,
                rotation: param_from_wire(caption.presentation.rotation, identity)?,
                clip_inset: param_from_wire(caption.presentation.clip_inset, identity)?,
                blur_sigma: param_from_wire(caption.presentation.blur_sigma, identity)?,
            },
            behavior: caption.behavior.map(caption_behavior_from_wire),
        })),
        wire::CaptionItemWire::Gap(gap) => Ok(CaptionItem::Gap(gap_from_wire(gap))),
    }
}

fn caption_item_into_wire(item: CaptionItem) -> wire::CaptionItemWire {
    match item {
        CaptionItem::Clip(caption) => wire::CaptionItemWire::Clip(wire::CaptionWire {
            id: caption.id,
            duration: caption.duration.into_exact(),
            runs: caption
                .runs
                .into_iter()
                .map(|run| wire::TextRunWire {
                    id: run.id,
                    text: run.text,
                    timing: run.timing.map(|timing| wire::TextRunTimingWire {
                        start: timing.start.into_exact(),
                        end: timing.end.into_exact(),
                    }),
                    style: run.style.map(|style| wire::TextRunStyleWire {
                        font_size: style.font_size,
                        color: style.color,
                    }),
                })
                .collect(),
            style: wire::CaptionStyleWire {
                font: caption.style.font,
                font_size: caption.style.font_size,
                color: caption.style.color,
                shadow: caption.style.shadow.map(|shadow| wire::CaptionShadowWire {
                    color: shadow.color,
                    offset: shadow.offset,
                    blur_sigma: shadow.blur_sigma,
                }),
            },
            layout: wire::CaptionLayoutWire {
                region: caption.layout.region.into_array(),
                align: caption_align_into_wire(caption.layout.align),
            },
            presentation: wire::CaptionPresentationWire {
                opacity: param_into_wire(caption.presentation.opacity, identity_into),
                translation: param_into_wire(caption.presentation.translation, identity_into),
                scale: param_into_wire(caption.presentation.scale, identity_into),
                rotation: param_into_wire(caption.presentation.rotation, identity_into),
                clip_inset: param_into_wire(caption.presentation.clip_inset, identity_into),
                blur_sigma: param_into_wire(caption.presentation.blur_sigma, identity_into),
            },
            behavior: caption.behavior.map(caption_behavior_into_wire),
        }),
        CaptionItem::Gap(gap) => wire::CaptionItemWire::Gap(gap_into_wire(gap)),
    }
}

fn caption_align_from_wire(value: wire::CaptionAlignWire) -> CaptionAlign {
    match value {
        wire::CaptionAlignWire::TopLeft => CaptionAlign::TopLeft,
        wire::CaptionAlignWire::TopCenter => CaptionAlign::TopCenter,
        wire::CaptionAlignWire::TopRight => CaptionAlign::TopRight,
        wire::CaptionAlignWire::CenterLeft => CaptionAlign::CenterLeft,
        wire::CaptionAlignWire::Center => CaptionAlign::Center,
        wire::CaptionAlignWire::CenterRight => CaptionAlign::CenterRight,
        wire::CaptionAlignWire::BottomLeft => CaptionAlign::BottomLeft,
        wire::CaptionAlignWire::BottomCenter => CaptionAlign::BottomCenter,
        wire::CaptionAlignWire::BottomRight => CaptionAlign::BottomRight,
    }
}

fn caption_align_into_wire(value: CaptionAlign) -> wire::CaptionAlignWire {
    match value {
        CaptionAlign::TopLeft => wire::CaptionAlignWire::TopLeft,
        CaptionAlign::TopCenter => wire::CaptionAlignWire::TopCenter,
        CaptionAlign::TopRight => wire::CaptionAlignWire::TopRight,
        CaptionAlign::CenterLeft => wire::CaptionAlignWire::CenterLeft,
        CaptionAlign::Center => wire::CaptionAlignWire::Center,
        CaptionAlign::CenterRight => wire::CaptionAlignWire::CenterRight,
        CaptionAlign::BottomLeft => wire::CaptionAlignWire::BottomLeft,
        CaptionAlign::BottomCenter => wire::CaptionAlignWire::BottomCenter,
        CaptionAlign::BottomRight => wire::CaptionAlignWire::BottomRight,
    }
}

fn caption_behavior_from_wire(value: wire::CaptionBehaviorWire) -> CaptionBehavior {
    match value {
        wire::CaptionBehaviorWire::Scroll { axis, speed } => CaptionBehavior::Scroll {
            axis: scroll_axis_from_wire(axis),
            speed,
        },
        wire::CaptionBehaviorWire::Karaoke { mode } => CaptionBehavior::Karaoke {
            mode: karaoke_mode_from_wire(mode),
        },
    }
}

fn caption_behavior_into_wire(value: CaptionBehavior) -> wire::CaptionBehaviorWire {
    match value {
        CaptionBehavior::Scroll { axis, speed } => wire::CaptionBehaviorWire::Scroll {
            axis: scroll_axis_into_wire(axis),
            speed,
        },
        CaptionBehavior::Karaoke { mode } => wire::CaptionBehaviorWire::Karaoke {
            mode: karaoke_mode_into_wire(mode),
        },
    }
}

fn scroll_axis_from_wire(value: wire::ScrollAxisWire) -> ScrollAxis {
    match value {
        wire::ScrollAxisWire::Horizontal => ScrollAxis::Horizontal,
        wire::ScrollAxisWire::Vertical => ScrollAxis::Vertical,
    }
}

fn scroll_axis_into_wire(value: ScrollAxis) -> wire::ScrollAxisWire {
    match value {
        ScrollAxis::Horizontal => wire::ScrollAxisWire::Horizontal,
        ScrollAxis::Vertical => wire::ScrollAxisWire::Vertical,
    }
}

fn karaoke_mode_from_wire(value: wire::KaraokeModeWire) -> KaraokeMode {
    match value {
        wire::KaraokeModeWire::Word => KaraokeMode::Word,
        wire::KaraokeModeWire::Line => KaraokeMode::Line,
    }
}

fn karaoke_mode_into_wire(value: KaraokeMode) -> wire::KaraokeModeWire {
    match value {
        KaraokeMode::Word => wire::KaraokeModeWire::Word,
        KaraokeMode::Line => wire::KaraokeModeWire::Line,
    }
}

fn camera_from_wire(camera: wire::CameraTrackWire) -> Result<CameraTrack, DocumentConversionError> {
    Ok(CameraTrack {
        center_x: param_from_wire(camera.center_x, identity)?,
        center_y: param_from_wire(camera.center_y, identity)?,
        zoom: param_from_wire(camera.zoom, identity)?,
        rotation: param_from_wire(camera.rotation, identity)?,
    })
}

fn camera_into_wire(camera: CameraTrack) -> wire::CameraTrackWire {
    wire::CameraTrackWire {
        center_x: param_into_wire(camera.center_x, std::convert::identity),
        center_y: param_into_wire(camera.center_y, std::convert::identity),
        zoom: param_into_wire(camera.zoom, std::convert::identity),
        rotation: param_into_wire(camera.rotation, std::convert::identity),
    }
}
