//! Valle's built-in caption-animation timeline presets.
//!
//! Public selections come from the closed catalog owned by `valle-timeline` and expand into
//! ordinary Timeline typed curves. Preset identifiers and pack metadata never enter
//! Canonical, Engine, or Render contracts.

use std::collections::BTreeSet;

use thiserror::Error;
pub use valle_timeline::caption_presets::{
    CAPTION_PRESET_COUNT, CAPTION_PRESET_PACK_ID, CaptionPreset, CaptionPresetDescriptor,
    CaptionPresetPhase, DisplayCaptionPresetName, EnterCaptionPresetName, ExitCaptionPresetName,
};
use valle_timeline::internal::{
    ExactRational, FrameRate, MAX_CAPTION_BLUR_SIGMA, RationalTime,
    wire::document::{
        CaptionPresentationWire, ConstantParamWire, CubicBezierEasingWire, CubicBezierTag,
        CurveWire, EasingWire, ExtrapolationWire, InterpolationWire, KeyframeWire, NamedEasingWire,
        ParamWire,
    },
};

pub const MAX_CAPTION_PRESET_SAMPLES: usize = 16_384;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedCaptionPreset {
    pub preset: CaptionPreset,
    pub duration: RationalTime,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayCaptionPreset {
    pub preset: CaptionPreset,
    pub rate: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptionPresetRequest {
    pub owner_id: String,
    pub caption_duration: RationalTime,
    pub fps: FrameRate,
    /// Canvas width and height in pixels. Spatial amplitudes scale from the shorter edge.
    pub canvas_size: [u32; 2],
    pub enter: Option<TimedCaptionPreset>,
    pub display: Option<DisplayCaptionPreset>,
    pub exit: Option<TimedCaptionPreset>,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum CaptionPresetError {
    #[error("owner id must be non-empty, contain no control characters, and be at most 128 bytes")]
    InvalidOwnerId,
    #[error("generated Timeline id `{0}` exceeds 128 bytes")]
    GeneratedIdTooLong(String),
    #[error("caption duration must be positive")]
    InvalidCaptionDuration,
    #[error("fps must be positive")]
    InvalidFps,
    #[error("canvas width and height must both be non-zero")]
    InvalidCanvasSize,
    #[error("{phase:?} preset `{id}` belongs to {actual:?}")]
    PhaseMismatch {
        phase: CaptionPresetPhase,
        id: &'static str,
        actual: CaptionPresetPhase,
    },
    #[error("{0:?} duration must be positive")]
    InvalidPhaseDuration(CaptionPresetPhase),
    #[error("enter and exit durations must fit within the caption duration")]
    InvalidPhaseWindow,
    #[error("display rate must be positive and finite")]
    InvalidDisplayRate,
    #[error("display preset needs a non-empty window between enter and exit phases")]
    InvalidDisplayWindow,
    #[error("unknown caption preset id `{0}`")]
    UnknownPresetId(String),
    #[error("caption preset sampling requires more than {maximum} samples")]
    SampleBudgetExceeded { maximum: usize },
    #[error("exact-time arithmetic overflowed")]
    TimeOverflow,
    #[error("preset `{id}` evaluated outside the closed caption geometry domain")]
    InvalidEvaluation { id: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CaptionPresetStyle {
    opacity: f64,
    translation: [f64; 2],
    scale: f64,
    /// Clockwise radians, matching Timeline.
    rotation: f64,
    clip_inset: Option<[f64; 4]>,
    blur_sigma: f64,
}

impl CaptionPresetStyle {
    const IDENTITY: Self = Self {
        opacity: 1.0,
        translation: [0.0, 0.0],
        scale: 1.0,
        rotation: 0.0,
        clip_inset: None,
        blur_sigma: 0.0,
    };

    fn compose(self, other: Self) -> Self {
        Self {
            opacity: self.opacity * other.opacity,
            translation: [
                self.translation[0] + other.translation[0],
                self.translation[1] + other.translation[1],
            ],
            scale: self.scale * other.scale,
            rotation: self.rotation + other.rotation,
            clip_inset: self.clip_inset.or(other.clip_inset),
            blur_sigma: self.blur_sigma + other.blur_sigma,
        }
    }

    fn is_closed(self) -> bool {
        self.opacity.is_finite()
            && (0.0..=1.0).contains(&self.opacity)
            && self.scale.is_finite()
            && self.scale >= 0.0
            && self.rotation.is_finite()
            && self.translation.into_iter().all(f64::is_finite)
            && self.blur_sigma.is_finite()
            && (0.0..=MAX_CAPTION_BLUR_SIGMA).contains(&self.blur_sigma)
            && self.clip_inset.is_none_or(|inset| {
                inset
                    .into_iter()
                    .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
            })
    }

    fn normalize_signed_zero(mut self) -> Self {
        fn zero(value: f64) -> f64 {
            if value == 0.0 { 0.0 } else { value }
        }

        self.opacity = zero(self.opacity);
        self.translation = self.translation.map(zero);
        self.scale = zero(self.scale);
        self.rotation = zero(self.rotation);
        self.clip_inset = self.clip_inset.map(|values| values.map(zero));
        self.blur_sigma = zero(self.blur_sigma);
        self
    }
}

const PI: f64 = core::f64::consts::PI;
const TAU: f64 = core::f64::consts::TAU;
const DEG_TO_RAD: f64 = PI / 180.0;
const ARRIVAL: f64 = 0.72;
const POP_PEAK_PROGRESS: f64 = 94.0 / 125.0;

fn sin(value: f64) -> f64 {
    libm::sin(value)
}

fn smoothstep(value: f64) -> f64 {
    let p = value.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

fn arrival(value: f64) -> f64 {
    smoothstep((value / ARRIVAL).min(1.0))
}

fn analytic_pop_scale(value: f64) -> f64 {
    let p = value.clamp(0.0, 1.0);
    0.82 + 0.18 * smoothstep(p) + 0.40 * p * (1.0 - p)
}

fn pop_scale(value: f64) -> f64 {
    let p = value.clamp(0.0, 1.0);
    let peak = analytic_pop_scale(POP_PEAK_PROGRESS);
    if p <= POP_PEAK_PROGRESS {
        0.82 + (peak - 0.82) * smoothstep(p / POP_PEAK_PROGRESS)
    } else {
        peak + (1.0 - peak) * smoothstep((p - POP_PEAK_PROGRESS) / (1.0 - POP_PEAK_PROGRESS))
    }
}

fn canvas_scale(canvas_size: [u32; 2]) -> f64 {
    f64::from(canvas_size[0].min(canvas_size[1])) / 1080.0
}

fn evaluate_enter(preset: CaptionPreset, value: f64, scale: f64) -> CaptionPresetStyle {
    use CaptionPreset::*;

    let p = value.clamp(0.0, 1.0);
    let s = smoothstep(p);
    let r = 1.0 - s;
    let a = arrival(p);
    let dx = 72.0 * scale;
    let dy = 48.0 * scale;
    let blur = 14.0 * scale;
    let mut style = CaptionPresetStyle::IDENTITY;

    match preset {
        EnterFade => style.opacity = s,
        EnterSlideUp => {
            style.opacity = a;
            style.translation[1] = dy * r;
        }
        EnterSlideDown => {
            style.opacity = a;
            style.translation[1] = -dy * r;
        }
        EnterSlideLeft => {
            style.opacity = a;
            style.translation[0] = dx * r;
        }
        EnterSlideRight => {
            style.opacity = a;
            style.translation[0] = -dx * r;
        }
        EnterPop => {
            style.opacity = a;
            style.scale = pop_scale(p);
        }
        EnterZoomIn => {
            style.opacity = a;
            style.scale = 0.88 + 0.12 * s;
        }
        EnterZoomOut => {
            style.opacity = a;
            style.scale = 1.12 - 0.12 * s;
        }
        EnterFocus => {
            style.opacity = s;
            style.scale = 0.985 + 0.015 * s;
            style.blur_sigma = blur * r;
        }
        EnterWipeRight => {
            style.opacity = 0.75 + 0.25 * s;
            style.clip_inset = Some([0.0, r, 0.0, 0.0]);
        }
        EnterWipeLeft => {
            style.opacity = 0.75 + 0.25 * s;
            style.clip_inset = Some([0.0, 0.0, 0.0, r]);
        }
        ExitFade | ExitSlideUp | ExitSlideDown | ExitSlideLeft | ExitSlideRight | ExitPop
        | ExitZoomIn | ExitZoomOut | ExitFocus | ExitWipeRight | ExitWipeLeft | DisplayBreathe
        | DisplayFloat | DisplaySway | DisplayPulse | DisplayShake => {
            unreachable!("evaluate_enter requires an enter preset")
        }
    }
    style.normalize_signed_zero()
}

fn inverse_enter_for_exit(preset: CaptionPreset) -> CaptionPreset {
    use CaptionPreset::*;
    match preset {
        ExitFade => EnterFade,
        ExitSlideUp => EnterSlideDown,
        ExitSlideDown => EnterSlideUp,
        ExitSlideLeft => EnterSlideRight,
        ExitSlideRight => EnterSlideLeft,
        ExitPop => EnterPop,
        ExitZoomIn => EnterZoomOut,
        ExitZoomOut => EnterZoomIn,
        ExitFocus => EnterFocus,
        ExitWipeRight => EnterWipeLeft,
        ExitWipeLeft => EnterWipeRight,
        _ => unreachable!("inverse_enter_for_exit requires an exit preset"),
    }
}

fn periodic_time(value: f64, period: f64) -> f64 {
    libm::fmod(value, period)
}

fn half_sine(value: f64) -> f64 {
    if value <= 0.0 || value >= 1.0 {
        0.0
    } else {
        sin(PI * value)
    }
}

fn evaluate_display(preset: CaptionPreset, time: f64, scale: f64) -> CaptionPresetStyle {
    use CaptionPreset::*;

    let mut style = CaptionPresetStyle::IDENTITY;
    match preset {
        DisplayBreathe => style.scale = 1.0 + 0.018 * sin(TAU * periodic_time(time, 2.8) / 2.8),
        DisplayFloat => {
            style.translation[1] = -6.0 * scale * sin(TAU * periodic_time(time, 2.8) / 2.8)
        }
        DisplaySway => {
            style.rotation = 2.0 * DEG_TO_RAD * sin(TAU * periodic_time(time, 2.6) / 2.6)
        }
        DisplayPulse => {
            let phase = periodic_time(time, 1.6);
            let bump = if phase < 0.12 {
                half_sine(phase / 0.12)
            } else if (0.18..0.30).contains(&phase) {
                0.55 * half_sine((phase - 0.18) / 0.12)
            } else {
                0.0
            };
            style.scale = 1.0 + 0.055 * bump;
        }
        DisplayShake => {
            let phase = periodic_time(time, 2.4);
            if phase > 0.0 && phase < 0.32 {
                let z = phase / 0.32;
                let envelope = (1.0 - z) * (1.0 - z);
                style.translation[0] = 3.0 * scale * envelope * sin(7.0 * PI * z);
                style.rotation = 0.6 * DEG_TO_RAD * envelope * sin(5.0 * PI * z);
            }
        }
        _ => unreachable!("evaluate_display requires a display preset"),
    }
    style.normalize_signed_zero()
}

fn evaluate_preset(preset: CaptionPreset, value: f64, canvas_size: [u32; 2]) -> CaptionPresetStyle {
    let scale = canvas_scale(canvas_size);
    match preset.phase() {
        CaptionPresetPhase::Enter => evaluate_enter(preset, value, scale),
        CaptionPresetPhase::Exit => {
            evaluate_enter(inverse_enter_for_exit(preset), 1.0 - value, scale)
        }
        CaptionPresetPhase::Display => evaluate_display(preset, value, scale),
    }
}

/// Resolves an author-facing identifier and verifies that it is used in the requested phase.
pub fn parse_caption_preset(
    id: &str,
    expected: CaptionPresetPhase,
) -> Result<CaptionPreset, CaptionPresetError> {
    let preset = CaptionPreset::from_id(id)
        .ok_or_else(|| CaptionPresetError::UnknownPresetId(id.to_owned()))?;
    validate_phase(preset, expected)?;
    Ok(preset)
}

fn validate_phase(
    preset: CaptionPreset,
    expected: CaptionPresetPhase,
) -> Result<(), CaptionPresetError> {
    let actual = preset.phase();
    if actual != expected {
        return Err(CaptionPresetError::PhaseMismatch {
            phase: expected,
            id: preset.id(),
            actual,
        });
    }
    Ok(())
}

fn checked_add(
    left: ExactRational,
    right: ExactRational,
) -> Result<ExactRational, CaptionPresetError> {
    left.checked_add(right)
        .map_err(|_| CaptionPresetError::TimeOverflow)
}

fn checked_sub(
    left: ExactRational,
    right: ExactRational,
) -> Result<ExactRational, CaptionPresetError> {
    left.checked_sub(right)
        .map_err(|_| CaptionPresetError::TimeOverflow)
}

fn checked_mul(
    left: ExactRational,
    right: ExactRational,
) -> Result<ExactRational, CaptionPresetError> {
    left.checked_mul(right)
        .map_err(|_| CaptionPresetError::TimeOverflow)
}

fn validate(request: &CaptionPresetRequest) -> Result<(), CaptionPresetError> {
    if request.owner_id.is_empty()
        || request.owner_id.len() > 128
        || request.owner_id.chars().any(char::is_control)
    {
        return Err(CaptionPresetError::InvalidOwnerId);
    }
    if !request.caption_duration.is_positive() {
        return Err(CaptionPresetError::InvalidCaptionDuration);
    }
    if !request.fps.into_exact().is_positive() {
        return Err(CaptionPresetError::InvalidFps);
    }
    if request
        .canvas_size
        .into_iter()
        .any(|dimension| dimension == 0)
    {
        return Err(CaptionPresetError::InvalidCanvasSize);
    }
    if let Some(enter) = request.enter {
        validate_phase(enter.preset, CaptionPresetPhase::Enter)?;
        if !enter.duration.is_positive() {
            return Err(CaptionPresetError::InvalidPhaseDuration(
                CaptionPresetPhase::Enter,
            ));
        }
        if [0.0, 0.5, 1.0]
            .into_iter()
            .any(|p| !evaluate_preset(enter.preset, p, request.canvas_size).is_closed())
        {
            return Err(CaptionPresetError::InvalidEvaluation {
                id: enter.preset.id(),
            });
        }
    }
    if let Some(exit) = request.exit {
        validate_phase(exit.preset, CaptionPresetPhase::Exit)?;
        if !exit.duration.is_positive() {
            return Err(CaptionPresetError::InvalidPhaseDuration(
                CaptionPresetPhase::Exit,
            ));
        }
        if [0.0, 0.5, 1.0]
            .into_iter()
            .any(|p| !evaluate_preset(exit.preset, p, request.canvas_size).is_closed())
        {
            return Err(CaptionPresetError::InvalidEvaluation {
                id: exit.preset.id(),
            });
        }
    }
    if let Some(display) = request.display {
        validate_phase(display.preset, CaptionPresetPhase::Display)?;
        if !display.rate.is_finite() || display.rate <= 0.0 {
            return Err(CaptionPresetError::InvalidDisplayRate);
        }
    }

    let enter_duration = request
        .enter
        .map_or(ExactRational::ZERO, |enter| enter.duration.into_exact());
    let exit_duration = request
        .exit
        .map_or(ExactRational::ZERO, |exit| exit.duration.into_exact());
    let occupied = checked_add(enter_duration, exit_duration)?;
    let caption_duration = request.caption_duration.into_exact();
    if occupied > caption_duration {
        return Err(CaptionPresetError::InvalidPhaseWindow);
    }
    if request.display.is_some() && occupied >= caption_duration {
        return Err(CaptionPresetError::InvalidDisplayWindow);
    }
    Ok(())
}

fn sample_times(request: &CaptionPresetRequest) -> Result<Vec<ExactRational>, CaptionPresetError> {
    let horizon = request.caption_duration.into_exact();
    let frame_step = ExactRational::ONE
        .checked_div(request.fps.into_exact())
        .map_err(|_| CaptionPresetError::TimeOverflow)?;
    let enter_end = request
        .enter
        .map_or(ExactRational::ZERO, |enter| enter.duration.into_exact());
    let exit_start = match request.exit {
        Some(exit) => checked_sub(horizon, exit.duration.into_exact())?,
        None => horizon,
    };

    let mut values = BTreeSet::new();
    values.insert(ExactRational::ZERO);
    values.insert(horizon);
    values.insert(enter_end);
    values.insert(exit_start);

    let mut at = ExactRational::ZERO;
    while at <= horizon {
        values.insert(at);
        if values.len() > MAX_CAPTION_PRESET_SAMPLES {
            return Err(CaptionPresetError::SampleBudgetExceeded {
                maximum: MAX_CAPTION_PRESET_SAMPLES,
            });
        }
        at = checked_add(at, frame_step)?;
    }
    if values.len() > MAX_CAPTION_PRESET_SAMPLES {
        return Err(CaptionPresetError::SampleBudgetExceeded {
            maximum: MAX_CAPTION_PRESET_SAMPLES,
        });
    }
    Ok(values.into_iter().collect())
}

fn resolve(
    request: &CaptionPresetRequest,
    local_time: ExactRational,
) -> Result<CaptionPresetStyle, CaptionPresetError> {
    let mut style = CaptionPresetStyle::IDENTITY;
    if let Some(enter) = request.enter
        && local_time < enter.duration.into_exact()
    {
        let p = RationalTime::from_exact(local_time)
            .checked_div(enter.duration)
            .map_err(|_| CaptionPresetError::TimeOverflow)?
            .into_exact()
            .as_f64()
            .clamp(0.0, 1.0);
        let value = evaluate_preset(enter.preset, p, request.canvas_size);
        if !value.is_closed() {
            return Err(CaptionPresetError::InvalidEvaluation {
                id: enter.preset.id(),
            });
        }
        style = style.compose(value);
    }
    if let Some(exit) = request.exit {
        let out_start = checked_sub(
            request.caption_duration.into_exact(),
            exit.duration.into_exact(),
        )?;
        if local_time > out_start {
            let elapsed = RationalTime::from_exact(checked_sub(local_time, out_start)?);
            let p = elapsed
                .checked_div(exit.duration)
                .map_err(|_| CaptionPresetError::TimeOverflow)?
                .into_exact()
                .as_f64()
                .clamp(0.0, 1.0);
            let value = evaluate_preset(exit.preset, p, request.canvas_size);
            if !value.is_closed() {
                return Err(CaptionPresetError::InvalidEvaluation {
                    id: exit.preset.id(),
                });
            }
            style = style.compose(value);
        }
    }
    if let Some(display) = request.display {
        let display_start = request
            .enter
            .map_or(ExactRational::ZERO, |enter| enter.duration.into_exact());
        let display_end = match request.exit {
            Some(exit) => checked_sub(
                request.caption_duration.into_exact(),
                exit.duration.into_exact(),
            )?,
            None => request.caption_duration.into_exact(),
        };
        if local_time >= display_start {
            // Display motion reaches the exit boundary continuously, then holds that exact style
            // while the independent exit recipe runs. Dropping the display component at
            // `display_end` would create a one-frame jump for periodic presets.
            let display_time = local_time.min(display_end);
            let elapsed = checked_sub(display_time, display_start)?;
            let t = elapsed.as_f64() * display.rate;
            if !t.is_finite() {
                return Err(CaptionPresetError::InvalidEvaluation {
                    id: display.preset.id(),
                });
            }
            let value = evaluate_preset(display.preset, t, request.canvas_size);
            if !value.is_closed() {
                return Err(CaptionPresetError::InvalidEvaluation {
                    id: display.preset.id(),
                });
            }
            style = style.compose(value);
        }
    }
    if !style.is_closed() {
        let id = request
            .enter
            .map(|value| value.preset.id())
            .or_else(|| request.exit.map(|value| value.preset.id()))
            .or_else(|| request.display.map(|value| value.preset.id()))
            .unwrap_or("identity");
        return Err(CaptionPresetError::InvalidEvaluation { id });
    }
    Ok(style.normalize_signed_zero())
}

fn constant<T>(value: T) -> ParamWire<T> {
    ParamWire::Constant(ConstantParamWire { value })
}

#[derive(Debug, Clone, PartialEq)]
struct NormalizedKey<T> {
    time: ExactRational,
    value: T,
    out_easing: Option<EasingWire>,
}

#[derive(Debug, Clone, PartialEq)]
struct AbsoluteKey<T> {
    time: ExactRational,
    value: T,
    out_easing: Option<EasingWire>,
}

#[derive(Debug, Clone, Default)]
struct EdgePlan {
    opacity: Vec<NormalizedKey<f64>>,
    translation: Vec<NormalizedKey<[f64; 2]>>,
    scale: Vec<NormalizedKey<f64>>,
    rotation: Vec<NormalizedKey<f64>>,
    clip_inset: Vec<NormalizedKey<[f64; 4]>>,
    blur_sigma: Vec<NormalizedKey<f64>>,
}

fn unit_time(numerator: i64, denominator: u32) -> ExactRational {
    ExactRational::new(numerator, denominator).expect("static normalized time")
}

fn cubic_easing(y1: f64, y2: f64) -> EasingWire {
    EasingWire::CubicBezier(CubicBezierEasingWire {
        kind: CubicBezierTag::CubicBezier,
        x1: 1.0 / 3.0,
        y1,
        x2: 2.0 / 3.0,
        y2,
    })
}

fn smoothstep_easing() -> EasingWire {
    cubic_easing(0.0, 1.0)
}

fn two_key_curve<T>(from: T, to: T, easing: EasingWire) -> Vec<NormalizedKey<T>> {
    vec![
        NormalizedKey {
            time: ExactRational::ZERO,
            value: from,
            out_easing: Some(easing),
        },
        NormalizedKey {
            time: ExactRational::ONE,
            value: to,
            out_easing: None,
        },
    ]
}

fn arrival_curve<T>(from: T, to: T) -> Vec<NormalizedKey<T>> {
    vec![
        NormalizedKey {
            time: ExactRational::ZERO,
            value: from,
            out_easing: Some(smoothstep_easing()),
        },
        NormalizedKey {
            time: unit_time(18, 25),
            value: to,
            out_easing: None,
        },
    ]
}

fn enter_plan(preset: CaptionPreset, canvas_size: [u32; 2]) -> EdgePlan {
    use CaptionPreset::*;

    let scale = canvas_scale(canvas_size);
    let dx = 72.0 * scale;
    let dy = 48.0 * scale;
    let blur = 14.0 * scale;
    let mut plan = EdgePlan::default();
    match preset {
        EnterFade => {
            plan.opacity = two_key_curve(0.0, 1.0, smoothstep_easing());
        }
        EnterSlideUp | EnterSlideDown | EnterSlideLeft | EnterSlideRight => {
            plan.opacity = arrival_curve(0.0, 1.0);
            let from = match preset {
                EnterSlideUp => [0.0, dy],
                EnterSlideDown => [0.0, -dy],
                EnterSlideLeft => [dx, 0.0],
                EnterSlideRight => [-dx, 0.0],
                _ => unreachable!(),
            };
            plan.translation = two_key_curve(from, [0.0, 0.0], smoothstep_easing());
        }
        EnterPop => {
            plan.opacity = arrival_curve(0.0, 1.0);
            // Timeline easing progress is canonically clamped to [0, 1], so overshoot must be
            // carried by a real middle value rather than an out-of-range bezier control point.
            // 94/125 is the sparse exact-time approximation of this recipe's analytic apex. The
            // evaluator uses these same two smoothstep segments when display sampling is present,
            // so adding a display preset cannot reshape the enter motion.
            let peak_time = unit_time(94, 125);
            let peak = analytic_pop_scale(POP_PEAK_PROGRESS);
            plan.scale = vec![
                NormalizedKey {
                    time: ExactRational::ZERO,
                    value: 0.82,
                    out_easing: Some(smoothstep_easing()),
                },
                NormalizedKey {
                    time: peak_time,
                    value: peak,
                    out_easing: Some(smoothstep_easing()),
                },
                NormalizedKey {
                    time: ExactRational::ONE,
                    value: 1.0,
                    out_easing: None,
                },
            ];
        }
        EnterZoomIn | EnterZoomOut => {
            plan.opacity = arrival_curve(0.0, 1.0);
            let from = if preset == EnterZoomIn { 0.88 } else { 1.12 };
            plan.scale = two_key_curve(from, 1.0, smoothstep_easing());
        }
        EnterFocus => {
            plan.opacity = two_key_curve(0.0, 1.0, smoothstep_easing());
            plan.scale = two_key_curve(0.985, 1.0, smoothstep_easing());
            plan.blur_sigma = two_key_curve(blur, 0.0, smoothstep_easing());
        }
        EnterWipeRight | EnterWipeLeft => {
            plan.opacity = two_key_curve(0.75, 1.0, smoothstep_easing());
            let from = if preset == EnterWipeRight {
                [0.0, 1.0, 0.0, 0.0]
            } else {
                [0.0, 0.0, 0.0, 1.0]
            };
            plan.clip_inset = two_key_curve(from, [0.0; 4], smoothstep_easing());
        }
        _ => unreachable!("enter_plan requires an enter preset"),
    }
    plan
}

fn reverse_easing(easing: EasingWire) -> EasingWire {
    match easing {
        EasingWire::CubicBezier(value) => EasingWire::CubicBezier(CubicBezierEasingWire {
            kind: CubicBezierTag::CubicBezier,
            x1: 1.0 - value.x2,
            y1: 1.0 - value.y2,
            x2: 1.0 - value.x1,
            y2: 1.0 - value.y1,
        }),
        EasingWire::Named(value) => EasingWire::Named(match value {
            NamedEasingWire::Linear => NamedEasingWire::Linear,
            NamedEasingWire::EaseIn => NamedEasingWire::EaseOut,
            NamedEasingWire::EaseOut => NamedEasingWire::EaseIn,
            NamedEasingWire::EaseInOut => NamedEasingWire::EaseInOut,
            // The built-in plans do not use CSS `ease`; preserve the named value defensively.
            NamedEasingWire::Ease => NamedEasingWire::Ease,
        }),
    }
}

fn reverse_curve<T: Clone>(curve: Vec<NormalizedKey<T>>) -> Vec<NormalizedKey<T>> {
    let mut reversed = Vec::with_capacity(curve.len());
    for index in (0..curve.len()).rev() {
        let original = &curve[index];
        reversed.push(NormalizedKey {
            time: ExactRational::ONE
                .checked_sub(original.time)
                .expect("preset plan remains within normalized time"),
            value: original.value.clone(),
            out_easing: if index == 0 {
                None
            } else {
                curve[index - 1].out_easing.clone().map(reverse_easing)
            },
        });
    }
    reversed
}

fn exit_plan(preset: CaptionPreset, canvas_size: [u32; 2]) -> EdgePlan {
    let plan = enter_plan(inverse_enter_for_exit(preset), canvas_size);
    EdgePlan {
        opacity: reverse_curve(plan.opacity),
        translation: reverse_curve(plan.translation),
        scale: reverse_curve(plan.scale),
        rotation: reverse_curve(plan.rotation),
        clip_inset: reverse_curve(plan.clip_inset),
        blur_sigma: reverse_curve(plan.blur_sigma),
    }
}

fn map_curve<T: Clone>(
    curve: &[NormalizedKey<T>],
    start: ExactRational,
    duration: ExactRational,
) -> Result<Vec<AbsoluteKey<T>>, CaptionPresetError> {
    curve
        .iter()
        .map(|key| {
            Ok(AbsoluteKey {
                time: checked_add(start, checked_mul(duration, key.time)?)?,
                value: key.value.clone(),
                out_easing: key.out_easing.clone(),
            })
        })
        .collect()
}

fn append_curve<T: Clone + PartialEq>(target: &mut Vec<AbsoluteKey<T>>, source: &[AbsoluteKey<T>]) {
    for key in source {
        if let Some(last) = target.last_mut()
            && last.time == key.time
        {
            debug_assert!(last.value == key.value, "edge plans must meet at identity");
            last.value = key.value.clone();
            last.out_easing = key.out_easing.clone();
        } else {
            target.push(key.clone());
        }
    }
}

fn compact_channel<T: Clone + PartialEq>(
    enter: &[NormalizedKey<T>],
    enter_duration: Option<ExactRational>,
    exit: &[NormalizedKey<T>],
    exit_start: ExactRational,
    exit_duration: Option<ExactRational>,
) -> Result<Vec<AbsoluteKey<T>>, CaptionPresetError> {
    let mut keys = Vec::with_capacity(enter.len() + exit.len());
    if let Some(duration) = enter_duration {
        append_curve(&mut keys, &map_curve(enter, ExactRational::ZERO, duration)?);
    }
    if let Some(duration) = exit_duration {
        append_curve(&mut keys, &map_curve(exit, exit_start, duration)?);
    }
    Ok(keys)
}

fn compact_parameter<T: Clone + PartialEq>(
    owner: &str,
    channel: &str,
    identity: T,
    keys: Vec<AbsoluteKey<T>>,
) -> Result<ParamWire<T>, CaptionPresetError> {
    if keys.is_empty() || keys.iter().all(|key| key.value == identity) {
        return Ok(constant(identity));
    }
    curve_parameter(owner, channel, keys)
}

fn curve_parameter<T>(
    owner: &str,
    channel: &str,
    keys: Vec<AbsoluteKey<T>>,
) -> Result<ParamWire<T>, CaptionPresetError> {
    let curve_id = format!("curve:{owner}:{channel}");
    if curve_id.len() > 128 {
        return Err(CaptionPresetError::GeneratedIdTooLong(curve_id));
    }
    let mut keyframes = Vec::with_capacity(keys.len());
    for (index, key) in keys.into_iter().enumerate() {
        let id = format!("keyframe:{owner}:{channel}:{index}");
        if id.len() > 128 {
            return Err(CaptionPresetError::GeneratedIdTooLong(id));
        }
        keyframes.push(KeyframeWire {
            id,
            time: key.time,
            value: key.value,
            out_easing: key.out_easing,
        });
    }
    Ok(ParamWire::Curve(CurveWire {
        id: curve_id,
        interpolation: InterpolationWire::Linear,
        keyframes,
        extrapolation: ExtrapolationWire::Clamp,
    }))
}

fn expand_compact_edges(
    request: &CaptionPresetRequest,
) -> Result<CaptionPresentationWire, CaptionPresetError> {
    let enter_plan = request
        .enter
        .map(|enter| enter_plan(enter.preset, request.canvas_size))
        .unwrap_or_default();
    let exit_plan = request
        .exit
        .map(|exit| exit_plan(exit.preset, request.canvas_size))
        .unwrap_or_default();
    let enter_duration = request.enter.map(|enter| enter.duration.into_exact());
    let exit_duration = request.exit.map(|exit| exit.duration.into_exact());
    let exit_start = match exit_duration {
        Some(duration) => checked_sub(request.caption_duration.into_exact(), duration)?,
        None => request.caption_duration.into_exact(),
    };

    let opacity = compact_channel(
        &enter_plan.opacity,
        enter_duration,
        &exit_plan.opacity,
        exit_start,
        exit_duration,
    )?;
    let translation = compact_channel(
        &enter_plan.translation,
        enter_duration,
        &exit_plan.translation,
        exit_start,
        exit_duration,
    )?;
    let scale = compact_channel(
        &enter_plan.scale,
        enter_duration,
        &exit_plan.scale,
        exit_start,
        exit_duration,
    )?;
    let rotation = compact_channel(
        &enter_plan.rotation,
        enter_duration,
        &exit_plan.rotation,
        exit_start,
        exit_duration,
    )?;
    let clip_inset = compact_channel(
        &enter_plan.clip_inset,
        enter_duration,
        &exit_plan.clip_inset,
        exit_start,
        exit_duration,
    )?;
    let blur_sigma = compact_channel(
        &enter_plan.blur_sigma,
        enter_duration,
        &exit_plan.blur_sigma,
        exit_start,
        exit_duration,
    )?;

    Ok(CaptionPresentationWire {
        opacity: compact_parameter(&request.owner_id, "opacity", 1.0, opacity)?,
        translation: compact_parameter(&request.owner_id, "translation", [0.0, 0.0], translation)?,
        scale: compact_parameter(&request.owner_id, "scale", 1.0, scale)?,
        rotation: compact_parameter(&request.owner_id, "rotation", 0.0, rotation)?,
        clip_inset: compact_parameter(&request.owner_id, "clipInset", [0.0; 4], clip_inset)?,
        blur_sigma: compact_parameter(&request.owner_id, "blurSigma", 0.0, blur_sigma)?,
    })
}

fn compress_plateau<T: Clone + PartialEq>(
    samples: &[(ExactRational, T)],
) -> Vec<(ExactRational, T)> {
    if samples.len() < 3 {
        return samples.to_vec();
    }
    let mut result = Vec::with_capacity(samples.len());
    result.push(samples[0].clone());
    for index in 1..samples.len() - 1 {
        if samples[index - 1].1 == samples[index].1 && samples[index].1 == samples[index + 1].1 {
            continue;
        }
        result.push(samples[index].clone());
    }
    result.push(samples[samples.len() - 1].clone());
    result
}

fn baked_parameter<T: Clone + PartialEq>(
    owner: &str,
    channel: &str,
    identity: T,
    samples: Vec<(ExactRational, T)>,
) -> Result<ParamWire<T>, CaptionPresetError> {
    if samples.iter().all(|(_, value)| value == &identity) {
        return Ok(constant(identity));
    }
    let keys = compress_plateau(&samples)
        .into_iter()
        .map(|(time, value)| AbsoluteKey {
            time,
            value,
            out_easing: None,
        })
        .collect();
    curve_parameter(owner, channel, keys)
}

fn expand_with_display(
    request: &CaptionPresetRequest,
) -> Result<CaptionPresentationWire, CaptionPresetError> {
    let times = sample_times(request)?;
    let mut opacity = Vec::with_capacity(times.len());
    let mut translation = Vec::with_capacity(times.len());
    let mut scale = Vec::with_capacity(times.len());
    let mut rotation = Vec::with_capacity(times.len());
    let mut clip_inset = Vec::with_capacity(times.len());
    let mut blur_sigma = Vec::with_capacity(times.len());
    for time in times {
        let style = resolve(request, time)?;
        opacity.push((time, style.opacity));
        translation.push((time, style.translation));
        scale.push((time, style.scale));
        rotation.push((time, style.rotation));
        clip_inset.push((time, style.clip_inset.unwrap_or([0.0; 4])));
        blur_sigma.push((time, style.blur_sigma));
    }
    Ok(CaptionPresentationWire {
        opacity: baked_parameter(&request.owner_id, "opacity", 1.0, opacity)?,
        translation: baked_parameter(&request.owner_id, "translation", [0.0, 0.0], translation)?,
        scale: baked_parameter(&request.owner_id, "scale", 1.0, scale)?,
        rotation: baked_parameter(&request.owner_id, "rotation", 0.0, rotation)?,
        clip_inset: baked_parameter(&request.owner_id, "clipInset", [0.0; 4], clip_inset)?,
        blur_sigma: baked_parameter(&request.owner_id, "blurSigma", 0.0, blur_sigma)?,
    })
}

/// Expands a timeline request into fully typed Timeline caption presentation values.
///
/// Edge-only requests use sparse eased curves. The presence of a display preset intentionally
/// drives frame sampling for the combined enter + display + exit result on the caption-local
/// frame grid, so periodic composition is renderer-independent.
pub fn expand_caption_presets(
    request: &CaptionPresetRequest,
) -> Result<CaptionPresentationWire, CaptionPresetError> {
    validate(request)?;
    if request.enter.is_none() && request.display.is_none() && request.exit.is_none() {
        return Ok(CaptionPresentationWire {
            opacity: constant(1.0),
            translation: constant([0.0, 0.0]),
            scale: constant(1.0),
            rotation: constant(0.0),
            clip_inset: constant([0.0; 4]),
            blur_sigma: constant(0.0),
        });
    }
    if request.display.is_some() {
        expand_with_display(request)
    } else {
        expand_compact_edges(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds(numerator: i64, denominator: u32) -> RationalTime {
        RationalTime::new(numerator, denominator).unwrap()
    }

    fn fps(value: i64) -> FrameRate {
        FrameRate::new(value, 1).unwrap()
    }

    fn request() -> CaptionPresetRequest {
        CaptionPresetRequest {
            owner_id: "caption-1".to_owned(),
            caption_duration: seconds(3, 1),
            fps: fps(25),
            canvas_size: [1920, 1080],
            enter: None,
            display: None,
            exit: None,
        }
    }

    fn curve<T>(parameter: &ParamWire<T>) -> &CurveWire<T> {
        match parameter {
            ParamWire::Curve(curve) => curve,
            ParamWire::Constant(_) => panic!("expected curve"),
        }
    }

    fn approx(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-12,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn catalog_is_closed_unique_and_described() {
        assert_eq!(CaptionPreset::ALL.len(), CAPTION_PRESET_COUNT);
        let ids = CaptionPreset::ALL
            .iter()
            .map(|preset| preset.id())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), CAPTION_PRESET_COUNT);
        assert_eq!(
            CaptionPreset::ALL
                .iter()
                .filter(|preset| preset.phase() == CaptionPresetPhase::Enter)
                .count(),
            11
        );
        assert_eq!(
            CaptionPreset::ALL
                .iter()
                .filter(|preset| preset.phase() == CaptionPresetPhase::Exit)
                .count(),
            11
        );
        assert_eq!(
            CaptionPreset::ALL
                .iter()
                .filter(|preset| preset.phase() == CaptionPresetPhase::Display)
                .count(),
            5
        );
        for preset in CaptionPreset::ALL {
            assert_eq!(CaptionPreset::from_id(preset.id()), Some(preset));
            let descriptor = preset.descriptor();
            assert_eq!(descriptor.id, preset.id());
            assert_eq!(descriptor.phase, preset.phase());
            assert!(!descriptor.label.is_empty());
            assert!(!descriptor.summary.is_empty());
            assert_eq!(
                descriptor.recommended_duration.is_some(),
                preset.phase() != CaptionPresetPhase::Display
            );
            assert_eq!(
                descriptor.recommended_rate,
                (preset.phase() == CaptionPresetPhase::Display).then_some(1.0)
            );
        }
        assert_eq!(CaptionPreset::from_id("fade_in"), None);
        assert_eq!(CaptionPreset::from_id("typewriter1_in"), None);
    }

    #[test]
    fn edge_formulas_follow_valle_motion_language() {
        let canvas = [1920, 1080];
        approx(
            evaluate_preset(CaptionPreset::EnterFade, 0.5, canvas).opacity,
            0.5,
        );
        approx(
            evaluate_preset(CaptionPreset::EnterSlideUp, 0.0, canvas).translation[1],
            48.0,
        );
        let pop_peak = evaluate_preset(CaptionPreset::EnterPop, POP_PEAK_PROGRESS, canvas).scale;
        assert!(pop_peak > 1.04);
        approx(
            evaluate_preset(CaptionPreset::ExitSlideUp, 1.0, canvas).translation[1],
            -48.0,
        );
        approx(
            evaluate_preset(CaptionPreset::ExitZoomIn, 1.0, canvas).scale,
            1.12,
        );
    }

    #[test]
    fn exit_catalog_is_the_strict_time_inverse_of_its_enter_pair() {
        let pairs = [
            (CaptionPreset::EnterFade, CaptionPreset::ExitFade),
            (CaptionPreset::EnterSlideDown, CaptionPreset::ExitSlideUp),
            (CaptionPreset::EnterSlideUp, CaptionPreset::ExitSlideDown),
            (CaptionPreset::EnterSlideRight, CaptionPreset::ExitSlideLeft),
            (CaptionPreset::EnterSlideLeft, CaptionPreset::ExitSlideRight),
            (CaptionPreset::EnterPop, CaptionPreset::ExitPop),
            (CaptionPreset::EnterZoomOut, CaptionPreset::ExitZoomIn),
            (CaptionPreset::EnterZoomIn, CaptionPreset::ExitZoomOut),
            (CaptionPreset::EnterFocus, CaptionPreset::ExitFocus),
            (CaptionPreset::EnterWipeLeft, CaptionPreset::ExitWipeRight),
            (CaptionPreset::EnterWipeRight, CaptionPreset::ExitWipeLeft),
        ];
        for (enter, exit) in pairs {
            for p in [0.0, 0.1, 0.5, 0.9, 1.0] {
                assert_eq!(
                    evaluate_preset(exit, p, [1080, 1080]),
                    evaluate_preset(enter, 1.0 - p, [1080, 1080]),
                    "{} / {} at {p}",
                    enter.id(),
                    exit.id()
                );
            }
        }
    }

    #[test]
    fn simple_edges_expand_to_sparse_eased_curves() {
        let mut value = request();
        value.enter = Some(TimedCaptionPreset {
            preset: CaptionPreset::EnterSlideUp,
            duration: seconds(7, 25),
        });
        let presentation = expand_caption_presets(&value).unwrap();
        let opacity = curve(&presentation.opacity);
        let translation = curve(&presentation.translation);
        assert_eq!(opacity.keyframes.len(), 2);
        assert_eq!(translation.keyframes.len(), 2);
        assert!(matches!(
            opacity.keyframes[0].out_easing,
            Some(EasingWire::CubicBezier(_))
        ));
        assert_eq!(opacity.keyframes[1].time, unit_time(126, 625));
        assert_eq!(translation.keyframes[1].time, unit_time(7, 25));
    }

    #[test]
    fn adding_display_does_not_reshape_pop_enter() {
        let mut sparse_request = request();
        sparse_request.enter = Some(TimedCaptionPreset {
            preset: CaptionPreset::EnterPop,
            duration: seconds(1, 1),
        });
        let sparse = expand_caption_presets(&sparse_request).unwrap();
        let sparse_scale = curve(&sparse.scale);
        assert_eq!(sparse_scale.keyframes.len(), 3);
        assert_eq!(sparse_scale.keyframes[1].time, unit_time(94, 125));
        approx(
            sparse_scale.keyframes[1].value,
            pop_scale(POP_PEAK_PROGRESS),
        );

        let mut sampled_request = sparse_request;
        sampled_request.display = Some(DisplayCaptionPreset {
            preset: CaptionPreset::DisplayBreathe,
            rate: 1.0,
        });
        let sampled = expand_caption_presets(&sampled_request).unwrap();
        for keyframe in &curve(&sampled.scale).keyframes {
            if keyframe.time <= ExactRational::ONE {
                approx(keyframe.value, pop_scale(keyframe.time.as_f64()));
            }
        }
    }

    #[test]
    fn spatial_amplitudes_follow_the_short_canvas_edge() {
        let style = evaluate_preset(CaptionPreset::EnterFocus, 0.0, [3840, 2160]);
        approx(style.blur_sigma, 28.0);
        let style = evaluate_preset(CaptionPreset::DisplayFloat, 0.7, [3840, 2160]);
        approx(style.translation[1], -12.0);
    }

    #[test]
    fn display_is_deterministically_sampled_through_caption_end() {
        let mut value = request();
        value.display = Some(DisplayCaptionPreset {
            preset: CaptionPreset::DisplayBreathe,
            rate: 1.0,
        });
        let first = expand_caption_presets(&value).unwrap();
        let second = expand_caption_presets(&value).unwrap();
        assert_eq!(first, second);
        let scale = curve(&first.scale);
        assert_eq!(scale.keyframes.first().unwrap().time, ExactRational::ZERO);
        assert_eq!(scale.keyframes.last().unwrap().time, unit_time(3, 1));
        approx(
            scale.keyframes.last().unwrap().value,
            1.0 + 0.018 * sin(TAU * 3.0 / 2.8),
        );
        assert!(
            scale
                .keyframes
                .iter()
                .all(|keyframe| keyframe.out_easing.is_none())
        );
    }

    #[test]
    fn display_holds_its_boundary_style_through_exit() {
        let mut value = request();
        value.display = Some(DisplayCaptionPreset {
            preset: CaptionPreset::DisplayFloat,
            rate: 1.0,
        });
        value.exit = Some(TimedCaptionPreset {
            preset: CaptionPreset::ExitFade,
            duration: seconds(1, 2),
        });

        let boundary = resolve(&value, unit_time(5, 2)).unwrap();
        let during_exit = resolve(&value, unit_time(11, 4)).unwrap();
        approx(during_exit.translation[1], boundary.translation[1]);
        assert!(during_exit.opacity < boundary.opacity);
    }

    #[test]
    fn completed_enter_wipe_does_not_mask_exit_wipe() {
        let mut value = request();
        value.enter = Some(TimedCaptionPreset {
            preset: CaptionPreset::EnterWipeRight,
            duration: seconds(1, 2),
        });
        value.display = Some(DisplayCaptionPreset {
            preset: CaptionPreset::DisplayBreathe,
            rate: 1.0,
        });
        value.exit = Some(TimedCaptionPreset {
            preset: CaptionPreset::ExitWipeLeft,
            duration: seconds(1, 2),
        });

        let during_exit = resolve(&value, unit_time(11, 4)).unwrap();
        let inset = during_exit.clip_inset.expect("exit wipe remains active");
        assert!(inset[1] > 0.0);
        assert_eq!(inset[3], 0.0);
    }

    #[test]
    fn shake_has_visible_horizontal_samples_at_common_frame_rates() {
        for frame_rate in [24, 25, 30, 60] {
            let mut value = request();
            value.fps = fps(frame_rate);
            value.display = Some(DisplayCaptionPreset {
                preset: CaptionPreset::DisplayShake,
                rate: 1.0,
            });
            let presentation = expand_caption_presets(&value).unwrap();
            assert!(
                curve(&presentation.translation)
                    .keyframes
                    .iter()
                    .any(|keyframe| keyframe.value[0].abs() > 0.05),
                "display.shake aliases to zero at {frame_rate} fps"
            );
        }
    }

    #[test]
    fn overflowing_display_clock_fails_closed_for_every_display_recipe() {
        for preset in CaptionPreset::ALL
            .into_iter()
            .filter(|preset| preset.phase() == CaptionPresetPhase::Display)
        {
            let mut value = request();
            value.display = Some(DisplayCaptionPreset {
                preset,
                rate: f64::MAX,
            });
            assert_eq!(
                expand_caption_presets(&value),
                Err(CaptionPresetError::InvalidEvaluation { id: preset.id() })
            );
        }
    }

    #[test]
    fn phase_windows_and_canvas_fail_closed() {
        let mut value = request();
        value.canvas_size = [0, 1080];
        assert_eq!(
            expand_caption_presets(&value),
            Err(CaptionPresetError::InvalidCanvasSize)
        );

        let mut value = request();
        value.canvas_size = [u32::MAX, u32::MAX];
        value.enter = Some(TimedCaptionPreset {
            preset: CaptionPreset::EnterFocus,
            duration: seconds(1, 2),
        });
        assert_eq!(
            expand_caption_presets(&value),
            Err(CaptionPresetError::InvalidEvaluation { id: "enter.focus" })
        );

        let mut value = request();
        value.enter = Some(TimedCaptionPreset {
            preset: CaptionPreset::EnterFade,
            duration: seconds(2, 1),
        });
        value.exit = Some(TimedCaptionPreset {
            preset: CaptionPreset::ExitFade,
            duration: seconds(2, 1),
        });
        assert_eq!(
            expand_caption_presets(&value),
            Err(CaptionPresetError::InvalidPhaseWindow)
        );

        value.caption_duration = seconds(4, 1);
        value.display = Some(DisplayCaptionPreset {
            preset: CaptionPreset::DisplayFloat,
            rate: 1.0,
        });
        assert_eq!(
            expand_caption_presets(&value),
            Err(CaptionPresetError::InvalidDisplayWindow)
        );
    }
}
