//! Deterministic transcendental math shared by native and WASM. Use the pinned pure-Rust libm
//! implementation rather than platform f64 methods. Changing the math engine is a semantic change
//! requiring numerical parity verification.

/// Math implementation identity, paired with an exact Cargo version pin.
pub const ENGINE_ID: &str = "libm-rs@0.2.16";

#[inline]
pub fn sqrt(value: f64) -> f64 {
    libm::sqrt(value)
}

#[inline]
pub fn cbrt(value: f64) -> f64 {
    libm::cbrt(value)
}

#[inline]
pub fn hypot(x: f64, y: f64) -> f64 {
    libm::hypot(x, y)
}

#[inline]
pub fn hypot_f32(x: f32, y: f32) -> f32 {
    libm::hypotf(x, y)
}

#[inline]
pub fn sin(value: f64) -> f64 {
    libm::sin(value)
}

#[inline]
pub fn cos(value: f64) -> f64 {
    libm::cos(value)
}

#[inline]
pub fn sin_cos(value: f64) -> (f64, f64) {
    libm::sincos(value)
}

#[inline]
pub fn sin_cos_f32(value: f32) -> (f32, f32) {
    // Evaluate the f32 input with the pinned f64 routine and round once on output.
    let (sin, cos) = libm::sincos(f64::from(value));
    (sin as f32, cos as f32)
}

#[inline]
pub fn tan(value: f64) -> f64 {
    libm::tan(value)
}

#[inline]
pub fn tan_f32(value: f32) -> f32 {
    // Match sin_cos_f32's single, deterministic f32 rounding step.
    libm::tan(f64::from(value)) as f32
}

#[inline]
pub fn asin(value: f64) -> f64 {
    libm::asin(value)
}

#[inline]
pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

#[inline]
pub fn exp(value: f64) -> f64 {
    libm::exp(value)
}

#[inline]
pub fn ln(value: f64) -> f64 {
    libm::log(value)
}

#[inline]
pub fn pow(base: f64, exponent: f64) -> f64 {
    libm::pow(base, exponent)
}

/// Use deterministic logarithms for prepare-time tick selection.
#[inline]
pub fn log10(value: f64) -> f64 {
    libm::log10(value)
}

/// Cross-target acceptance corpus. Public only so target-specific runners execute the exact same
/// input table; this is not an authoring or rendering API.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeterminismCorpus {
    pub engine: &'static str,
    pub samples: Vec<DeterminismSample>,
}

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeterminismSample {
    pub inputs: Vec<String>,
    pub operation: &'static str,
    pub output: String,
}

#[doc(hidden)]
pub fn determinism_corpus() -> DeterminismCorpus {
    fn bits(value: f64) -> String {
        format!("{:016x}", value.to_bits())
    }
    fn unary(
        operation: &'static str,
        inputs: &[f64],
        f: impl Fn(f64) -> f64,
        out: &mut Vec<DeterminismSample>,
    ) {
        for &input in inputs {
            out.push(DeterminismSample {
                inputs: vec![bits(input)],
                operation,
                output: bits(f(input)),
            });
        }
    }
    fn binary(
        operation: &'static str,
        inputs: &[(f64, f64)],
        f: impl Fn(f64, f64) -> f64,
        out: &mut Vec<DeterminismSample>,
    ) {
        for &(left, right) in inputs {
            out.push(DeterminismSample {
                inputs: vec![bits(left), bits(right)],
                operation,
                output: bits(f(left, right)),
            });
        }
    }

    let mut samples = Vec::new();
    unary(
        "cbrt",
        &[-1.0, -0.1, -0.0, 0.0, 0.1, 1.0, 1e-300, 1e300],
        cbrt,
        &mut samples,
    );
    binary(
        "hypot",
        &[
            (0.0, 0.0),
            (3.0, 4.0),
            (0.1, -0.3),
            (1e-300, 1e-300),
            (1e300, 1e300),
        ],
        hypot,
        &mut samples,
    );
    unary(
        "sqrt",
        &[0.0, -0.0, 0.5, 2.0, 1e-300, 1e300],
        sqrt,
        &mut samples,
    );
    unary(
        "sin",
        &[-1e6, -core::f64::consts::PI, -0.0, 0.1, 1.0, 1e6],
        sin,
        &mut samples,
    );
    unary(
        "cos",
        &[-1e6, -core::f64::consts::PI, -0.0, 0.1, 1.0, 1e6],
        cos,
        &mut samples,
    );
    unary("tan", &[-1.0, -0.1, 0.0, 0.1, 1.0], tan, &mut samples);
    unary("asin", &[-1.0, -0.25, 0.0, 0.75, 1.0], asin, &mut samples);
    binary(
        "atan2",
        &[
            (0.0, 1.0),
            (-0.0, -1.0),
            (1.0, 1.0),
            (-2.0, 3.0),
            (1e200, -1e200),
        ],
        atan2,
        &mut samples,
    );
    unary("exp", &[-20.0, -1.0, -0.0, 1.0, 10.0], exp, &mut samples);
    unary("ln", &[1e-300, 0.1, 1.0, 2.0, 1e300], ln, &mut samples);
    binary(
        "pow",
        &[
            (0.1, 2.4),
            (0.5, 0.25),
            (2.0, -3.0),
            (10.0, 0.1),
            (1e100, 0.5),
        ],
        pow,
        &mut samples,
    );

    DeterminismCorpus {
        engine: ENGINE_ID,
        samples: {
            samples.extend(crate::program::gradient_determinism_samples());
            samples
        },
    }
}
