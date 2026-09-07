//! Product bridge for the shared Demucs streaming adapter.

use crate::models::inference::demucs::{DemucsSession, SessionOptions};
use anyhow::Result;

use crate::models::ResolvedModel;

pub(crate) use crate::models::inference::demucs::{
    CHANNELS, SAMPLE_RATE, SeparationReport, StemChunk, StemSink, StereoSource, TrackStatsBuilder,
};

pub(crate) const MODEL_ID: &str = "demucs";

/// One load-once model session. File decoding and output publication stay in `tools::separate`.
pub(crate) struct SeparationSession {
    session: DemucsSession,
}

impl SeparationSession {
    pub(crate) fn open(model: &ResolvedModel, cpu_threads: usize) -> Result<Self> {
        let session = DemucsSession::load(
            &model.manifest,
            &model.route,
            &model.artifact,
            &model.resolved.root,
            &SessionOptions {
                intra_threads: Some(cpu_threads),
                ..SessionOptions::default()
            },
        )?;
        Ok(Self { session })
    }

    pub(crate) fn separate(
        &mut self,
        normalization: crate::models::inference::demucs::TrackNormalization,
        source: &mut impl StereoSource,
        sink: &mut impl StemSink,
    ) -> Result<SeparationReport> {
        self.session.separate_track(normalization, source, sink)
    }
}
