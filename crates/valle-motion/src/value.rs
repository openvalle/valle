//! Typed values shared by props, expressions, and keyframe tracks. Numeric and geometric types
//! interpolate continuously; bool, string, and enum values use discrete steps. Motion owns this
//! value domain; `PropValue` aliases `MotionValue` because authored defaults and evaluated results
//! share the same representation.

use serde::{Deserialize, Serialize};
use valle_draw::{Point, Rect, Vec2, program::AuthorColor};
use valle_timeline::easing::Easing;

use crate::geometry::PathData;

/// Length unit. Non-pixel units resolve during layout using the containing block or font context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum LengthUnit {
    #[default]
    Px,
    Percent,
    Em,
    Rem,
    Vw,
    Vh,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Length {
    pub value: f64,
    pub unit: LengthUnit,
}

impl Length {
    pub fn px(value: f64) -> Self {
        Length {
            value,
            unit: LengthUnit::Px,
        }
    }

    /// Parse a CSS length. Shared by compilation and property resolution so both consumers
    /// recognize the same units.
    pub fn parse(s: &str) -> Option<Length> {
        let s = s.trim();
        let (num, unit) = if let Some(p) = s.strip_suffix('%') {
            (p, LengthUnit::Percent)
        } else if let Some(p) = s.strip_suffix("px") {
            (p, LengthUnit::Px)
        } else if let Some(p) = s.strip_suffix("rem") {
            (p, LengthUnit::Rem)
        } else if let Some(p) = s.strip_suffix("em") {
            (p, LengthUnit::Em)
        } else if let Some(p) = s.strip_suffix("vw") {
            (p, LengthUnit::Vw)
        } else {
            let p = s.strip_suffix("vh")?;
            (p, LengthUnit::Vh)
        };
        let value: f64 = num.trim().parse().ok()?;
        value.is_finite().then_some(Length { value, unit })
    }
}

/// A pair of lengths with component-wise interpolation and arithmetic. Each component retains its
/// unit; cross-unit conversion requires layout context.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Length2 {
    pub x: Length,
    pub y: Length,
}

impl Length2 {
    pub fn px(x: f64, y: f64) -> Self {
        Length2 {
            x: Length::px(x),
            y: Length::px(y),
        }
    }

    /// Parse exactly two CSS lengths, such as `0px 24px`. A third Z component is unsupported.
    pub fn parse(s: &str) -> Option<Length2> {
        let mut parts = s.split_whitespace();
        let x = Length::parse(parts.next()?)?;
        let y = Length::parse(parts.next()?)?;
        parts.next().is_none().then_some(Length2 { x, y })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum AngleUnit {
    #[default]
    Deg,
    Rad,
    Turn,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Angle {
    pub value: f64,
    pub unit: AngleUnit,
}

impl Angle {
    pub fn deg(value: f64) -> Self {
        Angle {
            value,
            unit: AngleUnit::Deg,
        }
    }

    /// Parse a CSS angle with an explicit degree, turn, or radian unit.
    pub fn parse(s: &str) -> Option<Angle> {
        let s = s.trim();
        let (num, unit) = if let Some(p) = s.strip_suffix("deg") {
            (p, AngleUnit::Deg)
        } else if let Some(p) = s.strip_suffix("rad") {
            (p, AngleUnit::Rad)
        } else {
            let p = s.strip_suffix("turn")?;
            (p, AngleUnit::Turn)
        };
        let value: f64 = num.trim().parse().ok()?;
        value.is_finite().then_some(Angle { value, unit })
    }

    /// Normalize to degrees before interpolation so equivalent angles use the same unit.
    pub fn as_degrees(self) -> f64 {
        match self.unit {
            AngleUnit::Deg => self.value,
            AngleUnit::Rad => self.value.to_degrees(),
            AngleUnit::Turn => self.value * 360.0,
        }
    }
}

/// Value type. Enum variants carry their allowed values for schema validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum PropType {
    Number,
    Length,
    /// Two CSS lengths, such as a translate pair. Unlike a unitless Vec2, each component carries
    /// the unit required by CSS positioning.
    Length2,
    Angle,
    Color,
    Point,
    Vec2,
    Rect,
    PathData,
    Bool,
    Str,
    Enum {
        variants: Vec<String>,
    },
}

impl PropType {
    /// Whether this type supports continuous interpolation.
    pub fn is_continuous(&self) -> bool {
        matches!(
            self,
            PropType::Number
                | PropType::Length
                | PropType::Length2
                | PropType::Angle
                | PropType::Color
                | PropType::Point
                | PropType::Vec2
                | PropType::Rect
                | PropType::PathData
        )
    }
}

/// A typed value. Rect values also carry target bounds for cross-clip locate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "value",
    deny_unknown_fields
)]
pub enum MotionValue {
    Number(f64),
    Length(Length),
    Length2(Length2),
    Angle(Angle),
    Color(AuthorColor),
    Point(Point),
    Vec2(Vec2),
    Rect(Rect),
    PathData(PathData),
    Bool(bool),
    Str(String),
    Enum(String),
}

/// Alias for authored property values, sharing the evaluated value domain.
pub type PropValue = MotionValue;

impl MotionValue {
    /// Exact cache identity, including signed zero. Arithmetic equality alone is insufficient
    /// when the cache promises the same canonical floating-point program bytes.
    pub fn bitwise_eq(&self, other: &Self) -> bool {
        let number = |a: f64, b: f64| a.to_bits() == b.to_bits();
        let point = |a: Point, b: Point| number(a.x, b.x) && number(a.y, b.y);
        let length = |a: Length, b: Length| a.unit == b.unit && number(a.value, b.value);
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => number(*a, *b),
            (Self::Length(a), Self::Length(b)) => length(*a, *b),
            (Self::Length2(a), Self::Length2(b)) => length(a.x, b.x) && length(a.y, b.y),
            (Self::Angle(a), Self::Angle(b)) => a.unit == b.unit && number(a.value, b.value),
            (Self::Color(a), Self::Color(b)) => {
                [a.red, a.green, a.blue, a.alpha].map(f64::to_bits)
                    == [b.red, b.green, b.blue, b.alpha].map(f64::to_bits)
            }
            (Self::Point(a), Self::Point(b)) => point(*a, *b),
            (Self::Vec2(a), Self::Vec2(b)) => number(a.x, b.x) && number(a.y, b.y),
            (Self::Rect(a), Self::Rect(b)) => {
                [a.x, a.y, a.width, a.height].map(f64::to_bits)
                    == [b.x, b.y, b.width, b.height].map(f64::to_bits)
            }
            (Self::PathData(a), Self::PathData(b)) => {
                a.verbs == b.verbs
                    && a.points.len() == b.points.len()
                    && a.points.iter().zip(&b.points).all(|(a, b)| point(*a, *b))
            }
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Str(a), Self::Str(b)) | (Self::Enum(a), Self::Enum(b)) => a == b,
            _ => false,
        }
    }

    pub fn ty(&self) -> PropType {
        match self {
            MotionValue::Number(_) => PropType::Number,
            MotionValue::Length(_) => PropType::Length,
            MotionValue::Length2(_) => PropType::Length2,
            MotionValue::Angle(_) => PropType::Angle,
            MotionValue::Color(_) => PropType::Color,
            MotionValue::Point(_) => PropType::Point,
            MotionValue::Vec2(_) => PropType::Vec2,
            MotionValue::Rect(_) => PropType::Rect,
            MotionValue::PathData(_) => PropType::PathData,
            MotionValue::Bool(_) => PropType::Bool,
            MotionValue::Str(_) => PropType::Str,
            // The type stores allowed enum variants; the value stores only the selected name.
            MotionValue::Enum(_) => PropType::Enum {
                variants: Vec::new(),
            },
        }
    }

    /// Check type compatibility and enum membership. Empty enum lists match by type for
    /// `v.matches(&v.ty())`; schema validation separately rejects empty enum declarations.
    pub fn matches(&self, ty: &PropType) -> bool {
        match (self, ty) {
            (MotionValue::Enum(_), PropType::Enum { variants }) if variants.is_empty() => true,
            (MotionValue::Enum(v), PropType::Enum { variants }) => variants.contains(v),
            (v, t) => core::mem::discriminant(&v.ty()) == core::mem::discriminant(t),
        }
    }

    pub fn is_continuous(&self) -> bool {
        self.ty().is_continuous()
    }

    /// Whether every numeric component is finite. Schema validation rejects NaN and infinity, which
    /// cannot be represented faithfully in JSON or content hashes.
    pub fn is_finite(&self) -> bool {
        match self {
            MotionValue::Number(v) => v.is_finite(),
            MotionValue::Length(l) => l.value.is_finite(),
            MotionValue::Length2(l) => l.x.value.is_finite() && l.y.value.is_finite(),
            MotionValue::Angle(a) => a.value.is_finite(),
            MotionValue::Point(point) => point.x.is_finite() && point.y.is_finite(),
            MotionValue::Vec2(v) => v.x.is_finite() && v.y.is_finite(),
            MotionValue::Rect(r) => {
                r.x.is_finite() && r.y.is_finite() && r.width.is_finite() && r.height.is_finite()
            }
            MotionValue::PathData(path) => path.validate().is_ok(),
            MotionValue::Color(color) => color.is_finite(),
            MotionValue::Bool(_) | MotionValue::Str(_) | MotionValue::Enum(_) => true,
        }
    }
}

/// Convert a typed motion value into the shortest deterministic CSS token.
///
/// This belongs to the value domain rather than the expression evaluator. The Artifact and layout
/// bridge share this formatting path so unit and number serialization stay deterministic.
pub fn css_token(value: &MotionValue) -> String {
    match value {
        MotionValue::Number(value) => number_token(*value),
        MotionValue::Length(value) => length_token(value),
        MotionValue::Length2(value) => {
            format!("{} {}", length_token(&value.x), length_token(&value.y))
        }
        MotionValue::Angle(value) => {
            let unit = match value.unit {
                AngleUnit::Deg => "deg",
                AngleUnit::Rad => "rad",
                AngleUnit::Turn => "turn",
            };
            format!("{}{unit}", number_token(value.value))
        }
        MotionValue::Color(value) => value.to_css(),
        MotionValue::Point(value) => format!("{} {}", number_token(value.x), number_token(value.y)),
        MotionValue::Vec2(value) => format!("{} {}", number_token(value.x), number_token(value.y)),
        MotionValue::Rect(value) => format!(
            "{} {} {} {}",
            number_token(value.x),
            number_token(value.y),
            number_token(value.width),
            number_token(value.height)
        ),
        MotionValue::Bool(value) => value.to_string(),
        MotionValue::Str(value) | MotionValue::Enum(value) => value.clone(),
        MotionValue::PathData(value) => value.to_svg_path(),
    }
}

fn length_token(value: &Length) -> String {
    let unit = match value.unit {
        LengthUnit::Px => "px",
        LengthUnit::Percent => "%",
        LengthUnit::Em => "em",
        LengthUnit::Rem => "rem",
        LengthUnit::Vw => "vw",
        LengthUnit::Vh => "vh",
    };
    format!("{}{unit}", number_token(value.value))
}

fn number_token(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Typed keyframe indexed by frames. Seconds are authoring syntax converted at compilation;
/// negative frames are valid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionKeyframe {
    pub frame: i64,
    pub value: MotionValue,
    /// Easing for the interval to the next keyframe; ignored on the final keyframe.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub easing: Easing,
}

/// Typed keyframe track. Clamp outside the first and last frames. Discrete types use the preceding
/// keyframe value without interpolation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionTrack {
    pub ty: PropType,
    pub keyframes: Vec<MotionKeyframe>,
}

/// Structural track validation errors, independent of rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackError {
    Empty,
    /// Frame indices must strictly increase; duplicate frames have no defined ordering.
    NotMonotonic {
        at: usize,
    },
    /// A keyframe value does not match the declared track type.
    TypeMismatch {
        at: usize,
    },
    /// Non-finite keyframe value. Tracks validate independently of Artifacts.
    NonFinite {
        at: usize,
    },
    InvalidEasing {
        at: usize,
    },
    /// An enum track must declare at least one allowed value.
    EmptyEnum,
    PathTopologyMismatch {
        at: usize,
    },
    PathBudgetExceeded {
        at: usize,
    },
}

impl core::fmt::Display for TrackError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TrackError::Empty => f.write_str("track has no keyframes"),
            TrackError::NotMonotonic { at } => {
                write!(f, "keyframe {at}: frame must strictly increase")
            }
            TrackError::TypeMismatch { at } => write!(f, "keyframe {at}: value type mismatch"),
            TrackError::NonFinite { at } => {
                write!(f, "keyframe {at}: NaN/infinity is not representable")
            }
            TrackError::InvalidEasing { at } => write!(f, "keyframe {at}: invalid easing curve"),
            TrackError::EmptyEnum => f.write_str("track enum type needs at least one variant"),
            TrackError::PathTopologyMismatch { at } => {
                write!(
                    f,
                    "keyframe {at}: PathData topology differs from the first frame"
                )
            }
            TrackError::PathBudgetExceeded { at } => write!(
                f,
                "keyframe {at}: PathData exceeds the per-frame geometry budget"
            ),
        }
    }
}

impl MotionTrack {
    pub fn validate(&self) -> Result<(), TrackError> {
        if self.keyframes.is_empty() {
            return Err(TrackError::Empty);
        }
        if let PropType::Enum { variants } = &self.ty
            && variants.is_empty()
        {
            return Err(TrackError::EmptyEnum);
        }
        for (i, k) in self.keyframes.iter().enumerate() {
            if !k.value.matches(&self.ty) {
                return Err(TrackError::TypeMismatch { at: i });
            }
            if !k.value.is_finite() {
                return Err(TrackError::NonFinite { at: i });
            }
            if !k.easing.is_valid() {
                return Err(TrackError::InvalidEasing { at: i });
            }
            if i > 0 && k.frame <= self.keyframes[i - 1].frame {
                return Err(TrackError::NotMonotonic { at: i });
            }
            if let MotionValue::PathData(path) = &k.value {
                if path.points.len() > crate::geometry::MAX_FRAME_GEOMETRY_POINTS {
                    return Err(TrackError::PathBudgetExceeded { at: i });
                }
                if let Some(MotionValue::PathData(first)) =
                    self.keyframes.first().map(|frame| &frame.value)
                    && !first.has_same_topology(path)
                {
                    return Err(TrackError::PathTopologyMismatch { at: i });
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use valle_timeline::easing::StepPosition;

    use super::*;

    #[test]
    fn continuity_follows_the_type_not_the_caller() {
        assert!(PropType::Number.is_continuous());
        assert!(PropType::Color.is_continuous());
        assert!(PropType::Rect.is_continuous());
        assert!(!PropType::Bool.is_continuous());
        assert!(!PropType::Str.is_continuous());
        assert!(
            !PropType::Enum {
                variants: vec!["a".into()]
            }
            .is_continuous()
        );
    }

    #[test]
    fn enum_values_must_be_in_the_variant_table() {
        let ty = PropType::Enum {
            variants: vec!["left".into(), "right".into()],
        };
        assert!(MotionValue::Enum("left".into()).matches(&ty));
        assert!(!MotionValue::Enum("up".into()).matches(&ty));
        assert!(!MotionValue::Number(1.0).matches(&ty));
    }

    #[test]
    fn angle_units_normalize_to_the_same_degrees() {
        assert_eq!(Angle::deg(360.0).as_degrees(), 360.0);
        assert_eq!(
            Angle {
                value: 1.0,
                unit: AngleUnit::Turn
            }
            .as_degrees(),
            360.0
        );
        let rad = Angle {
            value: core::f64::consts::PI,
            unit: AngleUnit::Rad,
        };
        assert!((rad.as_degrees() - 180.0).abs() < 1e-12);
    }

    #[test]
    fn invalid_easing_parameters_cannot_enter_a_track() {
        for easing in [
            Easing::Steps {
                count: 0,
                position: StepPosition::JumpEnd,
            },
            Easing::CubicBezier {
                p: [-0.1, 0.0, 1.0, 1.0],
            },
        ] {
            let track = MotionTrack {
                ty: PropType::Number,
                keyframes: vec![MotionKeyframe {
                    frame: 0,
                    value: MotionValue::Number(0.0),
                    easing,
                }],
            };
            assert_eq!(track.validate(), Err(TrackError::InvalidEasing { at: 0 }));
        }
    }

    #[test]
    fn track_validation_catches_disorder_and_type_drift() {
        let ok = MotionTrack {
            ty: PropType::Number,
            keyframes: vec![
                MotionKeyframe {
                    frame: 0,
                    value: MotionValue::Number(0.0),
                    easing: Easing::Linear,
                },
                MotionKeyframe {
                    frame: 10,
                    value: MotionValue::Number(1.0),
                    easing: Easing::Linear,
                },
            ],
        };
        assert_eq!(ok.validate(), Ok(()));

        let mut dup = ok.clone();
        dup.keyframes[1].frame = 0;
        assert_eq!(dup.validate(), Err(TrackError::NotMonotonic { at: 1 }));

        let mut drift = ok.clone();
        drift.keyframes[1].value = MotionValue::Bool(true);
        assert_eq!(drift.validate(), Err(TrackError::TypeMismatch { at: 1 }));

        let empty = MotionTrack {
            ty: PropType::Number,
            keyframes: vec![],
        };
        assert_eq!(empty.validate(), Err(TrackError::Empty));
    }

    /// Every value must match its own inferred type, including enum values.
    #[test]
    fn every_value_matches_its_own_type() {
        let values = [
            MotionValue::Number(1.0),
            MotionValue::Length(Length::px(4.0)),
            MotionValue::Length2(Length2::px(4.0, 8.0)),
            MotionValue::Angle(Angle::deg(90.0)),
            MotionValue::Color(AuthorColor::from_srgb8(valle_draw::Rgba::rgb(1, 2, 3))),
            MotionValue::Point(Point::new(1.0, 2.0)),
            MotionValue::Vec2(Vec2::new(1.0, 2.0)),
            MotionValue::Rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
            MotionValue::PathData(
                PathData::new(
                    vec![valle_draw::PathVerb::Move, valle_draw::PathVerb::Line],
                    vec![Point::new(0.0, 0.0), Point::new(1.0, 1.0)],
                )
                .unwrap(),
            ),
            MotionValue::Bool(true),
            MotionValue::Str("x".into()),
            MotionValue::Enum("left".into()),
        ];
        for v in values {
            assert!(v.matches(&v.ty()), "{v:?} must match its own ty()");
        }
    }

    /// Track validation rejects non-finite values and empty enum declarations.
    #[test]
    fn track_rejects_non_finite_and_empty_enum() {
        let mut t = MotionTrack {
            ty: PropType::Number,
            keyframes: vec![MotionKeyframe {
                frame: 0,
                value: MotionValue::Number(f64::NAN),
                easing: Easing::Linear,
            }],
        };
        assert_eq!(t.validate(), Err(TrackError::NonFinite { at: 0 }));

        t.keyframes[0].value = MotionValue::Number(0.0);
        t.keyframes[0].easing = Easing::CubicBezier {
            p: [0.0, f64::INFINITY, 1.0, 1.0],
        };
        assert_eq!(t.validate(), Err(TrackError::InvalidEasing { at: 0 }));

        let empty_enum = MotionTrack {
            ty: PropType::Enum { variants: vec![] },
            keyframes: vec![MotionKeyframe {
                frame: 0,
                value: MotionValue::Enum("x".into()),
                easing: Easing::Linear,
            }],
        };
        assert_eq!(empty_enum.validate(), Err(TrackError::EmptyEnum));
    }

    /// Negative keyframe indices are valid.
    #[test]
    fn negative_frames_are_legal_in_tracks() {
        let t = MotionTrack {
            ty: PropType::Number,
            keyframes: vec![
                MotionKeyframe {
                    frame: -5,
                    value: MotionValue::Number(0.0),
                    easing: Easing::Linear,
                },
                MotionKeyframe {
                    frame: 5,
                    value: MotionValue::Number(1.0),
                    easing: Easing::Linear,
                },
            ],
        };
        assert_eq!(t.validate(), Ok(()));
    }

    #[test]
    fn path_tracks_require_one_topology_within_the_frame_budget() {
        let line = |end_y| {
            PathData::new(
                vec![valle_draw::PathVerb::Move, valle_draw::PathVerb::Line],
                vec![Point::new(0.0, 0.0), Point::new(10.0, end_y)],
            )
            .unwrap()
        };
        let mut track = MotionTrack {
            ty: PropType::PathData,
            keyframes: vec![
                MotionKeyframe {
                    frame: 0,
                    value: MotionValue::PathData(line(0.0)),
                    easing: Easing::Linear,
                },
                MotionKeyframe {
                    frame: 10,
                    value: MotionValue::PathData(line(10.0)),
                    easing: Easing::Linear,
                },
            ],
        };
        assert_eq!(track.validate(), Ok(()));

        track.keyframes[1].value = MotionValue::PathData(
            PathData::new(
                vec![valle_draw::PathVerb::Move, valle_draw::PathVerb::Cubic],
                vec![
                    Point::new(0.0, 0.0),
                    Point::new(2.0, 2.0),
                    Point::new(8.0, 2.0),
                    Point::new(10.0, 0.0),
                ],
            )
            .unwrap(),
        );
        assert_eq!(
            track.validate(),
            Err(TrackError::PathTopologyMismatch { at: 1 })
        );

        let mut verbs = vec![valle_draw::PathVerb::Move];
        verbs.extend(std::iter::repeat_n(
            valle_draw::PathVerb::Line,
            crate::geometry::MAX_FRAME_GEOMETRY_POINTS,
        ));
        let points = (0..=crate::geometry::MAX_FRAME_GEOMETRY_POINTS)
            .map(|x| Point::new(x as f64, 0.0))
            .collect();
        track.keyframes[1].value = MotionValue::PathData(PathData::new(verbs, points).unwrap());
        assert_eq!(
            track.validate(),
            Err(TrackError::PathBudgetExceeded { at: 1 })
        );
    }
}
