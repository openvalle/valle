//! Sparse Timeline timeline wire.
//!
//! This is the only Timeline shape intended for Agent, Studio and persistence
//! APIs. Defaults, generated IDs, gaps, preset expansion and execution-only
//! descriptors belong to normalization and never leak back into this wire.

use std::{collections::BTreeMap, fmt};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, DeserializeOwned, MapAccess, Visitor},
    ser,
};
use serde_json::{Value as JsonValue, value::RawValue};

use crate::caption_presets::{
    DisplayCaptionPresetName, EnterCaptionPresetName, ExitCaptionPresetName,
};

/// Open JSON object used only at the explicitly extensible public leaves.
pub type JsonObject = BTreeMap<String, JsonValue>;

// These enums deliberately live beside the sparse public Timeline wire. They
// are not aliases of the render contract: changing an internal enum must not
// silently widen or change the author-facing JSON vocabulary.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InterpolationWire {
    Step,
    Linear,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaEndBehaviorWire {
    Error,
    Hold,
    Loop,
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

/// One complete, sparse Timeline document.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineWire {
    pub canvas: TimelineCanvasWire,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resources: BTreeMap<String, String>,
    pub tracks: TimelineTracksWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineCanvasWire {
    pub width: u32,
    pub height: u32,
    pub fps: TimelineFrameRateWire,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
}

/// Strongly typed track bands. The object itself is required while empty
/// bands are omitted from the serialized author document.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(rename_all = "camelCase", deny_unknown_fields)
)]
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineTracksWire {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visual: Vec<TimelineVisualTrackWire>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio: Vec<TimelineAudioTrackWire>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caption: Vec<TimelineCaptionTrackWire>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjustment: Vec<TimelineAdjustmentTrackWire>,
}

impl<'de> Deserialize<'de> for TimelineTracksWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct TracksVisitor;

        impl<'de> Visitor<'de> for TracksVisitor {
            type Value = TimelineTracksWire;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a Timeline tracks object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut visual = None;
                let mut audio = None;
                let mut caption = None;
                let mut adjustment = None;
                while let Some(field) = map.next_key::<String>()? {
                    match field.as_str() {
                        "visual" => {
                            if visual.is_some() {
                                return Err(de::Error::duplicate_field("visual"));
                            }
                            visual = Some(map.next_value()?);
                        }
                        "audio" => {
                            if audio.is_some() {
                                return Err(de::Error::duplicate_field("audio"));
                            }
                            audio = Some(map.next_value()?);
                        }
                        "caption" => {
                            if caption.is_some() {
                                return Err(de::Error::duplicate_field("caption"));
                            }
                            caption = Some(map.next_value()?);
                        }
                        "adjustment" => {
                            if adjustment.is_some() {
                                return Err(de::Error::duplicate_field("adjustment"));
                            }
                            adjustment = Some(map.next_value()?);
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &field,
                                &["visual", "audio", "caption", "adjustment"],
                            ));
                        }
                    }
                }
                Ok(TimelineTracksWire {
                    visual: visual.unwrap_or_default(),
                    audio: audio.unwrap_or_default(),
                    caption: caption.unwrap_or_default(),
                    adjustment: adjustment.unwrap_or_default(),
                })
            }
        }

        deserializer.deserialize_map(TracksVisitor)
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineVisualTrackWire {
    pub clips: Vec<TimelineVisualClipWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineAudioTrackWire {
    pub clips: Vec<TimelineAudioClipWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineAdjustmentTrackWire {
    pub clips: Vec<TimelineAdjustmentClipWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineCaptionTrackWire {
    pub style: TimelineCaptionStyleWire,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<TimelineCaptionLayoutWire>,
    pub clips: Vec<TimelineCaptionClipWire>,
}

/// A non-negative decimal second value normalized to at most six fractional
/// digits as it crosses the public boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineTimeWire(String);

impl TimelineTimeWire {
    pub fn new(token: impl Into<String>) -> Result<Self, InvalidTimelineTime> {
        let token = token.into();
        let normalized = normalize_timeline_time_token(&token)?;
        exact_from_normalized_time(&normalized).ok_or(InvalidTimelineTime)?;
        Ok(Self(normalized))
    }

    pub fn token(&self) -> &str {
        &self.0
    }

    pub fn to_exact(&self) -> crate::time::ExactRational {
        exact_from_normalized_time(&self.0)
            .expect("TimelineTimeWire stores an ExactRational-compatible q6 decimal")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("time must be a finite non-negative JSON number normalizable to six decimal places")]
pub struct InvalidTimelineTime;

impl Serialize for TimelineTimeWire {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        RawValue::from_string(self.0.clone())
            .map_err(ser::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TimelineTimeWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        Self::new(raw.get()).map_err(de::Error::custom)
    }
}

fn normalize_timeline_time_token(token: &str) -> Result<String, InvalidTimelineTime> {
    if !matches!(
        serde_json::from_str::<JsonValue>(token),
        Ok(JsonValue::Number(_))
    ) {
        return Err(InvalidTimelineTime);
    }
    let (negative, unsigned) = token
        .strip_prefix('-')
        .map_or((false, token), |value| (true, value));
    let (mantissa, exponent) =
        unsigned
            .split_once(['e', 'E'])
            .map_or((unsigned, 0_i64), |(mantissa, exponent)| {
                exponent
                    .parse::<i64>()
                    .map(|exponent| (mantissa, exponent))
                    .unwrap_or((mantissa, i64::MIN))
            });
    if exponent == i64::MIN {
        return Err(InvalidTimelineTime);
    }
    let (integer, fractional) = mantissa
        .split_once('.')
        .map_or((mantissa, ""), |parts| parts);
    let mut digits = String::with_capacity(integer.len() + fractional.len());
    digits.push_str(integer);
    digits.push_str(fractional);
    if negative && digits.bytes().any(|digit| digit != b'0') {
        return Err(InvalidTimelineTime);
    }

    let decimal_position = i64::try_from(integer.len())
        .ok()
        .and_then(|position| position.checked_add(exponent))
        .ok_or(InvalidTimelineTime)?;
    let cut = decimal_position.checked_add(6).ok_or(InvalidTimelineTime)?;
    let micros_wire = if cut <= 0 {
        "0".to_owned()
    } else {
        let cut = usize::try_from(cut).map_err(|_| InvalidTimelineTime)?;
        if cut > 38 {
            return Err(InvalidTimelineTime);
        }
        let mut value = digits
            .get(..digits.len().min(cut))
            .unwrap_or_default()
            .to_owned();
        if cut > digits.len() {
            value.extend(std::iter::repeat_n('0', cut - digits.len()));
        }
        if value.is_empty() {
            value.push('0');
        }
        value
    };
    let round_digit = if cut < 0 {
        b'0'
    } else {
        usize::try_from(cut)
            .ok()
            .and_then(|index| digits.as_bytes().get(index).copied())
            .unwrap_or(b'0')
    };
    let mut micros = micros_wire
        .parse::<i128>()
        .map_err(|_| InvalidTimelineTime)?;
    if round_digit >= b'5' {
        micros = micros.checked_add(1).ok_or(InvalidTimelineTime)?;
    }
    let seconds = micros / 1_000_000;
    let fraction = micros % 1_000_000;
    if fraction == 0 {
        return Ok(seconds.to_string());
    }
    let mut fraction = format!("{fraction:06}");
    while fraction.ends_with('0') {
        fraction.pop();
    }
    Ok(format!("{seconds}.{fraction}"))
}

fn exact_from_normalized_time(token: &str) -> Option<crate::time::ExactRational> {
    let (whole, fraction) = token.split_once('.').unwrap_or((token, ""));
    let denominator = 10_u32.checked_pow(u32::try_from(fraction.len()).ok()?)?;
    let whole = whole.parse::<i64>().ok()?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<i64>().ok()?
    };
    let numerator = whole
        .checked_mul(i64::from(denominator))?
        .checked_add(fraction)?;
    crate::time::ExactRational::new(numerator, denominator).ok()
}

/// Frame rate is the one public time-like field that also accepts an exact
/// canonical rational such as `"30000/1001"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineFrameRateWire {
    Decimal(TimelineTimeWire),
    Rational(String),
}

impl TimelineFrameRateWire {
    pub fn try_to_frame_rate(&self) -> Result<crate::time::FrameRate, crate::time::TimeError> {
        let exact = match self {
            Self::Decimal(value) => value.to_exact(),
            Self::Rational(value) => crate::time::ExactRational::parse_canonical(value)?,
        };
        crate::time::FrameRate::from_exact(exact)
    }

    pub fn to_frame_rate(&self) -> crate::time::FrameRate {
        self.try_to_frame_rate()
            .expect("TimelineFrameRateWire always stores a positive canonical frame rate")
    }
}

impl Serialize for TimelineFrameRateWire {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Decimal(value) => value.serialize(serializer),
            Self::Rational(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for TimelineFrameRateWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        if raw.get().starts_with('"') {
            let value = String::deserialize(&mut serde_json::Deserializer::from_str(raw.get()))
                .map_err(de::Error::custom)?;
            let exact =
                crate::time::ExactRational::parse_canonical(&value).map_err(de::Error::custom)?;
            crate::time::FrameRate::from_exact(exact).map_err(de::Error::custom)?;
            Ok(Self::Rational(value))
        } else {
            let value = TimelineTimeWire::new(raw.get()).map_err(de::Error::custom)?;
            let exact = value.to_exact();
            crate::time::FrameRate::from_exact(exact).map_err(de::Error::custom)?;
            Ok(Self::Decimal(value))
        }
    }
}

/// Constants are written directly. Only animated values use the explicit
/// `{"keyframes":[...]}` object.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TimelineParamWire<T> {
    Curve(TimelineCurveWire<T>),
    Value(T),
}

impl<'de, T> Deserialize<'de> for TimelineParamWire<T>
where
    T: DeserializeOwned,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        let token = raw.get();
        if token.trim_start().starts_with('{') {
            let fields = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(token)
                .map_err(de::Error::custom)?;
            if fields.contains_key("keyframes") {
                return serde_json::from_str::<TimelineCurveWire<T>>(token)
                    .map(Self::Curve)
                    .map_err(de::Error::custom);
            }
        }
        serde_json::from_str::<T>(token)
            .map(Self::Value)
            .map_err(de::Error::custom)
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    bound(deserialize = "T: DeserializeOwned")
)]
pub struct TimelineCurveWire<T> {
    pub keyframes: Vec<TimelineKeyframeWire<T>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpolation: Option<InterpolationWire>,
}

/// Compact `[time, value]` or `[time, value, easing]` keyframe.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TimelineKeyframeWire<T> {
    Plain((TimelineTimeWire, T)),
    Eased((TimelineTimeWire, T, EasingWire)),
}

impl<'de, T> Deserialize<'de> for TimelineKeyframeWire<T>
where
    T: DeserializeOwned,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Vec::<Box<RawValue>>::deserialize(deserializer)?;
        match raw.as_slice() {
            [time, value] => Ok(Self::Plain((
                decode_raw(time).map_err(de::Error::custom)?,
                decode_raw(value).map_err(de::Error::custom)?,
            ))),
            [time, value, easing] => Ok(Self::Eased((
                decode_raw(time).map_err(de::Error::custom)?,
                decode_raw(value).map_err(de::Error::custom)?,
                decode_raw(easing).map_err(de::Error::custom)?,
            ))),
            _ => Err(de::Error::custom(
                "author keyframe must contain exactly two or three items",
            )),
        }
    }
}

impl<T> TimelineKeyframeWire<T> {
    pub fn into_parts(self) -> (TimelineTimeWire, T, Option<EasingWire>) {
        match self {
            Self::Plain((time, value)) => (time, value, None),
            Self::Eased((time, value, easing)) => (time, value, Some(easing)),
        }
    }

    pub fn parts(&self) -> (&TimelineTimeWire, &T, Option<&EasingWire>) {
        match self {
            Self::Plain((time, value)) => (time, value, None),
            Self::Eased((time, value, easing)) => (time, value, Some(easing)),
        }
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineVisualClipWire {
    pub start: TimelineTimeWire,
    pub duration: TimelineTimeWire,
    #[serde(flatten)]
    pub source: TimelineVisualSourceWire,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<TimelineParamWire<[f64; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<TimelineParamWire<[f64; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<TimelineParamWire<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<TimelineParamWire<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<BlendModeWire>,
}

impl<'de> Deserialize<'de> for TimelineVisualClipWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        let mut fields = raw_object(raw.get()).map_err(de::Error::custom)?;
        let start = required_raw(&mut fields, "start").map_err(de::Error::custom)?;
        let duration = required_raw(&mut fields, "duration").map_err(de::Error::custom)?;
        let position = take_raw(&mut fields, "position").map_err(de::Error::custom)?;
        let scale = take_raw(&mut fields, "scale").map_err(de::Error::custom)?;
        let rotation = take_raw(&mut fields, "rotation").map_err(de::Error::custom)?;
        let anchor = take_raw(&mut fields, "anchor").map_err(de::Error::custom)?;
        let opacity = take_raw(&mut fields, "opacity").map_err(de::Error::custom)?;
        let blend = take_raw(&mut fields, "blend").map_err(de::Error::custom)?;
        let source = parse_visual_source(&mut fields).map_err(de::Error::custom)?;
        Ok(Self {
            start,
            duration,
            source,
            position,
            scale,
            rotation,
            anchor,
            opacity,
            blend,
        })
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        tag = "kind",
        rename_all = "kebab-case",
        rename_all_fields = "camelCase",
        deny_unknown_fields
    )
)]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TimelineVisualSourceWire {
    Video {
        src: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trim_start: Option<TimelineTimeWire>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rate: Option<TimelineTimeWire>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<MediaEndBehaviorWire>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fit: Option<RasterFitWire>,
    },
    Image {
        src: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fit: Option<RasterFitWire>,
    },
    Lottie {
        src: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trim_start: Option<TimelineTimeWire>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rate: Option<TimelineTimeWire>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<MediaEndBehaviorWire>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fit: Option<RasterFitWire>,
    },
    Motion {
        component: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trim_start: Option<TimelineTimeWire>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_duration: Option<TimelineTimeWire>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rate: Option<TimelineTimeWire>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<MediaEndBehaviorWire>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        props: BTreeMap<String, TimelineParamWire<JsonValue>>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        cues: BTreeMap<String, TimelineMotionCueBindingWire>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        resources: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phases: Option<TimelineMotionPhaseOverridesWire>,
    },
    Solid {
        color: String,
    },
}

impl<'de> Deserialize<'de> for TimelineVisualSourceWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        let mut fields = raw_object(raw.get()).map_err(de::Error::custom)?;
        parse_visual_source(&mut fields).map_err(de::Error::custom)
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        tag = "type",
        rename_all = "kebab-case",
        rename_all_fields = "camelCase",
        deny_unknown_fields
    )
)]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TimelineMotionCueBindingWire {
    SourceRange {
        start: TimelineTimeWire,
        end: TimelineTimeWire,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enter_duration: Option<TimelineTimeWire>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_duration: Option<TimelineTimeWire>,
    },
}

impl<'de> Deserialize<'de> for TimelineMotionCueBindingWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        let mut fields = raw_object(raw.get()).map_err(de::Error::custom)?;
        let kind: String = required_raw(&mut fields, "type").map_err(de::Error::custom)?;
        let value = match kind.as_str() {
            "source-range" => Self::SourceRange {
                start: required_raw(&mut fields, "start").map_err(de::Error::custom)?,
                end: required_raw(&mut fields, "end").map_err(de::Error::custom)?,
                enter_duration: take_raw(&mut fields, "enterDuration")
                    .map_err(de::Error::custom)?,
                exit_duration: take_raw(&mut fields, "exitDuration").map_err(de::Error::custom)?,
            },
            _ => return Err(de::Error::custom("unknown Timeline author Motion cue type")),
        };
        reject_unknown(fields).map_err(de::Error::custom)?;
        Ok(value)
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineMotionPhaseOverridesWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enter_duration: Option<TimelineTimeWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_duration: Option<TimelineTimeWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        tag = "type",
        rename_all = "kebab-case",
        rename_all_fields = "camelCase",
        deny_unknown_fields
    )
)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TimelineCaptionBehaviorWire {
    Scroll { axis: ScrollAxisWire, speed: f64 },
    Karaoke { mode: KaraokeModeWire },
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineAudioClipWire {
    pub start: TimelineTimeWire,
    pub duration: TimelineTimeWire,
    pub src: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim_start: Option<TimelineTimeWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<TimelineTimeWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<MediaEndBehaviorWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain: Option<TimelineParamWire<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pan: Option<TimelineParamWire<f64>>,
}

/// One absolute-time adjustment clip. Adjustment kinds remain a narrow typed
/// set and lower to internal adjustments; extension parameter bags are not
/// part of the author wire.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineAdjustmentClipWire {
    pub start: TimelineTimeWire,
    pub duration: TimelineTimeWire,
    #[serde(flatten)]
    pub adjustment: TimelineAdjustmentWire,
}

impl<'de> Deserialize<'de> for TimelineAdjustmentClipWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        let mut fields = raw_object(raw.get()).map_err(de::Error::custom)?;
        let start = required_raw(&mut fields, "start").map_err(de::Error::custom)?;
        let duration = required_raw(&mut fields, "duration").map_err(de::Error::custom)?;
        let adjustment = parse_adjustment(&mut fields).map_err(de::Error::custom)?;
        Ok(Self {
            start,
            duration,
            adjustment,
        })
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        tag = "kind",
        rename_all = "kebab-case",
        rename_all_fields = "camelCase",
        deny_unknown_fields
    )
)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TimelineAdjustmentWire {
    ColorGrade { temperature: f64 },
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineCaptionStyleWire {
    pub font: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<TimelineCaptionShadowWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineCaptionShadowWire {
    pub color: String,
    pub offset: [f64; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blur: Option<f64>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineCaptionLayoutWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<[f64; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<CaptionAlignWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineCaptionClipWire {
    pub start: TimelineTimeWire,
    pub duration: TimelineTimeWire,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<Vec<TimelineTextRunWire>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<TimelineCaptionLayoutWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<TimelineCaptionBehaviorWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enter: Option<TimelineEnterPresetWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<TimelineDisplayPresetWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<TimelineExitPresetWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<TimelineCaptionPresentationWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineTextRunWire {
    pub text: String,
    /// Clip-local start of this run's karaoke reveal window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<TimelineTimeWire>,
    /// Clip-local end of this run's karaoke reveal window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<TimelineTimeWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineCaptionPresentationWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<TimelineParamWire<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<TimelineParamWire<[f64; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<TimelineParamWire<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<TimelineParamWire<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_inset: Option<TimelineParamWire<[f64; 4]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blur: Option<TimelineParamWire<f64>>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TimelineEnterPresetWire {
    Name(EnterCaptionPresetName),
    Options(TimelineEnterPresetOptionsWire),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineEnterPresetOptionsWire {
    pub preset: EnterCaptionPresetName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<TimelineTimeWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TimelineDisplayPresetWire {
    Name(DisplayCaptionPresetName),
    Options(TimelineDisplayPresetOptionsWire),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineDisplayPresetOptionsWire {
    pub preset: DisplayCaptionPresetName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TimelineExitPresetWire {
    Name(ExitCaptionPresetName),
    Options(TimelineExitPresetOptionsWire),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineExitPresetOptionsWire {
    pub preset: ExitCaptionPresetName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<TimelineTimeWire>,
}

fn raw_object(raw: &str) -> Result<BTreeMap<String, Box<RawValue>>, String> {
    serde_json::from_str(raw).map_err(|error| error.to_string())
}

fn decode_raw<T: DeserializeOwned>(raw: &RawValue) -> serde_json::Result<T> {
    serde_json::from_str(raw.get())
}

fn take_raw<T: DeserializeOwned>(
    fields: &mut BTreeMap<String, Box<RawValue>>,
    name: &str,
) -> Result<Option<T>, String> {
    fields
        .remove(name)
        .map(|raw| decode_raw(&raw).map_err(|error| error.to_string()))
        .transpose()
}

fn required_raw<T: DeserializeOwned>(
    fields: &mut BTreeMap<String, Box<RawValue>>,
    name: &str,
) -> Result<T, String> {
    take_raw(fields, name)?.ok_or_else(|| format!("missing field `{name}`"))
}

fn reject_unknown(fields: BTreeMap<String, Box<RawValue>>) -> Result<(), String> {
    match fields.keys().next() {
        Some(field) => Err(format!("unknown field `{field}`")),
        None => Ok(()),
    }
}

fn parse_adjustment(
    fields: &mut BTreeMap<String, Box<RawValue>>,
) -> Result<TimelineAdjustmentWire, String> {
    let kind: String = required_raw(fields, "kind")?;
    let adjustment = match kind.as_str() {
        "color-grade" => TimelineAdjustmentWire::ColorGrade {
            temperature: required_raw(fields, "temperature")?,
        },
        _ => return Err(format!("unknown Timeline author adjustment kind `{kind}`")),
    };
    reject_unknown(std::mem::take(fields))?;
    Ok(adjustment)
}

fn parse_visual_source(
    fields: &mut BTreeMap<String, Box<RawValue>>,
) -> Result<TimelineVisualSourceWire, String> {
    let kind: String = required_raw(fields, "kind")?;
    let source = match kind.as_str() {
        "video" => TimelineVisualSourceWire::Video {
            src: required_raw(fields, "src")?,
            trim_start: take_raw(fields, "trimStart")?,
            rate: take_raw(fields, "rate")?,
            end: take_raw(fields, "end")?,
            fit: take_raw(fields, "fit")?,
        },
        "image" => TimelineVisualSourceWire::Image {
            src: required_raw(fields, "src")?,
            fit: take_raw(fields, "fit")?,
        },
        "lottie" => TimelineVisualSourceWire::Lottie {
            src: required_raw(fields, "src")?,
            trim_start: take_raw(fields, "trimStart")?,
            rate: take_raw(fields, "rate")?,
            end: take_raw(fields, "end")?,
            fit: take_raw(fields, "fit")?,
        },
        "motion" => TimelineVisualSourceWire::Motion {
            component: required_raw(fields, "component")?,
            trim_start: take_raw(fields, "trimStart")?,
            source_duration: take_raw(fields, "sourceDuration")?,
            rate: take_raw(fields, "rate")?,
            end: take_raw(fields, "end")?,
            props: take_raw(fields, "props")?.unwrap_or_default(),
            cues: take_raw(fields, "cues")?.unwrap_or_default(),
            resources: take_raw(fields, "resources")?.unwrap_or_default(),
            phases: take_raw(fields, "phases")?,
        },
        "solid" => TimelineVisualSourceWire::Solid {
            color: required_raw(fields, "color")?,
        },
        _ => return Err(format!("unknown Timeline author visual kind `{kind}`")),
    };
    reject_unknown(std::mem::take(fields))?;
    Ok(source)
}

impl fmt::Display for TimelineTimeWire {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
