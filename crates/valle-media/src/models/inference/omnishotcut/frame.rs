use anyhow::{Result, ensure};

pub const MODEL_WIDTH: usize = 128;
pub const MODEL_HEIGHT: usize = 96;
pub const FRAME_BYTES: usize = MODEL_WIDTH * MODEL_HEIGHT * 3;

/// Packed byte layout accepted at the adapter boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb8,
    Rgba8,
}

impl PixelFormat {
    const fn channels(self) -> usize {
        match self {
            Self::Rgb8 => 3,
            Self::Rgba8 => 4,
        }
    }
}

/// Borrowed model-ready frame retained for callers that already own the published tensor shape.
///
/// Media decoding and temporal sampling stay in the caller. Frames must have
/// already been resized to the release contract's 128x96 geometry. RGBA alpha
/// is ignored; the model always receives tightly packed RGB bytes.
#[derive(Debug, Clone, Copy)]
pub struct FrameView<'a> {
    width: usize,
    height: usize,
    format: PixelFormat,
    data: &'a [u8],
}

impl<'a> FrameView<'a> {
    pub fn rgb8(width: usize, height: usize, data: &'a [u8]) -> Result<Self> {
        Self::new(width, height, PixelFormat::Rgb8, data)
    }

    pub fn rgba8(width: usize, height: usize, data: &'a [u8]) -> Result<Self> {
        Self::new(width, height, PixelFormat::Rgba8, data)
    }

    pub fn new(width: usize, height: usize, format: PixelFormat, data: &'a [u8]) -> Result<Self> {
        ensure!(
            width == MODEL_WIDTH && height == MODEL_HEIGHT,
            "OmniShotCut frame is {width}x{height}; expected {MODEL_WIDTH}x{MODEL_HEIGHT}"
        );
        let expected = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(format.channels()))
            .ok_or_else(|| anyhow::anyhow!("frame dimensions overflow"))?;
        ensure!(
            data.len() == expected,
            "OmniShotCut {:?} frame has {} bytes; expected {expected}",
            format,
            data.len()
        );
        Ok(Self {
            width,
            height,
            format,
            data,
        })
    }

    pub const fn width(self) -> usize {
        self.width
    }

    pub const fn height(self) -> usize {
        self.height
    }

    pub const fn format(self) -> PixelFormat {
        self.format
    }

    pub const fn data(self) -> &'a [u8] {
        self.data
    }

    pub(crate) fn copy_rgb_into(self, output: &mut Vec<u8>) {
        output.clear();
        if output.capacity() < FRAME_BYTES {
            output.reserve(FRAME_BYTES);
        }
        match self.format {
            PixelFormat::Rgb8 => output.extend_from_slice(self.data),
            PixelFormat::Rgba8 => {
                for pixel in self.data.chunks_exact(4) {
                    output.extend_from_slice(&pixel[..3]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_contract_rejects_wrong_geometry_or_byte_count() {
        let rgb = vec![0; FRAME_BYTES];
        assert!(FrameView::rgb8(MODEL_WIDTH, MODEL_HEIGHT, &rgb).is_ok());
        assert!(FrameView::rgb8(MODEL_WIDTH - 1, MODEL_HEIGHT, &rgb).is_err());
        assert!(FrameView::rgb8(MODEL_WIDTH, MODEL_HEIGHT, &rgb[..FRAME_BYTES - 1]).is_err());
    }

    #[test]
    fn rgba_is_converted_without_retaining_alpha() {
        let rgba = [9, 8, 7, 6].repeat(MODEL_WIDTH * MODEL_HEIGHT);
        let frame = FrameView::rgba8(MODEL_WIDTH, MODEL_HEIGHT, &rgba).unwrap();
        let mut rgb = Vec::new();
        frame.copy_rgb_into(&mut rgb);
        assert_eq!(rgb.len(), FRAME_BYTES);
        assert_eq!(&rgb[..6], &[9, 8, 7, 9, 8, 7]);
    }
}
