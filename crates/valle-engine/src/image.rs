//! Images: decoded from their files into colors as Chrome keeps them, sRGB-encoded and
//! premultiplied in 8 bits.

use std::io::Cursor;
use std::sync::Arc;

use png::{ColorType, Transformations};

/// An image: 4 bytes a pixel, red, green, blue and alpha, rows top down.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub size: [u32; 2],
    pub pixels: Arc<[u8]>,
}

impl Image {
    /// The image a PNG file holds, 16-bit channels cut to 8.
    pub fn decode(bytes: &[u8]) -> Result<Image, String> {
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_transformations(Transformations::normalize_to_color8());
        let fail = |e: png::DecodingError| format!("not a PNG image the engine reads: {e}");
        let mut reader = decoder.read_info().map_err(fail)?;
        let size = reader
            .output_buffer_size()
            .ok_or("the image is too large")?;
        let mut buffer = vec![0; size];
        let info = reader.next_frame(&mut buffer).map_err(fail)?;
        let channels = match info.color_type {
            ColorType::Grayscale => 1,
            ColorType::GrayscaleAlpha => 2,
            ColorType::Rgb => 3,
            _ => 4,
        };
        let width = info.width as usize;
        let mut pixels = Vec::with_capacity(width * info.height as usize * 4);
        for row in buffer[..info.buffer_size()].chunks(info.line_size) {
            for p in row[..width * channels].chunks(channels) {
                let [r, g, b, a] = match *p {
                    [v] => [v, v, v, 255],
                    [v, a] => [v, v, v, a],
                    [r, g, b] => [r, g, b, 255],
                    [r, g, b, a, ..] => [r, g, b, a],
                    _ => unreachable!("1 to 4 channels"),
                };
                pixels.extend([r, g, b].map(|c| premultiplied(c, a)));
                pixels.push(a);
            }
        }
        Ok(Image {
            size: [info.width, info.height],
            pixels: pixels.into(),
        })
    }
}

/// `c` times `a`, both of 255, rounded.
fn premultiplied(c: u8, a: u8) -> u8 {
    ((u32::from(c) * u32::from(a) * 2 + 255) / 510) as u8
}
