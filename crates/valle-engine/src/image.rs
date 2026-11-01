//! Images: decoded from their PNG, JPEG or WebP files into colors as Chrome keeps them,
//! sRGB-encoded and premultiplied in 8 bits.

use std::io::Cursor;
use std::sync::Arc;

use image_webp::WebPDecoder;
use png::{ColorType, Transformations};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

/// An image: 4 bytes a pixel, red, green, blue and alpha, rows top down.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub size: [u32; 2],
    pub pixels: Arc<[u8]>,
}

impl Image {
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
            Err("not a PNG, JPEG or WebP image".into())
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
            Err("not a PNG, JPEG or WebP image".into())
        );
    }
}
