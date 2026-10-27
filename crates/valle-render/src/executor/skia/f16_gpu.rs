//! Explicit binary16 rounding at effect-pass boundaries. Some Vulkan drivers truncate
//! fragment output to F16; a pyramid must use the CPU reference's nearest-even rounding.

pub(super) fn rounded_shader(source: &str) -> std::borrow::Cow<'_, str> {
    if !cfg!(all(feature = "native", any(target_os = "linux", target_os = "windows"))) {
        return source.into();
    }
    // Keep calculations in float until the explicit rounding below. All existing pyramid
    // entry points return half4, which can otherwise narrow before the wrapper runs.
    let body = source.replace("half4", "float4").replace("main(float2", "effectValue(float2");
    format!("{ROUND}\n{body}\nhalf4 main(float2 xy) {{ return roundF16(effectValue(xy)); }}").into()
}

const ROUND: &str = r#"
float roundF16Channel(float value) {
    float magnitude = min(abs(value), 65504.0);
    // Binary16 has ten fraction bits and subnormal spacing of 2^-24.
    float exponent = floor(log2(max(magnitude, 0.00006103515625)));
    float step = exp2(exponent - 10.0);
    float units = magnitude / step;
    float lower = floor(units);
    float fraction = units - lower;
    float up = fraction > 0.5 || (fraction == 0.5 && mod(lower, 2.0) != 0.0) ? 1.0 : 0.0;
    return sign(value) * (lower + up) * step;
}
half4 roundF16(float4 value) {
    return half4(roundF16Channel(value.r), roundF16Channel(value.g),
                 roundF16Channel(value.b), roundF16Channel(value.a));
}
"#;
