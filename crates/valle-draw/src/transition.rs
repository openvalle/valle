//! Built-in two-input transitions shared by Timeline and Motion.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum TransitionKind {
    Fade,
    WipeLeft,
    WipeRight,
    CircleOpen,
    SimpleZoom,
    CrossWarp,
    LinearBlur,
    DirectionalWarp,
    DreamyZoom,
    Ripple,
    FlyEye,
    MultiplyBlend,
    Perlin,
}

impl TransitionKind {
    pub const ALL: [Self; 13] = [
        Self::Fade,
        Self::WipeLeft,
        Self::WipeRight,
        Self::CircleOpen,
        Self::SimpleZoom,
        Self::CrossWarp,
        Self::LinearBlur,
        Self::DirectionalWarp,
        Self::DreamyZoom,
        Self::Ripple,
        Self::FlyEye,
        Self::MultiplyBlend,
        Self::Perlin,
    ];

    /// Conservative static bounds including child normalization and bilinear texture reads.
    pub const fn work_per_pixel(self) -> crate::requirements::ShaderWork {
        let samples = match self {
            Self::LinearBlur => 72,
            Self::FlyEye => 4,
            _ => 2,
        };
        crate::requirements::ShaderWork {
            operations: 256 + samples * 32,
            samples,
        }
    }

    pub const fn source(self) -> &'static str {
        match self {
            Self::Fade => include_str!("../assets/shaders/fade.sksl"),
            Self::WipeLeft => include_str!("../assets/shaders/wipeleft.sksl"),
            Self::WipeRight => include_str!("../assets/shaders/wiperight.sksl"),
            Self::CircleOpen => include_str!("../assets/shaders/circleopen.sksl"),
            Self::SimpleZoom => include_str!("../assets/shaders/simplezoom.sksl"),
            Self::CrossWarp => include_str!("../assets/shaders/crosswarp.sksl"),
            Self::LinearBlur => include_str!("../assets/shaders/linearblur.sksl"),
            Self::DirectionalWarp => include_str!("../assets/shaders/directionalwarp.sksl"),
            Self::DreamyZoom => include_str!("../assets/shaders/dreamyzoom.sksl"),
            Self::Ripple => include_str!("../assets/shaders/ripple.sksl"),
            Self::FlyEye => include_str!("../assets/shaders/flyeye.sksl"),
            Self::MultiplyBlend => include_str!("../assets/shaders/multiplyblend.sksl"),
            Self::Perlin => include_str!("../assets/shaders/perlin.sksl"),
        }
    }
}

/// Author parameters are resolved once for a Timeline transition, or per frame for Motion.
pub type TransitionParams = std::collections::BTreeMap<String, f64>;

/// Four canonical scalar slots, in the order declared by `parameter_specs`. Unused slots are zero.
/// The same layout is consumed by the reference executor, native Skia and CanvasKit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(transparent)]
pub struct TransitionValues(pub [f32; 4]);

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct TransitionParameterSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub default: f32,
    pub min: f32,
    pub max: f32,
    pub step: f32,
}

const fn parameter(
    name: &'static str,
    label: &'static str,
    default: f32,
    min: f32,
    max: f32,
    step: f32,
) -> TransitionParameterSpec {
    TransitionParameterSpec {
        name,
        label,
        default,
        min,
        max,
        step,
    }
}
const CENTER_X: TransitionParameterSpec = parameter("centerX", "Center X", 0.5, 0.0, 1.0, 0.01);
const CENTER_Y: TransitionParameterSpec = parameter("centerY", "Center Y", 0.5, 0.0, 1.0, 0.01);

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("transition parameter `{parameter}`: {reason}")]
pub struct TransitionParamError {
    pub parameter: String,
    pub reason: String,
}
impl TransitionParamError {
    fn new(parameter: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            parameter: parameter.into(),
            reason: reason.into(),
        }
    }
}

impl TransitionKind {
    pub const fn parameter_specs(self) -> &'static [TransitionParameterSpec] {
        match self {
            Self::Fade => const { &[] },
            Self::WipeLeft | Self::WipeRight => {
                const { &[parameter("softness", "Edge softness", 0.0, 0.0, 1.0, 0.01)] }
            }
            Self::CircleOpen => {
                const {
                    &[
                        CENTER_X,
                        CENTER_Y,
                        parameter("softness", "Edge softness (px)", 0.5, 0.0, 100.0, 0.5),
                    ]
                }
            }
            Self::SimpleZoom => {
                const {
                    &[
                        parameter("quickness", "Zoom completion", 0.8, 0.2, 1.0, 0.01),
                        CENTER_X,
                        CENTER_Y,
                    ]
                }
            }
            Self::CrossWarp => const { &[CENTER_X, CENTER_Y] },
            Self::LinearBlur => {
                const {
                    &[parameter(
                        "intensity",
                        "Blur intensity",
                        0.1,
                        0.0,
                        2.0,
                        0.01,
                    )]
                }
            }
            Self::DirectionalWarp => {
                const {
                    &[
                        parameter("directionX", "Direction X", -1.0, -1.0, 1.0, 0.1),
                        parameter("directionY", "Direction Y", 1.0, -1.0, 1.0, 0.1),
                        parameter("softness", "Edge softness", 0.5, 0.001, 1.0, 0.01),
                    ]
                }
            }
            Self::DreamyZoom => {
                const {
                    &[
                        parameter("strength", "Zoom strength", 0.5, 0.0, 4.0, 0.05),
                        CENTER_X,
                        CENTER_Y,
                    ]
                }
            }
            Self::Ripple => {
                const {
                    &[
                        parameter("frequency", "Wave frequency", 100.0, 0.0, 256.0, 1.0),
                        parameter("speed", "Wave speed", 50.0, 0.0, 256.0, 1.0),
                        parameter("amplitude", "Displacement", 1.0 / 30.0, 0.0, 1.0, 0.01),
                    ]
                }
            }
            Self::FlyEye => {
                const {
                    &[
                        parameter("size", "Displacement", 0.04, 0.0, 1.0, 0.01),
                        parameter("frequency", "Grid frequency", 50.0, 0.0, 256.0, 1.0),
                        parameter("colorSeparation", "Color separation", 0.3, 0.0, 1.0, 0.01),
                    ]
                }
            }
            Self::MultiplyBlend => {
                const {
                    &[parameter(
                        "midpoint",
                        "Multiply midpoint",
                        0.5,
                        0.001,
                        0.999,
                        0.01,
                    )]
                }
            }
            Self::Perlin => {
                const {
                    &[
                        parameter("scale", "Noise scale", 5.0, 0.001, 128.0, 0.5),
                        parameter("smoothness", "Reveal softness", 0.1, 0.001, 1.0, 0.01),
                    ]
                }
            }
        }
    }

    pub const fn default_values(self) -> TransitionValues {
        let specs = self.parameter_specs();
        let mut values = [0.0; 4];
        let mut index = 0;
        while index < specs.len() {
            values[index] = specs[index].default;
            index += 1;
        }
        TransitionValues(values)
    }

    pub fn resolve_params(
        self,
        params: &TransitionParams,
    ) -> Result<TransitionValues, TransitionParamError> {
        let specs = self.parameter_specs();
        let mut values = self.default_values();
        for (name, value) in params {
            let Some(index) = specs.iter().position(|spec| spec.name == name) else {
                return Err(TransitionParamError::new(
                    name,
                    format!(
                        "unsupported for {self:?}; available: {}",
                        specs.iter().map(|s| s.name).collect::<Vec<_>>().join(", ")
                    ),
                ));
            };
            // Validate the representable execution value, without saturating or truncating it.
            values.0[index] = *value as f32;
        }
        self.validate_values(values)?;
        Ok(values)
    }

    pub fn validate_values(self, values: TransitionValues) -> Result<(), TransitionParamError> {
        let specs = self.parameter_specs();
        for (index, value) in values.0.into_iter().enumerate() {
            if let Some(spec) = specs.get(index) {
                if !value.is_finite() || value < spec.min || value > spec.max {
                    return Err(TransitionParamError::new(
                        spec.name,
                        format!(
                            "value {value} must be finite and in [{}, {}]",
                            spec.min, spec.max
                        ),
                    ));
                }
            } else if value != 0.0 {
                return Err(TransitionParamError::new(
                    index.to_string(),
                    "unused slot must be zero",
                ));
            }
        }
        if self == Self::DirectionalWarp && values.0[0].abs() + values.0[1].abs() < 1.0e-6 {
            return Err(TransitionParamError::new(
                "directionX/directionY",
                "direction must have nonzero length (at least 0.000001)",
            ));
        }
        Ok(())
    }

    /// Canonical named parameters for author/internal serialization; omit default values.
    pub fn named_params(self, values: TransitionValues) -> TransitionParams {
        self.parameter_specs()
            .iter()
            .zip(values.0)
            .filter(|(spec, value)| spec.default != *value)
            .map(|(spec, value)| (spec.name.to_owned(), f64::from(value)))
            .collect()
    }
}

impl TransitionValues {
    pub fn uniforms(self, width: f32, height: f32, progress: f32) -> [f32; 7] {
        [
            width, height, progress, self.0[0], self.0[1], self.0[2], self.0[3],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parameter_table_is_closed_and_defaults_are_canonical() {
        for kind in TransitionKind::ALL {
            let defaults = kind.default_values();
            kind.validate_values(defaults).unwrap();
            assert_eq!(
                kind.resolve_params(&TransitionParams::new()).unwrap(),
                defaults
            );
            let explicit = kind
                .parameter_specs()
                .iter()
                .map(|spec| (spec.name.into(), f64::from(spec.default)))
                .collect();
            assert_eq!(kind.resolve_params(&explicit).unwrap(), defaults);
            assert!(kind.named_params(defaults).is_empty());
            assert!(
                kind.resolve_params(&TransitionParams::from([("unknown".into(), 1.0)]))
                    .is_err()
            );
            for spec in kind.parameter_specs() {
                for value in [
                    f64::NAN,
                    f64::INFINITY,
                    f64::from(spec.min) - 1.0,
                    f64::from(spec.max) + 1.0,
                ] {
                    assert!(
                        kind.resolve_params(&TransitionParams::from([(spec.name.into(), value)]))
                            .is_err(),
                        "{kind:?}/{}={value}",
                        spec.name
                    );
                }
                for value in [spec.min, spec.max] {
                    let input = TransitionParams::from([(spec.name.into(), f64::from(value))]);
                    let resolved = kind.resolve_params(&input).unwrap();
                    assert_eq!(
                        kind.resolve_params(&kind.named_params(resolved)).unwrap(),
                        resolved
                    );
                }
            }
            let mut invalid = defaults;
            invalid.0[3] = 1.0;
            assert!(kind.validate_values(invalid).is_err());
        }
        assert!(
            TransitionKind::DirectionalWarp
                .resolve_params(&TransitionParams::from([
                    ("directionX".into(), 0.0),
                    ("directionY".into(), 0.0),
                ]))
                .is_err()
        );
    }
}
