//! Executable common-profile implementation of `valle.audio/gain-multiplier@1`.
//!
//! Keep this module limited to the actual numeric kernel: its exact source bytes are the
//! implementation artifact hashed by `engine_owned_kernel_implementation_digest`.

pub(super) fn valid_multiplier(multiplier: f64) -> bool {
    multiplier.is_finite() && multiplier >= 0.0
}

pub(super) fn combine_multiplier(accumulator: f64, multiplier: f64) -> Option<f64> {
    if !valid_multiplier(multiplier) {
        return None;
    }
    let combined = accumulator * multiplier;
    combined.is_finite().then_some(combined)
}

pub(super) fn apply_multiplier(effect_multiplier: f64, authored_gain: f64) -> f64 {
    effect_multiplier * authored_gain
}
