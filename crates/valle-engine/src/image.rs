//! Images: decoded from their PNG, JPEG or WebP files into colors as Chrome keeps them,
//! sRGB-encoded and premultiplied in 8 bits; and what a scene's resource shows, one of them or
//! an SVG file.

use std::io::Cursor;
use std::sync::Arc;

use image_webp::WebPDecoder;
use png::Transformations;
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use crate::layout::Natural;
use crate::svg::file::{self, Document};

pub mod yuv;

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

/// An image's pixels: RGBA, or decoded YUV planes converted when uploaded to the GPU.
/// Empty RGBA stands for a picture a box draws each frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub size: [u32; 2],
    pub pixels: Arc<[u8]>,
    pub(crate) yuv: Option<yuv::Layout>,
}

impl Image {
    /// RGBA bytes, rows top down; an empty buffer is a picture drawn by the scene.
    pub fn new(size: [u32; 2], pixels: Arc<[u8]>) -> Self {
        Self {
            size,
            pixels,
            yuv: None,
        }
    }

    /// Keep a decoded video's planes in their original format until the renderer needs them.
    pub fn from_yuv(
        size: [u32; 2],
        pixels: impl Into<Arc<[u8]>>,
        layout: yuv::Layout,
    ) -> Result<Self, String> {
        let pixels = pixels.into();
        layout.planes(size, &pixels).validate()?;
        Ok(Self {
            size,
            pixels,
            yuv: Some(layout),
        })
    }

    pub(crate) fn same(&self, other: &Self) -> bool {
        self.size == other.size && self.yuv == other.yuv && Arc::ptr_eq(&self.pixels, &other.pixels)
    }

    /// CPU fallback for callers that need RGBA, including images larger than the device limit.
    pub fn to_rgba(&self) -> Self {
        match &self.yuv {
            None => self.clone(),
            Some(layout) => Self::new(
                self.size,
                yuv::rgba(&layout.planes(self.size, &self.pixels))
                    .expect("validated YUV")
                    .into(),
            ),
        }
    }

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
        Image::new([w, h], pixels.into())
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

    pub(crate) fn halved(&self) -> Image {
        if self.yuv.is_some() {
            return self.to_rgba().halved();
        }
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
        Image::new(size, pixels.into())
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
    let size = [reader.info().width, reader.info().height];
    let stride = (size[0] as usize)
        .checked_mul(4)
        .ok_or("the image is too large")?;
    let length = stride
        .checked_mul(size[1] as usize)
        .filter(|&n| n <= isize::MAX as usize)
        .ok_or("the image is too large")?;
    let channels = reader.output_color_type().0.samples();
    let mut pixels = Vec::new();
    let mut passes = Vec::new();
    // A valid header need not have valid image data. Grow from decoded rows rather than
    // reserving width × height from the header, which can describe petabytes in a tiny file.
    while let Some(row) = reader.next_interlaced_row().map_err(fail)? {
        let start = pixels.len();
        pixels.extend(row.data().chunks_exact(channels).flat_map(premultiply));
        if let png::InterlaceInfo::Adam7(info) = row.interlace() {
            passes.push((*info, start..pixels.len()));
        }
    }
    if pixels.len() != length {
        return Err("not a PNG image the engine reads: incomplete pixels".into());
    }
    if !passes.is_empty() {
        // All passes have decoded successfully; the full allocation now has pixels to back it.
        let mut ordered = vec![0; pixels.len()];
        for (info, range) in passes {
            png::expand_interlaced_row(&mut ordered, stride, &pixels[range], &info, 32);
        }
        pixels = ordered;
    }
    Ok(Image::new(size, pixels.into()))
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
        pixels.extend(
            row[..width * channels]
                .chunks(channels)
                .flat_map(premultiply),
        );
    }
    Image::new(size, pixels.into())
}

fn premultiply(p: &[u8]) -> [u8; 4] {
    let [r, g, b, a] = match *p {
        [v] => [v, v, v, 255],
        [v, a] => [v, v, v, a],
        [r, g, b] => [r, g, b, 255],
        [r, g, b, a, ..] => [r, g, b, a],
        _ => unreachable!("1 to 4 channels"),
    };
    [times(r, a), times(g, a), times(b, a), a]
}

/// `c` times `a`, both of 255, rounded.
fn times(c: u8, a: u8) -> u8 {
    ((u32::from(c) * u32::from(a) * 2 + 255) / 510) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use png::{BitDepth, ColorType};

    // A stored DEFLATE block lets tests supply PNG scanlines (including Adam7 passes)
    // independently of the encoder's non-interlaced row writer. Chunk CRCs are real.
    fn scanlines(info: png::Info<'_>, rows: &[u8]) -> Vec<u8> {
        let len = u16::try_from(rows.len()).unwrap();
        let mut zlib = vec![0x78, 0x01, 0x01];
        zlib.extend(len.to_le_bytes());
        zlib.extend((!len).to_le_bytes());
        zlib.extend(rows);
        let (mut a, mut b) = (1u32, 0u32);
        for &v in rows {
            a = (a + u32::from(v)) % 65521;
            b = (b + a) % 65521;
        }
        zlib.extend(((b << 16) | a).to_be_bytes());
        let mut file = Vec::new();
        let encoder = png::Encoder::with_info(&mut file, info).unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer.write_chunk(png::chunk::IDAT, &zlib).unwrap();
        writer.finish().unwrap();
        file
    }

    #[test]
    fn png_does_not_allocate_a_frame_from_an_untrusted_header() {
        for interlaced in [false, true] {
            let mut info = png::Info::with_size(262144, i32::MAX as u32);
            info.color_type = ColorType::Rgba;
            info.interlaced = interlaced;
            // A valid header and checksums, but only one pixel of scanline data for a
            // declared 2 PiB image. Decoding must fail before allocating that frame.
            let file = scanlines(info, &[0, 0, 0, 0, 255]);
            let reader = png::Decoder::new(Cursor::new(&file)).read_info().unwrap();
            assert_eq!(reader.info().width, 262144);
            assert_eq!(reader.info().height, i32::MAX as u32);
            assert!(Image::decode(&file).is_err());
        }
    }

    #[test]
    fn png_normalizes_gray_rgb_palette_and_alpha() {
        for (color_type, bit_depth, width, rows, expected) in [
            (
                ColorType::Grayscale,
                BitDepth::One,
                2,
                vec![0, 0x80],
                vec![255, 255, 255, 255, 0, 0, 0, 255],
            ),
            (
                ColorType::GrayscaleAlpha,
                BitDepth::Eight,
                1,
                vec![0, 90, 128],
                vec![45, 45, 45, 128],
            ),
            (
                ColorType::Rgb,
                BitDepth::Sixteen,
                1,
                vec![0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc],
                vec![0x12, 0x56, 0x9a, 255],
            ),
            (
                ColorType::Indexed,
                BitDepth::Two,
                3,
                vec![0, 0x18],
                vec![0, 0, 0, 0, 0, 128, 0, 128, 0, 0, 255, 255],
            ),
        ] {
            let mut info = png::Info::with_size(width, 1);
            info.color_type = color_type;
            info.bit_depth = bit_depth;
            if color_type == ColorType::Indexed {
                info.palette = Some(vec![255, 0, 0, 0, 255, 0, 0, 0, 255].into());
                info.trns = Some(vec![0, 128, 255].into());
            }
            let image = Image::decode(&scanlines(info, &rows)).unwrap();
            assert_eq!(image.size, [width, 1]);
            assert_eq!(&*image.pixels, expected, "{color_type:?}/{bit_depth:?}");
        }
    }

    #[test]
    fn png_interlaced_and_plain_rows_have_the_same_premultiplied_pixels() {
        let pixel =
            |x: usize, y: usize| [x as u8 * 28, y as u8 * 28, 200, [0, 128, 255][(x + y) % 3]];
        let expected: Vec<u8> = (0..9)
            .flat_map(|y| {
                (0..9).flat_map(move |x| {
                    let [r, g, b, a] = pixel(x, y);
                    let multiply = |c| ((u32::from(c) * u32::from(a) + 127) / 255) as u8;
                    [multiply(r), multiply(g), multiply(b), a]
                })
            })
            .collect();
        for interlaced in [false, true] {
            // Adam7's (x start, y start, x step, y step); 9 × 9 exercises all seven passes.
            let passes: &[(usize, usize, usize, usize)] = if interlaced {
                &[
                    (0, 0, 8, 8),
                    (4, 0, 8, 8),
                    (0, 4, 4, 8),
                    (2, 0, 4, 4),
                    (0, 2, 2, 4),
                    (1, 0, 2, 2),
                    (0, 1, 1, 2),
                ]
            } else {
                &[(0, 0, 1, 1)]
            };
            let mut rows = Vec::new();
            for &(x0, y0, dx, dy) in passes {
                for y in (y0..9).step_by(dy) {
                    rows.push(0); // No row filter.
                    rows.extend((x0..9).step_by(dx).flat_map(|x| pixel(x, y)));
                }
            }
            let mut info = png::Info::with_size(9, 9);
            info.color_type = ColorType::Rgba;
            info.interlaced = interlaced;
            let image = Image::decode(&scanlines(info.clone(), &rows)).unwrap();
            assert_eq!(image.size, [9, 9]);
            assert_eq!(&*image.pixels, expected);
            // Truncation after some complete rows must not yield a partial image.
            assert!(Image::decode(&scanlines(info, &rows[..rows.len() / 2])).is_err());
        }
    }

    #[test]
    fn mipmaps_halve_down_to_a_px() {
        // 3 × 2: the 3 across weighted 1, 2, 1, the 2 down evenly.
        let image = Image::new(
            [3, 2],
            [[0, 0, 0, 0], [40, 0, 0, 0], [80, 0, 0, 0]]
                .repeat(2)
                .concat()
                .into(),
        );
        let levels = image.mipmap();
        let sizes: Vec<_> = levels.iter().map(|l| l.size).collect();
        assert_eq!(sizes, [[1, 1]]);
        assert_eq!(levels[0].pixels[..], [40, 0, 0, 0]);
        let wide = Image::new([8, 1], [255, 0, 0, 255].repeat(8).into());
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
