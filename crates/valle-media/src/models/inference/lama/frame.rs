use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

/// Byte layout of a tightly packed, row-major image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    Rgb8,
    Rgba8,
}

impl PixelFormat {
    pub const fn channels(self) -> usize {
        match self {
            Self::Rgb8 => 3,
            Self::Rgba8 => 4,
        }
    }
}

/// Borrowed tightly packed RGB8/RGBA8 image. RGBA uses straight alpha.
#[derive(Debug, Clone, Copy)]
pub struct ImageView<'a> {
    width: u32,
    height: u32,
    format: PixelFormat,
    pixels: &'a [u8],
}

impl<'a> ImageView<'a> {
    pub fn rgb8(width: u32, height: u32, pixels: &'a [u8]) -> Result<Self> {
        Self::new(width, height, PixelFormat::Rgb8, pixels)
    }

    pub fn rgba8(width: u32, height: u32, pixels: &'a [u8]) -> Result<Self> {
        Self::new(width, height, PixelFormat::Rgba8, pixels)
    }

    pub fn new(width: u32, height: u32, format: PixelFormat, pixels: &'a [u8]) -> Result<Self> {
        ensure!(width > 0 && height > 0, "image dimensions must be positive");
        let expected = image_byte_len(width, height, format)?;
        ensure!(
            pixels.len() == expected,
            "{}x{} {:?} image requires {expected} bytes, got {}",
            width,
            height,
            format,
            pixels.len()
        );
        Ok(Self {
            width,
            height,
            format,
            pixels,
        })
    }

    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }

    pub const fn format(self) -> PixelFormat {
        self.format
    }

    pub const fn pixels(self) -> &'a [u8] {
        self.pixels
    }
}

/// Borrowed strict binary Gray8 mask. Zero preserves and 255 replaces.
#[derive(Debug, Clone, Copy)]
pub struct BinaryMaskView<'a> {
    width: u32,
    height: u32,
    pixels: &'a [u8],
}

impl<'a> BinaryMaskView<'a> {
    pub fn gray8(width: u32, height: u32, pixels: &'a [u8]) -> Result<Self> {
        ensure!(width > 0 && height > 0, "mask dimensions must be positive");
        let expected = pixel_count(width, height)?;
        ensure!(
            pixels.len() == expected,
            "{}x{} Gray8 mask requires {expected} bytes, got {}",
            width,
            height,
            pixels.len()
        );
        if let Some((index, value)) = pixels
            .iter()
            .copied()
            .enumerate()
            .find(|(_, value)| *value != 0 && *value != 255)
        {
            anyhow::bail!(
                "LaMa mask must be binary Gray8 (0 preserve, 255 replace); byte {index} is {value}"
            );
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }

    pub const fn pixels(self) -> &'a [u8] {
        self.pixels
    }
}

/// Owned inpainted image with the same geometry and format as its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageFrame {
    width: u32,
    height: u32,
    format: PixelFormat,
    pixels: Vec<u8>,
}

impl ImageFrame {
    pub(crate) fn new(
        width: u32,
        height: u32,
        format: PixelFormat,
        pixels: Vec<u8>,
    ) -> Result<Self> {
        let expected = image_byte_len(width, height, format)?;
        ensure!(
            pixels.len() == expected,
            "output image has {} bytes, expected {expected}",
            pixels.len()
        );
        Ok(Self {
            width,
            height,
            format,
            pixels,
        })
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
}

pub(crate) fn pixel_count(width: u32, height: u32) -> Result<usize> {
    (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| anyhow::anyhow!("image dimensions overflow addressable memory"))
}

pub(crate) fn image_byte_len(width: u32, height: u32, format: PixelFormat) -> Result<usize> {
    pixel_count(width, height)?
        .checked_mul(format.channels())
        .ok_or_else(|| anyhow::anyhow!("image byte length overflows addressable memory"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_require_tightly_packed_matching_buffers() {
        assert!(ImageView::rgb8(2, 2, &[0; 12]).is_ok());
        assert!(ImageView::rgba8(2, 2, &[0; 16]).is_ok());
        assert!(ImageView::rgb8(2, 2, &[0; 11]).is_err());
        assert!(BinaryMaskView::gray8(2, 2, &[0, 255, 0, 255]).is_ok());
        assert!(BinaryMaskView::gray8(2, 2, &[0; 3]).is_err());
    }

    #[test]
    fn soft_masks_are_rejected_with_the_bad_byte() {
        let error = BinaryMaskView::gray8(2, 2, &[0, 255, 127, 0])
            .unwrap_err()
            .to_string();
        assert!(error.contains("byte 2 is 127"));
    }
}
