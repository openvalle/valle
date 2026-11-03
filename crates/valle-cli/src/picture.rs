//! Frames as PNG files, and storyboards of them.

use std::path::Path;

use crate::Failure;

/// The widest a frame is drawn in a storyboard, in px.
const CELL: u32 = 480;
/// The gap around the frames of a storyboard, in px.
const GAP: u32 = 8;

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
pub fn storyboard(size: [u32; 2], frames: &[Vec<u8>]) -> ([u32; 2], Vec<u8>) {
    let n = frames.len().max(1) as u32;
    let columns = (n as f64).sqrt().ceil() as u32;
    let rows = n.div_ceil(columns);
    let scale = (CELL as f64 / size[0] as f64).min(1.0);
    let cell = size.map(|v| ((v as f64 * scale).round() as u32).max(1));
    let width = columns * cell[0] + (columns + 1) * GAP;
    let height = rows * cell[1] + (rows + 1) * GAP;
    let mut out = [32u8, 32, 32, 255].repeat((width * height) as usize);
    for (k, frame) in frames.iter().enumerate() {
        let k = k as u32;
        let x0 = GAP + (k % columns) * (cell[0] + GAP);
        let y0 = GAP + (k / columns) * (cell[1] + GAP);
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
    ([width, height], out)
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
