//! Straight-alpha RGBA8 pixel carrier for decoded and rasterized frames. Store tightly packed,
//! row-major [R,G,B,A] bytes; provide separate transparent and opaque-black constructors.

/// Straight-alpha RGBA8 frame with width*height*4 tightly packed bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaFrame {
    /// Create a transparent zero-filled frame.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; (width as usize) * (height as usize) * 4],
        }
    }

    /// Create an opaque-black frame.
    pub fn opaque_black(width: u32, height: u32) -> Self {
        let mut f = Self::new(width, height);
        for px in f.data.chunks_exact_mut(4) {
            px[3] = 255; // R=G=B=0, A=255
        }
        f
    }

    /// Total pixel count.
    pub fn pixel_count(&self) -> usize {
        (self.width as usize) * (self.height as usize)
    }

    /// Read one RGBA pixel, returning None out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let i = self.index(x, y)?;
        Some([
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ])
    }

    /// Write one pixel, ignoring out-of-bounds coordinates.
    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if let Some(i) = self.index(x, y) {
            self.data[i..i + 4].copy_from_slice(&rgba);
        }
    }

    /// Fill the entire frame.
    pub fn fill(&mut self, rgba: [u8; 4]) {
        for px in self.data.chunks_exact_mut(4) {
            px.copy_from_slice(&rgba);
        }
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(((y as usize) * (self.width as usize) + (x as usize)) * 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_transparent_tight() {
        let f = RgbaFrame::new(2, 3);
        assert_eq!(f.data.len(), 2 * 3 * 4);
        assert_eq!(f.pixel(0, 0), Some([0, 0, 0, 0]));
    }

    #[test]
    fn opaque_black_base() {
        let f = RgbaFrame::opaque_black(2, 2);
        assert_eq!(f.pixel(1, 1), Some([0, 0, 0, 255]));
    }

    #[test]
    fn set_and_get_pixel_bounds() {
        let mut f = RgbaFrame::new(2, 2);
        f.set_pixel(1, 0, [10, 20, 30, 40]);
        assert_eq!(f.pixel(1, 0), Some([10, 20, 30, 40]));
        assert_eq!(f.pixel(2, 0), None); // Out of bounds.
        f.set_pixel(9, 9, [1, 1, 1, 1]); // Out-of-bounds writes are ignored.
    }
}
