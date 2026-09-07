//! Canonical Timeline document wire DTOs.
//!
//! Canonical document fields are deliberately required, including nullable
//! fields. Closed objects reject unknown fields. The only open values are
//! metadata, Motion bindings, and the explicit `extension` kernel variants.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value as JsonValue;

use crate::internal::time::ExactRational;

pub type EntityId = String;
pub type ResourceId = String;
pub type JsonObject = BTreeMap<String, JsonValue>;
pub type Vec2 = [f64; 2];
pub type Rect = [f64; 4];

/// A render-semantic kind owned by an extension namespace.
///
/// Built-ins such as `cross-fade` and `color-grade` are intentionally not
/// accepted here. They have dedicated closed DTOs. Extension kinds use
/// `<namespace>/<name>` (for example `example.visual/glow@1`) and are retained
/// by wire decode for the later capability-admission step.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamespacedKernelTypeWire(String);

impl NamespacedKernelTypeWire {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidNamespacedKernelType> {
        let value = value.into();
        if is_namespaced_kernel_type(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidNamespacedKernelType)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl Serialize for NamespacedKernelTypeWire {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for NamespacedKernelTypeWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("kernel type must use the namespaced `<namespace>/<name>` form")]
pub struct InvalidNamespacedKernelType;

fn is_namespaced_kernel_type(value: &str) -> bool {
    let Some((namespace, name)) = value.split_once('/') else {
        return false;
    };
    !namespace.is_empty()
        && !name.is_empty()
        && !name.contains('/')
        && !value.chars().any(char::is_whitespace)
}

/// Complete canonical Timeline envelope.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineDocumentEnvelopeWire {
    pub document: TimelineDocumentWire,
}

/// Typed root bands. Array order is retained because it is render-semantic.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineDocumentWire {
    pub canvas: CanvasWire,
    pub background: BackgroundWire,
    pub visual: VisualCompositionWire,
    pub audio: AudioCompositionWire,
    pub adjustments: Vec<TimedAdjustmentWire>,
    pub captions: CaptionCompositionWire,
    #[cfg_attr(
        feature = "schema",
        schemars(with = "crate::internal::schema::RequiredNullable<CameraTrackWire>")
    )]
    #[serde(deserialize_with = "required_option")]
    pub camera: Option<CameraTrackWire>,
    pub metadata: JsonObject,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasWire {
    pub width: u32,
    pub height: u32,
    pub fps: ExactRational,
    pub sample_rate: u32,
    pub channel_layout: ChannelLayoutWire,
    pub color_space: ColorSpaceWire,
    pub duration: ExactRational,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelLayoutWire {
    Mono,
    Stereo,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorSpaceWire {
    Srgb,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackgroundWire {
    pub color: String,
}

// ---- Typed property automation ---------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ParamWire<T> {
    Constant(ConstantParamWire<T>),
    Curve(CurveWire<T>),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConstantParamWire<T> {
    pub value: T,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CurveWire<T> {
    pub id: EntityId,
    pub interpolation: InterpolationWire,
    pub keyframes: Vec<KeyframeWire<T>>,
    pub extrapolation: ExtrapolationWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyframeWire<T> {
    pub id: EntityId,
    pub time: ExactRational,
    pub value: T,
    #[cfg_attr(
        feature = "schema",
        schemars(with = "crate::internal::schema::RequiredNullable<EasingWire>")
    )]
    #[serde(deserialize_with = "required_option")]
    pub out_easing: Option<EasingWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InterpolationWire {
    Step,
    Linear,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtrapolationWire {
    Clamp,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EasingWire {
    Named(NamedEasingWire),
    CubicBezier(CubicBezierEasingWire),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NamedEasingWire {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CubicBezierEasingWire {
    #[serde(rename = "type")]
    pub kind: CubicBezierTag,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CubicBezierTag {
    CubicBezier,
}

// ---- Visual ----------------------------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualCompositionWire {
    pub tracks: Vec<VisualTrackWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualTrackWire {
    pub id: EntityId,
    pub items: Vec<VisualItemWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum VisualItemWire {
    Clip(VisualClipWire),
    Gap(GapWire),
    Transition(VisualTransitionWire),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GapWire {
    pub id: EntityId,
    pub duration: ExactRational,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualClipWire {
    pub id: EntityId,
    pub duration: ExactRational,
    pub layer: VisualLayerWire,
    pub source: VisualSourceWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualTransitionWire {
    pub id: EntityId,
    pub duration: ExactRational,
    pub kernel: TransitionKernelWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TransitionKernelWire {
    CrossFade(CrossFadeTransitionKernelWire),
    Extension(TransitionKernelExtensionWire),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CrossFadeTransitionKernelWire {
    #[serde(rename = "type")]
    pub kind: CrossFadeTransitionKernelTag,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CrossFadeTransitionKernelTag {
    CrossFade,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransitionKernelExtensionWire {
    #[serde(rename = "type")]
    pub kind: NamespacedKernelTypeWire,
    pub parameters: JsonObject,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualLayerWire {
    pub transform: LayerTransformWire,
    pub opacity: ParamWire<f64>,
    #[cfg_attr(
        feature = "schema",
        schemars(with = "crate::internal::schema::RequiredNullable<LayerMaskWire>")
    )]
    #[serde(deserialize_with = "required_option")]
    pub mask: Option<LayerMaskWire>,
    pub filters: Vec<VisualFilterWire>,
    pub blend: BlendModeWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayerTransformWire {
    pub position: ParamWire<Vec2>,
    pub scale: ParamWire<Vec2>,
    pub rotation: ParamWire<f64>,
    pub anchor: Vec2,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum LayerMaskWire {
    Rect {
        rect: ParamWire<Rect>,
        feather: ParamWire<f64>,
        invert: bool,
    },
    Ellipse {
        rect: ParamWire<Rect>,
        feather: ParamWire<f64>,
        invert: bool,
    },
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualFilterWire {
    pub id: EntityId,
    #[serde(rename = "type")]
    pub kind: NamespacedKernelTypeWire,
    pub parameters: JsonObject,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlendModeWire {
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

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum VisualSourceWire {
    Video(VideoSourceWire),
    Image(ImageSourceWire),
    Lottie(LottieSourceWire),
    Motion(MotionInstanceWire),
    Solid(SolidSourceWire),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoSourceWire {
    pub resource: ResourceId,
    pub source_start: ExactRational,
    pub rate: ExactRational,
    pub end_behavior: MediaEndBehaviorWire,
    pub sampling: RasterSamplingWire,
}

/// Intentionally has no sourceStart/rate/endBehavior fields.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageSourceWire {
    pub resource: ResourceId,
    pub sampling: RasterSamplingWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LottieSourceWire {
    pub resource: ResourceId,
    pub source_start: ExactRational,
    pub rate: ExactRational,
    pub end_behavior: MediaEndBehaviorWire,
    pub sampling: LottieSamplingWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionInstanceWire {
    pub component: ResourceId,
    pub source_start: ExactRational,
    pub source_duration: ExactRational,
    pub rate: ExactRational,
    pub end_behavior: MediaEndBehaviorWire,
    pub props: BTreeMap<String, ParamWire<JsonValue>>,
    pub cues: BTreeMap<String, MotionCueBindingWire>,
    pub resources: BTreeMap<String, ResourceId>,
    pub phases: MotionPhaseOverridesWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MotionCueBindingWire {
    SourceRange {
        start: ExactRational,
        end: ExactRational,
        enter_duration: ExactRational,
        exit_duration: ExactRational,
    },
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionPhaseOverridesWire {
    #[cfg_attr(
        feature = "schema",
        schemars(with = "crate::internal::schema::RequiredNullable<ExactRational>")
    )]
    #[serde(deserialize_with = "required_option")]
    pub enter_duration: Option<ExactRational>,
    #[cfg_attr(
        feature = "schema",
        schemars(with = "crate::internal::schema::RequiredNullable<ExactRational>")
    )]
    #[serde(deserialize_with = "required_option")]
    pub exit_duration: Option<ExactRational>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SolidSourceWire {
    pub color: String,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaEndBehaviorWire {
    Error,
    Hold,
    Loop,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RasterSamplingWire {
    pub fit: RasterFitWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LottieSamplingWire {
    pub fit: RasterFitWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RasterFitWire {
    Contain,
    Cover,
    Fill,
    None,
}

// ---- Audio -----------------------------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioCompositionWire {
    pub tracks: Vec<AudioTrackWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioTrackWire {
    pub id: EntityId,
    pub items: Vec<AudioItemWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AudioItemWire {
    Clip(AudioClipWire),
    Gap(GapWire),
    Crossfade(AudioCrossfadeWire),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioClipWire {
    pub id: EntityId,
    pub duration: ExactRational,
    pub source: AudioSourceWire,
    pub gain: ParamWire<f64>,
    pub pan: ParamWire<f64>,
    pub effects: Vec<AudioEffectWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioCrossfadeWire {
    pub id: EntityId,
    pub duration: ExactRational,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AudioSourceWire {
    Media(AudioMediaSourceWire),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioMediaSourceWire {
    pub resource: ResourceId,
    pub source_start: ExactRational,
    pub rate: ExactRational,
    pub end_behavior: MediaEndBehaviorWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioEffectWire {
    pub id: EntityId,
    #[serde(rename = "type")]
    pub kind: NamespacedKernelTypeWire,
    pub parameters: JsonObject,
}

// ---- Adjustments --------------------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimedAdjustmentWire {
    pub id: EntityId,
    pub start: ExactRational,
    pub duration: ExactRational,
    pub effect: AdjustmentEffectWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AdjustmentEffectWire {
    ColorGrade(ColorGradeEffectWire),
    Extension(AdjustmentEffectExtensionWire),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColorGradeEffectWire {
    #[serde(rename = "type")]
    pub kind: ColorGradeEffectTag,
    pub temperature: f64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorGradeEffectTag {
    ColorGrade,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdjustmentEffectExtensionWire {
    #[serde(rename = "type")]
    pub kind: NamespacedKernelTypeWire,
    pub parameters: JsonObject,
}

// ---- Captions -------------------------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionCompositionWire {
    pub tracks: Vec<CaptionTrackWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionTrackWire {
    pub id: EntityId,
    pub items: Vec<CaptionItemWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CaptionItemWire {
    Clip(CaptionWire),
    Gap(GapWire),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionWire {
    pub id: EntityId,
    pub duration: ExactRational,
    pub runs: Vec<TextRunWire>,
    pub style: CaptionStyleWire,
    pub layout: CaptionLayoutWire,
    pub presentation: CaptionPresentationWire,
    #[cfg_attr(
        feature = "schema",
        schemars(with = "crate::internal::schema::RequiredNullable<CaptionBehaviorWire>")
    )]
    #[serde(deserialize_with = "required_option")]
    pub behavior: Option<CaptionBehaviorWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextRunWire {
    pub id: EntityId,
    pub text: String,
    #[cfg_attr(
        feature = "schema",
        schemars(with = "crate::internal::schema::RequiredNullable<TextRunTimingWire>")
    )]
    #[serde(deserialize_with = "required_option")]
    pub timing: Option<TextRunTimingWire>,
    #[cfg_attr(
        feature = "schema",
        schemars(with = "crate::internal::schema::RequiredNullable<TextRunStyleWire>")
    )]
    #[serde(deserialize_with = "required_option")]
    pub style: Option<TextRunStyleWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextRunTimingWire {
    pub start: ExactRational,
    pub end: ExactRational,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextRunStyleWire {
    #[cfg_attr(
        feature = "schema",
        schemars(with = "crate::internal::schema::RequiredNullable<f64>")
    )]
    #[serde(deserialize_with = "required_option")]
    pub font_size: Option<f64>,
    #[cfg_attr(
        feature = "schema",
        schemars(with = "crate::internal::schema::RequiredNullable<String>")
    )]
    #[serde(deserialize_with = "required_option")]
    pub color: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionStyleWire {
    pub font: ResourceId,
    pub font_size: f64,
    pub color: String,
    #[cfg_attr(
        feature = "schema",
        schemars(with = "crate::internal::schema::RequiredNullable<CaptionShadowWire>")
    )]
    #[serde(deserialize_with = "required_option")]
    pub shadow: Option<CaptionShadowWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionShadowWire {
    pub color: String,
    pub offset: Vec2,
    pub blur_sigma: f64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionLayoutWire {
    pub region: Rect,
    pub align: CaptionAlignWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionPresentationWire {
    pub opacity: ParamWire<f64>,
    pub translation: ParamWire<Vec2>,
    pub scale: ParamWire<f64>,
    pub rotation: ParamWire<f64>,
    pub clip_inset: ParamWire<[f64; 4]>,
    pub blur_sigma: ParamWire<f64>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptionAlignWire {
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

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CaptionBehaviorWire {
    Scroll { axis: ScrollAxisWire, speed: f64 },
    Karaoke { mode: KaraokeModeWire },
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScrollAxisWire {
    Horizontal,
    Vertical,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KaraokeModeWire {
    Word,
    Line,
}

// ---- Camera ----------------------------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CameraTrackWire {
    pub center_x: ParamWire<f64>,
    pub center_y: ParamWire<f64>,
    pub zoom: ParamWire<f64>,
    pub rotation: ParamWire<f64>,
}

/// Unlike serde's default handling for `Option<T>`, this helper makes the key
/// itself required while still accepting an explicit JSON `null`.
fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
