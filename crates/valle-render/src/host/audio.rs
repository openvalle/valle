//! Fixed-package PCM mixing from [`CompiledAudioProgram`].

use std::{collections::BTreeMap, sync::Arc};

use thiserror::Error;
use valle_engine::{
    render::{
        CanvasClockError, CompiledAudioChannelMap, CompiledRender, ResourceKind, RuntimeFault,
        SampleRange,
    },
    resource::ContentDigest,
};
use valle_media::{
    codec::{AudioSource, LibavAudioSource},
    frame::AudioBuffer,
};

use super::{NativeResourceCatalog, NativeResourceSource};

#[derive(Debug, Clone, Copy)]
struct MixPoint {
    output: usize,
    source_sample_index: i64,
    left_gain: f64,
    right_gain: f64,
}

/// Streaming mixer pinned to the same immutable render and fulfillment catalog as video.
pub struct CompiledAudioMixer {
    render: Arc<CompiledRender>,
    catalog: Arc<NativeResourceCatalog>,
    live: BTreeMap<u32, Box<dyn AudioSource>>,
    sample_rate: u32,
    channels: u16,
    cursor: i64,
}

impl CompiledAudioMixer {
    pub fn new(
        render: Arc<CompiledRender>,
        catalog: Arc<NativeResourceCatalog>,
        sample_rate: u32,
        channels: u16,
        start_frame_boundary: i64,
    ) -> Result<Self, AudioMixError> {
        let canvas = render.canvas();
        if sample_rate != canvas.sample_rate() || channels != 2 {
            return Err(AudioMixError::UnsupportedOutputFormat {
                expected_rate: canvas.sample_rate(),
                actual_rate: sample_rate,
                channels,
            });
        }
        let cursor = canvas.sample_boundary_at_frame(start_frame_boundary)?;
        Ok(Self {
            render,
            catalog,
            live: BTreeMap::new(),
            sample_rate,
            channels,
            cursor,
        })
    }

    pub fn mix_until_frame_boundary(
        &mut self,
        frame_boundary: i64,
    ) -> Result<Option<AudioBuffer>, AudioMixError> {
        let target = self
            .render
            .canvas()
            .sample_boundary_at_frame(frame_boundary)?;
        self.mix_until_sample(target)
    }

    pub fn mix_until_sample(&mut self, target: i64) -> Result<Option<AudioBuffer>, AudioMixError> {
        if target < 0 || target > self.render.canvas().sample_count() {
            return Err(AudioMixError::SampleBoundaryOutOfRange {
                sample: target,
                sample_count: self.render.canvas().sample_count(),
            });
        }
        if target <= self.cursor {
            return Ok(None);
        }
        let range = SampleRange::new(self.cursor, target)?;
        let block = self.render.audio().evaluate_block(range)?;
        let frames = usize::try_from(range.len()).map_err(|_| AudioMixError::Budget)?;
        // The common audio ABI accumulates in f64 in stable track/endpoint/source order and
        // quantizes exactly once after the terminal clamp. Decoder PCM remains f32, but no
        // endpoint gain or partial sum is rounded back to f32 along the way.
        let mut mixed = vec![0.0_f64; frames.checked_mul(2).ok_or(AudioMixError::Budget)?];
        let mut lanes = BTreeMap::<(u32, usize, u32), (ContentDigest, Vec<MixPoint>)>::new();

        for (output, sample) in block.samples().iter().enumerate() {
            for track in sample.tracks() {
                for (endpoint_index, endpoint) in track.endpoints().iter().enumerate() {
                    let resource = endpoint.source().resource().ok_or(
                        AudioMixError::MissingCompiledResource {
                            source_index: endpoint.source_index(),
                        },
                    )?;
                    if resource.kind() != ResourceKind::Audio {
                        return Err(AudioMixError::WrongResourceKind {
                            source_index: endpoint.source_index(),
                        });
                    }
                    let digest = *resource.digest();
                    lanes
                        .entry((track.track_order(), endpoint_index, endpoint.source_index()))
                        .or_insert_with(|| (digest, Vec::new()))
                        .1
                        .push(MixPoint {
                            output,
                            source_sample_index: endpoint.source_sample_index(),
                            left_gain: endpoint.left_gain(),
                            right_gain: endpoint.right_gain(),
                        });
                }
            }
        }

        for ((_, _, source_index), (digest, points)) in lanes {
            let channel_map = self
                .render
                .sources()
                .source(source_index)
                .and_then(|source| source.audio_channel_map())
                .ok_or(AudioMixError::MissingCompiledChannelMap { source_index })?;
            let expected_channels = match channel_map {
                CompiledAudioChannelMap::StereoIdentity => 2,
                CompiledAudioChannelMap::MonoToStereo => 1,
            };
            self.ensure_decoder(source_index, &digest, expected_channels)?;
            let decoder = self
                .live
                .get_mut(&source_index)
                .expect("decoder was inserted");
            for run in contiguous_runs(&points) {
                let start_sample = run[0].source_sample_index;
                let end_sample = run
                    .last()
                    .expect("run is non-empty")
                    .source_sample_index
                    .checked_add(1)
                    .ok_or(AudioMixError::Budget)?;
                let requested_frames = usize::try_from(end_sample - start_sample)
                    .map_err(|_| AudioMixError::Budget)?;
                let decoded =
                    decoder
                        .samples_by_index(start_sample, end_sample)
                        .map_err(|error| AudioMixError::Decode {
                            source_index,
                            reason: error.to_string(),
                        })?;
                if decoded.frames() != requested_frames {
                    return Err(AudioMixError::DecodedRangeMismatch {
                        source_index,
                        requested_frames,
                        decoded_frames: decoded.frames(),
                    });
                }
                if decoded.channels != expected_channels {
                    return Err(AudioMixError::DecodedChannelLayoutMismatch {
                        source_index,
                        expected_channels,
                        actual_channels: decoded.channels,
                    });
                }
                let decoded_channels = usize::from(decoded.channels);
                for point in run {
                    let source_frame = usize::try_from(point.source_sample_index - start_sample)
                        .map_err(|_| AudioMixError::Budget)?;
                    let left = f64::from(decoded.samples[source_frame * decoded_channels]);
                    let right = match channel_map {
                        CompiledAudioChannelMap::StereoIdentity => {
                            f64::from(decoded.samples[source_frame * decoded_channels + 1])
                        }
                        // The common ABI owns mono expansion. Neither libswresample nor
                        // WebAudio may choose an implementation-specific mixing coefficient.
                        CompiledAudioChannelMap::MonoToStereo => left,
                    };
                    // Decoder output is still an input boundary. Check only the samples being
                    // mixed, without decoding or hashing the rest of the track up front.
                    if !left.is_finite() || !right.is_finite() {
                        return Err(AudioMixError::NonFiniteDecodedPcm { source_index });
                    }
                    mixed[point.output * 2] += left * point.left_gain;
                    mixed[point.output * 2 + 1] += right * point.right_gain;
                }
            }
        }

        let mixed = mixed
            .into_iter()
            .map(|sample| sample.clamp(-1.0, 1.0) as f32)
            .collect();
        self.cursor = target;
        Ok(Some(AudioBuffer {
            samples: mixed,
            sample_rate: self.sample_rate,
            channels: self.channels,
        }))
    }

    fn ensure_decoder(
        &mut self,
        source_index: u32,
        digest: &ContentDigest,
        decoded_channels: u16,
    ) -> Result<(), AudioMixError> {
        if self.live.contains_key(&source_index) {
            return Ok(());
        }
        let path = match self.catalog.source(digest) {
            Some(NativeResourceSource::File(path)) => path.clone(),
            Some(NativeResourceSource::Bytes(_)) => {
                return Err(AudioMixError::InMemoryAudio { source_index });
            }
            None => return Err(AudioMixError::MissingSource { source_index }),
        };
        let decoder =
            LibavAudioSource::open(&path, self.sample_rate, decoded_channels).map_err(|error| {
                AudioMixError::Decode {
                    source_index,
                    reason: error.to_string(),
                }
            })?;
        self.live.insert(source_index, Box::new(decoder));
        Ok(())
    }
}

fn contiguous_runs(points: &[MixPoint]) -> Vec<&[MixPoint]> {
    if points.is_empty() {
        return Vec::new();
    }
    let mut runs = Vec::new();
    let mut start = 0;
    for index in 1..points.len() {
        let previous = points[index - 1];
        let current = points[index];
        if current.output != previous.output + 1
            || current.source_sample_index != previous.source_sample_index + 1
        {
            runs.push(&points[start..index]);
            start = index;
        }
    }
    runs.push(&points[start..]);
    runs
}

#[derive(Debug, Error)]
pub enum AudioMixError {
    #[error(
        "Native PCM requires {expected_rate} Hz stereo, got {actual_rate} Hz/{channels} channels"
    )]
    UnsupportedOutputFormat {
        expected_rate: u32,
        actual_rate: u32,
        channels: u16,
    },
    #[error("compiled audio source {source_index} has no frozen resource")]
    MissingCompiledResource { source_index: u32 },
    #[error("compiled source {source_index} did not resolve to audio")]
    WrongResourceKind { source_index: u32 },
    #[error("compiled audio source {source_index} has no frozen channel map")]
    MissingCompiledChannelMap { source_index: u32 },
    #[error("audio source {source_index} is absent from the Native fulfillment catalog")]
    MissingSource { source_index: u32 },
    #[error("in-memory audio source {source_index} is not supported by the Native decoder")]
    InMemoryAudio { source_index: u32 },
    #[error("audio source {source_index} decode failed: {reason}")]
    Decode { source_index: u32, reason: String },
    #[error(
        "audio source {source_index} returned {decoded_frames} frames for an exact {requested_frames}-frame request"
    )]
    DecodedRangeMismatch {
        source_index: u32,
        requested_frames: usize,
        decoded_frames: usize,
    },
    #[error(
        "audio source {source_index} decoded to {actual_channels} channels; compiled map requires {expected_channels}"
    )]
    DecodedChannelLayoutMismatch {
        source_index: u32,
        expected_channels: u16,
        actual_channels: u16,
    },
    #[error("audio source {source_index} decoded non-finite PCM")]
    NonFiniteDecodedPcm { source_index: u32 },
    #[error("audio chunk exceeds addressable memory")]
    Budget,
    #[error("audio sample boundary {sample} is outside [0, {sample_count}]")]
    SampleBoundaryOutOfRange { sample: i64, sample_count: i64 },
    #[error(transparent)]
    Runtime(#[from] RuntimeFault),
    #[error(transparent)]
    Clock(#[from] CanvasClockError),
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::{Value, json};
    use valle_compiler::timeline_contract::{
        ResourceEntryWire, ResourceManifest, canonical_bytes, decode_canonical,
        decode_resource_manifest,
    };
    use valle_engine::render::{
        AUDIO_GAIN_EFFECT_ABI, AUDIO_GAIN_EFFECT_KIND, AudioFootprint, Capabilities,
        CompiledAudioItem, ExtensionKernelCapability, ResourceBinding, ResourceBindings,
        VerifiedHandleId, VerifiedResourceFacts, engine_owned_kernel_implementation_digest,
    };
    use valle_engine::{
        fixed_package::{
            COMMON_PROFILE_KEY, canonical_fixed_execution_profile,
            canonical_fixed_package_manifest, canonical_verified_binding_bundle,
            fixed_package_files, open_verified_fixed_package,
        },
        product::EngineRender,
    };

    use super::*;

    const LEFT_DIGEST: &str =
        "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const RIGHT_DIGEST: &str =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222";

    #[derive(Debug)]
    struct MemoryAudioSource {
        samples: Arc<[f32]>,
        sample_rate: u32,
        channels: u16,
        requests: Arc<Mutex<Vec<(i64, i64)>>>,
    }

    impl AudioSource for MemoryAudioSource {
        fn samples(&mut self, _t: f64, _dt: f64) -> anyhow::Result<AudioBuffer> {
            anyhow::bail!("compiled mixer must use exact source sample indices")
        }

        fn samples_by_index(
            &mut self,
            start_sample: i64,
            end_sample: i64,
        ) -> anyhow::Result<AudioBuffer> {
            let frame_count = self.samples.len() / usize::from(self.channels);
            anyhow::ensure!(
                start_sample >= 0
                    && end_sample >= start_sample
                    && usize::try_from(end_sample).is_ok_and(|end| end <= frame_count),
                "memory PCM request [{start_sample}, {end_sample}) is out of range"
            );
            self.requests
                .lock()
                .expect("request log mutex poisoned")
                .push((start_sample, end_sample));
            let start = usize::try_from(start_sample)? * usize::from(self.channels);
            let end = usize::try_from(end_sample)? * usize::from(self.channels);
            Ok(AudioBuffer {
                samples: self.samples[start..end].to_vec(),
                sample_rate: self.sample_rate,
                channels: self.channels,
            })
        }
    }

    fn constant(value: Value) -> Value {
        json!({"type": "constant", "value": value})
    }

    fn audio_render(source_channel_layout: &str) -> EngineRender {
        let timeline = decode_canonical(
            &serde_json::to_string(&json!({
                "document": {
                    "canvas": {
                        "width": 2,
                        "height": 2,
                        "fps": "2/1",
                        "sampleRate": 4,
                        "channelLayout": "stereo",
                        "colorSpace": "srgb",
                        "duration": "2/1"
                    },
                    "background": {"color": "#000000ff"},
                    "visual": {"tracks": []},
                    "audio": {"tracks": [{
                        "id": "audio:main",
                        "items": [
                            {
                                "type": "clip",
                                "id": "audio:left",
                                "duration": "1/1",
                                "source": {
                                    "type": "media",
                                    "resource": "audio:left",
                                    "sourceStart": "0/1",
                                    "rate": "1/1",
                                    "endBehavior": "hold"
                                },
                                "gain": constant(json!(0.7)),
                                "pan": constant(json!(-0.25)),
                                "effects": [
                                    {
                                        "id": "audio-effect:left-a",
                                        "type": AUDIO_GAIN_EFFECT_KIND,
                                        "parameters": {"multiplier": 0.1}
                                    },
                                    {
                                        "id": "audio-effect:left-b",
                                        "type": AUDIO_GAIN_EFFECT_KIND,
                                        "parameters": {"multiplier": 0.2}
                                    },
                                    {
                                        "id": "audio-effect:left-c",
                                        "type": AUDIO_GAIN_EFFECT_KIND,
                                        "parameters": {"multiplier": 0.3}
                                    }
                                ]
                            },
                            {
                                "type": "crossfade",
                                "id": "crossfade:cut",
                                "duration": "3/4"
                            },
                            {
                                "type": "clip",
                                "id": "audio:right",
                                "duration": "1/1",
                                "source": {
                                    "type": "media",
                                    "resource": "audio:right",
                                    "sourceStart": "0/1",
                                    "rate": "1/1",
                                    "endBehavior": "hold"
                                },
                                "gain": constant(json!(0.6)),
                                "pan": constant(json!(0.4)),
                                "effects": [{
                                    "id": "audio-effect:right",
                                    "type": AUDIO_GAIN_EFFECT_KIND,
                                    "parameters": {"multiplier": 0.3}
                                }]
                            }
                        ]
                    }]},
                    "adjustments": [],
                    "captions": {"tracks": []},
                    "camera": null,
                    "metadata": {}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let manifest = decode_resource_manifest(
            &serde_json::to_vec(&json!({
                "entries": {
                    "audio:left": audio_entry(LEFT_DIGEST, source_channel_layout),
                    "audio:right": audio_entry(RIGHT_DIGEST, source_channel_layout)
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let bindings = audio_bindings(&manifest);
        let capabilities = Capabilities::new()
            .with_extension_kernel(
                AUDIO_GAIN_EFFECT_KIND,
                ExtensionKernelCapability::new(
                    AUDIO_GAIN_EFFECT_ABI,
                    engine_owned_kernel_implementation_digest(AUDIO_GAIN_EFFECT_ABI).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        let timeline_json = String::from_utf8(canonical_bytes(&timeline).unwrap()).unwrap();
        let manifest_json = std::str::from_utf8(manifest.canonical_bytes()).unwrap();
        let bundle_json = canonical_verified_binding_bundle(&bindings, &capabilities).unwrap();
        let profile_json = canonical_fixed_execution_profile(COMMON_PROFILE_KEY).unwrap();
        let files = fixed_package_files(&timeline_json, manifest_json, &bundle_json, &profile_json);
        let package_manifest = canonical_fixed_package_manifest(&files).unwrap();
        open_verified_fixed_package(&package_manifest, &files)
            .unwrap()
            .engine_render()
    }

    fn audio_entry(digest: &str, channel_layout: &str) -> Value {
        json!({
            "kind": "audio",
            "digest": digest,
            "descriptor": {
                "duration": "2/1",
                "timeBase": "1/4",
                "presentationIndexDigest": digest,
                "sampleRate": 4,
                "channelLayout": channel_layout,
                "audioStream": 0
            }
        })
    }

    fn audio_bindings(manifest: &ResourceManifest) -> ResourceBindings {
        let mut bindings = ResourceBindings::new();
        for (handle, resource_id) in [(1, "audio:left"), (2, "audio:right")] {
            let ResourceEntryWire::Audio {
                digest, descriptor, ..
            } = manifest.entries().get(resource_id).unwrap()
            else {
                unreachable!()
            };
            bindings = bindings
                .with_binding(
                    resource_id,
                    ResourceBinding::new(
                        digest.clone(),
                        VerifiedHandleId::new(handle).unwrap(),
                        VerifiedResourceFacts::Audio {
                            descriptor: descriptor.clone(),
                            temporal_footprint: AudioFootprint::default(),
                        },
                    ),
                )
                .unwrap();
        }
        bindings
    }

    fn pcm_sources(channels: u16) -> (Arc<[f32]>, Arc<[f32]>) {
        match channels {
            1 => (
                Arc::from(vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8].into_boxed_slice()),
                Arc::from(
                    vec![-0.15, -0.25, -0.35, -0.45, -0.55, -0.65, -0.75, -0.85].into_boxed_slice(),
                ),
            ),
            2 => (
                Arc::from(
                    vec![
                        0.1, -0.2, 0.2, -0.3, 0.3, -0.4, 0.4, -0.5, 0.5, -0.6, 0.6, -0.7, 0.7,
                        -0.8, 0.8, -0.9,
                    ]
                    .into_boxed_slice(),
                ),
                Arc::from(
                    vec![
                        -0.15, 0.25, -0.25, 0.35, -0.35, 0.45, -0.45, 0.55, -0.55, 0.65, -0.65,
                        0.75, -0.75, 0.85, -0.85, 0.95,
                    ]
                    .into_boxed_slice(),
                ),
            ),
            _ => unreachable!("test PCM is mono or stereo"),
        }
    }

    type RequestLog = Arc<Mutex<Vec<(i64, i64)>>>;

    fn mixer_with_memory_pcm(
        render: Arc<CompiledRender>,
        start_frame_boundary: i64,
    ) -> (CompiledAudioMixer, RequestLog, RequestLog) {
        let source_channels = match render
            .sources()
            .source(0)
            .and_then(|source| source.audio_channel_map())
            .unwrap()
        {
            CompiledAudioChannelMap::StereoIdentity => 2,
            CompiledAudioChannelMap::MonoToStereo => 1,
        };
        let (left, right) = pcm_sources(source_channels);
        let left_requests = Arc::new(Mutex::new(Vec::new()));
        let right_requests = Arc::new(Mutex::new(Vec::new()));
        let mut mixer = CompiledAudioMixer::new(
            render,
            Arc::new(NativeResourceCatalog::new()),
            4,
            2,
            start_frame_boundary,
        )
        .unwrap();
        mixer.live.insert(
            0,
            Box::new(MemoryAudioSource {
                samples: left,
                sample_rate: 4,
                channels: source_channels,
                requests: Arc::clone(&left_requests),
            }),
        );
        mixer.live.insert(
            1,
            Box::new(MemoryAudioSource {
                samples: right,
                sample_rate: 4,
                channels: source_channels,
                requests: Arc::clone(&right_requests),
            }),
        );
        (mixer, left_requests, right_requests)
    }

    #[test]
    fn contiguous_runs_split_at_loop_boundaries() {
        let points = [
            MixPoint {
                output: 0,
                source_sample_index: 43_200,
                left_gain: 1.0,
                right_gain: 1.0,
            },
            MixPoint {
                output: 1,
                source_sample_index: 0,
                left_gain: 1.0,
                right_gain: 1.0,
            },
        ];
        assert_eq!(contiguous_runs(&points).len(), 2);
    }

    #[test]
    fn non_finite_decoded_samples_fail_before_advancing_the_mix_cursor() {
        for (layout, channels) in [("mono", 1), ("stereo", 2)] {
            let render = audio_render(layout);
            for channel in 0..channels {
                for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                    let (mut mixer, _, _) = mixer_with_memory_pcm(render.compiled_arc(), 0);
                    let mut samples = vec![0.0; usize::from(channels)];
                    samples[usize::from(channel)] = invalid;
                    mixer.live.insert(
                        0,
                        Box::new(MemoryAudioSource {
                            samples: samples.into(),
                            sample_rate: 4,
                            channels,
                            requests: Arc::new(Mutex::new(Vec::new())),
                        }),
                    );
                    assert!(matches!(
                        mixer.mix_until_sample(1),
                        Err(AudioMixError::NonFiniteDecodedPcm { source_index: 0 })
                    ));
                    assert_eq!(mixer.cursor, 0);
                }
            }
        }
    }

    #[test]
    fn common_profile_pcm_is_bit_exact_and_seek_chunk_invariant() {
        let render = audio_render("stereo");
        let project = crate::host::NativeProject::from_render(
            render.clone(),
            Arc::new(NativeResourceCatalog::new()),
        );
        let CompiledAudioItem::Clip { clip } = &project.compiled().audio().tracks()[0].items()[0]
        else {
            panic!("first audio item must be the admitted clip")
        };
        assert_eq!(clip.effects().len(), 3);

        let (mut whole_mixer, left_requests, right_requests) =
            mixer_with_memory_pcm(render.compiled_arc(), 0);
        let whole = whole_mixer.mix_until_sample(8).unwrap().unwrap();
        let actual_bits = whole
            .samples
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>();
        assert_eq!(
            actual_bits,
            [
                0x39dc_3372,
                0xba25_2696,
                0x3a5c_3372,
                0xba77_b9e1,
                0x3aa5_2696,
                0xbaa5_2696,
                0x3adc_3372,
                0xbace_703b,
                0xbbe7_03b0,
                0x3cb0_941c,
                0xbcdd_2f1b,
                0x3d81_0625,
                0xbd1a_d42c,
                0x3da5_e354,
                0xbd47_10cb,
                0x3dca_c083,
            ]
        );

        let (mut chunked_mixer, _, _) = mixer_with_memory_pcm(render.compiled_arc(), 0);
        let mut chunked = Vec::new();
        for boundary in [1, 3, 6, 8] {
            chunked.extend(
                chunked_mixer
                    .mix_until_sample(boundary)
                    .unwrap()
                    .unwrap()
                    .samples,
            );
        }
        assert_eq!(chunked, whole.samples);

        let (mut seek_mixer, _, _) = mixer_with_memory_pcm(render.compiled_arc(), 1);
        let seek = seek_mixer.mix_until_sample(8).unwrap().unwrap();
        assert_eq!(seek.samples, whole.samples[4..]);

        assert_eq!(
            *left_requests.lock().expect("left request log poisoned"),
            vec![(0, 6)]
        );
        assert_eq!(
            *right_requests.lock().expect("right request log poisoned"),
            vec![(2, 4), (0, 1), (0, 2)]
        );
    }

    #[test]
    fn common_profile_mono_pcm_is_explicitly_duplicated_before_endpoint_gains() {
        let render = audio_render("mono");
        for source_index in 0..render.compiled().sources().len() {
            assert_eq!(
                render
                    .compiled()
                    .sources()
                    .source(source_index as u32)
                    .unwrap()
                    .audio_channel_map(),
                Some(CompiledAudioChannelMap::MonoToStereo)
            );
        }
        let (mut mixer, _, _) = mixer_with_memory_pcm(render.compiled_arc(), 0);
        let mixed = mixer.mix_until_sample(8).unwrap().unwrap();
        let bits = mixed
            .samples
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>();
        assert_eq!(
            bits,
            [
                0x39dc_3372,
                0x39a5_2696,
                0x3a5c_3372,
                0x3a25_2696,
                0x3aa5_2696,
                0x3a77_b9e1,
                0x3adc_3372,
                0x3aa5_2696,
                0xbbe7_03b0,
                0xbc50_4818,
                0xbcdd_2f1b,
                0xbd38_51ec,
                0xbd1a_d42c,
                0xbd81_0625,
                0xbd47_10cb,
                0xbda5_e354,
            ]
        );
    }
}
