//! Easing curves shared by Motion and Timeline: one vocabulary, one parser and one evaluator.
//!
//! The spellings are CSS's keywords and functions, the camelCase forms of its three named curves,
//! and the easings.net curves. Motion artifacts and both Timeline documents write a curve as its
//! canonical spelling, so every JSON surface reads the way authors write it.

use std::{borrow::Cow, fmt};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use valle_draw::math;

/// Every spelling [`Easing::parse`] accepts, for diagnostics.
pub const EASING_SPELLINGS: &str = "linear, ease, easeIn, easeOut, easeInOut (or ease-in, \
ease-out, ease-in-out), easeIn, easeOut or easeInOut followed by Sine, Quad, Cubic, Quart, Quint, \
Expo, Circ, Back, Elastic or Bounce (such as easeOutCubic), step-start, step-end, steps(n) or \
steps(n, jump-start|jump-end|jump-none|jump-both|start|end) with a positive integer n (at least 2 \
for jump-none), and cubic-bezier(x1, y1, x2, y2) with finite numbers and x1, x2 in [0, 1]";

/// A curve from linear progress in [0, 1] to eased progress. Every curve ends at 1 and, unless it
/// is `steps()` jumping at the start, starts at 0; back and elastic curves, and Bézier control
/// points outside [0, 1], leave [0, 1] in between, as [`Easing::range`] reports.
///
/// It serializes as its canonical spelling and deserializes from any of the
/// [`EASING_SPELLINGS`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Easing {
    #[default]
    Linear,
    /// CSS `ease`, `cubic-bezier(0.25, 0.1, 0.25, 1)`.
    Ease,
    /// CSS `ease-in`, `cubic-bezier(0.42, 0, 1, 1)`.
    EaseIn,
    /// CSS `ease-out`, `cubic-bezier(0, 0, 0.58, 1)`.
    EaseOut,
    /// CSS `ease-in-out`, `cubic-bezier(0.42, 0, 0.58, 1)`.
    EaseInOut,
    EaseInSine,
    EaseOutSine,
    EaseInOutSine,
    EaseInQuad,
    EaseOutQuad,
    EaseInOutQuad,
    EaseInCubic,
    EaseOutCubic,
    EaseInOutCubic,
    EaseInQuart,
    EaseOutQuart,
    EaseInOutQuart,
    EaseInQuint,
    EaseOutQuint,
    EaseInOutQuint,
    EaseInExpo,
    EaseOutExpo,
    EaseInOutExpo,
    EaseInCirc,
    EaseOutCirc,
    EaseInOutCirc,
    EaseInBack,
    EaseOutBack,
    EaseInOutBack,
    EaseInElastic,
    EaseOutElastic,
    EaseInOutElastic,
    EaseInBounce,
    EaseOutBounce,
    EaseInOutBounce,
    /// CSS `steps()`.
    Steps {
        count: u32,
        position: StepPosition,
    },
    /// CSS `cubic-bezier()`, with endpoints fixed at (0, 0) and (1, 1).
    CubicBezier {
        p: [f64; 4],
    },
}

/// Where a CSS `steps()` curve jumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepPosition {
    /// `jump-start`, or `start`: the first jump is at the start.
    JumpStart,
    /// `jump-end`, or `end`, the default: the last jump is at the end.
    JumpEnd,
    /// `jump-none`: no jump at either end, so `n` steps take `n - 1` jumps.
    JumpNone,
    /// `jump-both`: jumps at both ends, so `n` steps take `n + 1` jumps.
    JumpBoth,
}

impl StepPosition {
    fn keyword(self) -> &'static str {
        match self {
            Self::JumpStart => "jump-start",
            Self::JumpEnd => "jump-end",
            Self::JumpNone => "jump-none",
            Self::JumpBoth => "jump-both",
        }
    }
}

/// The named curves by their canonical spelling.
const NAMED: [(&str, Easing); 35] = [
    ("linear", Easing::Linear),
    ("ease", Easing::Ease),
    ("easeIn", Easing::EaseIn),
    ("easeOut", Easing::EaseOut),
    ("easeInOut", Easing::EaseInOut),
    ("easeInSine", Easing::EaseInSine),
    ("easeOutSine", Easing::EaseOutSine),
    ("easeInOutSine", Easing::EaseInOutSine),
    ("easeInQuad", Easing::EaseInQuad),
    ("easeOutQuad", Easing::EaseOutQuad),
    ("easeInOutQuad", Easing::EaseInOutQuad),
    ("easeInCubic", Easing::EaseInCubic),
    ("easeOutCubic", Easing::EaseOutCubic),
    ("easeInOutCubic", Easing::EaseInOutCubic),
    ("easeInQuart", Easing::EaseInQuart),
    ("easeOutQuart", Easing::EaseOutQuart),
    ("easeInOutQuart", Easing::EaseInOutQuart),
    ("easeInQuint", Easing::EaseInQuint),
    ("easeOutQuint", Easing::EaseOutQuint),
    ("easeInOutQuint", Easing::EaseInOutQuint),
    ("easeInExpo", Easing::EaseInExpo),
    ("easeOutExpo", Easing::EaseOutExpo),
    ("easeInOutExpo", Easing::EaseInOutExpo),
    ("easeInCirc", Easing::EaseInCirc),
    ("easeOutCirc", Easing::EaseOutCirc),
    ("easeInOutCirc", Easing::EaseInOutCirc),
    ("easeInBack", Easing::EaseInBack),
    ("easeOutBack", Easing::EaseOutBack),
    ("easeInOutBack", Easing::EaseInOutBack),
    ("easeInElastic", Easing::EaseInElastic),
    ("easeOutElastic", Easing::EaseOutElastic),
    ("easeInOutElastic", Easing::EaseInOutElastic),
    ("easeInBounce", Easing::EaseInBounce),
    ("easeOutBounce", Easing::EaseOutBounce),
    ("easeInOutBounce", Easing::EaseInOutBounce),
];

/// CSS's spellings of its three named curves.
const CSS_NAMED: [(&str, Easing); 3] = [
    ("ease-in", Easing::EaseIn),
    ("ease-out", Easing::EaseOut),
    ("ease-in-out", Easing::EaseInOut),
];

const EASE: [f64; 4] = [0.25, 0.1, 0.25, 1.0];
const EASE_IN: [f64; 4] = [0.42, 0.0, 1.0, 1.0];
const EASE_OUT: [f64; 4] = [0.0, 0.0, 0.58, 1.0];
const EASE_IN_OUT: [f64; 4] = [0.42, 0.0, 0.58, 1.0];

impl Easing {
    /// Parse one of the [`EASING_SPELLINGS`].
    pub fn parse(text: &str) -> Option<Easing> {
        let text = text.trim();
        if let Some((_, easing)) = NAMED
            .iter()
            .chain(&CSS_NAMED)
            .find(|(name, _)| *name == text)
        {
            return Some(*easing);
        }
        let easing = match text {
            "step-start" => Easing::Steps {
                count: 1,
                position: StepPosition::JumpStart,
            },
            "step-end" => Easing::Steps {
                count: 1,
                position: StepPosition::JumpEnd,
            },
            _ => {
                if let Some(args) = function_args(text, "steps") {
                    let mut args = args.split(',').map(str::trim);
                    let count = args.next()?;
                    if count.is_empty() || !count.bytes().all(|byte| byte.is_ascii_digit()) {
                        return None;
                    }
                    let count = count.parse().ok()?;
                    let position = match args.next() {
                        None | Some("end" | "jump-end") => StepPosition::JumpEnd,
                        Some("start" | "jump-start") => StepPosition::JumpStart,
                        Some("jump-none") => StepPosition::JumpNone,
                        Some("jump-both") => StepPosition::JumpBoth,
                        Some(_) => return None,
                    };
                    if args.next().is_some() {
                        return None;
                    }
                    Easing::Steps { count, position }
                } else {
                    let mut args = function_args(text, "cubic-bezier")?.split(',');
                    let mut p = [0.0; 4];
                    for value in &mut p {
                        *value = args.next()?.trim().parse().ok()?;
                    }
                    if args.next().is_some() {
                        return None;
                    }
                    Easing::CubicBezier { p }
                }
            }
        };
        easing.is_valid().then_some(easing)
    }

    /// Check the parameters, including those of curves deserialized from artifacts and documents:
    /// Bézier x control points in [0, 1] keep the curve a function of progress.
    pub fn is_valid(&self) -> bool {
        match *self {
            Easing::CubicBezier { p } => {
                p.iter().all(|value| value.is_finite())
                    && (0.0..=1.0).contains(&p[0])
                    && (0.0..=1.0).contains(&p[2])
            }
            Easing::Steps { count, position } => {
                count > 0 && (position != StepPosition::JumpNone || count > 1)
            }
            _ => true,
        }
    }

    /// Evaluate the curve at `t`, clamped to [0, 1].
    pub fn evaluate(self, t: f64) -> f64 {
        self.evaluate_with_floor(t, f64::floor)
    }

    /// [`Easing::evaluate`], with `floor` choosing the step of `steps()` from `t` times the
    /// count: for a `t` computed from the clock, a floor that holds at frame boundaries.
    pub fn evaluate_with_floor(self, t: f64, floor: fn(f64) -> f64) -> f64 {
        use std::f64::consts::{FRAC_PI_2, PI, TAU};

        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Steps { count, position } => {
                let count = f64::from(count);
                let (jump, jumps) = match position {
                    StepPosition::JumpStart => (1.0, count),
                    StepPosition::JumpEnd => (0.0, count),
                    StepPosition::JumpNone => (0.0, count - 1.0),
                    StepPosition::JumpBoth => (1.0, count + 1.0),
                };
                (floor(t * count) + jump).min(jumps) / jumps
            }
            // The formulas meet their endpoints only up to rounding.
            _ if t == 0.0 => 0.0,
            _ if t == 1.0 => 1.0,
            Easing::Linear => t,
            Easing::Ease => unit_bezier(EASE, t),
            Easing::EaseIn => unit_bezier(EASE_IN, t),
            Easing::EaseOut => unit_bezier(EASE_OUT, t),
            Easing::EaseInOut => unit_bezier(EASE_IN_OUT, t),
            Easing::EaseInSine => 1.0 - math::cos(t * FRAC_PI_2),
            Easing::EaseOutSine => math::sin(t * FRAC_PI_2),
            Easing::EaseInOutSine => -(math::cos(PI * t) - 1.0) / 2.0,
            Easing::EaseInQuad => t * t,
            Easing::EaseOutQuad => 1.0 - power(1.0 - t, 2),
            Easing::EaseInOutQuad => in_out(t, |t| 2.0 * t * t, 2),
            Easing::EaseInCubic => power(t, 3),
            Easing::EaseOutCubic => 1.0 - power(1.0 - t, 3),
            Easing::EaseInOutCubic => in_out(t, |t| 4.0 * power(t, 3), 3),
            Easing::EaseInQuart => power(t, 4),
            Easing::EaseOutQuart => 1.0 - power(1.0 - t, 4),
            Easing::EaseInOutQuart => in_out(t, |t| 8.0 * power(t, 4), 4),
            Easing::EaseInQuint => power(t, 5),
            Easing::EaseOutQuint => 1.0 - power(1.0 - t, 5),
            Easing::EaseInOutQuint => in_out(t, |t| 16.0 * power(t, 5), 5),
            Easing::EaseInExpo => math::pow(2.0, 10.0 * (t - 1.0)),
            Easing::EaseOutExpo => 1.0 - math::pow(2.0, -10.0 * t),
            Easing::EaseInOutExpo => {
                if t < 0.5 {
                    math::pow(2.0, 20.0 * t - 10.0) / 2.0
                } else {
                    (2.0 - math::pow(2.0, 10.0 - 20.0 * t)) / 2.0
                }
            }
            Easing::EaseInCirc => 1.0 - math::sqrt(1.0 - t * t),
            Easing::EaseOutCirc => math::sqrt(1.0 - power(t - 1.0, 2)),
            Easing::EaseInOutCirc => {
                if t < 0.5 {
                    (1.0 - math::sqrt(1.0 - power(2.0 * t, 2))) / 2.0
                } else {
                    (math::sqrt(1.0 - power(2.0 - 2.0 * t, 2)) + 1.0) / 2.0
                }
            }
            Easing::EaseInBack => back_in(t),
            Easing::EaseOutBack => 1.0 - back_in(1.0 - t),
            Easing::EaseInOutBack => {
                const C: f64 = 1.70158 * 1.525;
                let x = if t < 0.5 { 2.0 * t } else { 2.0 * t - 2.0 };
                if t < 0.5 {
                    x * x * ((C + 1.0) * x - C) * 0.5
                } else {
                    (x * x * ((C + 1.0) * x + C) + 2.0) * 0.5
                }
            }
            Easing::EaseInElastic => {
                -math::pow(2.0, 10.0 * t - 10.0) * math::sin((10.0 * t - 10.75) * (TAU / 3.0))
            }
            Easing::EaseOutElastic => {
                math::pow(2.0, -10.0 * t) * math::sin((10.0 * t - 0.75) * (TAU / 3.0)) + 1.0
            }
            Easing::EaseInOutElastic => {
                let wave = math::sin((20.0 * t - 11.125) * (TAU / 4.5));
                if t < 0.5 {
                    -(math::pow(2.0, 20.0 * t - 10.0) * wave) / 2.0
                } else {
                    math::pow(2.0, 10.0 - 20.0 * t) * wave / 2.0 + 1.0
                }
            }
            Easing::EaseInBounce => 1.0 - bounce_out(1.0 - t),
            Easing::EaseOutBounce => bounce_out(t),
            Easing::EaseInOutBounce => {
                if t < 0.5 {
                    (1.0 - bounce_out(1.0 - 2.0 * t)) / 2.0
                } else {
                    (1.0 + bounce_out(2.0 * t - 1.0)) / 2.0
                }
            }
            Easing::CubicBezier { p } => unit_bezier(p, t),
        }
    }

    /// The least and the greatest eased progress over [0, 1]. Back and elastic curves, and Bézier
    /// y control points outside [0, 1], widen it beyond [0, 1]; the constants round outward.
    pub fn range(self) -> [f64; 2] {
        match self {
            Easing::EaseInBack => [-0.100_004_1, 1.0],
            Easing::EaseOutBack => [0.0, 1.100_004_1],
            Easing::EaseInOutBack => [-0.100_151_4, 1.100_151_4],
            Easing::EaseInElastic => [-0.373_098_1, 1.0],
            Easing::EaseOutElastic => [0.0, 1.373_098_1],
            Easing::EaseInOutElastic => [-0.118_348, 1.118_348],
            Easing::CubicBezier { p: [_, y1, _, y2] } => bezier_range(y1, y2),
            _ => [0.0, 1.0],
        }
    }

    /// The canonical spelling of a named curve.
    fn name(self) -> Option<&'static str> {
        NAMED
            .iter()
            .find(|(_, easing)| *easing == self)
            .map(|(name, _)| *name)
    }
}

/// The canonical spelling, which [`Easing::parse`] reads back to the same curve.
impl fmt::Display for Easing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Easing::Steps {
                count,
                position: StepPosition::JumpEnd,
            } => write!(formatter, "steps({count})"),
            Easing::Steps { count, position } => {
                write!(formatter, "steps({count}, {})", position.keyword())
            }
            Easing::CubicBezier {
                p: [x1, y1, x2, y2],
            } => write!(formatter, "cubic-bezier({x1}, {y1}, {x2}, {y2})"),
            named => formatter.write_str(named.name().unwrap_or("linear")),
        }
    }
}

impl Serialize for Easing {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Easing {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        Easing::parse(&text).ok_or_else(|| {
            de::Error::custom(format!("invalid easing `{text}`; use {EASING_SPELLINGS}"))
        })
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Easing {
    fn schema_name() -> Cow<'static, str> {
        "Easing".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": format!("An easing: {EASING_SPELLINGS}."),
        })
    }
}

/// The arguments of `name(...)`, written with no space before the parenthesis as in CSS.
fn function_args<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.strip_prefix(name)?
        .strip_prefix('(')?
        .strip_suffix(')')
}

fn power(x: f64, exponent: u32) -> f64 {
    (1..exponent).fold(x, |product, _| product * x)
}

/// easings.net's in-out form of a polynomial: `ease_in` doubles into the first half and the
/// mirrored `(2 - 2t)^exponent / 2` falls into the second.
fn in_out(t: f64, ease_in: impl Fn(f64) -> f64, exponent: u32) -> f64 {
    if t < 0.5 {
        ease_in(t)
    } else {
        1.0 - power(2.0 - 2.0 * t, exponent) / 2.0
    }
}

fn back_in(t: f64) -> f64 {
    // Polynomial form preserves both endpoints exactly.
    t * t * (t + 1.70158 * (t - 1.0))
}

fn bounce_out(t: f64) -> f64 {
    const N: f64 = 7.5625;
    const D: f64 = 2.75;
    let (x, offset) = if t < 1.0 / D {
        (t, 0.0)
    } else if t < 2.0 / D {
        (t - 1.5 / D, 0.75)
    } else if t < 2.5 / D {
        (t - 2.25 / D, 0.9375)
    } else {
        (t - 2.625 / D, 0.984375)
    };
    N * x * x + offset
}

/// The extremes of a unit Bézier's y over its parameter: at the ends, or where the derivative of
/// y vanishes. x control points in [0, 1] make x reach every parameter's progress.
fn bezier_range(y1: f64, y2: f64) -> [f64; 2] {
    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;
    let mut range = [0.0_f64, 1.0_f64];
    // A curve flat at an end, such as any with y2 = 1, has a root there that rounds just inside;
    // y at it is the end's. An extreme beyond [0, 1] rounds outward, past the rounding of y near
    // it.
    let mut include = |s: f64| {
        if s > 1e-9 && s < 1.0 - 1e-9 {
            let y = ((ay * s + by) * s + cy) * s;
            if y < 0.0 {
                range[0] = range[0].min(y - 1e-12);
            } else if y > 1.0 {
                range[1] = range[1].max(y + 1e-12);
            }
        }
    };
    // y'(s) = 3 ay s² + 2 by s + cy.
    let (a, b, c) = (3.0 * ay, 2.0 * by, cy);
    if a == 0.0 {
        if b != 0.0 {
            include(-c / b);
        }
    } else {
        let discriminant = b * b - 4.0 * a * c;
        if discriminant >= 0.0 {
            let root = math::sqrt(discriminant);
            include((-b + root) / (2.0 * a));
            include((-b - root) / (2.0 * a));
        }
    }
    range
}

/// WebKit-style unit cubic Bézier solve with fixed iteration and convergence constants.
fn unit_bezier([x1, y1, x2, y2]: [f64; 4], x: f64) -> f64 {
    const SOLVE_X_EPSILON: f64 = 1e-6;
    const NEWTON_MAX_ITERS: usize = 8;
    const MIN_SLOPE: f64 = 1e-9;
    const BISECT_MIN_INTERVAL: f64 = 1e-7;

    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }

    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;
    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;
    let sample_x = |t: f64| ((ax * t + bx) * t + cx) * t;
    let sample_dx = |t: f64| (3.0 * ax * t + 2.0 * bx) * t + cx;
    let sample_y = |t: f64| ((ay * t + by) * t + cy) * t;

    let mut t = x;
    for _ in 0..NEWTON_MAX_ITERS {
        let error = sample_x(t) - x;
        if error.abs() < SOLVE_X_EPSILON {
            return sample_y(t);
        }
        let slope = sample_dx(t);
        if slope.abs() < MIN_SLOPE {
            break;
        }
        t -= error / slope;
    }

    let (mut lower, mut upper) = (0.0, 1.0);
    t = x;
    while lower < upper {
        let sampled = sample_x(t);
        if (sampled - x).abs() < SOLVE_X_EPSILON {
            break;
        }
        if x > sampled {
            lower = t;
        } else {
            upper = t;
        }
        t = (lower + upper) * 0.5;
        if upper - lower < BISECT_MIN_INTERVAL {
            break;
        }
    }
    sample_y(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_curve() -> Vec<Easing> {
        NAMED
            .iter()
            .map(|(_, easing)| *easing)
            .chain([
                Easing::Steps {
                    count: 4,
                    position: StepPosition::JumpStart,
                },
                Easing::Steps {
                    count: 4,
                    position: StepPosition::JumpEnd,
                },
                Easing::Steps {
                    count: 4,
                    position: StepPosition::JumpNone,
                },
                Easing::Steps {
                    count: 4,
                    position: StepPosition::JumpBoth,
                },
                Easing::CubicBezier {
                    p: [0.34, 1.56, 0.64, 1.0],
                },
                Easing::CubicBezier {
                    p: [0.6, -0.28, 0.735, 0.045],
                },
                Easing::CubicBezier {
                    p: [0.5, -0.5, 0.5, 1.5],
                },
            ])
            .collect()
    }

    #[test]
    fn every_spelling_parses_and_names_print_back() {
        for (name, easing) in NAMED {
            assert_eq!(Easing::parse(name), Some(easing), "{name}");
            assert_eq!(easing.to_string(), name);
        }
        for (name, easing) in CSS_NAMED {
            assert_eq!(Easing::parse(name), Some(easing), "{name}");
        }
        assert_eq!(Easing::parse("  easeOutCubic "), Some(Easing::EaseOutCubic));
        let steps = |count, position| Some(Easing::Steps { count, position });
        assert_eq!(
            Easing::parse("step-start"),
            steps(1, StepPosition::JumpStart)
        );
        assert_eq!(Easing::parse("step-end"), steps(1, StepPosition::JumpEnd));
        assert_eq!(Easing::parse("steps(3)"), steps(3, StepPosition::JumpEnd));
        assert_eq!(
            Easing::parse("steps(3, end)"),
            steps(3, StepPosition::JumpEnd)
        );
        assert_eq!(
            Easing::parse("steps(3,start)"),
            steps(3, StepPosition::JumpStart)
        );
        assert_eq!(
            Easing::parse("steps(3, jump-start)"),
            steps(3, StepPosition::JumpStart)
        );
        assert_eq!(
            Easing::parse("steps(3, jump-none)"),
            steps(3, StepPosition::JumpNone)
        );
        assert_eq!(
            Easing::parse("steps(3, jump-both)"),
            steps(3, StepPosition::JumpBoth)
        );
        assert_eq!(
            Easing::parse("cubic-bezier(0.25, .1, 0.25, 1)"),
            Some(Easing::CubicBezier {
                p: [0.25, 0.1, 0.25, 1.0]
            })
        );
        for easing in every_curve() {
            assert_eq!(Easing::parse(&easing.to_string()), Some(easing), "{easing}");
        }
    }

    #[test]
    fn malformed_and_retired_spellings_are_rejected() {
        for text in [
            "",
            "exp",
            "elastic",
            "bounce",
            "EaseIn",
            "ease_in",
            "easeOutcubic",
            "spring",
            "steps(0)",
            "steps(1, jump-none)",
            "steps(-1)",
            "steps(2.5)",
            "steps(3, middle)",
            "steps(3, end, end)",
            "steps (3)",
            "cubic-bezier(1.1, 0, 0.5, 1)",
            "cubic-bezier(0.5, 0, -0.1, 1)",
            "cubic-bezier(0.5, 0, 0.5)",
            "cubic-bezier(0.5, 0, 0.5, 1, 1)",
            "cubic-bezier(0.5, NaN, 0.5, 1)",
            "cubic-bezier(0.5, inf, 0.5, 1)",
        ] {
            assert_eq!(Easing::parse(text), None, "{text}");
        }
    }

    #[test]
    fn curves_start_at_zero_and_end_at_one_within_their_range() {
        for easing in every_curve() {
            let jumps_at_start = matches!(
                easing,
                Easing::Steps {
                    position: StepPosition::JumpStart | StepPosition::JumpBoth,
                    ..
                }
            );
            if !jumps_at_start {
                assert_eq!(easing.evaluate(0.0), 0.0, "{easing} at 0");
                assert_eq!(easing.evaluate(-1.0), 0.0, "{easing} before 0");
            }
            assert_eq!(easing.evaluate(1.0), 1.0, "{easing} at 1");
            assert_eq!(easing.evaluate(2.0), 1.0, "{easing} after 1");
            let [low, high] = easing.range();
            let (mut least, mut greatest) = (f64::INFINITY, f64::NEG_INFINITY);
            for step in 0..=100_000 {
                let value = easing.evaluate(f64::from(step) / 100_000.0);
                least = least.min(value);
                greatest = greatest.max(value);
            }
            assert!(
                low <= least && greatest <= high,
                "{easing}: {least}..{greatest} outside {low}..{high}"
            );
            // The range is tight: sampling comes within a few millionths of both ends. Steps
            // report [0, 1] whatever their jumps.
            if !matches!(easing, Easing::Steps { .. }) {
                assert!(
                    least - low < 1e-5 && high - greatest < 1e-5,
                    "{easing}: {least}..{greatest} inside {low}..{high}"
                );
            }
        }
    }

    #[test]
    fn curves_match_their_definitions() {
        let close = |easing: Easing, t: f64, expected: f64| {
            let value = easing.evaluate(t);
            assert!(
                (value - expected).abs() < 1e-12,
                "{easing} at {t}: {value} vs {expected}"
            );
        };
        close(Easing::Linear, 0.25, 0.25);
        close(Easing::EaseInQuad, 0.5, 0.25);
        close(Easing::EaseOutQuad, 0.5, 0.75);
        close(Easing::EaseInOutQuad, 0.25, 0.125);
        close(Easing::EaseOutCubic, 0.5, 0.875);
        close(Easing::EaseInOutCubic, 0.75, 0.9375);
        close(Easing::EaseInQuart, 0.5, 0.0625);
        close(Easing::EaseOutQuint, 0.5, 0.96875);
        close(Easing::EaseInOutSine, 0.5, 0.5);
        close(
            Easing::EaseInSine,
            0.5,
            1.0 - std::f64::consts::FRAC_1_SQRT_2,
        );
        close(Easing::EaseInExpo, 0.5, 2f64.powi(-5));
        close(Easing::EaseOutExpo, 0.5, 1.0 - 2f64.powi(-5));
        close(Easing::EaseInOutExpo, 0.5, 0.5);
        close(Easing::EaseInCirc, 0.6, 0.2);
        close(Easing::EaseOutCirc, 0.4, 0.8);
        close(Easing::EaseInOutElastic, 0.5, 0.5);
        close(Easing::EaseInOutBounce, 0.5, 0.5);
        close(Easing::EaseOutBounce, 1.5 / 2.75, 0.75);
        close(Easing::EaseOutBounce, 2.25 / 2.75, 0.9375);
        close(Easing::EaseOutBounce, 2.625 / 2.75, 0.984375);
        close(Easing::EaseInBounce, 1.0 - 1.5 / 2.75, 0.25);
        // In and out curves mirror each other.
        for [ease_in, ease_out] in [
            [Easing::EaseInSine, Easing::EaseOutSine],
            [Easing::EaseInCubic, Easing::EaseOutCubic],
            [Easing::EaseInExpo, Easing::EaseOutExpo],
            [Easing::EaseInCirc, Easing::EaseOutCirc],
            [Easing::EaseInBack, Easing::EaseOutBack],
            [Easing::EaseInElastic, Easing::EaseOutElastic],
            [Easing::EaseInBounce, Easing::EaseOutBounce],
        ] {
            for t in [0.1, 0.3, 0.55, 0.8] {
                close(ease_out, t, 1.0 - ease_in.evaluate(1.0 - t));
            }
        }
        // The CSS curves are the Bézier curves CSS names.
        for (easing, p) in [
            (Easing::Ease, EASE),
            (Easing::EaseIn, EASE_IN),
            (Easing::EaseOut, EASE_OUT),
            (Easing::EaseInOut, EASE_IN_OUT),
        ] {
            for t in [0.2, 0.5, 0.9] {
                assert_eq!(easing.evaluate(t), Easing::CubicBezier { p }.evaluate(t));
            }
        }
        assert!(Easing::EaseOutBack.evaluate(0.75) > 1.0);
        assert!(Easing::EaseInOutBack.evaluate(0.25) < 0.0);
        assert!(Easing::EaseOutElastic.evaluate(0.15) > 1.3);
    }

    #[test]
    fn steps_jump_where_css_puts_them() {
        let at = |count, position, t| Easing::Steps { count, position }.evaluate(t);
        let values = |position| [0.0, 0.2, 0.5, 0.74, 0.75, 0.99].map(|t| at(4, position, t));
        assert_eq!(
            values(StepPosition::JumpEnd),
            [0.0, 0.0, 0.5, 0.5, 0.75, 0.75]
        );
        assert_eq!(
            values(StepPosition::JumpStart),
            [0.25, 0.25, 0.75, 0.75, 1.0, 1.0]
        );
        assert_eq!(
            values(StepPosition::JumpNone),
            [0.0, 0.0, 2.0 / 3.0, 2.0 / 3.0, 1.0, 1.0]
        );
        assert_eq!(
            values(StepPosition::JumpBoth),
            [0.2, 0.2, 0.6, 0.6, 0.8, 0.8]
        );
    }

    #[test]
    fn bezier_ranges_follow_their_y_control_points() {
        assert_eq!(bezier_range(0.1, 1.0), [0.0, 1.0]);
        // Flat at the end, where y' has a root that rounds to just below 1.
        for y1 in [0.04, 0.44, 0.72, 0.8] {
            assert_eq!(bezier_range(y1, 1.0), [0.0, 1.0], "{y1}");
        }
        let [low, high] = bezier_range(1.56, 1.0);
        assert_eq!(low, 0.0);
        assert!(high > 1.09 && high < 1.1, "{high}");
        let [low, high] = bezier_range(-0.5, 1.5);
        assert!(low < -0.05 && high > 1.05, "{low} {high}");
    }

    #[test]
    fn serde_writes_the_canonical_spelling() {
        let json = |easing: Easing| serde_json::to_string(&easing).unwrap();
        let read = |text: &str| serde_json::from_str::<Easing>(text);
        assert_eq!(json(Easing::EaseOutCubic), r#""easeOutCubic""#);
        assert_eq!(
            json(Easing::Steps {
                count: 3,
                position: StepPosition::JumpNone
            }),
            r#""steps(3, jump-none)""#
        );
        assert_eq!(
            json(Easing::CubicBezier {
                p: [0.25, 0.1, 0.25, 1.0]
            }),
            r#""cubic-bezier(0.25, 0.1, 0.25, 1)""#
        );
        for easing in every_curve() {
            assert_eq!(read(&json(easing)).unwrap(), easing);
        }
        assert_eq!(read(r#""ease-out""#).unwrap(), Easing::EaseOut);
        assert_eq!(read(r#""steps(2, end)""#).unwrap().to_string(), "steps(2)");
        let error = read(r#""exp""#).unwrap_err().to_string();
        assert!(
            error.contains("invalid easing `exp`; use linear"),
            "{error}"
        );
        assert!(read(r#"{"kind":"linear"}"#).is_err());
    }
}
