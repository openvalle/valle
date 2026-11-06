//! Easing curves as CSS writes them: `linear`, `ease` and its kin, `cubic-bezier()`, `steps()` and
//! `linear()` (CSS Easing 2), where Chrome is the reference for what the specification leaves
//! open. easings.net's names are refused, with the CSS that draws the same curve.

use std::f64::consts::PI;

use crate::js;
use crate::style::css;

/// How far a solved point of a Bézier curve may be from where it was asked for.
const TOLERANCE: f64 = 1e-10;

#[derive(Clone, Debug, PartialEq)]
pub enum Easing {
    Linear,
    /// `cubic-bezier(x1, y1, x2, y2)`.
    Bezier([f64; 4]),
    Steps {
        count: u32,
        jump: Jump,
    },
    /// `linear()`: input and output, the inputs never going back.
    Points(Vec<[f64; 2]>),
}

/// Which ends of `steps()` jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Jump {
    Start,
    End,
    None,
    Both,
}

const EASE: [f64; 4] = [0.25, 0.1, 0.25, 1.0];
const EASE_IN: [f64; 4] = [0.42, 0.0, 1.0, 1.0];
const EASE_OUT: [f64; 4] = [0.0, 0.0, 0.58, 1.0];
const EASE_IN_OUT: [f64; 4] = [0.42, 0.0, 0.58, 1.0];

impl Easing {
    pub fn parse(text: &str) -> Result<Easing, String> {
        let trimmed = text.trim();
        if let Some(css) = instead(trimmed) {
            return Err(format!("easing {trimmed} is not CSS; write {css}"));
        }
        let s = trimmed.to_ascii_lowercase();
        Ok(match s.as_str() {
            "linear" => Easing::Linear,
            "ease" => Easing::Bezier(EASE),
            "ease-in" => Easing::Bezier(EASE_IN),
            "ease-out" => Easing::Bezier(EASE_OUT),
            "ease-in-out" => Easing::Bezier(EASE_IN_OUT),
            "step-start" => Easing::Steps {
                count: 1,
                jump: Jump::Start,
            },
            "step-end" => Easing::Steps {
                count: 1,
                jump: Jump::End,
            },
            _ => {
                let call = s.strip_suffix(')').and_then(|s| s.split_once('('));
                match call {
                    Some(("cubic-bezier", args)) => bezier(args)?,
                    Some(("steps", args)) => steps(args)?,
                    Some(("linear", args)) => points(args)?,
                    _ => {
                        return Err(format!(
                            "{trimmed} is not an easing: linear, ease, ease-in, ease-out, \
                             ease-in-out, step-start, step-end, cubic-bezier(), steps() or \
                             linear()"
                        ));
                    }
                }
            }
        })
    }

    /// The eased progress at `t`, which is outside [0, 1] where an interpolation extends past its
    /// stops.
    pub fn at(&self, t: f64) -> f64 {
        match self {
            Easing::Linear => t,
            Easing::Bezier(b) => bezier_at(*b, t),
            Easing::Steps { count, jump } => steps_at(*count, *jump, t),
            Easing::Points(p) => points_at(p, t),
        }
    }
}

fn bezier(args: &str) -> Result<Easing, String> {
    let n: Vec<f64> = (args.split(',').map(number)).collect::<Result<_, _>>()?;
    let [x1, y1, x2, y2] = n[..] else {
        return Err(format!("cubic-bezier({args}) takes four numbers"));
    };
    if !(0.0..=1.0).contains(&x1) || !(0.0..=1.0).contains(&x2) {
        return Err(format!(
            "cubic-bezier({args}): the x values must be within 0 and 1"
        ));
    }
    Ok(Easing::Bezier([x1, y1, x2, y2]))
}

fn number(s: &str) -> Result<f64, String> {
    match css::num(s) {
        Some(n) if n.unit.is_empty() => Ok(n.value),
        _ => Err(format!("{} is not a number", s.trim())),
    }
}

fn steps(args: &str) -> Result<Easing, String> {
    let (count, position) = args.split_once(',').unwrap_or((args, "end"));
    let bad = || format!("steps({args}) takes a whole number of steps and a position");
    let count = number(count).map_err(|_| bad())?;
    let jump = match position.trim() {
        "jump-start" | "start" => Jump::Start,
        "jump-end" | "end" => Jump::End,
        "jump-none" => Jump::None,
        "jump-both" => Jump::Both,
        _ => return Err(bad()),
    };
    let least = if jump == Jump::None { 2.0 } else { 1.0 };
    if count.fract() != 0.0 || !(least..=f64::from(u32::MAX)).contains(&count) {
        return Err(format!("steps({args}) takes at least {least} steps"));
    }
    Ok(Easing::Steps {
        count: count as u32,
        jump,
    })
}

/// `linear()`'s stops, their missing inputs filled in as CSS does: the ends at 0 and 1, none below
/// one before it, and the ones between spread evenly.
fn points(args: &str) -> Result<Easing, String> {
    let mut stops: Vec<(f64, Option<f64>)> = Vec::new();
    for stop in args.split(',') {
        let (mut output, mut inputs) = (None, Vec::new());
        for word in css::words(stop) {
            match css::num(word) {
                Some(n) if n.unit == "%" && inputs.len() < 2 => inputs.push(n.value / 100.0),
                Some(n) if n.unit.is_empty() && output.is_none() => output = Some(n.value),
                _ => return Err(format!("linear({args}): {} is not a stop", stop.trim())),
            }
        }
        let Some(output) = output else {
            return Err(format!("linear({args}): {} has no output", stop.trim()));
        };
        match inputs[..] {
            [] => stops.push((output, None)),
            [a] => stops.push((output, Some(a))),
            _ => stops.extend(inputs.iter().map(|&i| (output, Some(i)))),
        }
    }
    if stops.len() < 2 {
        return Err(format!("linear({args}) takes at least 2 stops"));
    }
    let last = stops.len() - 1;
    stops[0].1.get_or_insert(0.0);
    stops[last].1.get_or_insert(1.0);
    let mut most = f64::NEG_INFINITY;
    for stop in &mut stops {
        if let Some(input) = &mut stop.1 {
            most = input.max(most);
            *input = most;
        }
    }
    let mut known = 0;
    for k in 1..stops.len() {
        if let Some(to) = stops[k].1 {
            let from = stops[known].1.unwrap_or(0.0);
            let gaps = (k - known) as f64;
            for (j, stop) in stops[known + 1..k].iter_mut().enumerate() {
                stop.1 = Some(from + (to - from) * (j + 1) as f64 / gaps);
            }
            known = k;
        }
    }
    let points = stops.iter().map(|&(y, x)| [x.unwrap_or(0.0), y]);
    Ok(Easing::Points(points.collect()))
}

fn points_at(p: &[[f64; 2]], t: f64) -> f64 {
    let after = p.partition_point(|q| q[0] <= t);
    if after > 0 && p[after - 1][0] == t {
        return p[after - 1][1];
    }
    // Past the ends the first and the last two go on.
    let k = after.clamp(1, p.len() - 1);
    let ([x0, y0], [x1, y1]) = (p[k - 1], p[k]);
    match x1 > x0 {
        true => y0 + (t - x0) / (x1 - x0) * (y1 - y0),
        false => y1,
    }
}

fn steps_at(count: u32, jump: Jump, t: f64) -> f64 {
    let steps = f64::from(count);
    let mut step = (t * steps).floor();
    if matches!(jump, Jump::Start | Jump::Both) {
        step += 1.0;
    }
    let jumps = match jump {
        Jump::Both => steps + 1.0,
        Jump::None => steps - 1.0,
        Jump::Start | Jump::End => steps,
    };
    if t >= 0.0 && step < 0.0 {
        step = 0.0;
    }
    if t <= 1.0 && step > jumps {
        step = jumps;
    }
    step / jumps
}

fn bezier_at([x1, y1, x2, y2]: [f64; 4], x: f64) -> f64 {
    // A curve's end tangents go on past it.
    if x < 0.0 {
        let slope = if x1 > 0.0 {
            y1 / x1
        } else if y1 == 0.0 && x2 > 0.0 {
            y2 / x2
        } else if y1 == 0.0 && y2 == 0.0 {
            1.0
        } else {
            0.0
        };
        return slope * x;
    }
    if x > 1.0 {
        let slope = if x2 < 1.0 {
            (y2 - 1.0) / (x2 - 1.0)
        } else if y2 == 1.0 && x1 < 1.0 {
            (y1 - 1.0) / (x1 - 1.0)
        } else if y1 == 1.0 && y2 == 1.0 {
            1.0
        } else {
            0.0
        };
        return 1.0 + slope * (x - 1.0);
    }
    let curve = |a: f64, b: f64, s: f64| {
        let m = 1.0 - s;
        3.0 * m * m * s * a + 3.0 * m * s * s * b + s * s * s
    };
    let (mut lo, mut hi, mut s) = (0.0, 1.0, x);
    for _ in 0..64 {
        let off = curve(x1, x2, s) - x;
        if off.abs() < TOLERANCE {
            break;
        }
        if off > 0.0 {
            hi = s;
        } else {
            lo = s;
        }
        let m = 1.0 - s;
        let slope = 3.0 * m * m * x1 + 6.0 * m * s * (x2 - x1) + 3.0 * s * s * (1.0 - x2);
        let next = s - off / slope;
        // Newton's step where it lands inside what is known, else halfway.
        s = if next > lo && next < hi {
            next
        } else {
            (lo + hi) / 2.0
        };
    }
    curve(y1, y2, s)
}

/// The CSS that draws what an easings.net or camelCase name means, if `name` is one.
fn instead(name: &str) -> Option<String> {
    const BEZIERS: [(&str, &str); 27] = [
        ("easeIn", "ease-in"),
        ("easeOut", "ease-out"),
        ("easeInOut", "ease-in-out"),
        ("easeInSine", "cubic-bezier(0.12, 0, 0.39, 0)"),
        ("easeOutSine", "cubic-bezier(0.61, 1, 0.88, 1)"),
        ("easeInOutSine", "cubic-bezier(0.37, 0, 0.63, 1)"),
        ("easeInQuad", "cubic-bezier(0.11, 0, 0.5, 0)"),
        ("easeOutQuad", "cubic-bezier(0.5, 1, 0.89, 1)"),
        ("easeInOutQuad", "cubic-bezier(0.45, 0, 0.55, 1)"),
        ("easeInCubic", "cubic-bezier(0.32, 0, 0.67, 0)"),
        ("easeOutCubic", "cubic-bezier(0.33, 1, 0.68, 1)"),
        ("easeInOutCubic", "cubic-bezier(0.65, 0, 0.35, 1)"),
        ("easeInQuart", "cubic-bezier(0.5, 0, 0.75, 0)"),
        ("easeOutQuart", "cubic-bezier(0.25, 1, 0.5, 1)"),
        ("easeInOutQuart", "cubic-bezier(0.76, 0, 0.24, 1)"),
        ("easeInQuint", "cubic-bezier(0.64, 0, 0.78, 0)"),
        ("easeOutQuint", "cubic-bezier(0.22, 1, 0.36, 1)"),
        ("easeInOutQuint", "cubic-bezier(0.83, 0, 0.17, 1)"),
        ("easeInExpo", "cubic-bezier(0.7, 0, 0.84, 0)"),
        ("easeOutExpo", "cubic-bezier(0.16, 1, 0.3, 1)"),
        ("easeInOutExpo", "cubic-bezier(0.87, 0, 0.13, 1)"),
        ("easeInCirc", "cubic-bezier(0.55, 0, 1, 0.45)"),
        ("easeOutCirc", "cubic-bezier(0, 0.55, 0.45, 1)"),
        ("easeInOutCirc", "cubic-bezier(0.85, 0, 0.15, 1)"),
        ("easeInBack", "cubic-bezier(0.36, 0, 0.66, -0.56)"),
        ("easeOutBack", "cubic-bezier(0.34, 1.56, 0.64, 1)"),
        ("easeInOutBack", "cubic-bezier(0.68, -0.6, 0.32, 1.6)"),
    ];
    if let Some((_, css)) = BEZIERS.iter().find(|(n, _)| *n == name) {
        return Some(css.to_string());
    }
    let curve: fn(f64) -> f64 = match name {
        "easeInElastic" => |x| 1.0 - out_elastic(1.0 - x),
        "easeOutElastic" => out_elastic,
        "easeInOutElastic" => in_out_elastic,
        "easeInBounce" => |x| 1.0 - out_bounce(1.0 - x),
        "easeOutBounce" => out_bounce,
        "easeInOutBounce" => |x| match x < 0.5 {
            true => (1.0 - out_bounce(1.0 - 2.0 * x)) / 2.0,
            false => (1.0 + out_bounce(2.0 * x - 1.0)) / 2.0,
        },
        _ => return None,
    };
    let stops: Vec<String> = (0..=40)
        .map(|i| js::number_to_string((curve(f64::from(i) / 40.0) * 1000.0).round() / 1000.0))
        .collect();
    Some(format!("linear({})", stops.join(", ")))
}

fn out_elastic(x: f64) -> f64 {
    match x {
        0.0 | 1.0 => x,
        _ => (2.0f64).powf(-10.0 * x) * ((x * 10.0 - 0.75) * 2.0 * PI / 3.0).sin() + 1.0,
    }
}

fn in_out_elastic(x: f64) -> f64 {
    let wave = ((20.0 * x - 11.125) * 2.0 * PI / 4.5).sin();
    match x {
        0.0 | 1.0 => x,
        x if x < 0.5 => -(2.0f64).powf(20.0 * x - 10.0) * wave / 2.0,
        x => (2.0f64).powf(10.0 - 20.0 * x) * wave / 2.0 + 1.0,
    }
}

fn out_bounce(x: f64) -> f64 {
    const K: f64 = 7.5625;
    if x < 1.0 / 2.75 {
        K * x * x
    } else if x < 2.0 / 2.75 {
        let x = x - 1.5 / 2.75;
        K * x * x + 0.75
    } else if x < 2.5 / 2.75 {
        let x = x - 2.25 / 2.75;
        K * x * x + 0.9375
    } else {
        let x = x - 2.625 / 2.75;
        K * x * x + 0.984375
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, t: f64) -> f64 {
        Easing::parse(text).unwrap().at(t)
    }

    #[test]
    fn curves_go_on_past_their_ends() {
        assert!((at("cubic-bezier(0.5, 0.25, 0.5, 0.75)", -1.0) + 0.5).abs() < 1e-12);
        assert!((at("cubic-bezier(0.5, 0.25, 0.5, 0.75)", 2.0) - 1.5).abs() < 1e-12);
        assert_eq!(at("linear(0, 0.5 50%, 1)", 1.5), 1.5);
        assert_eq!(at("linear(0, 0.5 50%, 1)", -0.5), -0.5);
        assert_eq!(at("steps(2)", 1.5), 1.5);
    }

    #[test]
    fn names_that_are_not_css_say_what_is() {
        assert_eq!(
            Easing::parse("easeOutCubic"),
            Err("easing easeOutCubic is not CSS; write cubic-bezier(0.33, 1, 0.68, 1)".into())
        );
        assert_eq!(
            Easing::parse("easeIn"),
            Err("easing easeIn is not CSS; write ease-in".into())
        );
        let bounce = Easing::parse("easeOutBounce").unwrap_err();
        let css = bounce
            .strip_prefix("easing easeOutBounce is not CSS; write ")
            .unwrap();
        assert!((Easing::parse(css).unwrap().at(0.5) - out_bounce(0.5)).abs() < 2e-2);
    }

    #[test]
    fn what_is_no_easing_is_an_error() {
        for bad in [
            "wobbly",
            "cubic-bezier(1, 2, 3)",
            "cubic-bezier(2, 0, 0, 1)",
            "steps(0)",
            "steps(1, jump-none)",
            "steps(1.5)",
            "steps(2, middle)",
            "linear(1)",
            "linear(0, 50%)",
            "linear(0, 1 2 3)",
        ] {
            assert!(Easing::parse(bad).is_err(), "{bad}");
        }
    }
}
