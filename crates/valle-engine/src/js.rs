//! JavaScript's number semantics, the same on every platform and in wasm: `Math` functions through
//! libm instead of the platform's C library, and `Number` to `String` as ECMAScript prints it.
//! Tracing and frame evaluation both go through here, so a value seen while tracing is bit for bit
//! the value a frame computes.

/// A `Math` function scene code may call on traced values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathFn {
    Abs,
    Acos,
    Acosh,
    Asin,
    Asinh,
    Atan,
    Atan2,
    Atanh,
    Cbrt,
    Ceil,
    Clz32,
    Cos,
    Cosh,
    Exp,
    Expm1,
    Floor,
    Fround,
    Hypot,
    Imul,
    Log,
    Log10,
    Log1p,
    Log2,
    Max,
    Min,
    Pow,
    Round,
    Sign,
    Sin,
    Sinh,
    Sqrt,
    Tan,
    Tanh,
    Trunc,
}

impl MathFn {
    pub fn parse(name: &str) -> Option<MathFn> {
        use MathFn::*;
        Some(match name {
            "abs" => Abs,
            "acos" => Acos,
            "acosh" => Acosh,
            "asin" => Asin,
            "asinh" => Asinh,
            "atan" => Atan,
            "atan2" => Atan2,
            "atanh" => Atanh,
            "cbrt" => Cbrt,
            "ceil" => Ceil,
            "clz32" => Clz32,
            "cos" => Cos,
            "cosh" => Cosh,
            "exp" => Exp,
            "expm1" => Expm1,
            "floor" => Floor,
            "fround" => Fround,
            "hypot" => Hypot,
            "imul" => Imul,
            "log" => Log,
            "log10" => Log10,
            "log1p" => Log1p,
            "log2" => Log2,
            "max" => Max,
            "min" => Min,
            "pow" => Pow,
            "round" => Round,
            "sign" => Sign,
            "sin" => Sin,
            "sinh" => Sinh,
            "sqrt" => Sqrt,
            "tan" => Tan,
            "tanh" => Tanh,
            "trunc" => Trunc,
            _ => return None,
        })
    }

    /// The function over its arguments, missing ones `undefined` (NaN), as `Math.f(...args)`.
    pub fn call(self, args: &[f64]) -> f64 {
        use MathFn::*;
        let a = args.first().copied().unwrap_or(f64::NAN);
        let b = args.get(1).copied().unwrap_or(f64::NAN);
        match self {
            Abs => a.abs(),
            Acos => libm::acos(a),
            Acosh => libm::acosh(a),
            Asin => libm::asin(a),
            Asinh => libm::asinh(a),
            Atan => libm::atan(a),
            Atan2 => libm::atan2(a, b),
            Atanh => libm::atanh(a),
            Cbrt => libm::cbrt(a),
            Ceil => a.ceil(),
            Clz32 => f64::from((to_int32(a) as u32).leading_zeros()),
            Cos => libm::cos(a),
            Cosh => libm::cosh(a),
            Exp => libm::exp(a),
            Expm1 => libm::expm1(a),
            Floor => a.floor(),
            Fround => f64::from(a as f32),
            Hypot => hypot(args),
            Imul => f64::from(to_int32(a).wrapping_mul(to_int32(b))),
            Log => libm::log(a),
            Log10 => libm::log10(a),
            Log1p => libm::log1p(a),
            Log2 => libm::log2(a),
            Max => args.iter().fold(f64::NEG_INFINITY, |m, &x| max(m, x)),
            Min => args.iter().fold(f64::INFINITY, |m, &x| min(m, x)),
            Pow => pow(a, b),
            Round => round(a),
            Sign => {
                if a.is_nan() || a == 0.0 {
                    a
                } else {
                    a.signum()
                }
            }
            Sin => libm::sin(a),
            Sinh => libm::sinh(a),
            Sqrt => a.sqrt(),
            Tan => libm::tan(a),
            Tanh => libm::tanh(a),
            Trunc => a.trunc(),
        }
    }
}

/// `a ** b` and `Math.pow`: C's `pow` except where ECMAScript differs, a base of ±1 to an infinite
/// or NaN power being NaN.
pub fn pow(a: f64, b: f64) -> f64 {
    if b.is_nan() || (a.abs() == 1.0 && b.is_infinite()) {
        return f64::NAN;
    }
    libm::pow(a, b)
}

/// `Math.round`: halves round up, toward +∞, and the sign of zero is kept.
fn round(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    if (-0.5..0.0).contains(&x) {
        return -0.0;
    }
    let floor = x.floor();
    // Exact: a double minus its floor needs no rounding.
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// `Math.max` of two: NaN wins, and +0 is above -0.
fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() { b } else { a }
    } else {
        a.max(b)
    }
}

/// `Math.min` of two: NaN wins, and -0 is below +0.
fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() { a } else { b }
    } else {
        a.min(b)
    }
}

/// `Math.hypot`: an infinite argument wins over NaN.
fn hypot(args: &[f64]) -> f64 {
    if args.iter().any(|x| x.is_infinite()) {
        return f64::INFINITY;
    }
    args.iter().fold(0.0, |h, &x| libm::hypot(h, x))
}

/// ECMAScript `ToInt32`.
fn to_int32(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    x.trunc().rem_euclid(4_294_967_296.0) as u32 as i32
}

/// A number as `String(x)` prints it: the shortest digits that read back to `x`, in plain notation
/// from 1e-6 up to (but not including) 1e21, and in exponent notation beyond.
pub fn number_to_string(x: f64) -> String {
    let mut out = String::new();
    push_number(&mut out, x);
    out
}

/// Appends a number without allocating the intermediate strings a template would copy.
pub(crate) fn push_number(out: &mut String, x: f64) {
    out.push_str(ryu_js::Buffer::new().format(x));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_as_javascript_does() {
        let cases = [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-2.5, "-2.5"),
            (0.1 + 0.2, "0.30000000000000004"),
            (123456789.0, "123456789"),
            // Equally close shortest decimals choose an even last digit, unlike `{:e}`.
            (f64::from_bits(0x4319_4a5a_0220_10f5), "1779656215233597.2"),
            (f64::from_bits(0xc2de_89e8_4d3a_04e8), "-134310626912275.62"),
            (1e21, "1e+21"),
            (1.5e21, "1.5e+21"),
            (1e20, "100000000000000000000"),
            (123e18, "123000000000000000000"),
            (1e-6, "0.000001"),
            (1.5e-6, "0.0000015"),
            (1e-7, "1e-7"),
            (-1.25e-7, "-1.25e-7"),
            (f64::MAX, "1.7976931348623157e+308"),
            (5e-324, "5e-324"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
        ];
        for (x, want) in cases {
            assert_eq!(number_to_string(x), want, "{x:e}");
            let mut appended = String::from("prefix ");
            push_number(&mut appended, x);
            appended.push_str(" suffix");
            assert_eq!(appended, format!("prefix {want} suffix"), "{x:e}");
        }
    }

    #[test]
    fn number_strings_match_quickjs_at_boundaries_and_across_float_bits() {
        let mut numbers = vec![0.0, -0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY];
        // Neighbors on both sides of every decimal exponent, including the subnormal range
        // and the boundaries where JavaScript switches between plain and exponent notation.
        for exp in -324..=308 {
            let x: f64 = format!("1e{exp}").parse().unwrap();
            for x in [x.next_down(), x, x.next_up()] {
                numbers.extend([x, -x]);
            }
        }
        let mut bits = 0xd1b5_4a32_d192_ed03u64;
        for _ in 0..10_000 {
            bits ^= bits << 13;
            bits ^= bits >> 7;
            bits ^= bits << 17;
            numbers.push(f64::from_bits(bits));
        }
        let rt = rquickjs::Runtime::new().unwrap();
        let ctx = rquickjs::Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            // QuickJS's native String(), without Valle's runtime or host formatter installed.
            let string: rquickjs::Function = ctx.eval("x => String(x)").unwrap();
            for x in numbers {
                let expected: String = string.call((x,)).unwrap();
                assert_eq!(number_to_string(x), expected, "bits {:016x}", x.to_bits());
                let mut text = String::from("before:");
                push_number(&mut text, x);
                text.push_str(":after");
                assert_eq!(text, format!("before:{expected}:after"));
            }
        });
    }

    #[test]
    fn math_follows_ecmascript() {
        let call = |name: &str, args: &[f64]| MathFn::parse(name).unwrap().call(args);
        assert_eq!(call("round", &[2.5]), 3.0);
        assert_eq!(call("round", &[-2.5]), -2.0);
        assert!(call("round", &[-0.4]).is_sign_negative());
        assert_eq!(call("round", &[0.49999999999999994]), 0.0);
        assert_eq!(call("round", &[4503599627370497.0]), 4503599627370497.0);
        assert!(call("max", &[-0.0, 0.0]).is_sign_positive());
        assert!(call("min", &[0.0, -0.0]).is_sign_negative());
        assert!(call("max", &[1.0, f64::NAN]).is_nan());
        assert_eq!(call("max", &[]), f64::NEG_INFINITY);
        assert_eq!(call("min", &[3.0, 1.0, 2.0]), 1.0);
        assert!(call("sign", &[-0.0]).is_sign_negative());
        assert_eq!(call("sign", &[-3.0]), -1.0);
        assert_eq!(call("hypot", &[f64::NAN, f64::INFINITY]), f64::INFINITY);
        assert_eq!(call("hypot", &[3.0, 4.0]), 5.0);
        assert_eq!(call("hypot", &[]), 0.0);
        assert_eq!(call("clz32", &[1.0]), 31.0);
        assert_eq!(call("clz32", &[-1.0]), 0.0);
        assert_eq!(call("imul", &[0xffff_ffff_u32 as f64, 5.0]), -5.0);
        assert_eq!(call("fround", &[5.5]), 5.5);
        assert_eq!(call("fround", &[5.05]), 5.050000190734863);
        assert!(call("sin", &[]).is_nan());
        assert!(pow(1.0, f64::INFINITY).is_nan());
        assert!(pow(-1.0, f64::NEG_INFINITY).is_nan());
        assert!(pow(1.0, f64::NAN).is_nan());
        assert_eq!(pow(f64::NAN, 0.0), 1.0);
        assert_eq!(pow(2.0, 10.0), 1024.0);
        assert!(MathFn::parse("random").is_none());
    }
}
