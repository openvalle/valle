//! Tightly packed 8-bit grayscale image used at model and tool boundaries.

/// A row-major `width * height` grayscale frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gray8Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Gray8Frame {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; (width as usize).saturating_mul(height as usize)],
        }
    }

    pub fn from_data(width: u32, height: u32, data: Vec<u8>) -> Result<Self, String> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .ok_or_else(|| "grayscale frame dimensions overflow address space".to_owned())?;
        if data.len() != expected {
            return Err(format!(
                "grayscale frame has {} samples, expected {expected}",
                data.len()
            ));
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    pub fn pixel_count(&self) -> usize {
        (self.width as usize).saturating_mul(self.height as usize)
    }

    pub fn is_binary_mask(&self) -> bool {
        self.data.iter().all(|sample| matches!(sample, 0 | 255))
    }

    pub fn threshold(&self, threshold: u8) -> Self {
        Self {
            width: self.width,
            height: self.height,
            data: self
                .data
                .iter()
                .map(|sample| if *sample >= threshold { 255 } else { 0 })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_data_requires_tight_storage() {
        assert!(Gray8Frame::from_data(2, 2, vec![0; 4]).is_ok());
        assert!(Gray8Frame::from_data(2, 2, vec![0; 3]).is_err());
    }

    #[test]
    fn threshold_produces_the_cross_tool_binary_mask_contract() {
        let source = Gray8Frame::from_data(4, 1, vec![0, 126, 127, 255]).unwrap();
        let mask = source.threshold(127);
        assert_eq!(mask.data, vec![0, 0, 255, 255]);
        assert!(mask.is_binary_mask());
    }
}
