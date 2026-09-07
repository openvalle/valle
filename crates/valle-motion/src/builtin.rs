//! Deterministic author builtins shared by compile-time folding and frame-time evaluation.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{MathBinaryOp, MathUnaryOp, NumberFormat};

pub const MAX_FORMAT_OUTPUT_BYTES: usize = 256;

pub fn math_unary(op: MathUnaryOp, value: f64) -> Result<f64, &'static str> {
    if !value.is_finite() {
        return Err("math input must be finite");
    }
    let result = match op {
        MathUnaryOp::Sqrt => valle_draw::math::sqrt(value),
        MathUnaryOp::Exp => valle_draw::math::exp(value),
        MathUnaryOp::Sin => valle_draw::math::sin(value),
        MathUnaryOp::Cos => valle_draw::math::cos(value),
        MathUnaryOp::Tan => valle_draw::math::tan(value),
        MathUnaryOp::Floor => value.floor(),
        MathUnaryOp::Ceil => value.ceil(),
        MathUnaryOp::Round => ecma_round(value),
        MathUnaryOp::Trunc => value.trunc(),
        MathUnaryOp::Fract => value - value.floor(),
    };
    result
        .is_finite()
        .then_some(result)
        .ok_or("math result is not finite")
}

pub fn math_binary(op: MathBinaryOp, lhs: f64, rhs: f64) -> Result<f64, &'static str> {
    if !lhs.is_finite() || !rhs.is_finite() {
        return Err("math inputs must be finite");
    }
    let result = match op {
        MathBinaryOp::Atan2 => valle_draw::math::atan2(lhs, rhs),
        MathBinaryOp::Pow => valle_draw::math::pow(lhs, rhs),
        MathBinaryOp::Mod => {
            if rhs <= 0.0 {
                return Err("mod period must be finite and positive");
            }
            lhs.rem_euclid(rhs)
        }
        MathBinaryOp::Remainder => {
            if rhs == 0.0 {
                return Err("remainder divisor must not be zero");
            }
            lhs % rhs
        }
        MathBinaryOp::PingPong => {
            if rhs <= 0.0 {
                return Err("pingPong length must be finite and positive");
            }
            let period = rhs * 2.0;
            if !period.is_finite() {
                return Err("pingPong period overflowed");
            }
            rhs - (lhs.rem_euclid(period) - rhs).abs()
        }
    };
    result
        .is_finite()
        .then_some(result)
        .ok_or("math result is not finite")
}

/// ECMAScript `Math.round`: ties move toward +∞ and negative zero is observable.
pub fn ecma_round(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let rounded = (value + 0.5).floor();
    if rounded == 0.0 && value.is_sign_negative() {
        -0.0
    } else {
        rounded
    }
}

pub fn format_number(value: f64, format: NumberFormat) -> Result<String, &'static str> {
    format.validate()?;
    if !value.is_finite() {
        return Err("format input must be finite");
    }
    let mut value = match format {
        NumberFormat::Percent { .. } => value * 100.0,
        _ => value,
    };
    if !value.is_finite() {
        return Err("format result overflowed");
    }
    let output = match format {
        NumberFormat::Number { decimals, grouping }
        | NumberFormat::Percent { decimals, grouping } => {
            let mut text = format!("{:.*}", usize::from(decimals), value);
            if grouping {
                text = group_ascii(&text);
            }
            if matches!(format, NumberFormat::Percent { .. }) {
                text.push('%');
            }
            text
        }
        NumberFormat::Pad { width } => {
            value = ecma_round(value);
            let negative = value.is_sign_negative();
            let digits = format!("{:.0}", value.abs());
            let zeros = usize::from(width).saturating_sub(digits.len());
            format!(
                "{}{}{}",
                if negative { "-" } else { "" },
                "0".repeat(zeros),
                digits
            )
        }
    };
    (output.len() <= MAX_FORMAT_OUTPUT_BYTES)
        .then_some(output)
        .ok_or("formatted output exceeds 256 UTF-8 bytes")
}

fn group_ascii(text: &str) -> String {
    let (sign, unsigned) = text
        .strip_prefix('-')
        .map_or(("", text), |rest| ("-", rest));
    let (integer, fraction) = unsigned
        .split_once('.')
        .map_or((unsigned, None), |(a, b)| (a, Some(b)));
    let mut grouped = String::with_capacity(text.len() + integer.len() / 3);
    grouped.push_str(sign);
    for (index, ch) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    if let Some(fraction) = fraction {
        grouped.push('.');
        grouped.push_str(fraction);
    }
    grouped
}

#[derive(Deserialize)]
struct Request {
    op: String,
    #[serde(default)]
    args: Value,
}

/// Static-sandbox bridge. The JS shim only marshals values; every algorithm stays in Rust.
pub fn dispatch(request: &str) -> String {
    let result = run(request);
    match result {
        Ok(value) => json!({ "ok": value }).to_string(),
        Err(error) => json!({ "error": error }).to_string(),
    }
}

fn run(request: &str) -> Result<Value, String> {
    let request: Request = serde_json::from_str(request).map_err(|error| error.to_string())?;
    let number = |name: &str| {
        request
            .args
            .get(name)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("`{name}` must be a finite number"))
    };
    let value = match request.op.as_str() {
        "sqrt" => json!(math_unary(MathUnaryOp::Sqrt, number("value")?).map_err(str::to_owned)?),
        "exp" => json!(math_unary(MathUnaryOp::Exp, number("value")?).map_err(str::to_owned)?),
        "sin" => json!(math_unary(MathUnaryOp::Sin, number("value")?).map_err(str::to_owned)?),
        "cos" => json!(math_unary(MathUnaryOp::Cos, number("value")?).map_err(str::to_owned)?),
        "tan" => json!(math_unary(MathUnaryOp::Tan, number("value")?).map_err(str::to_owned)?),
        "floor" => json!(math_unary(MathUnaryOp::Floor, number("value")?).map_err(str::to_owned)?),
        "ceil" => json!(math_unary(MathUnaryOp::Ceil, number("value")?).map_err(str::to_owned)?),
        "round" => json!(math_unary(MathUnaryOp::Round, number("value")?).map_err(str::to_owned)?),
        "trunc" => json!(math_unary(MathUnaryOp::Trunc, number("value")?).map_err(str::to_owned)?),
        "fract" => json!(math_unary(MathUnaryOp::Fract, number("value")?).map_err(str::to_owned)?),
        "atan2" => json!(
            math_binary(MathBinaryOp::Atan2, number("lhs")?, number("rhs")?)
                .map_err(str::to_owned)?
        ),
        "pow" => json!(
            math_binary(MathBinaryOp::Pow, number("lhs")?, number("rhs")?)
                .map_err(str::to_owned)?
        ),
        "mod" => json!(
            math_binary(MathBinaryOp::Mod, number("lhs")?, number("rhs")?)
                .map_err(str::to_owned)?
        ),
        "pingPong" => json!(
            math_binary(MathBinaryOp::PingPong, number("lhs")?, number("rhs")?)
                .map_err(str::to_owned)?
        ),
        "noise1d" => json!(crate::compute::noise::value_noise_1d(
            seed(&request.args)?,
            number("x")?
        )),
        "noise2d" => json!(crate::compute::noise::value_noise_2d(
            seed(&request.args)?,
            number("x")?,
            number("y")?
        )),
        "format" => {
            let format: NumberFormat = serde_json::from_value(
                request
                    .args
                    .get("format")
                    .cloned()
                    .ok_or("missing format")?,
            )
            .map_err(|error| error.to_string())?;
            json!(format_number(number("value")?, format).map_err(str::to_owned)?)
        }
        other => return Err(format!("unknown Motion builtin `{other}`")),
    };
    Ok(value)
}

fn seed(args: &Value) -> Result<u64, String> {
    let value = args
        .get("seed")
        .and_then(Value::as_f64)
        .ok_or("`seed` must be a number")?;
    if !value.is_finite()
        || value < 0.0
        || value.fract() != 0.0
        || value >= 18_446_744_073_709_551_616.0
    {
        return Err("`seed` must be a non-negative integer below 2^64".into());
    }
    Ok(value as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecmascript_round_ties_and_negative_zero_are_exact() {
        for (input, expected) in [
            (-1.5_f64, -1.0_f64),
            (-0.5, -0.0),
            (-0.1, -0.0),
            (0.5, 1.0),
            (1.5, 2.0),
        ] {
            assert_eq!(ecma_round(input).to_bits(), expected.to_bits(), "{input}");
        }
        assert_eq!(
            math_unary(MathUnaryOp::Trunc, -0.0).unwrap().to_bits(),
            (-0.0_f64).to_bits()
        );
        assert_eq!(
            math_unary(MathUnaryOp::Tan, 0.25).unwrap().to_bits(),
            valle_draw::math::tan(0.25).to_bits()
        );
        assert!(math_unary(MathUnaryOp::Sqrt, -1.0).is_err());
    }

    #[test]
    fn euclidean_animation_math_has_closed_ranges() {
        assert_eq!(math_binary(MathBinaryOp::Pow, 4.0, 0.5), Ok(2.0));
        assert!(math_binary(MathBinaryOp::Pow, -1.0, 0.5).is_err());
        assert_eq!(math_binary(MathBinaryOp::Mod, -1.0, 5.0), Ok(4.0));
        assert_eq!(math_binary(MathBinaryOp::Remainder, -5.5, 2.0), Ok(-1.5));
        assert_eq!(
            math_binary(MathBinaryOp::Remainder, 1e308, 1e-308),
            Ok(1e308 % 1e-308)
        );
        for value in -20..=20 {
            let ping = math_binary(MathBinaryOp::PingPong, f64::from(value), 3.0).unwrap();
            assert!((0.0..=3.0).contains(&ping), "{value} -> {ping}");
        }
        assert!(math_binary(MathBinaryOp::Mod, 1.0, 0.0).is_err());
        assert!(math_binary(MathBinaryOp::Remainder, 1.0, 0.0).is_err());
    }

    #[test]
    fn number_formatting_is_ascii_and_bounded() {
        assert_eq!(
            format_number(
                1234.5,
                NumberFormat::Number {
                    decimals: 2,
                    grouping: true,
                },
            ),
            Ok("1,234.50".into())
        );
        assert_eq!(
            format_number(
                0.125,
                NumberFormat::Percent {
                    decimals: 1,
                    grouping: false,
                },
            ),
            Ok("12.5%".into())
        );
        assert_eq!(
            format_number(-7.0, NumberFormat::Pad { width: 3 }),
            Ok("-007".into())
        );
        assert!(
            format_number(
                1e300,
                NumberFormat::Number {
                    decimals: 12,
                    grouping: true,
                },
            )
            .is_err()
        );
    }
}
