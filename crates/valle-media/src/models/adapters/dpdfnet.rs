//! Product adapter for DPDFNet's load-once 48 kHz mono PCM runtime.

use crate::models::inference::dpdfnet::{DpdfNetSession, SessionLoadRequest};
use anyhow::{Context, Result, ensure};

use crate::{frame::AudioBuffer, models::manager::ResolvedModel};

pub(crate) const SAMPLE_RATE: u32 = crate::models::inference::dpdfnet::SAMPLE_RATE;
pub(crate) const CHANNELS: u16 = 1;
pub(crate) const MODEL_ID: &str = "dpdfnet";

pub(crate) struct EnhancementSession {
    inner: DpdfNetSession,
}

impl EnhancementSession {
    pub(crate) fn open(model: &ResolvedModel, cpu_threads: usize) -> Result<Self> {
        ensure!(cpu_threads > 0, "DPDFNet cpu_threads must be positive");
        let mut route = model.route.clone();
        apply_cpu_threads(&mut route.options, cpu_threads)?;
        let inner = DpdfNetSession::load(SessionLoadRequest {
            manifest: &model.manifest,
            route: &route,
            artifact: &model.artifact,
            artifact_root: &model.resolved.root,
        })
        .with_context(|| {
            format!(
                "load DPDFNet artifact {} from {}",
                model.resolved.artifact,
                model.resolved.root.display()
            )
        })?;
        Ok(Self { inner })
    }

    pub(crate) fn start_stream(&mut self) -> EnhancementStream<'_> {
        EnhancementStream {
            inner: self.inner.start_stream(),
        }
    }
}

fn apply_cpu_threads(options: &mut serde_json::Value, cpu_threads: usize) -> Result<()> {
    if options.is_null() {
        *options = serde_json::json!({});
    }
    options
        .as_object_mut()
        .context("DPDFNet ONNX route options must be an object")?
        .insert("intra_threads".to_owned(), cpu_threads.into());
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EnhancementStats {
    pub(crate) input_samples: u64,
    pub(crate) output_samples: u64,
    pub(crate) processing_seconds: f64,
    pub(crate) inference_seconds: f64,
    pub(crate) dsp_seconds: f64,
    pub(crate) real_time_factor: f64,
    pub(crate) real_time_multiple: f64,
}

pub(crate) struct FinishedEnhancement {
    pub(crate) audio: AudioBuffer,
    pub(crate) stats: EnhancementStats,
}

/// Narrow backend-neutral seam used by the file workflow and its fake-backend tests.
pub(crate) trait PcmEnhancementStream {
    fn push(&mut self, audio: &AudioBuffer) -> Result<AudioBuffer>;
    fn finish(self) -> Result<FinishedEnhancement>;
    fn working_set_bytes(&self) -> usize;
}

pub(crate) struct EnhancementStream<'session> {
    inner: crate::models::inference::dpdfnet::DpdfNetStream<'session>,
}

impl PcmEnhancementStream for EnhancementStream<'_> {
    fn push(&mut self, audio: &AudioBuffer) -> Result<AudioBuffer> {
        validate_pcm(audio)?;
        let samples = self.inner.push(&audio.samples)?;
        ensure!(
            samples.iter().all(|sample| sample.is_finite()),
            "DPDFNet returned non-finite PCM"
        );
        Ok(AudioBuffer {
            samples,
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
        })
    }

    fn finish(self) -> Result<FinishedEnhancement> {
        let finished = self.inner.finish()?;
        ensure!(
            finished.samples.iter().all(|sample| sample.is_finite()),
            "DPDFNet returned non-finite tail PCM"
        );
        Ok(FinishedEnhancement {
            audio: AudioBuffer {
                samples: finished.samples,
                sample_rate: SAMPLE_RATE,
                channels: CHANNELS,
            },
            stats: EnhancementStats {
                input_samples: finished.stats.input_samples,
                output_samples: finished.stats.output_samples,
                processing_seconds: finished.stats.processing_ms / 1_000.0,
                inference_seconds: finished.stats.inference_ms / 1_000.0,
                dsp_seconds: finished.stats.dsp_ms / 1_000.0,
                real_time_factor: finished.stats.real_time_factor,
                real_time_multiple: finished.stats.real_time_multiple,
            },
        })
    }

    fn working_set_bytes(&self) -> usize {
        self.inner.working_set().heap_capacity_bytes
    }
}

fn validate_pcm(audio: &AudioBuffer) -> Result<()> {
    ensure!(
        audio.sample_rate == SAMPLE_RATE,
        "DPDFNet expects {SAMPLE_RATE} Hz PCM, got {} Hz",
        audio.sample_rate
    );
    ensure!(
        audio.channels == CHANNELS,
        "DPDFNet expects mono PCM, got {} channels",
        audio.channels
    );
    ensure!(
        audio.samples.iter().all(|sample| sample.is_finite()),
        "DPDFNet PCM contains non-finite samples"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_cpu_threads_override_the_release_default() {
        let mut options = serde_json::json!({
            "intra_threads": 1,
            "memory_pattern": true,
        });
        apply_cpu_threads(&mut options, 6).unwrap();
        assert_eq!(options["intra_threads"], 6);
        assert_eq!(options["memory_pattern"], true);

        let mut empty = serde_json::Value::Null;
        apply_cpu_threads(&mut empty, 3).unwrap();
        assert_eq!(empty["intra_threads"], 3);

        let mut invalid = serde_json::json!([]);
        assert!(apply_cpu_threads(&mut invalid, 2).is_err());
    }

    #[test]
    fn pcm_boundary_rejects_wrong_rate_channels_and_non_finite_samples() {
        assert!(validate_pcm(&AudioBuffer::silence(SAMPLE_RATE, CHANNELS, 1)).is_ok());
        assert!(validate_pcm(&AudioBuffer::silence(44_100, CHANNELS, 1)).is_err());
        assert!(validate_pcm(&AudioBuffer::silence(SAMPLE_RATE, 2, 1)).is_err());
        assert!(
            validate_pcm(&AudioBuffer {
                samples: vec![f32::NAN],
                sample_rate: SAMPLE_RATE,
                channels: CHANNELS,
            })
            .is_err()
        );
    }
}
