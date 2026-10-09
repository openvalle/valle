//! Execute the compact audio program without materializing per-sample object trees.

use super::*;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSourceRequirement {
    pub source_index: u32,
    pub digest: ContentDigest,
    pub source_channels: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct AudioMixPoint {
    pub output: usize,
    pub track_order: u32,
    pub endpoint_index: usize,
    pub source_index: u32,
    pub source_sample_index: i64,
    pub left_gain: f64,
    pub right_gain: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum AudioPcmMixError {
    #[error(transparent)]
    Runtime(#[from] RuntimeFault),
    #[error("audio source {source_index} has no decoded PCM")]
    MissingSource { source_index: u32 },
    #[error("audio source {source_index} sample {sample} is outside decoded PCM")]
    SourceSampleOutOfRange { source_index: u32, sample: i64 },
    #[error("audio source {source_index} contains non-finite PCM")]
    NonFinitePcm { source_index: u32 },
    #[error("audio block exceeds addressable memory")]
    Budget,
}

impl CompiledAudioProgram {
    fn validate_mix_range(&self, range: SampleRange) -> Result<(), RuntimeFault> {
        if range.start < 0 || range.end < range.start || range.end > self.sample_count {
            return Err(RuntimeFault::SampleRangeOutOfRange {
                start: range.start,
                end: range.end,
                limit: self.sample_count,
            });
        }
        Ok(())
    }

    /// One resource record per active source, independent of the number of output samples.
    pub fn source_requirements(
        &self,
        range: SampleRange,
    ) -> Result<Vec<AudioSourceRequirement>, RuntimeFault> {
        self.validate_mix_range(range)?;
        let overlaps = |other: SampleRange| range.start < other.end && other.start < range.end;
        let mut indices = BTreeSet::new();
        if range.start == range.end {
            return Ok(Vec::new());
        }
        for track in &self.tracks {
            for item in &track.items {
                match item {
                    CompiledAudioItem::Clip { clip } if overlaps(clip.range) => {
                        indices.insert(clip.source);
                    }
                    CompiledAudioItem::Crossfade { crossfade } if overlaps(crossfade.window) => {
                        indices.insert(crossfade.from_source);
                        indices.insert(crossfade.to_source);
                    }
                    _ => {}
                }
            }
        }
        Ok(indices
            .into_iter()
            .map(|source_index| {
                let source = self
                    .sources
                    .source(source_index)
                    .expect("admitted audio source");
                let resource = &self.resources.resources
                    [source.resource_target.expect("audio resource") as usize];
                let source_channels = match source.audio_channel_map().expect("audio channel map") {
                    CompiledAudioChannelMap::MonoToStereo => 1,
                    CompiledAudioChannelMap::StereoIdentity => 2,
                };
                AudioSourceRequirement {
                    source_index,
                    digest: resource.digest,
                    source_channels,
                }
            })
            .collect())
    }

    /// Visit numeric mix points in stable track/endpoint order. Nothing is retained per sample.
    pub fn visit_mix_points<E: From<RuntimeFault>>(
        &self,
        range: SampleRange,
        mut visit: impl FnMut(AudioMixPoint) -> Result<(), E>,
    ) -> Result<(), E> {
        self.validate_mix_range(range)?;
        for sample in range.start..range.end {
            evaluate::visit_audio_endpoints(
                self,
                sample,
                |track_order, endpoint_index, clip, time, crossfade| {
                    let endpoint =
                        evaluate::evaluate_audio_mix_endpoint(self, clip, time, crossfade)?;
                    visit(AudioMixPoint {
                        output: (sample - range.start) as usize,
                        track_order,
                        endpoint_index,
                        source_index: clip.source,
                        source_sample_index: endpoint.source_sample_index,
                        left_gain: endpoint.left_gain,
                        right_gain: endpoint.right_gain,
                    })
                },
            )?;
        }
        Ok(())
    }

    /// Mix directly into interleaved stereo PCM. The host supplies decoded source samples only;
    /// time mapping, curves, effects, pan and crossfades stay in the shared engine.
    pub fn mix_pcm(
        &self,
        range: SampleRange,
        mut read: impl FnMut(u32, i64) -> Result<[f32; 2], AudioPcmMixError>,
    ) -> Result<Vec<f32>, AudioPcmMixError> {
        self.validate_mix_range(range)?;
        let frames = usize::try_from(range.len()).map_err(|_| AudioPcmMixError::Budget)?;
        let mut output = vec![0.0_f64; frames.checked_mul(2).ok_or(AudioPcmMixError::Budget)?];
        self.visit_mix_points::<AudioPcmMixError>(range, |point| {
            let [left, right] = read(point.source_index, point.source_sample_index)?;
            if !left.is_finite() || !right.is_finite() {
                return Err(AudioPcmMixError::NonFinitePcm {
                    source_index: point.source_index,
                });
            }
            output[point.output * 2] += f64::from(left) * point.left_gain;
            output[point.output * 2 + 1] += f64::from(right) * point.right_gain;
            Ok(())
        })?;
        Ok(output
            .into_iter()
            .map(|value| value.clamp(-1.0, 1.0) as f32)
            .collect())
    }
}
