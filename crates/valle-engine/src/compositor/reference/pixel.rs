use thiserror::Error;

use crate::resource::Extent2d;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferenceTolerance {
    pub absolute: f32,
    pub relative: f32,
}

impl ReferenceTolerance {
    pub fn matches(self, actual: f32, reference: f32) -> bool {
        actual.is_finite()
            && reference.is_finite()
            && (actual - reference).abs() <= self.absolute.max(self.relative * reference.abs())
    }
}

/// Frozen comparison budget for future RGBA16F production executors against RGBA32F reference.
pub const F16_REFERENCE_TOLERANCE: ReferenceTolerance = ReferenceTolerance {
    absolute: 0.0001,
    relative: 0.001,
};

/// One Linear Rec.2020 D65 pixel with premultiplied coverage alpha.
///
/// RGB may be negative or greater than one. Alpha is always in `[0, 1]`; zero-alpha pixels always
/// have exactly zero RGB. Construction and every fallible operation reject non-finite results.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PremulRgba32 {
    channels: [f32; 4],
}

impl PremulRgba32 {
    pub const TRANSPARENT: Self = Self { channels: [0.0; 4] };

    pub fn from_premultiplied(channels: [f32; 4]) -> Result<Self, PixelError> {
        validate_finite(&channels)?;
        let alpha = channels[3];
        validate_unit(alpha, "alpha")?;
        if alpha == 0.0 {
            if channels[..3].iter().any(|value| *value != 0.0) {
                return Err(PixelError::NonZeroTransparentRgb);
            }
            return Ok(Self::TRANSPARENT);
        }
        Ok(Self { channels })
    }

    pub fn from_straight(rgb: [f32; 3], alpha: f32) -> Result<Self, PixelError> {
        validate_finite(&rgb)?;
        validate_unit(alpha, "alpha")?;
        if alpha == 0.0 {
            return Ok(Self::TRANSPARENT);
        }
        Self::from_premultiplied([
            mul_finite(rgb[0], alpha)?,
            mul_finite(rgb[1], alpha)?,
            mul_finite(rgb[2], alpha)?,
            alpha,
        ])
    }

    pub const fn channels(self) -> [f32; 4] {
        self.channels
    }

    pub const fn rgb(self) -> [f32; 3] {
        [self.channels[0], self.channels[1], self.channels[2]]
    }

    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }

    pub fn straight_rgb(self) -> Result<[f32; 3], PixelError> {
        let alpha = self.alpha();
        if alpha == 0.0 {
            return Ok([0.0; 3]);
        }
        Ok([
            div_finite(self.channels[0], alpha)?,
            div_finite(self.channels[1], alpha)?,
            div_finite(self.channels[2], alpha)?,
        ])
    }

    /// Porter-Duff source-over in the working linear premultiplied domain.
    pub fn source_over(self, destination: Self) -> Result<Self, PixelError> {
        // These exact premultiplied identities dominate sparse effect contributions such as
        // Glass. Avoid four checked f64 operations for every transparent ROI padding pixel, and
        // avoid touching the destination when an opaque source replaces it completely.
        if self.alpha() == 0.0 {
            return Ok(destination);
        }
        if self.alpha() == 1.0 {
            return Ok(self);
        }
        let inverse_source_alpha = 1.0 - self.alpha();
        Self::from_premultiplied([
            add_finite(
                self.channels[0],
                mul_finite(destination.channels[0], inverse_source_alpha)?,
            )?,
            add_finite(
                self.channels[1],
                mul_finite(destination.channels[1], inverse_source_alpha)?,
            )?,
            add_finite(
                self.channels[2],
                mul_finite(destination.channels[2], inverse_source_alpha)?,
            )?,
            add_finite(
                self.alpha(),
                mul_finite(destination.alpha(), inverse_source_alpha)?,
            )?,
        ])
    }

    /// Multiply both premultiplied RGB and coverage alpha by the same factor.
    pub fn scale_coverage(self, factor: f32) -> Result<Self, PixelError> {
        validate_unit(factor, "coverage")?;
        if factor == 0.0 || self.alpha() == 0.0 {
            return Ok(Self::TRANSPARENT);
        }
        Self::from_premultiplied([
            mul_finite(self.channels[0], factor)?,
            mul_finite(self.channels[1], factor)?,
            mul_finite(self.channels[2], factor)?,
            mul_finite(self.channels[3], factor)?,
        ])
    }

    pub fn approx_eq(self, other: Self, tolerance: f32) -> bool {
        tolerance.is_finite()
            && tolerance >= 0.0
            && self
                .channels
                .iter()
                .zip(other.channels)
                .all(|(left, right)| (*left - right).abs() <= tolerance)
    }
}

/// Small, explicit reference image. Pixel count is checked against the extent.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceImage {
    extent: Extent2d,
    pixels: Vec<PremulRgba32>,
}

impl ReferenceImage {
    pub fn new(extent: Extent2d, pixels: Vec<PremulRgba32>) -> Result<Self, PixelError> {
        let expected = pixel_len(extent)?;
        if pixels.len() != expected {
            return Err(PixelError::PixelCount {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self { extent, pixels })
    }

    pub fn solid(extent: Extent2d, pixel: PremulRgba32) -> Result<Self, PixelError> {
        Ok(Self {
            extent,
            pixels: vec![pixel; pixel_len(extent)?],
        })
    }

    pub fn transparent(extent: Extent2d) -> Result<Self, PixelError> {
        Self::solid(extent, PremulRgba32::TRANSPARENT)
    }

    pub const fn extent(&self) -> Extent2d {
        self.extent
    }

    pub fn pixels(&self) -> &[PremulRgba32] {
        &self.pixels
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<PremulRgba32> {
        if x >= self.extent.width() || y >= self.extent.height() {
            return None;
        }
        let index = y as usize * self.extent.width() as usize + x as usize;
        self.pixels.get(index).copied()
    }

    pub fn source_over(&self, destination: &Self) -> Result<Self, PixelError> {
        self.ensure_same_extent(destination)?;
        // Start from the destination's contiguous copy and touch only covered source pixels.
        // Effect contributions are intentionally sparse inside their conservative sampling ROI;
        // collecting a Result for every transparent padding pixel dominated reference compositing.
        let mut output = destination.clone();
        for (target, source) in output.pixels.iter_mut().zip(&self.pixels) {
            if source.alpha() != 0.0 {
                *target = source.source_over(*target)?;
            }
        }
        Ok(output)
    }

    pub fn scale_coverage(&self, factor: f32) -> Result<Self, PixelError> {
        Self::new(
            self.extent,
            self.pixels
                .iter()
                .map(|pixel| pixel.scale_coverage(factor))
                .collect::<Result<_, _>>()?,
        )
    }

    pub(crate) fn ensure_same_extent(&self, other: &Self) -> Result<(), PixelError> {
        if self.extent == other.extent {
            Ok(())
        } else {
            Err(PixelError::ExtentMismatch {
                left: self.extent,
                right: other.extent,
            })
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum PixelError {
    #[error("pixel channel must be finite")]
    NonFinite,
    #[error("{name} must be within [0, 1], got {value}")]
    OutOfUnitRange { name: &'static str, value: f32 },
    #[error("premultiplied RGB must be exactly zero when alpha is zero")]
    NonZeroTransparentRgb,
    #[error("pixel arithmetic overflowed the finite f32 domain")]
    ArithmeticOverflow,
    #[error("image pixel count mismatch: expected {expected}, got {actual}")]
    PixelCount { expected: usize, actual: usize },
    #[error("image extents differ: {left:?} vs {right:?}")]
    ExtentMismatch { left: Extent2d, right: Extent2d },
    #[error("image extent exceeds addressable memory")]
    ImageTooLarge,
}

pub(crate) fn validate_unit(value: f32, name: &'static str) -> Result<(), PixelError> {
    if !value.is_finite() {
        Err(PixelError::NonFinite)
    } else if !(0.0..=1.0).contains(&value) {
        Err(PixelError::OutOfUnitRange { name, value })
    } else {
        Ok(())
    }
}

fn validate_finite(values: &[f32]) -> Result<(), PixelError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(PixelError::NonFinite)
    }
}

fn mul_finite(left: f32, right: f32) -> Result<f32, PixelError> {
    finite_result(f64::from(left) * f64::from(right))
}

fn add_finite(left: f32, right: f32) -> Result<f32, PixelError> {
    finite_result(f64::from(left) + f64::from(right))
}

fn div_finite(left: f32, right: f32) -> Result<f32, PixelError> {
    finite_result(f64::from(left) / f64::from(right))
}

fn finite_result(value: f64) -> Result<f32, PixelError> {
    let result = value as f32;
    if result.is_finite() {
        Ok(result)
    } else {
        Err(PixelError::ArithmeticOverflow)
    }
}

fn pixel_len(extent: Extent2d) -> Result<usize, PixelError> {
    (extent.width() as usize)
        .checked_mul(extent.height() as usize)
        .ok_or(PixelError::ImageTooLarge)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_rgb_is_canonical() {
        assert_eq!(
            PremulRgba32::from_straight([100.0, -3.0, 7.0], 0.0).unwrap(),
            PremulRgba32::TRANSPARENT
        );
        assert!(PremulRgba32::from_premultiplied([1.0, 0.0, 0.0, 0.0]).is_err());
    }

    #[test]
    fn source_over_matches_hand_calculation_and_keeps_hdr_rgb() {
        let source = PremulRgba32::from_straight([2.0, 0.0, -0.5], 0.25).unwrap();
        let destination = PremulRgba32::from_straight([0.0, 1.0, 0.5], 0.5).unwrap();
        let result = source.source_over(destination).unwrap();
        assert!(result.approx_eq(
            PremulRgba32::from_premultiplied([0.5, 0.375, 0.0625, 0.625]).unwrap(),
            1e-7
        ));
    }

    #[test]
    fn source_over_is_associative_for_reference_fixtures() {
        let values = [
            PremulRgba32::TRANSPARENT,
            PremulRgba32::from_straight([1.0, 0.0, 0.0], 0.25).unwrap(),
            PremulRgba32::from_straight([0.0, 2.0, -0.5], 0.5).unwrap(),
            PremulRgba32::from_straight([0.1, 0.2, 0.3], 1.0).unwrap(),
        ];
        for a in values {
            for b in values {
                for c in values {
                    let left = a.source_over(b).unwrap().source_over(c).unwrap();
                    let right = a.source_over(b.source_over(c).unwrap()).unwrap();
                    assert!(left.approx_eq(right, 2e-7), "{left:?} != {right:?}");
                }
            }
        }
    }

    #[test]
    fn f16_production_tolerance_is_explicit() {
        assert!(F16_REFERENCE_TOLERANCE.matches(1.0005, 1.0));
        assert!(F16_REFERENCE_TOLERANCE.matches(0.00005, 0.0));
        assert!(!F16_REFERENCE_TOLERANCE.matches(1.01, 1.0));
    }
}
