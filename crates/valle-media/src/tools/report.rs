use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    ToolError, ToolErrorCode,
    output::{FileOutputTransaction, paths_refer_to_same_file},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRun<T> {
    pub result: T,
    pub report: ProcessingReport,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<ToolWarning>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputArtifact {
    pub role: String,
    pub path: PathBuf,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<MediaSummary>,
}

/// Alpha semantics of a raster media artifact.
///
/// Audio and strict mask artifacts use [`Self::NotApplicable`]. `Opaque` means the encoded raster
/// has no usable alpha channel, while `Straight` identifies Valle's non-premultiplied RGBA
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlphaMode {
    NotApplicable,
    Opaque,
    Straight,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate_hz: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_format: Option<String>,
    /// Lowercase container family, independent of the output path extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    /// Lowercase codec identifier for a still-image bitstream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_codec: Option<String>,
    /// Lowercase codec identifier for the video stream, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    /// Lowercase codec identifier for the audio stream, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_mode: Option<AlphaMode>,
}

impl MediaSummary {
    /// Attach the encoded representation of a still-image output.
    pub fn with_image_encoding(
        mut self,
        container: &str,
        image_codec: &str,
        pixel_format: &str,
        alpha_mode: AlphaMode,
    ) -> Self {
        self.container = Some(container.to_owned());
        self.image_codec = Some(image_codec.to_owned());
        self.video_codec = None;
        self.audio_codec = None;
        self.pixel_format = Some(pixel_format.to_owned());
        self.alpha_mode = Some(alpha_mode);
        self
    }

    /// Attach the encoded representation of a video output and its optional audio stream.
    pub fn with_video_encoding(
        mut self,
        container: &str,
        video_codec: &str,
        audio_codec: Option<&str>,
        pixel_format: &str,
        alpha_mode: AlphaMode,
    ) -> Self {
        self.container = Some(container.to_owned());
        self.image_codec = None;
        self.video_codec = Some(video_codec.to_owned());
        self.audio_codec = audio_codec.map(str::to_owned);
        self.pixel_format = Some(pixel_format.to_owned());
        self.alpha_mode = Some(alpha_mode);
        self
    }

    /// Attach the encoded representation of an audio-only output.
    pub fn with_audio_encoding(mut self, container: &str, audio_codec: &str) -> Self {
        self.container = Some(container.to_owned());
        self.image_codec = None;
        self.video_codec = None;
        self.audio_codec = Some(audio_codec.to_owned());
        self.pixel_format = None;
        self.alpha_mode = Some(AlphaMode::NotApplicable);
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessedMedia {
    pub frames: u64,
    pub audio_seconds: f64,
}

/// Logical queue capacity and the largest number of simultaneously queued media items observed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineUsage {
    pub capacity: usize,
    pub high_watermark: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingReport {
    pub operation: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, serde_json::Value>,
    pub input: PathBuf,
    pub input_media_type: String,
    pub input_summary: MediaSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<OutputArtifact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelProvenance>,
    pub processed: ProcessedMedia,
    pub timing: StageTiming,
}

/// Source-container facts used by audio-oriented tools when constructing a run report.
///
/// This is deliberately not part of the serialized report schema. The public shape remains
/// [`ProcessingReport`]; this helper only prevents a tool's normalized PCM from being mistaken for
/// the media that the user supplied.
#[cfg(any(
    feature = "tool-enhance",
    feature = "tool-separate",
    all(feature = "tool-transcribe", not(target_os = "windows")),
    all(test, feature = "libav")
))]
pub(crate) struct ProbedAudioInput {
    pub media_type: String,
    pub summary: MediaSummary,
}

/// Probe the original container and its first usable A/V streams without decoding the full file.
#[cfg(any(
    feature = "tool-enhance",
    feature = "tool-separate",
    all(feature = "tool-transcribe", not(target_os = "windows")),
    all(test, feature = "libav")
))]
pub(crate) fn probe_audio_input(path: &Path) -> Result<ProbedAudioInput, ToolError> {
    let details = crate::codec::decode::probe_av_details(path).map_err(|error| {
        ToolError::invalid_input(format!("probe audio input {}: {error:#}", path.display()))
    })?;
    let probe = details.probe;
    if probe.audio.is_none() {
        return Err(ToolError::invalid_input(format!(
            "no audio stream in {}",
            path.display()
        )));
    }

    let has_video = probe.video.is_some();
    let (width, height, video_duration) = probe
        .video
        .map_or((None, None, None), |(width, height, duration)| {
            (Some(width), Some(height), valid_duration(duration))
        });
    let duration_seconds = video_duration.or_else(|| probe.audio.and_then(valid_duration));

    Ok(ProbedAudioInput {
        media_type: if has_video { "video/*" } else { "audio/*" }.to_owned(),
        summary: MediaSummary {
            duration_seconds,
            width,
            height,
            sample_rate_hz: details.audio_sample_rate_hz,
            channels: details.audio_channels,
            sample_format: details.audio_sample_format,
            pixel_format: details.video_pixel_format,
            ..MediaSummary::default()
        },
    })
}

#[cfg(any(
    feature = "tool-enhance",
    feature = "tool-separate",
    all(feature = "tool-transcribe", not(target_os = "windows")),
    all(test, feature = "libav")
))]
fn valid_duration(duration: f64) -> Option<f64> {
    (duration.is_finite() && duration > 0.0).then_some(duration)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageTiming {
    pub load_seconds: f64,
    pub decode_seconds: f64,
    pub preprocess_seconds: f64,
    pub inference_seconds: f64,
    pub postprocess_seconds: f64,
    pub encode_seconds: f64,
    pub total_seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProvenance {
    pub id: String,
    pub version: String,
    pub revision: String,
    pub manifest_path: String,
    pub adapter: String,
    pub artifact: String,
    pub route: String,
    pub backend: String,
    pub precision: String,
}

impl From<&crate::models::ResolvedArtifact> for ModelProvenance {
    fn from(model: &crate::models::ResolvedArtifact) -> Self {
        Self {
            id: model.id.clone(),
            version: model.version.clone(),
            revision: model.revision.clone(),
            manifest_path: model.manifest_path.clone(),
            adapter: format!("{}@{}", model.adapter, model.adapter_version),
            artifact: model.artifact.clone(),
            route: model.route.clone(),
            backend: model.backend.clone(),
            precision: model.precision.clone(),
        }
    }
}

/// Versioned machine envelope shared by CLI JSON output and persisted run reports.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaRunEnvelope<'a, T> {
    pub format: &'static str,
    pub format_version: u32,
    pub status: &'static str,
    pub result: Option<&'a T>,
    pub report: Option<&'a ProcessingReport>,
    pub warnings: &'a [ToolWarning],
    pub error: Option<&'a ToolError>,
}

impl<'a, T> MediaRunEnvelope<'a, T> {
    pub fn success(run: &'a ToolRun<T>) -> Self {
        Self {
            format: "valle.media-run",
            format_version: 1,
            status: "ok",
            result: Some(&run.result),
            report: Some(&run.report),
            warnings: &run.warnings,
            error: None,
        }
    }
}

impl<'a> MediaRunEnvelope<'a, ()> {
    pub fn failure(error: &'a ToolError, warnings: &'a [ToolWarning]) -> Self {
        Self {
            format: "valle.media-run",
            format_version: 1,
            status: "error",
            result: None,
            report: None,
            warnings,
            error: Some(error),
        }
    }
}

/// A report destination prepared before media processing starts.
///
/// The empty staging file proves that the destination is writable without exposing a partial
/// report. After the media output has committed, [`Self::write`] publishes the versioned report.
/// A late report failure is appended to the successful run as a warning and never rolls media
/// output back.
pub struct PreparedRunReport {
    transaction: FileOutputTransaction,
}

impl PreparedRunReport {
    pub fn new(path: &Path, overwrite: bool, protected_paths: &[&Path]) -> Result<Self, ToolError> {
        if protected_paths
            .iter()
            .any(|protected| paths_refer_to_same_file(path, protected))
        {
            return Err(ToolError::invalid_input(format!(
                "report path must differ from every media input and output: {}",
                path.display()
            )));
        }
        let transaction = FileOutputTransaction::new(path, overwrite)?;
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(transaction.staging_path())
            .map_err(|error| {
                ToolError::new(
                    ToolErrorCode::OutputValidationFailed,
                    format!(
                        "prepare report {}: {error}",
                        transaction.staging_path().display()
                    ),
                )
            })?;
        file.sync_all().map_err(|error| {
            ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!("prepare report {}: {error}", path.display()),
            )
        })?;
        Ok(Self { transaction })
    }

    pub fn write<T>(self, run: &mut ToolRun<T>)
    where
        T: Serialize,
    {
        if let Err(error) = self.write_strict(run) {
            run.warnings.push(ToolWarning {
                code: "report_write_failed".to_owned(),
                message: error.to_string(),
            });
        }
    }

    fn write_strict<T>(self, run: &ToolRun<T>) -> Result<(), ToolError>
    where
        T: Serialize,
    {
        let mut file = OpenOptions::new()
            .truncate(true)
            .write(true)
            .open(self.transaction.staging_path())
            .map_err(|error| {
                ToolError::new(
                    ToolErrorCode::OutputValidationFailed,
                    format!(
                        "open report staging {}: {error}",
                        self.transaction.staging_path().display()
                    ),
                )
            })?;
        serde_json::to_writer_pretty(&mut file, &MediaRunEnvelope::success(run)).map_err(
            |error| {
                ToolError::new(
                    ToolErrorCode::OutputValidationFailed,
                    format!("serialize run report: {error}"),
                )
            },
        )?;
        file.write_all(b"\n").map_err(|error| {
            ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!("write run report: {error}"),
            )
        })?;
        file.sync_all().map_err(|error| {
            ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!("flush run report: {error}"),
            )
        })?;
        drop(file);
        self.transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_report_uses_the_versioned_envelope() {
        let root = std::env::temp_dir().join(format!("valle-media-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let target = root.join("run.json");
        let prepared = PreparedRunReport::new(&target, false, &[]).unwrap();
        let mut run = ToolRun {
            result: "done",
            report: ProcessingReport {
                operation: "test".to_owned(),
                parameters: BTreeMap::new(),
                input: PathBuf::from("input"),
                input_media_type: "application/octet-stream".to_owned(),
                input_summary: MediaSummary::default(),
                outputs: Vec::new(),
                models: Vec::new(),
                processed: ProcessedMedia::default(),
                timing: StageTiming::default(),
            },
            warnings: Vec::new(),
        };
        prepared.write(&mut run);
        assert!(run.warnings.is_empty());
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
        assert_eq!(json["format"], "valle.media-run");
        assert_eq!(json["formatVersion"], 1);
        assert_eq!(json["status"], "ok");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn failure_envelope_uses_null_result_and_stable_error_code() {
        let error = ToolError::new(ToolErrorCode::InvalidInput, "bad input")
            .with_context("input", "fixture.wav");
        let json = serde_json::to_value(MediaRunEnvelope::failure(&error, &[])).unwrap();
        assert_eq!(json["format"], "valle.media-run");
        assert_eq!(json["status"], "error");
        assert!(json["result"].is_null());
        assert_eq!(json["error"]["code"], "invalid_input");
        assert_eq!(json["error"]["context"]["input"], "fixture.wav");
    }

    #[test]
    fn media_summary_serializes_independent_video_and_audio_codecs() {
        let summary = MediaSummary {
            duration_seconds: Some(2.0),
            width: Some(64),
            height: Some(48),
            frame_count: Some(8),
            sample_rate_hz: Some(48_000),
            channels: Some(2),
            sample_format: Some("fltp".to_owned()),
            ..MediaSummary::default()
        }
        .with_video_encoding("mp4", "h264", Some("aac"), "yuv420p", AlphaMode::Opaque);

        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["container"], "mp4");
        assert_eq!(json["videoCodec"], "h264");
        assert_eq!(json["audioCodec"], "aac");
        assert_eq!(json["pixelFormat"], "yuv420p");
        assert_eq!(json["alphaMode"], "opaque");
        assert!(json.get("imageCodec").is_none());
        assert_eq!(
            serde_json::from_value::<MediaSummary>(json).unwrap(),
            summary
        );
    }

    #[test]
    fn media_encoding_builders_report_only_real_stream_kinds() {
        let image =
            MediaSummary::default().with_image_encoding("png", "png", "rgba", AlphaMode::Straight);
        assert_eq!(image.image_codec.as_deref(), Some("png"));
        assert_eq!(image.video_codec, None);
        assert_eq!(image.audio_codec, None);

        let silent_video = MediaSummary::default().with_video_encoding(
            "mp4",
            "h264",
            None,
            "yuv420p",
            AlphaMode::Opaque,
        );
        let silent_video_json = serde_json::to_value(silent_video).unwrap();
        assert_eq!(silent_video_json["videoCodec"], "h264");
        assert!(silent_video_json.get("audioCodec").is_none());

        let audio = MediaSummary {
            sample_format: Some("f32".to_owned()),
            ..MediaSummary::default()
        }
        .with_audio_encoding("wav", "pcm_f32le");
        assert_eq!(audio.audio_codec.as_deref(), Some("pcm_f32le"));
        assert_eq!(audio.pixel_format, None);
        assert_eq!(audio.alpha_mode, Some(AlphaMode::NotApplicable));

        let analysis = serde_json::to_value(MediaSummary {
            duration_seconds: Some(1.0),
            ..MediaSummary::default()
        })
        .unwrap();
        assert!(analysis.get("container").is_none());
        assert!(analysis.get("imageCodec").is_none());
        assert!(analysis.get("videoCodec").is_none());
        assert!(analysis.get("audioCodec").is_none());
        assert!(analysis.get("alphaMode").is_none());
    }

    #[test]
    fn alpha_mode_rejects_unrecognized_report_values() {
        assert!(serde_json::from_str::<AlphaMode>(r#""premultiplied""#).is_err());
    }

    #[test]
    fn processing_report_keeps_output_encoding_in_the_versioned_envelope() {
        let output = OutputArtifact {
            role: "interpolated_video".to_owned(),
            path: PathBuf::from("output.mp4"),
            media_type: "video/mp4".to_owned(),
            summary: Some(
                MediaSummary {
                    width: Some(64),
                    height: Some(48),
                    sample_rate_hz: Some(48_000),
                    channels: Some(2),
                    sample_format: Some("fltp".to_owned()),
                    ..MediaSummary::default()
                }
                .with_video_encoding(
                    "mp4",
                    "h264",
                    Some("aac"),
                    "yuv420p",
                    AlphaMode::Opaque,
                ),
            ),
        };
        let run = ToolRun {
            result: (),
            report: ProcessingReport {
                operation: "interpolate".to_owned(),
                parameters: BTreeMap::new(),
                input: PathBuf::from("input.mp4"),
                input_media_type: "video/mp4".to_owned(),
                input_summary: MediaSummary::default(),
                outputs: vec![output],
                models: Vec::new(),
                processed: ProcessedMedia::default(),
                timing: StageTiming::default(),
            },
            warnings: Vec::new(),
        };

        let json = serde_json::to_value(MediaRunEnvelope::success(&run)).unwrap();
        let summary = &json["report"]["outputs"][0]["summary"];
        assert_eq!(summary["container"], "mp4");
        assert_eq!(summary["videoCodec"], "h264");
        assert_eq!(summary["audioCodec"], "aac");
        assert_eq!(summary["pixelFormat"], "yuv420p");
        assert_eq!(summary["alphaMode"], "opaque");
    }

    #[test]
    fn report_cannot_alias_an_input_or_media_output() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.wav");
        std::fs::write(&input, b"input").unwrap();
        assert!(PreparedRunReport::new(&input, true, &[&input]).is_err());

        let output = root.path().join("output.wav");
        let aliased_output = root.path().join(".").join("output.wav");
        assert!(PreparedRunReport::new(&aliased_output, true, &[&output]).is_err());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let alias = root.path().join("input-alias.wav");
            symlink(&input, &alias).unwrap();
            assert!(PreparedRunReport::new(&alias, true, &[&input]).is_err());

            let real_directory = root.path().join("real");
            let alias_directory = root.path().join("alias");
            std::fs::create_dir(&real_directory).unwrap();
            symlink(&real_directory, &alias_directory).unwrap();
            let real_output = real_directory.join("foreground.mov");
            let aliased_report = alias_directory.join("foreground.mov");
            assert!(PreparedRunReport::new(&aliased_report, true, &[&real_output]).is_err());
        }
    }

    #[test]
    fn late_report_failure_is_a_warning_and_does_not_change_the_run_result() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("run.json");
        let prepared = PreparedRunReport::new(&target, false, &[]).unwrap();
        std::fs::remove_file(prepared.transaction.staging_path()).unwrap();
        let mut run = ToolRun {
            result: "media already committed",
            report: ProcessingReport {
                operation: "test".to_owned(),
                parameters: BTreeMap::new(),
                input: PathBuf::from("input"),
                input_media_type: "application/octet-stream".to_owned(),
                input_summary: MediaSummary::default(),
                outputs: Vec::new(),
                models: Vec::new(),
                processed: ProcessedMedia::default(),
                timing: StageTiming::default(),
            },
            warnings: Vec::new(),
        };
        prepared.write(&mut run);
        assert_eq!(run.result, "media already committed");
        assert_eq!(run.warnings.len(), 1);
        assert_eq!(run.warnings[0].code, "report_write_failed");
        assert!(!target.exists());
    }

    #[cfg(feature = "libav")]
    #[test]
    fn audio_input_probe_reports_original_wav_sampling() {
        use crate::{codec::FloatWavWriter, frame::AudioBuffer};

        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("source.wav");
        let sample_rate = 22_050;
        let frames = 2_205;
        let mut writer = FloatWavWriter::create(&input, sample_rate, 1).unwrap();
        writer
            .write(&AudioBuffer {
                samples: vec![0.125; frames],
                sample_rate,
                channels: 1,
            })
            .unwrap();
        writer.finish().unwrap();

        let report = probe_audio_input(&input).unwrap();
        assert_eq!(report.media_type, "audio/*");
        assert_eq!(report.summary.width, None);
        assert_eq!(report.summary.height, None);
        assert_eq!(report.summary.sample_rate_hz, Some(sample_rate));
        assert_eq!(report.summary.channels, Some(1));
        assert_eq!(report.summary.sample_format.as_deref(), Some("f32"));
        assert!(
            (report.summary.duration_seconds.unwrap() - 0.1).abs() < 1e-6,
            "WAV duration must describe the source stream"
        );
    }

    #[cfg(feature = "libav")]
    #[test]
    fn audio_input_probe_reports_video_container_and_original_streams() {
        use crate::{codec::Muxer, frame::RgbaFrame};

        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("source.mp4");
        let (width, height) = (64, 48);
        let mut muxer = Muxer::open(&input, width, height, 30, None).unwrap();
        for shade in [32, 96, 160] {
            let mut frame = RgbaFrame::new(width, height);
            frame.fill([shade, 24, 180, 255]);
            muxer.encode_video(&frame).unwrap();
        }
        muxer.finish().unwrap();

        let report = probe_audio_input(&input).unwrap();
        assert_eq!(report.media_type, "video/*");
        assert_eq!(report.summary.width, Some(width));
        assert_eq!(report.summary.height, Some(height));
        assert_eq!(report.summary.sample_rate_hz, Some(48_000));
        assert_eq!(report.summary.channels, Some(2));
        assert_eq!(report.summary.sample_format.as_deref(), Some("f32p"));
        assert_eq!(report.summary.pixel_format.as_deref(), Some("yuv420p"));
        assert!(report.summary.duration_seconds.is_some());
    }
}
