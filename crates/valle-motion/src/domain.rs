use serde::{Deserialize, Serialize};

/// Logical pixel viewport used when a Motion artifact is compiled and laid out.
///
/// This is a Motion build input, not a Timeline canvas contract. Keeping the type in the Motion
/// domain prevents Motion bundles from depending on the removed Timeline 1.x canvas model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionViewport {
    pub width: u32,
    pub height: u32,
}

impl MotionViewport {
    pub const DEFAULT: Self = Self::new(1920, 1080);

    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub const fn tuple(self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub const fn is_valid_pixel_extent(self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= i32::MAX as u32
            && self.height <= i32::MAX as u32
    }
}

impl Default for MotionViewport {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_is_a_closed_motion_domain_value() {
        assert!(MotionViewport::DEFAULT.is_valid_pixel_extent());
        assert!(!MotionViewport::new(0, 1080).is_valid_pixel_extent());
        assert!(!MotionViewport::new(i32::MAX as u32 + 1, 1).is_valid_pixel_extent());
        assert!(serde_json::from_str::<MotionViewport>(r#"{"width":1,"height":1,"x":0}"#).is_err());
    }
}
