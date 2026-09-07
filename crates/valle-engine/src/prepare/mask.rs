use serde::{Deserialize, Serialize};
use valle_draw::math::sqrt;

use super::{BoundsError, DeviceTransform};

/// Timeline `feather` denotes an authored radius. The compositor freezes that radius as a
/// Gaussian sigma before planning so executors never reinterpret canvas units.
pub const MASK_FEATHER_SIGMA_PER_RADIUS: f64 = 0.5;

/// Mask feather uses one deterministic finite Gaussian support domain on every executor.
pub const MASK_GAUSSIAN_SUPPORT_SIGMAS: f64 = 4.0;

/// Fully prepared geometric mask. `device_from_mask` maps the unit mask shape to output-device
/// pixels; no canvas-normalized geometry or camera state is allowed past Prepare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreparedMaskShape {
    Rect,
    Ellipse,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PreparedMaskWire", into = "PreparedMaskWire")]
pub struct PreparedMask {
    shape: PreparedMaskShape,
    device_from_mask: DeviceTransform,
    feather_sigma_device_px: f64,
    invert: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PreparedMaskGeometry {
    pub center_device_px: [f64; 2],
    pub axis_x: [f64; 2],
    pub axis_y: [f64; 2],
    pub half_extent_device_px: [f64; 2],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedMaskWire {
    shape: PreparedMaskShape,
    device_from_mask: DeviceTransform,
    feather_sigma_device_px: f64,
    invert: bool,
}

impl TryFrom<PreparedMaskWire> for PreparedMask {
    type Error = BoundsError;

    fn try_from(value: PreparedMaskWire) -> Result<Self, Self::Error> {
        Self::new(
            value.shape,
            value.device_from_mask,
            value.feather_sigma_device_px,
            value.invert,
        )
    }
}

impl From<PreparedMask> for PreparedMaskWire {
    fn from(value: PreparedMask) -> Self {
        Self {
            shape: value.shape,
            device_from_mask: value.device_from_mask,
            feather_sigma_device_px: value.feather_sigma_device_px,
            invert: value.invert,
        }
    }
}

impl PreparedMask {
    pub(crate) fn new(
        shape: PreparedMaskShape,
        device_from_mask: DeviceTransform,
        feather_sigma_device_px: f64,
        invert: bool,
    ) -> Result<Self, BoundsError> {
        let value = Self {
            shape,
            device_from_mask,
            feather_sigma_device_px,
            invert,
        };
        value.validate_wire()?;
        Ok(value)
    }

    pub const fn shape(self) -> PreparedMaskShape {
        self.shape
    }

    pub const fn device_from_mask(self) -> DeviceTransform {
        self.device_from_mask
    }

    pub const fn feather_sigma_device_px(self) -> f64 {
        self.feather_sigma_device_px
    }

    pub const fn invert(self) -> bool {
        self.invert
    }

    pub fn support_radius_device_px(self) -> f64 {
        self.feather_sigma_device_px * MASK_GAUSSIAN_SUPPORT_SIGMAS
    }

    pub(crate) fn geometry(self) -> Result<PreparedMaskGeometry, BoundsError> {
        let matrix = self.device_from_mask.matrix();
        if matrix[6] != 0.0 || matrix[7] != 0.0 || matrix[8] != 1.0 {
            return Err(BoundsError::InvalidPreparedMask);
        }
        let x = [matrix[0], matrix[3]];
        let y = [matrix[1], matrix[4]];
        let width = sqrt(x[0] * x[0] + x[1] * x[1]);
        let height = sqrt(y[0] * y[0] + y[1] * y[1]);
        let dot = x[0] * y[0] + x[1] * y[1];
        let determinant = x[0] * y[1] - x[1] * y[0];
        if ![width, height, dot, determinant]
            .iter()
            .all(|value| value.is_finite())
            || width <= 0.0
            || height <= 0.0
            || determinant <= 0.0
            || dot.abs() > width * height * 1.0e-10
        {
            return Err(BoundsError::InvalidPreparedMask);
        }
        Ok(PreparedMaskGeometry {
            center_device_px: [
                matrix[2] + (x[0] + y[0]) * 0.5,
                matrix[5] + (x[1] + y[1]) * 0.5,
            ],
            axis_x: [x[0] / width, x[1] / width],
            axis_y: [y[0] / height, y[1] / height],
            half_extent_device_px: [width * 0.5, height * 0.5],
        })
    }

    pub(crate) fn validate_wire(self) -> Result<(), BoundsError> {
        self.device_from_mask.validate_wire()?;
        self.geometry()?;
        let support = self.support_radius_device_px();
        if !self.feather_sigma_device_px.is_finite()
            || self.feather_sigma_device_px < 0.0
            || !support.is_finite()
            || support > super::bounds::MAX_DEVICE_COORDINATE
        {
            return Err(BoundsError::InvalidPreparedMask);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_is_device_only_and_rejects_unsupported_geometry() {
        let mask = PreparedMask::new(
            PreparedMaskShape::Ellipse,
            DeviceTransform::from_affine([40.0, 0.0, 10.0, 0.0, 20.0, 5.0]),
            2.0,
            false,
        )
        .unwrap();
        let wire = serde_json::to_value(mask).unwrap();
        assert_eq!(wire["featherSigmaDevicePx"], serde_json::json!(2.0));
        assert!(wire.get("x").is_none());
        assert!(wire.get("feather").is_none());
        assert_eq!(
            serde_json::from_value::<PreparedMask>(wire.clone()).unwrap(),
            mask
        );

        let mut unsupported = wire;
        unsupported["x"] = serde_json::json!(0.5);
        assert!(serde_json::from_value::<PreparedMask>(unsupported).is_err());
    }

    #[test]
    fn wire_rejects_negative_or_unbounded_feather() {
        let mask = PreparedMask::new(
            PreparedMaskShape::Rect,
            DeviceTransform::from_affine([40.0, 0.0, 10.0, 0.0, 20.0, 5.0]),
            2.0,
            true,
        )
        .unwrap();
        let wire = serde_json::to_value(mask).unwrap();

        let mut negative = wire.clone();
        negative["featherSigmaDevicePx"] = serde_json::json!(-1.0);
        assert!(serde_json::from_value::<PreparedMask>(negative).is_err());

        let mut unbounded = wire;
        unbounded["featherSigmaDevicePx"] =
            serde_json::json!(super::super::bounds::MAX_DEVICE_COORDINATE);
        assert!(serde_json::from_value::<PreparedMask>(unbounded).is_err());
    }

    #[test]
    fn wire_rejects_projective_sheared_or_reflected_shapes() {
        let mask = PreparedMask::new(
            PreparedMaskShape::Rect,
            DeviceTransform::from_affine([40.0, 0.0, 10.0, 0.0, 20.0, 5.0]),
            0.0,
            false,
        )
        .unwrap();
        let wire = serde_json::to_value(mask).unwrap();

        let mut projective = wire.clone();
        projective["deviceFromMask"]["matrix"][6] = serde_json::json!(0.001);
        assert!(serde_json::from_value::<PreparedMask>(projective).is_err());

        let mut sheared = wire.clone();
        sheared["deviceFromMask"]["matrix"][1] = serde_json::json!(1.0);
        assert!(serde_json::from_value::<PreparedMask>(sheared).is_err());

        let mut reflected = wire;
        reflected["deviceFromMask"]["matrix"][4] = serde_json::json!(-20.0);
        assert!(serde_json::from_value::<PreparedMask>(reflected).is_err());
    }
}
