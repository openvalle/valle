//! Backend-independent terminal delivery math.
//!
//! Executors consume this narrow pixel contract instead of reaching into the RGBA32F reference
//! executor. GPU/Web kernels are required to match these results; CPU executors may call it
//! directly after reading their private working surface.

use crate::resource::{ColorPrimaries, OutputSpec};

pub use super::reference::{
    OutputMathError as DeliveryError, OutputStorage as DeliveryStorage,
    ReferenceOutputSample as DeliverySample, gamut_map as map_output_gamut,
    luminance_weights as output_luminance_weights,
};

/// Linear Rec.2020 to target primaries, using the reference executor's f64 coefficients.
pub const fn output_primaries_matrix(primaries: ColorPrimaries) -> [[f64; 3]; 3] {
    super::reference::working_primaries_matrix(primaries)
}

/// Transforms one Linear Rec.2020 premultiplied working pixel through the complete OutputSpec.
pub fn transform_working_pixel(
    channels: [f32; 4],
    spec: OutputSpec,
    position: [u32; 2],
) -> Result<DeliverySample, DeliveryError> {
    let pixel = super::reference::PremulRgba32::from_premultiplied(channels)?;
    super::reference::transform_output_pixel(pixel, spec, position)
}
