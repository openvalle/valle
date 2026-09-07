use anyhow::{Result, ensure};

/// Byte layout of a tightly packed, row-major frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Borrowed RGB8/RGBA8 frame. RGBA pixels always use straight alpha.
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
        let expected = byte_len(width, height, format)?;
        ensure!(
            pixels.len() == expected,
            "{}x{} {:?} image needs {expected} bytes, got {}",
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

    pub(crate) fn rgb_at(self, x: usize, y: usize) -> [u8; 3] {
        let offset = (y * self.width as usize + x) * self.format.channels();
        [
            self.pixels[offset],
            self.pixels[offset + 1],
            self.pixels[offset + 2],
        ]
    }

    pub(crate) fn alpha_at(self, x: usize, y: usize) -> u8 {
        if self.format == PixelFormat::Rgb8 {
            return 255;
        }
        let offset = (y * self.width as usize + x) * 4;
        self.pixels[offset + 3]
    }
}

/// Owned interpolated frame.
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
        let expected = byte_len(width, height, format)?;
        ensure!(
            pixels.len() == expected,
            "output buffer has {} bytes, expected {expected}",
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

    pub fn as_view(&self) -> ImageView<'_> {
        ImageView {
            width: self.width,
            height: self.height,
            format: self.format,
            pixels: &self.pixels,
        }
    }
}

pub(crate) fn byte_len(width: u32, height: u32, format: PixelFormat) -> Result<usize> {
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| anyhow::anyhow!("image dimensions overflow addressable memory"))?;
    pixels
        .checked_mul(format.channels())
        .ok_or_else(|| anyhow::anyhow!("image byte length overflows addressable memory"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_reject_empty_or_malformed_buffers() {
        assert!(ImageView::rgb8(0, 1, &[]).is_err());
        assert!(ImageView::rgb8(2, 2, &[0; 11]).is_err());
        assert!(ImageView::rgba8(2, 2, &[0; 16]).is_ok());
    }

    #[test]
    fn rgb_alpha_is_implicitly_opaque() {
        let view = ImageView::rgb8(1, 1, &[1, 2, 3]).unwrap();
        assert_eq!(view.alpha_at(0, 0), 255);
    }
}
