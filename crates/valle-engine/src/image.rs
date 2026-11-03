//! Images: decoded from their PNG, JPEG or WebP files into colors as Chrome keeps them,
//! sRGB-encoded and premultiplied in 8 bits; and what a scene's resource shows, one of them or
//! an SVG file.

use std::io::Cursor;
use std::sync::Arc;

use image_webp::WebPDecoder;
use png::{ColorType, Transformations};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use crate::layout::Natural;
use crate::svg::file::{self, Document};

/// What a resource shows: one of the scene's images, by its index there, an SVG file, or nothing.
#[derive(Debug)]
pub enum Picture {
    Image {
        image: u32,
        size: [u32; 2],
    },
    Svg(Box<Document>),
    /// A font resource's, which no image names.
    None,
}

impl Picture {
    /// What a resource's file shows, by the bytes it starts with: an SVG file, or an image, which
    /// goes onto `images`.
    pub fn load(bytes: &[u8], images: &mut Vec<Image>) -> Result<Picture, String> {
        if file::looks(bytes) {
            let text = std::str::from_utf8(bytes).map_err(|_| "an SVG file is UTF-8 text")?;
            let text = text.strip_prefix('\u{feff}').unwrap_or(text);
            return Ok(Picture::Svg(Box::new(Document::parse(text, images)?)));
        }
        let image = Image::decode(bytes)?;
        let size = image.size;
        images.push(image);
        Ok(Picture::Image {
            image: (images.len() - 1) as u32,
            size,
        })
    }

    pub fn natural(&self) -> Natural {
        match self {
            Picture::Image { size, .. } => Natural::Size(size.map(|v| v as f32)),
            Picture::Svg(document) => document.natural,
            Picture::None => Natural::Ratio(None),
        }
    }
}

/// An image: 4 bytes a pixel, red, green, blue and alpha, rows top down; none for a picture a
/// box draws, which the GPU draws each frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub size: [u32; 2],
    pub pixels: Arc<[u8]>,
}

impl Image {
    /// What stands for the `k`th picture the Timeline gives a scene, when it shows alone: a ramp
    /// from corner to corner, blue for the first, orange for the second, with a check of light
    /// squares over it, 8 across, so that moving it shows.
    pub fn placeholder(size: [u32; 2], k: usize) -> Image {
        let ends: [[f32; 3]; 2] = match k {
            0 => [[30.0, 136.0, 229.0], [0.0, 172.0, 193.0]],
            _ => [[255.0, 112.0, 67.0], [236.0, 64.0, 122.0]],
        };
        let [w, h] = size.map(|v| v.max(1));
        let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h {
            for x in 0..w {
                let t = ((x as f32 + 0.5) / w as f32 + (y as f32 + 0.5) / h as f32) / 2.0;
                let light = (x * 8 / w + y * 8 / w) % 2 == 1;
                for (a, b) in ends[0].iter().zip(&ends[1]) {
                    let v = a + (b - a) * t;
                    let v = if light { v + (255.0 - v) * 0.2 } else { v };
                    pixels.push(v.round() as u8);
                }
                pixels.push(255);
            }
        }
        Image {
            size: [w, h],
            pixels: pixels.into(),
        }
    }

    /// The image a PNG, JPEG or WebP file holds, by the bytes the file starts with, its colors
    /// taken as sRGB whatever profile it embeds; 16-bit channels are cut to 8. An animated one is
    /// an error.
    pub fn decode(bytes: &[u8]) -> Result<Image, String> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            png(bytes)
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            jpeg(bytes)
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            webp(bytes)
        } else {
            Err("not a PNG, JPEG, WebP or SVG image".into())
        }
    }

    /// The levels of the image's mipmap below it, down to 1 × 1 px, as Skia makes them: each
    /// half as big as the one above on each side, but at least 1 px, a pixel the mean of the
    /// pixels under it, on each axis the 2 where the level above is even, the 3 weighted 1, 2, 1
    /// where it is odd, or the 1 where it is a single px.
    pub fn mipmap(&self) -> Vec<Image> {
        let mut levels: Vec<Image> = Vec::new();
        let mut above = self;
        while above.size != [1, 1] {
            let level = above.halved();
            levels.push(level);
            above = levels.last().expect("just pushed");
        }
        levels
    }

    fn halved(&self) -> Image {
        let [w, h] = self.size.map(|n| n as usize);
        let size = self.size.map(|n| (n / 2).max(1));
        // Each axis's taps from the first under a pixel, and their weights.
        let taps = |n: usize| -> &'static [usize] {
            match (n, n % 2) {
                (1, _) => &[1],
                (_, 0) => &[1, 1],
                _ => &[1, 2, 1],
            }
        };
        let (across, down) = (taps(w), taps(h));
        let total: usize = across.iter().sum::<usize>() * down.iter().sum::<usize>();
        let mut pixels = Vec::with_capacity(size[0] as usize * size[1] as usize * 4);
        for y in 0..size[1] as usize {
            for x in 0..size[0] as usize {
                let mut sum = [0; 4];
                for (j, wy) in down.iter().enumerate() {
                    let row = ((2 * y + j).min(h - 1)) * w;
                    for (i, wx) in across.iter().enumerate() {
                        let at = (row + (2 * x + i).min(w - 1)) * 4;
                        for (c, s) in sum.iter_mut().enumerate() {
                            *s += usize::from(self.pixels[at + c]) * wx * wy;
                        }
                    }
                }
                pixels.extend(sum.map(|s| ((s + total / 2) / total) as u8));
            }
        }
        Image {
            size,
            pixels: pixels.into(),
        }
    }
}

fn png(bytes: &[u8]) -> Result<Image, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(Transformations::normalize_to_color8());
    let fail = |e: png::DecodingError| format!("not a PNG image the engine reads: {e}");
    let mut reader = decoder.read_info().map_err(fail)?;
    if reader.info().animation_control.is_some() {
        return Err("an animated PNG is not supported yet".into());
    }
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
    let rows = buffer[..info.buffer_size()].chunks(info.line_size);
    Ok(premultiplied([info.width, info.height], channels, rows))
}

fn jpeg(bytes: &[u8]) -> Result<Image, String> {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let pixels =
        (decoder.decode()).map_err(|e| format!("not a JPEG image the engine reads: {e}"))?;
    let info = decoder
        .info()
        .ok_or("engine bug: a JPEG decoded without its size")?;
    let size = [info.width, info.height].map(u32::from);
    Ok(premultiplied(size, 3, pixels.chunks(size[0] as usize * 3)))
}

fn webp(bytes: &[u8]) -> Result<Image, String> {
    let fail = |e: image_webp::DecodingError| format!("not a WebP image the engine reads: {e}");
    let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(fail)?;
    if decoder.is_animated() {
        return Err("an animated WebP is not supported yet".into());
    }
    let size = decoder
        .output_buffer_size()
        .ok_or("the image is too large")?;
    let mut pixels = vec![0; size];
    decoder.read_image(&mut pixels).map_err(fail)?;
    let (width, height) = decoder.dimensions();
    let channels = if decoder.has_alpha() { 4 } else { 3 };
    let rows = pixels.chunks(width as usize * channels);
    Ok(premultiplied([width, height], channels, rows))
}

/// The image of `size` whose rows hold 1 to 4 channels a pixel, gray, gray and alpha, RGB or
/// RGBA, each row maybe padded past its pixels.
fn premultiplied<'a>(
    size: [u32; 2],
    channels: usize,
    rows: impl Iterator<Item = &'a [u8]>,
) -> Image {
    let width = size[0] as usize;
    let mut pixels = Vec::with_capacity(width * size[1] as usize * 4);
    for row in rows {
        for p in row[..width * channels].chunks(channels) {
            let [r, g, b, a] = match *p {
                [v] => [v, v, v, 255],
                [v, a] => [v, v, v, a],
                [r, g, b] => [r, g, b, 255],
                [r, g, b, a, ..] => [r, g, b, a],
                _ => unreachable!("1 to 4 channels"),
            };
            pixels.extend([r, g, b].map(|c| times(c, a)));
            pixels.push(a);
        }
    }
    Image {
        size,
        pixels: pixels.into(),
    }
}

/// `c` times `a`, both of 255, rounded.
fn times(c: u8, a: u8) -> u8 {
    ((u32::from(c) * u32::from(a) * 2 + 255) / 510) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mipmaps_halve_down_to_a_px() {
        // 3 × 2: the 3 across weighted 1, 2, 1, the 2 down evenly.
        let image = Image {
            size: [3, 2],
            pixels: [[0, 0, 0, 0], [40, 0, 0, 0], [80, 0, 0, 0]]
                .repeat(2)
                .concat()
                .into(),
        };
        let levels = image.mipmap();
        let sizes: Vec<_> = levels.iter().map(|l| l.size).collect();
        assert_eq!(sizes, [[1, 1]]);
        assert_eq!(levels[0].pixels[..], [40, 0, 0, 0]);
        let wide = Image {
            size: [8, 1],
            pixels: [255, 0, 0, 255].repeat(8).into(),
        };
        let sizes: Vec<_> = wide.mipmap().iter().map(|l| l.size).collect();
        assert_eq!(sizes, [[4, 1], [2, 1], [1, 1]]);
    }

    #[test]
    fn an_animated_png_is_an_error() {
        let mut file = Vec::new();
        let mut encoder = png::Encoder::new(&mut file, 1, 1);
        encoder.set_color(ColorType::Rgba);
        encoder.set_animated(2, 0).unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0; 4]).unwrap();
        writer.write_image_data(&[255; 4]).unwrap();
        writer.finish().unwrap();
        assert_eq!(
            Image::decode(&file),
            Err("an animated PNG is not supported yet".into())
        );
        assert_eq!(
            Image::decode(b"GIF89a"),
            Err("not a PNG, JPEG, WebP or SVG image".into())
        );
    }
}
