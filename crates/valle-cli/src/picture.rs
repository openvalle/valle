//! Frames as PNG files, and storyboards of them.

use std::path::Path;

use crate::Failure;
use serde_json::{Value, json};

/// The widest a frame is drawn in a storyboard, in px.
const CELL: u32 = 480;
/// The gap around the frames of a storyboard, in px.
const GAP: u32 = 8;
const LABEL: u32 = 18;

pub struct Storyboard {
    pub size: [u32; 2],
    pub pixels: Vec<u8>,
    /// Content rectangles in sheet pixels, in requested order, with zero-based frame numbers.
    pub layout: Value,
}

pub fn write_png(path: &Path, size: [u32; 2], pixels: &[u8]) -> Result<(), Failure> {
    let bad = |e: &dyn std::fmt::Display| Failure::new("io", format!("{}: {e}", path.display()));
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| bad(&e))?;
    }
    let file = std::fs::File::create(path).map_err(|e| bad(&e))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| bad(&e))?;
    writer.write_image_data(pixels).map_err(|e| bad(&e))?;
    writer.finish().map_err(|e| bad(&e))
}

/// Frames of `size` side by side, as near a square as they go, each at most `CELL` px wide, on
/// a dark gray: the storyboard's size and pixels.
pub fn storyboard(size: [u32; 2], frames: &[Vec<u8>], indices: &[i64]) -> Storyboard {
    assert_eq!(frames.len(), indices.len());
    let n = frames.len().max(1) as u32;
    let columns = (n as f64).sqrt().ceil() as u32;
    let rows = n.div_ceil(columns);
    let scale = (CELL as f64 / size[0] as f64).min(1.0);
    let cell = size.map(|v| ((v as f64 * scale).round() as u32).max(1));
    let label_width = indices
        .iter()
        .map(|i| i.to_string().len() as u32 * 8)
        .max()
        .unwrap_or(0);
    let column_width = cell[0].max(label_width);
    let width = columns * column_width + (columns + 1) * GAP;
    let height = rows * (cell[1] + LABEL) + (rows + 1) * GAP;
    let mut out = [32u8, 32, 32, 255].repeat((width * height) as usize);
    let mut cells = Vec::new();
    for (k, frame) in frames.iter().enumerate() {
        let index = indices[k];
        let k = k as u32;
        let x0 = GAP + (k % columns) * (column_width + GAP) + (column_width - cell[0]) / 2;
        let y0 = GAP + (k / columns) * (cell[1] + LABEL + GAP);
        cells.push(json!({"frame":index,"x":x0,"y":y0,"width":cell[0],"height":cell[1]}));
        // A border outside the content keeps dark frames distinguishable without hiding pixels.
        for y in y0 - 1..=y0 + cell[1] {
            for x in x0 - 1..=x0 + cell[0] {
                if x < x0 || x == x0 + cell[0] || y < y0 || y == y0 + cell[1] {
                    put(&mut out, width, x, y, [128, 128, 128, 255]);
                }
            }
        }
        let label = index.to_string();
        let left = GAP
            + (k % columns) * (column_width + GAP)
            + (column_width - label.len() as u32 * 8) / 2;
        for (i, digit) in label.bytes().enumerate() {
            number(
                &mut out,
                width,
                [left + i as u32 * 8, y0 + cell[1] + 5],
                digit,
            );
        }
        for y in 0..cell[1] {
            for x in 0..cell[0] {
                let pixel = average(size, frame, cell, [x, y]);
                let at = (((y0 + y) * width + x0 + x) * 4) as usize;
                // Over the background, as the frame's alpha has it.
                let a = pixel[3] as u32;
                for (o, p) in out[at..at + 3].iter_mut().zip(pixel) {
                    *o = ((p as u32 * a + *o as u32 * (255 - a) + 127) / 255) as u8;
                }
            }
        }
    }
    Storyboard {
        size: [width, height],
        pixels: out,
        layout: json!({"width":width,"height":height,"columns":columns,"rows":rows,"cells":cells}),
    }
}

fn put(out: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 4]) {
    let at = ((y * width + x) * 4) as usize;
    out[at..at + 4].copy_from_slice(&color);
}

/// Small numeric labels without a font download or another render pass.
fn number(out: &mut [u8], width: u32, at: [u32; 2], digit: u8) {
    const DIGITS: [[u8; 5]; 10] = [
        [7, 5, 5, 5, 7],
        [2, 6, 2, 2, 7],
        [7, 1, 7, 4, 7],
        [7, 1, 7, 1, 7],
        [5, 5, 7, 1, 1],
        [7, 4, 7, 1, 7],
        [7, 4, 7, 5, 7],
        [7, 1, 1, 1, 1],
        [7, 5, 7, 5, 7],
        [7, 5, 7, 1, 7],
    ];
    for (y, bits) in DIGITS[(digit - b'0') as usize].iter().enumerate() {
        for x in 0..3 {
            if bits & (1 << (2 - x)) != 0 {
                for dy in 0..2 {
                    for dx in 0..2 {
                        put(
                            out,
                            width,
                            at[0] + x * 2 + dx,
                            at[1] + y as u32 * 2 + dy,
                            [224, 224, 224, 255],
                        );
                    }
                }
            }
        }
    }
}

/// The mean of the pixels of a frame of `size` under pixel `at` of it drawn at `cell`.
fn average(size: [u32; 2], frame: &[u8], cell: [u32; 2], at: [u32; 2]) -> [u8; 4] {
    let span = |axis: usize| {
        let a = at[axis] as u64 * size[axis] as u64 / cell[axis] as u64;
        let b = (at[axis] as u64 + 1) * size[axis] as u64 / cell[axis] as u64;
        a as u32..(b as u32).max(a as u32 + 1).min(size[axis])
    };
    let (xs, ys) = (span(0), span(1));
    let mut sum = [0u64; 4];
    let mut count = 0;
    for y in ys {
        for x in xs.clone() {
            let at = ((y * size[0] + x) * 4) as usize;
            for (s, v) in sum.iter_mut().zip(&frame[at..at + 4]) {
                *s += *v as u64;
            }
            count += 1;
        }
    }
    sum.map(|s| (s / count.max(1)) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storyboard_identifies_order_duplicates_and_content_rectangles() {
        let frames = vec![[32, 32, 32, 255].repeat(16 * 9); 3];
        let board = storyboard([16, 9], &frames, &[20, 0, 20]);
        assert_eq!(board.layout["columns"], 2);
        assert_eq!(board.layout["rows"], 2);
        let cells = board.layout["cells"].as_array().unwrap();
        for (cell, index) in cells.iter().zip([20, 0, 20]) {
            assert_eq!(cell["frame"], index);
            assert_eq!(cell["width"], 16);
            assert_eq!(cell["height"], 9);
            let x = cell["x"].as_u64().unwrap() as u32;
            let y = cell["y"].as_u64().unwrap() as u32;
            let pixel = |x, y| {
                let i = ((y * board.size[0] + x) * 4) as usize;
                &board.pixels[i..i + 4]
            };
            assert_eq!(pixel(x, y), [32, 32, 32, 255]);
            assert_eq!(pixel(x - 1, y - 1), [128, 128, 128, 255]);
            assert!(
                (y + 9..y + 9 + LABEL)
                    .any(|y| (x..x + 16).any(|x| pixel(x, y) == [224, 224, 224, 255]))
            );
        }
        // Even a one-pixel source leaves enough room for its entire frame number.
        let tiny = storyboard([1, 1], &[vec![0, 0, 0, 255]], &[12345]);
        assert_eq!(tiny.size[0], 40 + GAP * 2);
    }
}
