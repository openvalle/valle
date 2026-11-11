//! Frames as PNG files, and storyboards of them.

use std::path::{Path, PathBuf};

use valle_engine::front::Inputs;
use valle_engine::gpu::{Gpu, Renderer};
use valle_engine::scene::Scene;
use valle_engine::text::Fonts;
use valle_timeline::{
    time::{Rate, Tick},
    timecode,
};

use crate::Failure;
use serde_json::{Value, json};

/// The widest a frame is drawn in a storyboard, in px.
const CELL: u32 = 480;
/// The gap around the frames of a storyboard, in px.
const GAP: u32 = 8;
const LABEL: u32 = 36;
const HEADER: u32 = 40;
const ADVANCE: u32 = 10;
const TEXT_HEIGHT: u32 = 24;
const CHARACTERS: &str = "0123456789:;./ FramefpsND";

struct Label {
    text: String,
    at: [u32; 2],
    gray: u8,
}

/// The label alphabet rendered once per export in the engine's built-in monospace font.
pub struct Labels {
    alpha: Vec<u8>,
}

impl Labels {
    pub fn new(gpu: &Gpu, fonts: Fonts) -> Result<Self, Failure> {
        let width = CHARACTERS.len() as u32 * ADVANCE;
        let source = format!(
            r##"
            export const composition = {{ width: {width}, height: {TEXT_HEIGHT}, fps: 1, duration: 1 }};
            const letters = {};
            export default function Labels() {{
              return <View style={{{{ width: {width}, height: {TEXT_HEIGHT} }}}}>{{letters.map((letter, i) => <Text key={{i}} style={{{{
                position: "absolute", left: i * {ADVANCE}, width: {ADVANCE}, height: {TEXT_HEIGHT},
                fontFamily: "monospace", fontSize: 16, lineHeight: "{TEXT_HEIGHT}px", color: "white"
              }}}}>{{letter}}</Text>)}}</View>;
            }}
        "##,
            json!(
                CHARACTERS
                    .chars()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
            )
        );
        let entry = "storyboard-labels.motion.tsx";
        let mut scene = Scene::load(
            entry,
            &|path| (path == entry).then(|| source.as_bytes().to_vec()),
            &Inputs::default(),
            fonts,
        )
        .map_err(|e| Failure::new("scene", e))?;
        let mut renderer = Renderer::new(gpu, width, TEXT_HEIGHT);
        let pixels = crate::motion::draw(&mut scene, &mut renderer, Ok(Tick::ZERO))?;
        Ok(Self {
            alpha: pixels.as_chunks::<4>().0.iter().map(|p| p[3]).collect(),
        })
    }

    fn paint(&self, pixels: &mut [u8], width: u32, label: &Label) {
        let stride = CHARACTERS.len() as u32 * ADVANCE;
        for (i, c) in label.text.bytes().enumerate() {
            let glyph = CHARACTERS
                .bytes()
                .position(|g| g == c)
                .expect("label alphabet") as u32;
            for y in 0..TEXT_HEIGHT {
                for x in 0..ADVANCE {
                    let alpha = u32::from(self.alpha[(y * stride + glyph * ADVANCE + x) as usize]);
                    let at = (((label.at[1] + y) * width + label.at[0] + i as u32 * ADVANCE + x)
                        * 4) as usize;
                    for p in &mut pixels[at..at + 3] {
                        *p = ((u32::from(label.gray) * alpha + u32::from(*p) * (255 - alpha) + 127)
                            / 255) as u8;
                    }
                }
            }
        }
    }
}

pub struct Storyboard {
    pub size: [u32; 2],
    pub pixels: Vec<u8>,
    /// Content rectangles in sheet pixels, in requested order, with zero-based frame numbers.
    pub layout: Value,
    source: [u32; 2],
    cell: [u32; 2],
    origins: Vec<[u32; 2]>,
    labels: Vec<Label>,
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

/// The requested name for one page, or numbered siblings for several pages.
pub fn page_path(out: &Path, page: usize, pages: usize) -> PathBuf {
    if pages == 1 {
        return out.to_path_buf();
    }
    let stem = out.file_stem().unwrap_or_default().to_string_lossy();
    let ext = out.extension().unwrap_or_default().to_string_lossy();
    out.with_file_name(format!("{stem}-{:03}.{ext}", page + 1))
}

impl Storyboard {
    /// Allocate one page. Frames are downsampled directly into it by `paint`, so full-sized
    /// source pixels never accumulate between frames or pages.
    pub fn new(size: [u32; 2], indices: &[i64], fps: Rate) -> Result<Self, Failure> {
        let large = || Failure::new("usage", "storyboard page is too large; reduce --per-page");
        let n = u32::try_from(indices.len().max(1)).map_err(|_| large())?;
        let columns = (n as f64).sqrt().ceil() as u32;
        let rows = n.div_ceil(columns);
        let scale = (CELL as f64 / size[0] as f64).min(1.0);
        let cell = size.map(|v| ((v as f64 * scale).round() as u32).max(1));
        let times: Vec<f64> = indices
            .iter()
            .map(|&i| {
                fps.frame_start(i)
                    .map(|t| t.seconds())
                    .map_err(|e| Failure::new("scene", e))
            })
            .collect::<Result<_, _>>()?;
        let codes: Vec<_> = indices
            .iter()
            .map(|&i| timecode::format(i, fps.as_f64()))
            .collect();
        let frames: Vec<_> = indices.iter().map(|i| format!("Frame {i}")).collect();
        let label_width = codes
            .iter()
            .zip(&frames)
            .map(|(tc, frame)| (tc.len() + frame.len()) as u32 * ADVANCE + 4 * GAP)
            .max()
            .unwrap_or(0);
        let column_width = cell[0].max(label_width);
        let rate = format!("{:.3}", fps.as_f64());
        let mode = if timecode::format(0, fps.as_f64()).contains(';') {
            "DF"
        } else {
            "NDF"
        };
        let heading = format!(
            "{} fps / {mode}",
            rate.trim_end_matches('0').trim_end_matches('.')
        );
        let width = columns
            .checked_mul(column_width + GAP)
            .and_then(|v| v.checked_add(GAP))
            .ok_or_else(large)?
            .max(heading.len() as u32 * ADVANCE + 2 * GAP);
        let height = rows
            .checked_mul(cell[1] + LABEL + GAP)
            .and_then(|v| v.checked_add(GAP + HEADER))
            .ok_or_else(large)?;
        let len = (u64::from(width) * u64::from(height))
            .checked_mul(4)
            .and_then(|v| usize::try_from(v).ok())
            .ok_or_else(large)?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(len).map_err(|_| large())?;
        pixels.resize(len, 32);
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        let mut cells = Vec::new();
        let mut origins = Vec::new();
        let mut labels = vec![Label {
            text: heading,
            at: [GAP, (HEADER - TEXT_HEIGHT) / 2],
            gray: 160,
        }];
        for (k, &index) in indices.iter().enumerate() {
            let left = GAP + (k as u32 % columns) * (column_width + GAP);
            let x0 = left + (column_width - cell[0]) / 2;
            let y0 = HEADER + GAP + (k as u32 / columns) * (cell[1] + LABEL + GAP);
            origins.push([x0, y0]);
            cells.push(json!({"frame":index,"time":times[k],"timecode":codes[k],"x":x0,"y":y0,"width":cell[0],"height":cell[1]}));
            // Keep borders outside the content, including fully opaque dark frames.
            for y in y0 - 1..=y0 + cell[1] {
                for x in x0 - 1..=x0 + cell[0] {
                    if x < x0 || x == x0 + cell[0] || y < y0 || y == y0 + cell[1] {
                        put(&mut pixels, width, x, y, [128, 128, 128, 255]);
                    }
                }
            }
            let top = y0 + cell[1] + (LABEL - TEXT_HEIGHT) / 2;
            labels.push(Label {
                text: codes[k].clone(),
                at: [left + GAP, top],
                gray: 224,
            });
            labels.push(Label {
                text: frames[k].clone(),
                at: [
                    left + column_width - GAP - frames[k].len() as u32 * ADVANCE,
                    top,
                ],
                gray: 160,
            });
        }
        Ok(Self {
            size: [width, height],
            pixels,
            source: size,
            cell,
            origins,
            labels,
            layout: json!({"width":width,"height":height,"columns":columns,"rows":rows,"cells":cells}),
        })
    }

    pub fn label(&mut self, font: &Labels) {
        for label in &self.labels {
            font.paint(&mut self.pixels, self.size[0], label);
        }
    }

    pub fn paint(&mut self, index: usize, frame: &[u8]) {
        let [x0, y0] = self.origins[index];
        for y in 0..self.cell[1] {
            for x in 0..self.cell[0] {
                let pixel = average(self.source, frame, self.cell, [x, y]);
                let at = (((y0 + y) * self.size[0] + x0 + x) * 4) as usize;
                let background = if (x / 8 + y / 8) % 2 == 0 { 64 } else { 96 };
                let a = u32::from(pixel[3]);
                for (out, p) in self.pixels[at..at + 3].iter_mut().zip(pixel) {
                    *out = ((u32::from(p) * a + background * (255 - a) + 127) / 255) as u8;
                }
            }
        }
    }
}

fn put(out: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 4]) {
    let at = ((y * width + x) * 4) as usize;
    out[at..at + 4].copy_from_slice(&color);
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
            let p = &frame[at..at + 4];
            let alpha = u64::from(p[3]);
            for (s, v) in sum[..3].iter_mut().zip(p) {
                *s += u64::from(*v) * alpha;
            }
            sum[3] += alpha;
            count += 1;
        }
    }
    // Average premultiplied colors, then return straight RGBA. Transparent texels must not
    // darken an opaque neighbor when a full-sized frame is reduced to a thumbnail.
    [
        (sum[0] / sum[3].max(1)) as u8,
        (sum[1] / sum[3].max(1)) as u8,
        (sum[2] / sum[3].max(1)) as u8,
        (sum[3] / count.max(1)) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storyboard_identifies_order_duplicates_and_content_rectangles() {
        let frames = vec![[32, 32, 32, 255].repeat(16 * 9); 3];
        let mut board = Storyboard::new([16, 9], &[20, 0, 20], Rate::new(30, 1).unwrap())
            .unwrap_or_else(|e| panic!("{}", e.message));
        for (i, frame) in frames.iter().enumerate() {
            board.paint(i, frame);
        }
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
            assert_eq!(cell["timecode"], timecode::format(index, 30.0));
        }
        // Even a one-pixel source leaves room for both labels and the page heading.
        let tiny = Storyboard::new([1, 1], &[12345], Rate::new(30, 1).unwrap())
            .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(tiny.layout["cells"][0]["timecode"], "00:06:51:15");
        for label in &tiny.labels {
            assert!(label.at[0] + label.text.len() as u32 * ADVANCE <= tiny.size[0]);
            assert!(label.at[1] + TEXT_HEIGHT <= tiny.size[1]);
        }
    }

    #[test]
    fn transparency_is_visible_and_fractional_frame_times_are_reported() {
        let mut board = Storyboard::new([32, 16], &[30, 0, 30], Rate::new(30000, 1001).unwrap())
            .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(board.layout["cells"][0]["time"], 1.001);
        assert_eq!(board.layout["cells"][0]["timecode"], "00:00:01;00");
        assert_eq!(board.labels[0].text, "29.97 fps / DF");
        assert_eq!(board.layout["cells"][1]["time"], 0.0);
        board.paint(0, &[0, 0, 0, 0].repeat(32 * 16));
        let [x, y] = board.origins[0];
        let at = |x, y| ((y * board.size[0] + x) * 4) as usize;
        assert_eq!(&board.pixels[at(x, y)..at(x, y) + 4], &[64, 64, 64, 255]);
        assert_eq!(
            &board.pixels[at(x + 8, y)..at(x + 8, y) + 4],
            &[96, 96, 96, 255]
        );
        board.paint(0, &[32, 32, 32, 255].repeat(32 * 16));
        let index = ((y * board.size[0] + x) * 4) as usize;
        assert_eq!(&board.pixels[index..index + 4], &[32, 32, 32, 255]);
        assert_eq!(
            average([2, 1], &[255, 0, 0, 255, 0, 0, 255, 0], [1, 1], [0, 0]),
            [255, 0, 0, 127]
        );
    }

    #[test]
    fn pages_keep_the_requested_directory_and_single_page_name() {
        let path = Path::new("renders/片段.test.PNG");
        assert_eq!(page_path(path, 0, 1), path);
        assert_eq!(
            page_path(path, 0, 2),
            Path::new("renders/片段.test-001.PNG")
        );
        assert_eq!(
            page_path(path, 1, 2),
            Path::new("renders/片段.test-002.PNG")
        );
    }
}
