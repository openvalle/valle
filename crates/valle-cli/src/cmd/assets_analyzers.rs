//! Native asset analyzers. Shot detection uses weighted HSV differences, minimum shot duration,
//! flash suppression, and black-frame boundaries. The detection kernel is independent of decoding.

use serde_json::{Value, json};

use valle_project::assets::{
    AssetKind,
    analysis::{Analyzer, AnalyzerInput, AnalyzerOutput},
};

// Detection kernel.

/// Downsampled HSV grid statistics for one frame.
#[derive(Clone, Debug)]
pub struct FrameStats {
    /// Mean (h, s, v) per cell, each scaled to 0..255.
    pub cells: Vec<(f32, f32, f32)>,
    /// Mean frame brightness for black-frame detection.
    pub mean_v: f32,
}

/// Convert RGBA pixels into resolution-independent HSV grid statistics.
pub fn frame_stats(w: u32, h: u32, rgba: &[u8], grid: u32) -> FrameStats {
    let gw = grid.max(1);
    let gh = grid.max(1);
    let mut cells = Vec::with_capacity((gw * gh) as usize);
    let mut total_v = 0.0f64;
    for gy in 0..gh {
        for gx in 0..gw {
            let x0 = (gx * w) / gw;
            let x1 = (((gx + 1) * w) / gw).max(x0 + 1).min(w);
            let y0 = (gy * h) / gh;
            let y1 = (((gy + 1) * h) / gh).max(y0 + 1).min(h);
            let (mut sh, mut ss, mut sv, mut n) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            // Subsample within each cell to reduce computation.
            let step = (((x1 - x0) * (y1 - y0)) as f64 / 64.0).sqrt().max(1.0) as u32;
            let mut y = y0;
            while y < y1 {
                let mut x = x0;
                while x < x1 {
                    let i = ((y * w + x) * 4) as usize;
                    if i + 2 < rgba.len() {
                        let (hh, s, v) =
                            rgb_to_hsv(rgba[i] as f32, rgba[i + 1] as f32, rgba[i + 2] as f32);
                        sh += hh as f64;
                        ss += s as f64;
                        sv += v as f64;
                        n += 1.0;
                    }
                    x += step;
                }
                y += step;
            }
            let n = n.max(1.0);
            let cell = ((sh / n) as f32, (ss / n) as f32, (sv / n) as f32);
            total_v += cell.2 as f64;
            cells.push(cell);
        }
    }
    let mean_v = (total_v / cells.len().max(1) as f64) as f32;
    FrameStats { cells, mean_v }
}

/// Convert RGB to HSV with all channels scaled to 0..255.
fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let v = max;
    let s = if max > 0.0 { d / max * 255.0 } else { 0.0 };
    let h = if d == 0.0 {
        0.0
    } else if (max - r).abs() < f32::EPSILON {
        60.0 * (((g - b) / d) % 6.0)
    } else if (max - g).abs() < f32::EPSILON {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let h = if h < 0.0 { h + 360.0 } else { h };
    (h / 360.0 * 255.0, s, v)
}

/// Mean absolute grid difference with equal H/S/V weights.
pub fn content_diff(a: &FrameStats, b: &FrameStats) -> f32 {
    let n = a.cells.len().min(b.cells.len()).max(1);
    let mut sum = 0.0f64;
    for i in 0..n {
        let (ha, sa, va) = a.cells[i];
        let (hb, sb, vb) = b.cells[i];
        // Circular hue distance.
        let dh = (ha - hb).abs();
        let dh = dh.min(255.0 - dh);
        sum += ((dh + (sa - sb).abs() + (va - vb).abs()) / 3.0) as f64;
    }
    (sum / n as f64) as f32
}

/// Shot detector with thresholding, minimum duration, flash suppression, and black-frame
/// boundaries. Confirm a candidate on the next frame; returning to the previous scene cancels a
/// flash.
pub struct ShotDetector {
    threshold: f32,
    min_shot_frames: usize,
    black_v: f32,
    cuts: Vec<usize>,
    idx: usize,
    prev: Option<FrameStats>,
    /// Candidate frame index and pre-spike statistics.
    pending: Option<(usize, FrameStats)>,
    in_black: bool,
    last_cut: usize,
}

impl ShotDetector {
    pub fn new(threshold: f32, min_shot_frames: usize) -> Self {
        ShotDetector {
            threshold,
            min_shot_frames,
            black_v: 15.0,
            cuts: Vec::new(),
            idx: 0,
            prev: None,
            pending: None,
            in_black: false,
            last_cut: 0,
        }
    }

    fn commit(&mut self, at: usize) {
        if at >= self.last_cut + self.min_shot_frames {
            self.cuts.push(at);
            self.last_cut = at;
        }
    }

    pub fn push(&mut self, cur: FrameStats) {
        let idx = self.idx;
        self.idx += 1;

        // Entering and leaving black frames both mark boundaries, including fades.
        let is_black = cur.mean_v < self.black_v;
        if is_black != self.in_black {
            self.in_black = is_black;
            self.pending = None;
            self.commit(idx);
            self.prev = Some(cur);
            return;
        }

        if let Some((cand, pre_spike)) = self.pending.take() {
            // Confirm a cut only if the post-spike frame still differs from the previous scene.
            if content_diff(&pre_spike, &cur) > self.threshold {
                self.commit(cand);
            }
            self.prev = Some(cur);
            return;
        }
        if let Some(prev) = &self.prev {
            if content_diff(prev, &cur) > self.threshold {
                self.pending = Some((idx, prev.clone()));
            }
        }
        self.prev = Some(cur);
    }

    /// Treat a pending final candidate as a cut and return cut frame indices.
    pub fn finish(mut self) -> Vec<usize> {
        if let Some((cand, _)) = self.pending.take() {
            self.commit(cand);
        }
        self.cuts
    }
}

/// Convert cut indices to millisecond shot intervals, including the first and last boundaries.
pub fn cuts_to_items(cuts: &[usize], total_frames: usize, sample_fps: f64) -> Vec<Value> {
    let ms = |f: usize| ((f as f64 / sample_fps) * 1000.0).round() as i64;
    let mut bounds = vec![0usize];
    bounds.extend_from_slice(cuts);
    bounds.push(total_frames);
    bounds.dedup();
    bounds
        .windows(2)
        .filter(|w| w[1] > w[0])
        .map(|w| json!({ "start_ms": ms(w[0]), "end_ms": ms(w[1]) }))
        .collect()
}

// Native video decoding adapter.

pub struct ShotsAnalyzer;

impl Analyzer for ShotsAnalyzer {
    fn name(&self) -> &'static str {
        "shots"
    }
    fn version(&self) -> u32 {
        1
    }
    fn accepts(&self, kind: AssetKind) -> bool {
        kind == AssetKind::Video
    }
    fn default_params(&self) -> Value {
        json!({ "threshold": 27.0, "min_shot_s": 0.6, "sample_fps": 12.0, "grid": 8 })
    }
    fn analyze(&self, input: &AnalyzerInput<'_>) -> valle_project::assets::Result<AnalyzerOutput> {
        use valle_media::codec::VideoSource;
        let t0 = std::time::Instant::now();
        let p = input.params;
        let threshold = p["threshold"].as_f64().unwrap_or(27.0) as f32;
        let min_shot_s = p["min_shot_s"].as_f64().unwrap_or(0.6);
        let sample_fps = p["sample_fps"].as_f64().unwrap_or(12.0).max(1.0);
        let grid = p["grid"].as_u64().unwrap_or(8) as u32;

        let mut src = valle_media::codec::LibavVideoSource::open(input.path).map_err(|e| {
            valle_project::assets::AssetsError::analyzer_failed(format!(
                "failed to open decoder: {e}"
            ))
        })?;
        let dur = src.meta().duration_s.unwrap_or(0.0).max(0.0);
        let total = (dur * sample_fps).ceil() as usize;
        let mut det = ShotDetector::new(threshold, (min_shot_s * sample_fps).round() as usize);
        for i in 0..total {
            let t = i as f64 / sample_fps;
            let frame = src.frame_at(t).map_err(|e| {
                valle_project::assets::AssetsError::analyzer_failed(format!(
                    "frame decoding failed at t={t:.2}: {e}"
                ))
            })?;
            det.push(frame_stats(frame.width, frame.height, &frame.data, grid));
        }
        let cuts = det.finish();
        let items = cuts_to_items(&cuts, total, sample_fps);
        Ok(AnalyzerOutput {
            items,
            cost_ms: t0.elapsed().as_millis() as i64,
            cost_fen: 0,
        })
    }
}

// Beat analysis using DSP and libav decoding.

pub struct BeatsAnalyzer;

impl Analyzer for BeatsAnalyzer {
    fn name(&self) -> &'static str {
        "beats"
    }
    fn version(&self) -> u32 {
        1
    }
    fn accepts(&self, kind: AssetKind) -> bool {
        matches!(kind, AssetKind::Video | AssetKind::Audio)
    }
    fn default_params(&self) -> Value {
        json!({ "sample_hz": 22050, "min_bpm": 60.0, "max_bpm": 200.0 })
    }
    fn analyze(&self, input: &AnalyzerInput<'_>) -> valle_project::assets::Result<AnalyzerOutput> {
        let t0 = std::time::Instant::now();
        let p = input.params;
        let hz = p["sample_hz"].as_u64().unwrap_or(22050) as u32;
        let min_bpm = p["min_bpm"].as_f64().unwrap_or(60.0);
        let max_bpm = p["max_bpm"].as_f64().unwrap_or(200.0);
        let samples = valle_media::codec::decode_audio_mono_f32(input.path, hz).map_err(|e| {
            valle_project::assets::AssetsError::analyzer_failed(format!(
                "audio decoding failed: {e:#}"
            ))
        })?;
        let doc = valle_media::analysis::beat::detect(&samples, hz, min_bpm, max_bpm);
        // Store overall tempo, beat positions, and onsets. Consumers select the timing data they
        // need.
        let mut items = Vec::with_capacity(1 + doc.beats.len() + doc.onsets.len());
        if doc.bpm > 0.0 {
            items.push(json!({
                "kind": "tempo",
                "bpm": (doc.bpm * 10.0).round() / 10.0,
                "support": (doc.support * 100.0).round() / 100.0,
            }));
        }
        for b in &doc.beats {
            let ms = (b * 1000.0).round() as i64;
            items.push(json!({ "kind": "beat", "start_ms": ms, "end_ms": ms }));
        }
        for o in &doc.onsets {
            let ms = (o.t * 1000.0).round() as i64;
            items.push(json!({
                "kind": "onset",
                "start_ms": ms,
                "end_ms": ms,
                "strength": (f64::from(o.strength) * 1000.0).round() / 1000.0,
            }));
        }
        Ok(AnalyzerOutput {
            items,
            cost_ms: t0.elapsed().as_millis() as i64,
            cost_fen: 0,
        })
    }
}

// Extract representative video frames into the PNG cache.

pub struct CodecFrameExtractor;

impl valle_project::assets::vlm::FrameExtractor for CodecFrameExtractor {
    fn extract(
        &self,
        home: &valle_project::assets::Home,
        hash: &str,
        media_path: &std::path::Path,
        times_ms: &[i64],
    ) -> valle_project::assets::Result<Vec<Option<String>>> {
        use valle_media::codec::VideoSource;
        let mut src = valle_media::codec::LibavVideoSource::open(media_path).map_err(|e| {
            valle_project::assets::AssetsError::analyzer_failed(format!(
                "failed to open decoder: {e}"
            ))
        })?;
        let mut out = Vec::with_capacity(times_ms.len());
        // Sorted timestamps avoid unnecessary seeks.
        for &ms in times_ms {
            let dest = valle_project::assets::cachefs::ensure_cache_path(
                home,
                "frames",
                hash,
                &ms.to_string(),
                "png",
            )?;
            if dest.exists() {
                out.push(Some(dest.to_string_lossy().into_owned()));
                continue;
            }
            match src.frame_at(ms as f64 / 1000.0) {
                Ok(frame) => match write_png(&dest, frame.width, frame.height, &frame.data) {
                    Ok(()) => out.push(Some(dest.to_string_lossy().into_owned())),
                    Err(_) => out.push(None),
                },
                Err(_) => out.push(None), // Allow missing frames.
            }
        }
        Ok(out)
    }
}

fn write_png(dest: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> anyhow::Result<()> {
    let f = std::fs::File::create(dest)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}

/// Register shot and beat analyzers and the frame extractor. Assets never runs ASR, but VLM and
/// search may read an existing `asr@1` slot. `VALLE_VLM_CMD` supplies the external VLM command.
pub fn assemble() -> Vec<Box<dyn Analyzer>> {
    let mut v: Vec<Box<dyn Analyzer>> = vec![Box::new(ShotsAnalyzer), Box::new(BeatsAnalyzer)];
    if let Ok(cmd) = std::env::var("VALLE_VLM_CMD") {
        let argv: Vec<String> = cmd.split_whitespace().map(|s| s.to_owned()).collect();
        if !argv.is_empty() {
            let extractor: Box<dyn valle_project::assets::vlm::FrameExtractor> =
                Box::new(CodecFrameExtractor);

            v.push(Box::new(valle_project::assets::vlm::VlmAnalyzer {
                argv,
                timeout: std::time::Duration::from_secs(3600),
                params: json!({ "model": "qwen3.5:4b" }),
                extractor,
            }));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_assembly_never_registers_an_asr_runner() {
        assert!(!assemble().iter().any(|analyzer| analyzer.name() == "asr"));
    }

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            v.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        v
    }

    fn stats(rgb: [u8; 3]) -> FrameStats {
        frame_stats(64, 36, &solid(64, 36, rgb), 8)
    }

    #[test]
    fn hard_cut_detected() {
        let mut det = ShotDetector::new(27.0, 3);
        for _ in 0..10 {
            det.push(stats([200, 30, 30])); // Red scene.
        }
        for _ in 0..10 {
            det.push(stats([30, 60, 200])); // Blue scene.
        }
        let cuts = det.finish();
        assert_eq!(
            cuts,
            vec![10],
            "exactly one cut at the red-to-blue boundary"
        );
        let items = cuts_to_items(&cuts, 20, 10.0);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["end_ms"], 1000);
        assert_eq!(items[1]["start_ms"], 1000);
        assert_eq!(items[1]["end_ms"], 2000);
    }

    #[test]
    fn flash_suppressed() {
        let mut det = ShotDetector::new(27.0, 3);
        for _ in 0..8 {
            det.push(stats([200, 30, 30]));
        }
        det.push(stats([255, 255, 255])); // Single-frame flash.
        for _ in 0..8 {
            det.push(stats([200, 30, 30])); // Return to the original scene.
        }
        assert!(det.finish().is_empty(), "flashes must not produce cuts");
    }

    #[test]
    fn fast_motion_gradual_no_cut() {
        let mut det = ShotDetector::new(27.0, 3);
        // Gradual brightness change approximates continuous motion.
        for i in 0..30u8 {
            det.push(stats([100 + i, 80, 60]));
        }
        assert!(
            det.finish().is_empty(),
            "gradual changes must not produce cuts"
        );
    }

    #[test]
    fn black_gap_bounds_detected() {
        let mut det = ShotDetector::new(27.0, 2);
        for _ in 0..6 {
            det.push(stats([200, 30, 30]));
        }
        for _ in 0..4 {
            det.push(stats([2, 2, 2])); // Black frames.
        }
        for _ in 0..6 {
            det.push(stats([30, 200, 60]));
        }
        let cuts = det.finish();
        assert_eq!(
            cuts,
            vec![6, 10],
            "one cut entering black and one leaving black"
        );
    }

    #[test]
    fn min_shot_len_respected() {
        let mut det = ShotDetector::new(27.0, 5);
        for _ in 0..6 {
            det.push(stats([200, 30, 30]));
        }
        // Rapid color changes are suppressed by the minimum shot duration.
        for i in 0..10 {
            let c = if i % 2 == 0 {
                [30, 60, 200]
            } else {
                [30, 200, 60]
            };
            det.push(stats(c));
            det.push(stats(c));
        }
        let cuts = det.finish();
        for w in cuts.windows(2) {
            assert!(w[1] - w[0] >= 5, "minimum shot duration: {cuts:?}");
        }
    }
}
