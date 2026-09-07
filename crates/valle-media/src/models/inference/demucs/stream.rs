use std::collections::VecDeque;

use anyhow::{Context, Result, ensure};

use crate::models::inference::demucs::{
    CHANNELS, SEGMENT_SAMPLES, STEMS, STRIDE_SAMPLES, TrackNormalization, triangular_weight,
};

/// Pull-based stereo source used for the second pass over a decoded/spooled track.
pub trait StereoSource {
    /// Fill equally-sized left/right buffers and return the number of sample frames written.
    /// Returning zero signals EOF.
    fn read_stereo(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<usize>;
}

#[derive(Debug, Clone, Copy)]
pub struct StereoSamples<'a> {
    pub left: &'a [f32],
    pub right: &'a [f32],
}

/// Finalized, denormalized output whose samples can no longer receive future overlap weight.
#[derive(Debug, Clone, Copy)]
pub struct StemChunk<'a> {
    pub start_sample: u64,
    pub vocals: StereoSamples<'a>,
    pub instrumental: StereoSamples<'a>,
}

impl StemChunk<'_> {
    pub fn samples(&self) -> usize {
        self.vocals.left.len()
    }
}

/// Sink for streaming output. Slices are valid only for the duration of the call.
pub trait StemSink {
    fn write_stems(&mut self, chunk: StemChunk<'_>) -> Result<()>;
}

/// Fixed-size model output in stem-major, channel-major order.
#[derive(Debug)]
pub struct SegmentOutput {
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

/// Backend boundary for one normalized 7.8-second segment.
pub trait SegmentBackend {
    fn infer_segment(&mut self, mix: &[f32], shape: [usize; 3]) -> Result<SegmentOutput>;
}

#[derive(Debug, Clone, Copy)]
pub struct AdapterOptions {
    pub read_chunk_samples: usize,
}

impl Default for AdapterOptions {
    fn default() -> Self {
        Self {
            read_chunk_samples: 65_536,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DemucsAdapter {
    options: AdapterOptions,
}

impl DemucsAdapter {
    pub fn new(options: AdapterOptions) -> Result<Self> {
        ensure!(
            options.read_chunk_samples > 0 && options.read_chunk_samples <= SEGMENT_SAMPLES,
            "read_chunk_samples must be within 1..={SEGMENT_SAMPLES}"
        );
        Ok(Self { options })
    }

    pub const fn options(&self) -> AdapterOptions {
        self.options
    }

    /// Separate one track using a bounded two-pass source/sink contract.
    ///
    /// The first pass is represented by `normalization`; callers typically compute it while
    /// decoding to a rewindable spool. This second pass reads only fixed chunks, keeps at most one
    /// model segment plus one overlap tail, and emits at most one stride of finalized stems.
    pub fn separate(
        &self,
        backend: &mut impl SegmentBackend,
        normalization: TrackNormalization,
        source: &mut impl StereoSource,
        sink: &mut impl StemSink,
    ) -> Result<SeparationSummary> {
        let total_samples = usize::try_from(normalization.samples())
            .context("track sample count exceeds addressable memory")?;
        ensure!(
            total_samples >= 2,
            "Demucs track must contain at least two samples"
        );

        let mut read_left = vec![0.0_f32; self.options.read_chunk_samples];
        let mut read_right = vec![0.0_f32; self.options.read_chunk_samples];
        let mut input_left = VecDeque::with_capacity(SEGMENT_SAMPLES);
        let mut input_right = VecDeque::with_capacity(SEGMENT_SAMPLES);
        let mut source_samples = 0_usize;
        let mut emitted_samples = 0_usize;
        let mut segments = 0_usize;
        let overlap = SEGMENT_SAMPLES - STRIDE_SAMPLES;
        let mut prior_tail = vec![0.0_f32; STEMS * CHANNELS as usize * overlap];
        let mut has_prior_tail = false;

        while emitted_samples < total_samples {
            while input_left.len() < SEGMENT_SAMPLES && source_samples < total_samples {
                let requested = self
                    .options
                    .read_chunk_samples
                    .min(total_samples - source_samples)
                    .min(SEGMENT_SAMPLES - input_left.len());
                let count = source
                    .read_stereo(&mut read_left[..requested], &mut read_right[..requested])?;
                ensure!(
                    count > 0,
                    "stereo source ended after {source_samples} samples; statistics describe {total_samples}"
                );
                ensure!(
                    count <= requested,
                    "stereo source returned too many samples"
                );
                for (&left, &right) in read_left[..count].iter().zip(&read_right[..count]) {
                    ensure!(
                        left.is_finite() && right.is_finite(),
                        "stereo source contains NaN or Inf"
                    );
                    input_left.push_back(normalization.normalize(left));
                    input_right.push_back(normalization.normalize(right));
                }
                source_samples += count;
            }

            let available = input_left.len();
            ensure!(
                available == input_right.len(),
                "internal stereo queue drifted"
            );
            input_left.resize(SEGMENT_SAMPLES, 0.0);
            input_right.resize(SEGMENT_SAMPLES, 0.0);
            let mut mix = vec![0.0_f32; CHANNELS as usize * SEGMENT_SAMPLES];
            for (index, &sample) in input_left.iter().enumerate() {
                mix[index] = sample;
            }
            for (index, &sample) in input_right.iter().enumerate() {
                mix[SEGMENT_SAMPLES + index] = sample;
            }
            ensure!(
                available == SEGMENT_SAMPLES || source_samples == total_samples,
                "attempted to pad a non-final Demucs segment: buffered {available}, read {source_samples}/{total_samples} samples"
            );
            let output = backend.infer_segment(&mix, [1, 2, SEGMENT_SAMPLES])?;
            validate_output(&output)?;
            let emit = STRIDE_SAMPLES.min(total_samples - emitted_samples);
            let mut finalized = vec![0.0_f32; STEMS * CHANNELS as usize * emit];
            for stem in 0..STEMS {
                for channel in 0..CHANNELS as usize {
                    let output_offset = (stem * CHANNELS as usize + channel) * SEGMENT_SAMPLES;
                    let finalized_offset = (stem * CHANNELS as usize + channel) * emit;
                    let tail_offset = (stem * CHANNELS as usize + channel) * overlap;
                    for index in 0..emit {
                        let current_weight = triangular_weight(index);
                        let mut weighted = output.data[output_offset + index] * current_weight;
                        let mut weight = current_weight;
                        if has_prior_tail && index < overlap {
                            weighted += prior_tail[tail_offset + index];
                            weight += triangular_weight(STRIDE_SAMPLES + index);
                        }
                        let normalized = weighted / weight;
                        finalized[finalized_offset + index] =
                            normalization.denormalize(stem, normalized);
                    }
                    for index in 0..overlap {
                        prior_tail[tail_offset + index] = output.data
                            [output_offset + STRIDE_SAMPLES + index]
                            * triangular_weight(STRIDE_SAMPLES + index);
                    }
                }
            }

            let channels = CHANNELS as usize;
            let vocals_left = &finalized[0..emit];
            let vocals_right = &finalized[emit..2 * emit];
            let instrumental_left = &finalized[channels * emit..(channels + 1) * emit];
            let instrumental_right = &finalized[(channels + 1) * emit..(channels + 2) * emit];
            sink.write_stems(StemChunk {
                start_sample: emitted_samples as u64,
                vocals: StereoSamples {
                    left: vocals_left,
                    right: vocals_right,
                },
                instrumental: StereoSamples {
                    left: instrumental_left,
                    right: instrumental_right,
                },
            })?;

            emitted_samples += emit;
            segments += 1;
            has_prior_tail = true;
            input_left.drain(..STRIDE_SAMPLES);
            input_right.drain(..STRIDE_SAMPLES);
        }

        ensure!(
            source_samples == total_samples,
            "internal source count drifted"
        );
        let mut extra_left = [0.0_f32; 1];
        let mut extra_right = [0.0_f32; 1];
        ensure!(
            source.read_stereo(&mut extra_left, &mut extra_right)? == 0,
            "stereo source contains more samples than its whole-track statistics"
        );
        Ok(SeparationSummary {
            samples: emitted_samples as u64,
            segments,
            max_input_samples: CHANNELS as usize * SEGMENT_SAMPLES,
            max_output_samples: STEMS * CHANNELS as usize * SEGMENT_SAMPLES,
            max_finalized_samples: STEMS * CHANNELS as usize * STRIDE_SAMPLES,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeparationSummary {
    pub samples: u64,
    pub segments: usize,
    pub max_input_samples: usize,
    pub max_output_samples: usize,
    pub max_finalized_samples: usize,
}

fn validate_output(output: &SegmentOutput) -> Result<()> {
    let expected_shape = vec![1, STEMS, CHANNELS as usize, SEGMENT_SAMPLES];
    ensure!(
        output.shape == expected_shape,
        "Demucs backend returned shape {:?}, expected {:?}",
        output.shape,
        expected_shape
    );
    let expected_values = expected_shape.iter().product::<usize>();
    ensure!(
        output.data.len() == expected_values,
        "Demucs backend returned {} values, expected {expected_values}",
        output.data.len()
    );
    ensure!(
        output.data.iter().all(|value| value.is_finite()),
        "Demucs backend returned NaN or Inf"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::models::inference::demucs::{
        OVERLAP_SAMPLES, SAMPLE_RATE, STRIDE_SAMPLES, TrackStatsBuilder,
    };

    struct SliceSource {
        left: Vec<f32>,
        right: Vec<f32>,
        position: usize,
        max_read: usize,
    }

    impl SliceSource {
        fn new(left: Vec<f32>, right: Vec<f32>, max_read: usize) -> Self {
            Self {
                left,
                right,
                position: 0,
                max_read,
            }
        }
    }

    impl StereoSource for SliceSource {
        fn read_stereo(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<usize> {
            let count = left
                .len()
                .min(right.len())
                .min(self.max_read)
                .min(self.left.len().saturating_sub(self.position));
            left[..count].copy_from_slice(&self.left[self.position..self.position + count]);
            right[..count].copy_from_slice(&self.right[self.position..self.position + count]);
            self.position += count;
            Ok(count)
        }
    }

    #[derive(Default)]
    struct CaptureSink {
        starts: Vec<u64>,
        vocals_left: Vec<f32>,
        vocals_right: Vec<f32>,
        instrumental_left: Vec<f32>,
        instrumental_right: Vec<f32>,
    }

    impl StemSink for CaptureSink {
        fn write_stems(&mut self, chunk: StemChunk<'_>) -> Result<()> {
            assert_eq!(chunk.vocals.left.len(), chunk.samples());
            assert_eq!(chunk.vocals.right.len(), chunk.samples());
            assert_eq!(chunk.instrumental.left.len(), chunk.samples());
            assert_eq!(chunk.instrumental.right.len(), chunk.samples());
            self.starts.push(chunk.start_sample);
            self.vocals_left.extend_from_slice(chunk.vocals.left);
            self.vocals_right.extend_from_slice(chunk.vocals.right);
            self.instrumental_left
                .extend_from_slice(chunk.instrumental.left);
            self.instrumental_right
                .extend_from_slice(chunk.instrumental.right);
            Ok(())
        }
    }

    struct IdentityVocalsBackend {
        calls: usize,
        unpadded: usize,
    }

    impl SegmentBackend for IdentityVocalsBackend {
        fn infer_segment(&mut self, mix: &[f32], shape: [usize; 3]) -> Result<SegmentOutput> {
            assert_eq!(shape, [1, CHANNELS as usize, SEGMENT_SAMPLES]);
            assert_eq!(mix.len(), CHANNELS as usize * SEGMENT_SAMPLES);
            for channel in 0..CHANNELS as usize {
                let offset = channel * SEGMENT_SAMPLES;
                assert!(
                    mix[offset + self.unpadded..offset + SEGMENT_SAMPLES]
                        .iter()
                        .all(|sample| *sample == 0.0)
                );
            }
            self.calls += 1;
            let mut data = vec![0.0; STEMS * CHANNELS as usize * SEGMENT_SAMPLES];
            data[..CHANNELS as usize * SEGMENT_SAMPLES].copy_from_slice(mix);
            Ok(SegmentOutput {
                shape: vec![1, STEMS, CHANNELS as usize, SEGMENT_SAMPLES],
                data,
            })
        }
    }

    struct ConstantBackend {
        calls: usize,
        values: Vec<f32>,
    }

    impl SegmentBackend for ConstantBackend {
        fn infer_segment(&mut self, _mix: &[f32], _shape: [usize; 3]) -> Result<SegmentOutput> {
            let value = self.values[self.calls.min(self.values.len() - 1)];
            self.calls += 1;
            Ok(SegmentOutput {
                shape: vec![1, STEMS, CHANNELS as usize, SEGMENT_SAMPLES],
                data: vec![value; STEMS * CHANNELS as usize * SEGMENT_SAMPLES],
            })
        }
    }

    struct CountingSink {
        chunks: Vec<(u64, usize)>,
        samples: usize,
    }

    impl StemSink for CountingSink {
        fn write_stems(&mut self, chunk: StemChunk<'_>) -> Result<()> {
            self.chunks.push((chunk.start_sample, chunk.samples()));
            self.samples += chunk.samples();
            Ok(())
        }
    }

    fn normalization(samples: usize, mean: f32, std: f32) -> TrackNormalization {
        TrackNormalization {
            samples: samples as u64,
            mean,
            std,
        }
    }

    #[test]
    fn final_segment_is_right_zero_padded_and_output_is_cropped() {
        let samples = 127;
        let left = (0..samples)
            .map(|index| index as f32 * 0.01 - 0.25)
            .collect::<Vec<_>>();
        let right = (0..samples)
            .map(|index| 0.5 - index as f32 * 0.002)
            .collect::<Vec<_>>();
        let mut stats = TrackStatsBuilder::new(SAMPLE_RATE, CHANNELS).unwrap();
        stats.push(&left, &right).unwrap();
        let normalization = stats.finish().unwrap();
        let mut source = SliceSource::new(left.clone(), right.clone(), 17);
        let mut backend = IdentityVocalsBackend {
            calls: 0,
            unpadded: samples,
        };
        let mut sink = CaptureSink::default();

        let summary = DemucsAdapter::default()
            .separate(&mut backend, normalization, &mut source, &mut sink)
            .unwrap();

        assert_eq!(backend.calls, 1);
        assert_eq!(summary.samples, samples as u64);
        assert_eq!(summary.segments, 1);
        assert_eq!(sink.starts, [0]);
        assert_eq!(sink.vocals_left.len(), samples);
        for (got, expected) in sink.vocals_left.iter().zip(left) {
            assert!((got - expected).abs() < 2e-6);
        }
        for (got, expected) in sink.vocals_right.iter().zip(right) {
            assert!((got - expected).abs() < 2e-6);
        }
        for got in sink
            .instrumental_left
            .iter()
            .chain(&sink.instrumental_right)
        {
            assert!((*got - 3.0 * normalization.mean()).abs() < 2e-6);
        }
    }

    #[test]
    fn triangular_overlap_add_blends_adjacent_segments_without_a_seam() {
        let samples = SEGMENT_SAMPLES;
        let mut source = SliceSource::new(vec![0.0; samples], vec![0.0; samples], 32_768);
        let mut backend = ConstantBackend {
            calls: 0,
            values: vec![1.0, 3.0],
        };
        let mut sink = CaptureSink::default();

        let summary = DemucsAdapter::default()
            .separate(
                &mut backend,
                normalization(samples, 0.0, 1.0),
                &mut source,
                &mut sink,
            )
            .unwrap();

        assert_eq!(summary.segments, 2);
        assert_eq!(sink.starts, [0, STRIDE_SAMPLES as u64]);
        assert_eq!(sink.vocals_left.len(), samples);
        assert_eq!(sink.vocals_left[STRIDE_SAMPLES - 1], 1.0);
        for index in [
            0,
            1,
            OVERLAP_SAMPLES / 2,
            OVERLAP_SAMPLES - 2,
            OVERLAP_SAMPLES - 1,
        ] {
            let prior_weight = triangular_weight(STRIDE_SAMPLES + index);
            let current_weight = triangular_weight(index);
            let expected = (prior_weight + 3.0 * current_weight) / (prior_weight + current_weight);
            let got = sink.vocals_left[STRIDE_SAMPLES + index];
            assert!(
                (got - expected).abs() < 2e-6,
                "index {index}: {got} != {expected}"
            );
        }
    }

    #[test]
    fn duration_changes_segment_count_but_not_reported_working_buffers() {
        let adapter = DemucsAdapter::new(AdapterOptions {
            read_chunk_samples: 4_093,
        })
        .unwrap();
        let run = |samples: usize| {
            let mut source = SliceSource::new(vec![0.0; samples], vec![0.0; samples], 3_001);
            let mut backend = ConstantBackend {
                calls: 0,
                values: vec![0.0],
            };
            let mut sink = CountingSink {
                chunks: Vec::new(),
                samples: 0,
            };
            let summary = adapter
                .separate(
                    &mut backend,
                    normalization(samples, 0.0, 1.0),
                    &mut source,
                    &mut sink,
                )
                .unwrap();
            (summary, backend.calls, sink)
        };

        let (short, short_calls, short_sink) = run(STRIDE_SAMPLES + 1);
        let (long, long_calls, long_sink) = run(STRIDE_SAMPLES * 3 + 17);
        assert_eq!(short_calls, 2);
        assert_eq!(
            short_sink.chunks,
            [(0, STRIDE_SAMPLES), (STRIDE_SAMPLES as u64, 1)]
        );
        assert_eq!(long_calls, 4);
        assert_eq!(long_sink.samples, STRIDE_SAMPLES * 3 + 17);
        assert_eq!(
            long_sink.chunks.last(),
            Some(&((STRIDE_SAMPLES * 3) as u64, 17))
        );
        assert_eq!(short.max_input_samples, long.max_input_samples);
        assert_eq!(short.max_output_samples, long.max_output_samples);
        assert_eq!(short.max_finalized_samples, long.max_finalized_samples);
        assert_eq!(short.max_input_samples, CHANNELS as usize * SEGMENT_SAMPLES);
        assert_eq!(
            short.max_output_samples,
            STEMS * CHANNELS as usize * SEGMENT_SAMPLES
        );
        assert_eq!(
            short.max_finalized_samples,
            STEMS * CHANNELS as usize * STRIDE_SAMPLES
        );
    }

    #[test]
    fn source_length_must_match_first_pass_statistics() {
        let adapter = DemucsAdapter::default();
        let mut backend = ConstantBackend {
            calls: 0,
            values: vec![0.0],
        };
        let mut sink = CountingSink {
            chunks: Vec::new(),
            samples: 0,
        };
        let mut short = SliceSource::new(vec![0.0; 7], vec![0.0; 7], 7);
        assert!(
            adapter
                .separate(
                    &mut backend,
                    normalization(8, 0.0, 1.0),
                    &mut short,
                    &mut sink,
                )
                .unwrap_err()
                .to_string()
                .contains("ended after")
        );

        let calls = Cell::new(0);
        struct ExtraSource<'a> {
            calls: &'a Cell<usize>,
        }
        impl StereoSource for ExtraSource<'_> {
            fn read_stereo(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<usize> {
                let call = self.calls.get();
                self.calls.set(call + 1);
                let count = if call == 0 { left.len().min(8) } else { 1 };
                left[..count].fill(0.0);
                right[..count].fill(0.0);
                Ok(count)
            }
        }
        let mut extra = ExtraSource { calls: &calls };
        let mut backend = ConstantBackend {
            calls: 0,
            values: vec![0.0],
        };
        assert!(
            adapter
                .separate(
                    &mut backend,
                    normalization(8, 0.0, 1.0),
                    &mut extra,
                    &mut sink,
                )
                .unwrap_err()
                .to_string()
                .contains("more samples")
        );
    }

    #[test]
    fn backend_output_contract_is_enforced() {
        struct BadBackend;
        impl SegmentBackend for BadBackend {
            fn infer_segment(&mut self, _mix: &[f32], _shape: [usize; 3]) -> Result<SegmentOutput> {
                Ok(SegmentOutput {
                    shape: vec![1, 1],
                    data: vec![0.0],
                })
            }
        }
        let mut source = SliceSource::new(vec![0.0; 2], vec![0.0; 2], 2);
        let mut sink = CaptureSink::default();
        assert!(
            DemucsAdapter::default()
                .separate(
                    &mut BadBackend,
                    normalization(2, 0.0, 1.0),
                    &mut source,
                    &mut sink,
                )
                .is_err()
        );
    }
}
