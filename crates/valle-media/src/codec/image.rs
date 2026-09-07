//! In-process lossless PNG I/O for media-tool image and mask artifacts.

use std::{
    fs::File,
    io::{BufReader, BufWriter},
    path::Path,
};

use anyhow::{Context, Result, anyhow, ensure};

use crate::frame::{Gray8Frame, RgbaFrame};

pub fn write_rgba_png(path: &Path, frame: &RgbaFrame) -> Result<()> {
    let expected = (frame.width as usize)
        .checked_mul(frame.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| anyhow!("RGBA PNG dimensions overflow address space"))?;
    ensure!(
        frame.width > 0 && frame.height > 0 && frame.data.len() == expected,
        "RGBA PNG frame is empty or not tightly packed"
    );
    write_png(
        path,
        frame.width,
        frame.height,
        png::ColorType::Rgba,
        &frame.data,
    )
}

pub fn write_gray8_png(path: &Path, frame: &Gray8Frame) -> Result<()> {
    ensure!(
        frame.width > 0 && frame.height > 0 && frame.data.len() == frame.pixel_count(),
        "grayscale PNG frame is empty or not tightly packed"
    );
    write_png(
        path,
        frame.width,
        frame.height,
        png::ColorType::Grayscale,
        &frame.data,
    )
}

/// Decode a PNG into the media boundary's tightly packed straight-alpha RGBA8 representation.
pub fn read_rgba_png(path: &Path) -> Result<RgbaFrame> {
    let file = File::open(path).with_context(|| format!("open PNG {}", path.display()))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::STRIP_16 | png::Transformations::EXPAND);
    let mut reader = decoder
        .read_info()
        .with_context(|| format!("read PNG header {}", path.display()))?;
    let capacity = reader
        .output_buffer_size()
        .ok_or_else(|| anyhow!("PNG output buffer size is unavailable"))?;
    let mut decoded = vec![0; capacity];
    let info = reader
        .next_frame(&mut decoded)
        .with_context(|| format!("decode PNG {}", path.display()))?;
    ensure!(
        info.bit_depth == png::BitDepth::Eight,
        "image PNG must decode to 8-bit samples, got {:?}",
        info.bit_depth
    );
    decoded.truncate(info.buffer_size());
    let pixels = (info.width as usize)
        .checked_mul(info.height as usize)
        .ok_or_else(|| anyhow!("PNG dimensions overflow address space"))?;
    let mut rgba = Vec::with_capacity(
        pixels
            .checked_mul(4)
            .ok_or_else(|| anyhow!("RGBA PNG dimensions overflow address space"))?,
    );
    let rgb_len = pixels
        .checked_mul(3)
        .ok_or_else(|| anyhow!("RGB PNG dimensions overflow address space"))?;
    let gray_alpha_len = pixels
        .checked_mul(2)
        .ok_or_else(|| anyhow!("GrayAlpha PNG dimensions overflow address space"))?;
    let rgba_len = pixels
        .checked_mul(4)
        .ok_or_else(|| anyhow!("RGBA PNG dimensions overflow address space"))?;
    match info.color_type {
        png::ColorType::Rgba => {
            ensure!(decoded.len() == rgba_len, "RGBA PNG has invalid storage");
            rgba.extend_from_slice(&decoded);
        }
        png::ColorType::Rgb => {
            ensure!(decoded.len() == rgb_len, "RGB PNG has invalid storage");
            for rgb in decoded.chunks_exact(3) {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            ensure!(
                decoded.len() == gray_alpha_len,
                "GrayAlpha PNG has invalid storage"
            );
            for gray_alpha in decoded.chunks_exact(2) {
                rgba.extend_from_slice(&[
                    gray_alpha[0],
                    gray_alpha[0],
                    gray_alpha[0],
                    gray_alpha[1],
                ]);
            }
        }
        png::ColorType::Grayscale => {
            ensure!(decoded.len() == pixels, "Gray PNG has invalid storage");
            for gray in decoded {
                rgba.extend_from_slice(&[gray, gray, gray, 255]);
            }
        }
        png::ColorType::Indexed => {
            return Err(anyhow!(
                "indexed PNG remained after expansion; decoder contract is unsupported"
            ));
        }
    }
    ensure!(
        rgba.len() == rgba_len,
        "decoded RGBA PNG has invalid storage"
    );
    Ok(RgbaFrame {
        width: info.width,
        height: info.height,
        data: rgba,
    })
}

/// Decode the strict `Gray8` exchange format used by `segment -> inpaint`.
pub fn read_gray8_png(path: &Path) -> Result<Gray8Frame> {
    let file = File::open(path).with_context(|| format!("open PNG {}", path.display()))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::STRIP_16 | png::Transformations::EXPAND);
    let mut reader = decoder
        .read_info()
        .with_context(|| format!("read PNG header {}", path.display()))?;
    let capacity = reader
        .output_buffer_size()
        .ok_or_else(|| anyhow!("PNG output buffer size is unavailable"))?;
    let mut data = vec![0; capacity];
    let info = reader
        .next_frame(&mut data)
        .with_context(|| format!("decode PNG {}", path.display()))?;
    ensure!(
        info.color_type == png::ColorType::Grayscale && info.bit_depth == png::BitDepth::Eight,
        "mask PNG must decode to Gray8, got {:?}/{:?}",
        info.color_type,
        info.bit_depth
    );
    data.truncate(info.buffer_size());
    Gray8Frame::from_data(info.width, info.height, data).map_err(anyhow::Error::msg)
}

fn write_png(
    path: &Path,
    width: u32,
    height: u32,
    color_type: png::ColorType,
    data: &[u8],
) -> Result<()> {
    let file = File::create(path).with_context(|| format!("create PNG {}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(color_type);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .with_context(|| format!("write PNG header {}", path.display()))?;
    writer
        .write_image_data(data)
        .with_context(|| format!("write PNG pixels {}", path.display()))?;
    writer
        .finish()
        .with_context(|| format!("finish PNG {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gray8_png_roundtrips_without_changing_mask_samples() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mask.png");
        let frame = Gray8Frame::from_data(3, 2, vec![0, 255, 0, 255, 0, 255]).unwrap();
        write_gray8_png(&path, &frame).unwrap();
        assert_eq!(read_gray8_png(&path).unwrap(), frame);
    }

    #[test]
    fn rgba_png_is_not_accepted_as_a_cross_tool_mask() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rgba.png");
        let mut frame = RgbaFrame::new(1, 1);
        frame.data = vec![255, 255, 255, 255];
        write_rgba_png(&path, &frame).unwrap();
        assert!(read_gray8_png(&path).is_err());
    }

    #[test]
    fn rgba_png_roundtrips_straight_alpha() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rgba.png");
        let frame = RgbaFrame {
            width: 2,
            height: 1,
            data: vec![10, 20, 30, 40, 50, 60, 70, 255],
        };
        write_rgba_png(&path, &frame).unwrap();
        assert_eq!(read_rgba_png(&path).unwrap(), frame);
    }

    #[test]
    fn rgb_png_is_normalized_to_opaque_rgba() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rgb.png");
        write_png(&path, 2, 1, png::ColorType::Rgb, &[1, 2, 3, 4, 5, 6]).unwrap();
        assert_eq!(
            read_rgba_png(&path).unwrap().data,
            vec![1, 2, 3, 255, 4, 5, 6, 255]
        );
    }
}
