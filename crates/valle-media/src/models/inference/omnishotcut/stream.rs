use std::{collections::VecDeque, time::Instant};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::models::inference::omnishotcut::{
    frame::{FRAME_BYTES, FrameView},
    preprocess::RgbaPreprocessor,
};

pub const WINDOW_FRAMES: usize = 100;
pub const OVERLAP_FRAMES: usize = 20;
pub const STRIDE_FRAMES: usize = WINDOW_FRAMES - OVERLAP_FRAMES;
pub const QUERY_COUNT: usize = 24;
pub const MODEL_WINDOW_BYTES: usize = WINDOW_FRAMES * FRAME_BYTES;
/// One look-ahead frame is needed to decide whether a full window is final.
pub const MAX_BUFFERED_FRAMES: usize = WINDOW_FRAMES + 1;

pub const INTRA_LABELS: [&str; 9] = [
    "General", "Dissolve", "Wipes", "Push", "Slide", "Zoom", "Fade", "Doorway", "Padding",
];
pub const INTER_LABELS: [&str; 6] = [
    "New_Start",
    "Hard_Cut",
    "Transition_Source",
    "Transition",
    "Sudden_Jump",
    "Padding",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum IntraTransition {
    General = 0,
    Dissolve = 1,
    Wipes = 2,
    Push = 3,
    Slide = 4,
    Zoom = 5,
    Fade = 6,
    Doorway = 7,
    Padding = 8,
}

impl IntraTransition {
    pub const fn index(self) -> u8 {
        self as u8
    }

    pub const fn label(self) -> &'static str {
        INTRA_LABELS[self.index() as usize]
    }
}

impl TryFrom<i64> for IntraTransition {
    type Error = anyhow::Error;

    fn try_from(value: i64) -> Result<Self> {
        match value {
            0 => Ok(Self::General),
            1 => Ok(Self::Dissolve),
            2 => Ok(Self::Wipes),
            3 => Ok(Self::Push),
            4 => Ok(Self::Slide),
            5 => Ok(Self::Zoom),
            6 => Ok(Self::Fade),
            7 => Ok(Self::Doorway),
            8 => Ok(Self::Padding),
            _ => anyhow::bail!("unknown OmniShotCut intra transition index {value}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum InterTransition {
    NewStart = 0,
    HardCut = 1,
    TransitionSource = 2,
    Transition = 3,
    SuddenJump = 4,
    Padding = 5,
}

impl InterTransition {
    pub const fn index(self) -> u8 {
        self as u8
    }

    pub const fn label(self) -> &'static str {
        INTER_LABELS[self.index() as usize]
    }
}

impl TryFrom<i64> for InterTransition {
    type Error = anyhow::Error;

    fn try_from(value: i64) -> Result<Self> {
        match value {
            0 => Ok(Self::NewStart),
            1 => Ok(Self::HardCut),
            2 => Ok(Self::TransitionSource),
            3 => Ok(Self::Transition),
            4 => Ok(Self::SuddenJump),
            5 => Ok(Self::Padding),
            _ => anyhow::bail!("unknown OmniShotCut inter transition index {value}"),
        }
    }
}

/// A half-open model-predicted shot range and its transition classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shot {
    pub start_frame: usize,
    pub end_frame_exclusive: usize,
    pub intra: IntraTransition,
    pub inter: InterTransition,
}

/// Final result for one video stream. Raw frames and per-window tensors are not retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShotList {
    pub frame_count: usize,
    pub window_count: usize,
    pub max_buffered_frames: usize,
    pub shots: Vec<Shot>,
}

/// Raw predictions returned by one 100-frame backend invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowPrediction {
    pub intra: Vec<i64>,
    pub inter: Vec<i64>,
    pub ranges: Vec<i64>,
}

impl WindowPrediction {
    pub fn new(intra: Vec<i64>, inter: Vec<i64>, ranges: Vec<i64>) -> Self {
        Self {
            intra,
            inter,
            ranges,
        }
    }
}

/// Backend boundary for deterministic host tests and platform runtimes.
///
/// `frames` is a borrowed packed `[100,96,128,3]` RGB u8 tensor. A backend must
/// reuse its loaded model/session across calls.
pub trait WindowPredictor {
    fn predict_window(&mut self, frames: &[u8]) -> Result<WindowPrediction>;
}

/// Host-side work performed while one raw RGBA frame crosses the adapter boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PushTiming {
    pub preprocess_seconds: f64,
    pub inference_seconds: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Boundary {
    end_frame_exclusive: usize,
    intra: IntraTransition,
    inter: InterTransition,
}

/// Fixed-memory rolling OmniShotCut adapter backed by one reusable predictor.
pub struct OmniShotCutSession<P> {
    predictor: P,
    rgba_preprocessor: RgbaPreprocessor,
    frames: VecDeque<Vec<u8>>,
    recycled_frames: Vec<Vec<u8>>,
    window: Vec<u8>,
    buffer_start: usize,
    frame_count: usize,
    next_window_start: usize,
    window_count: usize,
    max_buffered_frames: usize,
    boundaries: Vec<Boundary>,
}

impl<P: WindowPredictor> OmniShotCutSession<P> {
    pub fn with_predictor(predictor: P) -> Self {
        Self {
            predictor,
            rgba_preprocessor: RgbaPreprocessor::new(),
            frames: VecDeque::with_capacity(MAX_BUFFERED_FRAMES),
            recycled_frames: Vec::with_capacity(MAX_BUFFERED_FRAMES),
            window: Vec::with_capacity(MODEL_WINDOW_BYTES),
            buffer_start: 0,
            frame_count: 0,
            next_window_start: 0,
            window_count: 0,
            max_buffered_frames: 0,
            boundaries: Vec::new(),
        }
    }

    /// Append one model-ready frame and process every window whose ownership is final.
    pub fn push_frame(&mut self, frame: FrameView<'_>) -> Result<()> {
        let mut storage = self.take_frame_storage();
        frame.copy_rgb_into(&mut storage);
        self.push_owned_frame(storage)
    }

    /// Preprocess and append one tightly packed, display-oriented straight-alpha RGBA8 frame.
    ///
    /// Resize, RGB packing and model geometry are release-adapter semantics. Media hosts should
    /// call this method instead of reproducing the published 128x96 tensor boundary themselves.
    pub fn push_rgba(&mut self, width: usize, height: usize, rgba: &[u8]) -> Result<PushTiming> {
        let mut storage = self.take_frame_storage();
        let preprocess_started = Instant::now();
        if let Err(error) =
            self.rgba_preprocessor
                .resize_rgba_into(width, height, rgba, &mut storage)
        {
            self.recycle_frame_storage(storage);
            return Err(error);
        }
        let preprocess_seconds = preprocess_started.elapsed().as_secs_f64();
        let inference_started = Instant::now();
        self.push_owned_frame(storage)?;
        Ok(PushTiming {
            preprocess_seconds,
            inference_seconds: inference_started.elapsed().as_secs_f64(),
        })
    }

    fn push_owned_frame(&mut self, frame: Vec<u8>) -> Result<()> {
        ensure!(
            frame.len() == FRAME_BYTES,
            "OmniShotCut model frame has {} bytes; expected {FRAME_BYTES}",
            frame.len()
        );
        self.frames.push_back(frame);
        self.frame_count = self
            .frame_count
            .checked_add(1)
            .context("OmniShotCut frame count overflow")?;
        self.max_buffered_frames = self.max_buffered_frames.max(self.frames.len());
        ensure!(
            self.frames.len() <= MAX_BUFFERED_FRAMES,
            "OmniShotCut rolling buffer exceeded {MAX_BUFFERED_FRAMES} frames"
        );
        self.process_ready_windows(false)
    }

    fn take_frame_storage(&mut self) -> Vec<u8> {
        let mut storage = self
            .recycled_frames
            .pop()
            .unwrap_or_else(|| Vec::with_capacity(FRAME_BYTES));
        storage.clear();
        storage
    }

    fn recycle_frame_storage(&mut self, mut storage: Vec<u8>) {
        storage.clear();
        if self.recycled_frames.len() < MAX_BUFFERED_FRAMES {
            self.recycled_frames.push(storage);
        }
    }

    pub const fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub const fn processed_window_count(&self) -> usize {
        self.window_count
    }

    pub fn buffered_frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Flush the final black-padded window and reset stream state for reuse.
    ///
    /// The predictor remains loaded. Calling `finish` again without pushing a
    /// frame returns an empty result and never invokes the backend.
    pub fn finish(&mut self) -> Result<ShotList> {
        self.process_ready_windows(true)?;
        let shots = boundaries_to_shots(std::mem::take(&mut self.boundaries));
        let result = ShotList {
            frame_count: self.frame_count,
            window_count: self.window_count,
            max_buffered_frames: self.max_buffered_frames,
            shots,
        };
        self.reset_stream();
        Ok(result)
    }

    /// Discard an unfinished stream without unloading the predictor.
    pub fn reset(&mut self) {
        self.reset_stream();
    }

    pub fn predictor(&self) -> &P {
        &self.predictor
    }

    pub fn predictor_mut(&mut self) -> &mut P {
        &mut self.predictor
    }

    fn process_ready_windows(&mut self, finishing: bool) -> Result<()> {
        while self.next_window_start < self.frame_count {
            let full_end = self
                .next_window_start
                .checked_add(WINDOW_FRAMES)
                .context("OmniShotCut window position overflow")?;
            if !finishing && self.frame_count <= full_end {
                break;
            }
            self.process_window(finishing)?;
            self.window_count = self
                .window_count
                .checked_add(1)
                .context("OmniShotCut window count overflow")?;
            self.next_window_start = self
                .next_window_start
                .checked_add(STRIDE_FRAMES)
                .context("OmniShotCut window position overflow")?;
            self.discard_before(self.next_window_start);
            if finishing {
                break;
            }
        }
        Ok(())
    }

    fn process_window(&mut self, is_final: bool) -> Result<()> {
        let start = self.next_window_start;
        let valid_len = WINDOW_FRAMES.min(self.frame_count - start);
        let left_overlap = OVERLAP_FRAMES / 2;
        let right_overlap = OVERLAP_FRAMES - left_overlap;
        let valid_start = if start == 0 { 0 } else { start + left_overlap };
        let valid_end = if is_final {
            self.frame_count
        } else {
            start + WINDOW_FRAMES - right_overlap
        };

        self.build_window(start, valid_len)?;
        let prediction = self.predictor.predict_window(&self.window)?;
        validate_prediction(&prediction)?;

        let mut window_boundaries = Vec::new();
        let mut local_start = 0usize;
        for query in 0..QUERY_COUNT {
            let raw_end = usize::try_from(prediction.ranges[query]).with_context(|| {
                format!(
                    "OmniShotCut range_idx[{query}] is negative: {}",
                    prediction.ranges[query]
                )
            })?;
            let local_end = raw_end.min(valid_len);
            if local_start >= local_end {
                continue;
            }
            let global_end = start + local_end;
            if valid_start < global_end && global_end <= valid_end {
                window_boundaries.push(Boundary {
                    end_frame_exclusive: global_end,
                    intra: IntraTransition::try_from(prediction.intra[query])?,
                    inter: InterTransition::try_from(prediction.inter[query])?,
                });
            }
            local_start = local_end;
            if local_end >= valid_len {
                break;
            }
        }
        window_boundaries.sort_unstable_by_key(|boundary| boundary.end_frame_exclusive);
        for boundary in window_boundaries {
            if self.boundaries.last().is_some_and(|previous| {
                boundary
                    .end_frame_exclusive
                    .abs_diff(previous.end_frame_exclusive)
                    <= 2
            }) {
                continue;
            }
            self.boundaries.push(boundary);
        }
        Ok(())
    }

    fn build_window(&mut self, start: usize, valid_len: usize) -> Result<()> {
        ensure!(
            start >= self.buffer_start,
            "OmniShotCut frame {start} was discarded before inference"
        );
        self.window.clear();
        for frame_index in start..start + valid_len {
            let offset = frame_index - self.buffer_start;
            let frame = self.frames.get(offset).with_context(|| {
                format!(
                    "OmniShotCut frame {frame_index} is unavailable in rolling buffer {}..{}",
                    self.buffer_start,
                    self.buffer_start + self.frames.len()
                )
            })?;
            self.window.extend_from_slice(frame);
        }
        self.window.resize(MODEL_WINDOW_BYTES, 0);
        Ok(())
    }

    fn discard_before(&mut self, target: usize) {
        while self.buffer_start < target {
            let Some(frame) = self.frames.pop_front() else {
                break;
            };
            self.recycle_frame_storage(frame);
            self.buffer_start += 1;
        }
    }

    fn reset_stream(&mut self) {
        while let Some(frame) = self.frames.pop_front() {
            self.recycle_frame_storage(frame);
        }
        self.rgba_preprocessor.reset_stream();
        self.window.clear();
        self.buffer_start = 0;
        self.frame_count = 0;
        self.next_window_start = 0;
        self.window_count = 0;
        self.max_buffered_frames = 0;
        self.boundaries.clear();
    }
}

fn validate_prediction(prediction: &WindowPrediction) -> Result<()> {
    ensure!(
        prediction.intra.len() == QUERY_COUNT,
        "OmniShotCut intra_idx has {} values; expected {QUERY_COUNT}",
        prediction.intra.len()
    );
    ensure!(
        prediction.inter.len() == QUERY_COUNT,
        "OmniShotCut inter_idx has {} values; expected {QUERY_COUNT}",
        prediction.inter.len()
    );
    ensure!(
        prediction.ranges.len() == QUERY_COUNT,
        "OmniShotCut range_idx has {} values; expected {QUERY_COUNT}",
        prediction.ranges.len()
    );
    Ok(())
}

fn boundaries_to_shots(boundaries: Vec<Boundary>) -> Vec<Shot> {
    let mut shots = Vec::with_capacity(boundaries.len());
    let mut start = 0usize;
    for boundary in boundaries {
        if boundary.end_frame_exclusive <= start {
            continue;
        }
        shots.push(Shot {
            start_frame: start,
            end_frame_exclusive: boundary.end_frame_exclusive,
            intra: boundary.intra,
            inter: boundary.inter,
        });
        start = boundary.end_frame_exclusive;
    }
    shots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::inference::omnishotcut::{MODEL_HEIGHT, MODEL_WIDTH};

    #[derive(Default)]
    struct ScriptedPredictor {
        predictions: VecDeque<WindowPrediction>,
        windows: Vec<Vec<u8>>,
    }

    impl ScriptedPredictor {
        fn with_predictions(predictions: Vec<WindowPrediction>) -> Self {
            Self {
                predictions: predictions.into(),
                windows: Vec::new(),
            }
        }
    }

    impl WindowPredictor for ScriptedPredictor {
        fn predict_window(&mut self, frames: &[u8]) -> Result<WindowPrediction> {
            self.windows.push(frames.to_vec());
            self.predictions
                .pop_front()
                .context("test predictor has no scripted prediction")
        }
    }

    fn prediction(ranges: &[i64], intra: i64, inter: i64) -> WindowPrediction {
        let mut range_values = vec![100; QUERY_COUNT];
        range_values[..ranges.len()].copy_from_slice(ranges);
        WindowPrediction::new(
            vec![intra; QUERY_COUNT],
            vec![inter; QUERY_COUNT],
            range_values,
        )
    }

    fn push_solid_frames<P: WindowPredictor>(
        session: &mut OmniShotCutSession<P>,
        count: usize,
    ) -> Result<()> {
        for index in 0..count {
            let frame = vec![(index % 251) as u8; FRAME_BYTES];
            session.push_frame(FrameView::rgb8(MODEL_WIDTH, MODEL_HEIGHT, &frame)?)?;
        }
        Ok(())
    }

    #[test]
    fn raw_rgba_boundary_owns_resize_rgb_packing_and_stream_reset() {
        let predictor = ScriptedPredictor::with_predictions(vec![
            prediction(&[1], 0, 1),
            prediction(&[1], 0, 1),
        ]);
        let mut session = OmniShotCutSession::with_predictor(predictor);
        let first = [7, 9, 11, 0].repeat(4);
        let timing = session.push_rgba(2, 2, &first).unwrap();
        assert!(timing.preprocess_seconds >= 0.0);
        assert!(timing.inference_seconds >= 0.0);
        session.finish().unwrap();
        assert!(
            session.predictor().windows[0][..FRAME_BYTES]
                .chunks_exact(3)
                .all(|pixel| pixel == [7, 9, 11])
        );

        // Finishing resets source geometry while retaining the loaded predictor and buffers.
        let second = [13, 17, 19, 255].repeat(6);
        session.push_rgba(3, 2, &second).unwrap();
        session.finish().unwrap();
        assert!(
            session.predictor().windows[1][..FRAME_BYTES]
                .chunks_exact(3)
                .all(|pixel| pixel == [13, 17, 19])
        );
    }

    #[test]
    fn raw_rgba_boundary_rejects_midstream_geometry_changes() {
        let predictor = ScriptedPredictor::with_predictions(vec![prediction(&[2], 0, 1)]);
        let mut session = OmniShotCutSession::with_predictor(predictor);
        session.push_rgba(2, 2, &[0; 16]).unwrap();
        assert!(session.push_rgba(3, 2, &[0; 24]).is_err());
        assert_eq!(session.frame_count(), 1);
    }

    #[test]
    fn empty_finish_is_a_backend_free_noop_and_session_is_reusable() {
        let mut session = OmniShotCutSession::with_predictor(ScriptedPredictor::default());
        assert_eq!(
            session.finish().unwrap(),
            ShotList {
                frame_count: 0,
                window_count: 0,
                max_buffered_frames: 0,
                shots: Vec::new(),
            }
        );
        assert!(session.predictor().windows.is_empty());
        assert_eq!(session.finish().unwrap().window_count, 0);
    }

    #[test]
    fn final_window_is_black_padded_and_state_does_not_leak() {
        let predictor = ScriptedPredictor::with_predictions(vec![
            prediction(&[2, 3], 0, 1),
            prediction(&[1], 1, 3),
        ]);
        let mut session = OmniShotCutSession::with_predictor(predictor);
        push_solid_frames(&mut session, 3).unwrap();
        let first = session.finish().unwrap();
        assert_eq!(first.frame_count, 3);
        assert_eq!(first.window_count, 1);
        assert_eq!(first.shots.len(), 1);
        let first_window = &session.predictor().windows[0];
        assert_eq!(first_window.len(), MODEL_WINDOW_BYTES);
        assert!(
            first_window[3 * FRAME_BYTES..]
                .iter()
                .all(|value| *value == 0)
        );

        push_solid_frames(&mut session, 1).unwrap();
        let second = session.finish().unwrap();
        assert_eq!(second.frame_count, 1);
        assert_eq!(second.window_count, 1);
        assert_eq!(second.shots[0].start_frame, 0);
        assert_eq!(second.shots[0].end_frame_exclusive, 1);
        assert_eq!(session.predictor().windows.len(), 2);
    }

    #[test]
    fn one_frame_lookahead_preserves_final_overlap_ownership() {
        let predictor = ScriptedPredictor::with_predictions(vec![
            prediction(&[25, 50, 75, 100], 0, 1),
            prediction(&[21], 0, 1),
        ]);
        let mut session = OmniShotCutSession::with_predictor(predictor);
        push_solid_frames(&mut session, 100).unwrap();
        assert_eq!(session.processed_window_count(), 0);
        assert_eq!(session.buffered_frame_count(), 100);
        push_solid_frames(&mut session, 1).unwrap();
        assert_eq!(session.processed_window_count(), 1);
        assert_eq!(session.buffered_frame_count(), 21);
        let result = session.finish().unwrap();
        assert_eq!(result.frame_count, 101);
        assert_eq!(result.window_count, 2);
        assert!(result.max_buffered_frames <= MAX_BUFFERED_FRAMES);
        assert_eq!(
            result
                .shots
                .iter()
                .map(|shot| shot.end_frame_exclusive)
                .collect::<Vec<_>>(),
            [25, 50, 75, 101]
        );
    }

    #[test]
    fn exact_full_window_is_inferred_once_as_the_final_window() {
        let predictor =
            ScriptedPredictor::with_predictions(vec![prediction(&[25, 50, 75, 100], 0, 1)]);
        let mut session = OmniShotCutSession::with_predictor(predictor);
        push_solid_frames(&mut session, WINDOW_FRAMES).unwrap();
        assert_eq!(session.processed_window_count(), 0);
        let result = session.finish().unwrap();
        assert_eq!(result.window_count, 1);
        assert_eq!(session.predictor().windows.len(), 1);
        assert_eq!(result.shots.last().unwrap().end_frame_exclusive, 100);
    }

    #[test]
    fn long_stream_has_duration_independent_frame_retention() {
        let windows = ((10_000usize.saturating_sub(1)) / STRIDE_FRAMES) + 1;
        let predictions = (0..windows).map(|_| prediction(&[100], 0, 0)).collect();
        let mut session =
            OmniShotCutSession::with_predictor(ScriptedPredictor::with_predictions(predictions));
        push_solid_frames(&mut session, 10_000).unwrap();
        let result = session.finish().unwrap();
        assert!(result.max_buffered_frames <= MAX_BUFFERED_FRAMES);
        assert_eq!(result.window_count, windows);
        assert_eq!(session.buffered_frame_count(), 0);
    }

    #[test]
    fn invalid_prediction_shape_or_class_is_rejected() {
        let malformed = WindowPrediction::new(vec![0], vec![0], vec![1]);
        let mut session =
            OmniShotCutSession::with_predictor(ScriptedPredictor::with_predictions(vec![
                malformed,
            ]));
        push_solid_frames(&mut session, 1).unwrap();
        assert!(
            session
                .finish()
                .unwrap_err()
                .to_string()
                .contains("intra_idx")
        );

        let mut session =
            OmniShotCutSession::with_predictor(ScriptedPredictor::with_predictions(vec![
                prediction(&[1], 99, 0),
            ]));
        push_solid_frames(&mut session, 1).unwrap();
        assert!(
            session
                .finish()
                .unwrap_err()
                .to_string()
                .contains("intra transition")
        );
    }

    #[test]
    fn transition_indices_and_labels_are_stable() {
        assert_eq!(IntraTransition::Fade.index(), 6);
        assert_eq!(IntraTransition::Fade.label(), "Fade");
        assert_eq!(InterTransition::HardCut.index(), 1);
        assert_eq!(InterTransition::HardCut.label(), "Hard_Cut");
        assert!(InterTransition::try_from(6).is_err());
    }
}
