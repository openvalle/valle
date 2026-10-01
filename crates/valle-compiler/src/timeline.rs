//! Sparse Timeline document → internal canonical Timeline normalization.
//!
//! The public document keeps absolute clip starts, resource aliases, inherited caption
//! defaults and compact animation intent. This module deterministically lowers that timeline
//! shape directly into the canonical document; generated IDs, sequence gaps, expanded presets
//! and execution defaults remain internal. There is deliberately no second JSON-shaped Draft
//! contract between the public Timeline and canonical persistence waist.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;
use valle_timeline::internal::{
    CanonicalTimeline, ContentDigest, ExactRational, FrameRate, LocalInvariantReport, RationalTime,
    time::TimeError, wire::document,
};
use valle_timeline::{Timeline, wire::timeline};

use crate::caption_presets::{
    CaptionPreset, CaptionPresetError, CaptionPresetRequest, DisplayCaptionPreset,
    TimedCaptionPreset, expand_caption_presets,
};

const DEFAULT_BACKGROUND: &str = "#000000ff";
const RESOURCE_ID_NAMESPACE: &str = "resource";

/// Failures specific to the compact Timeline boundary. Canonical local-invariant failures are
/// retained verbatim through [`CompileTimelineError::InvalidCanonical`].
#[derive(Debug, Error)]
pub enum CompileTimelineError {
    #[error("Motion component `{component}` declared role `{role}` at `{path}`: {reason}")]
    MotionRole {
        component: String,
        role: String,
        path: String,
        reason: String,
    },
    #[error("Motion preparation failed: {reason}")]
    MotionPreparation { reason: String },
    #[error(
        "Motion at `{path}` needs its composition duration and role resolved before Timeline compilation"
    )]
    MissingMotionMetadata { path: String },
    #[error("resource alias `{alias}` at `{path}` must match [A-Za-z0-9._-]{{1,64}}")]
    InvalidResourceAlias { alias: String, path: String },
    #[error("resource `{alias}` has an empty locator")]
    EmptyResourceLocator { alias: String },
    #[error("resource alias `{alias}` referenced at `{path}` is not declared in resources")]
    UnknownResourceAlias { alias: String, path: String },
    #[error("clip at `{path}` starts at {start}, before the previous clip ends at {previous_end}")]
    TrackOverlap {
        path: String,
        start: ExactRational,
        previous_end: ExactRational,
    },
    #[error("time arithmetic overflow at `{path}`")]
    TimeOverflow { path: String },
    #[error("timeline must contain at least one positive-duration clip")]
    EmptyTimeline,
    #[error("caption at `{path}` must specify exactly one of `text` or `runs`")]
    InvalidCaptionContent { path: String },
    #[error("caption at `{path}` cannot combine presets with custom `presentation`")]
    PresetWithCustomPresentation { path: String },
    #[error("invalid frame rate: {0}")]
    InvalidFrameRate(#[source] TimeError),
    #[error("caption preset at `{path}` is invalid: {source}")]
    CaptionPreset {
        path: String,
        #[source]
        source: CaptionPresetError,
    },
    #[error("normalized Timeline violates local invariants: {0}")]
    InvalidCanonical(LocalInvariantReport),
}

/// Compile one complete sparse Timeline into Valle's existing internal canonical document.
///
/// This function performs no I/O. URLs stay in the persisted Timeline; Canonical resource
/// references are deterministic `resource:<alias>` IDs that can be resolved by the project layer.
pub fn compile_timeline(timeline: Timeline) -> Result<CanonicalTimeline, CompileTimelineError> {
    compile_timeline_with_motion_sources(timeline, &BTreeMap::new())
}

/// Compile a public Timeline with duration and role metadata obtained from its prepared Motion artifacts.
pub fn compile_timeline_with_motion_sources(
    timeline: Timeline,
    motion_sources: &BTreeMap<String, valle_timeline::MotionSourceMetadata>,
) -> Result<CanonicalTimeline, CompileTimelineError> {
    let mut timeline = timeline.into_wire();
    let mut resolved_sources = motion_sources.clone();
    bind_motion_instances(&mut timeline, &mut resolved_sources)?;
    TimelineNormalizer::new(&timeline, &resolved_sources)?.compile(NormalizationInput { timeline })
}

/// Identity of one Motion instance's preparation inputs. Native and Web hosts use this
/// same key when binding a compiled Artifact to the normalized Timeline resource.
pub fn motion_instance_key(
    locator: &str,
    resources: &serde_json::Value,
    data: &serde_json::Value,
) -> String {
    let inputs = serde_json::json!([locator, resources, data]);
    let digest = ContentDigest::of_bytes(
        &serde_json::to_vec(&inputs).expect("Motion instance inputs serialize"),
    );
    format!("motion-{}", &digest.as_hex()[..56])
}

fn bind_motion_instances(
    timeline: &mut timeline::TimelineWire,
    motion_sources: &mut BTreeMap<String, valle_timeline::MotionSourceMetadata>,
) -> Result<(), CompileTimelineError> {
    let inputs = crate::motion_inputs::motion_preparation_inputs_wire(timeline)?;
    let original = timeline.resources.clone();
    let mut by_path = BTreeMap::new();
    for input in inputs {
        let key = input.key();
        if original.contains_key(&key) {
            return Err(CompileTimelineError::InvalidResourceAlias {
                alias: key,
                path: "/resources".into(),
            });
        }
        if !motion_sources.contains_key(&key) {
            if let Some(metadata) = motion_sources.get(&input.component).copied() {
                motion_sources.insert(key.clone(), metadata);
            }
        }
        timeline.resources.insert(key.clone(), input.locator);
        by_path.insert(input.clip_path, key);
    }
    for (ti, track) in timeline.tracks.visual.iter_mut().enumerate() {
        for (ci, clip) in track.clips.iter_mut().enumerate() {
            if let timeline::TimelineVisualSourceWire::Motion { component, .. } = &mut clip.source {
                *component = by_path[&format!("/tracks/visual/{ti}/clips/{ci}")].clone();
            }
        }
    }
    Ok(())
}

/// Private normalization carrier. It intentionally has no serde/schema surface.
struct NormalizationInput {
    timeline: timeline::TimelineWire,
}

struct TimelineNormalizer {
    resources: BTreeSet<String>,
    resource_locators: BTreeMap<String, String>,
    motion_sources: BTreeMap<String, valle_timeline::MotionSourceMetadata>,
    frame_rate: FrameRate,
    canvas_size: [u32; 2],
}

impl TimelineNormalizer {
    fn new(
        timeline: &timeline::TimelineWire,
        motion_sources: &BTreeMap<String, valle_timeline::MotionSourceMetadata>,
    ) -> Result<Self, CompileTimelineError> {
        validate_resources(&timeline.resources)?;
        let frame_rate = frame_rate(&timeline.canvas.fps)?;
        Ok(Self {
            resources: timeline.resources.keys().cloned().collect(),
            resource_locators: timeline.resources.clone(),
            motion_sources: motion_sources.clone(),
            frame_rate,
            canvas_size: [timeline.canvas.width, timeline.canvas.height],
        })
    }

    fn compile(self, input: NormalizationInput) -> Result<CanonicalTimeline, CompileTimelineError> {
        let timeline = input.timeline;
        let duration = timeline_duration(&timeline)?;
        let mut visual = Vec::new();
        let mut audio = Vec::new();
        let mut adjustments = Vec::new();
        let mut captions = Vec::new();

        for (track_index, track) in timeline.tracks.visual.into_iter().enumerate() {
            visual.push(self.visual_track(track_index, track.clips, track.transitions)?);
        }
        for (track_index, track) in timeline.tracks.audio.into_iter().enumerate() {
            audio.push(self.audio_track(track_index, track.clips)?);
        }
        for (track_index, track) in timeline.tracks.adjustment.into_iter().enumerate() {
            adjustments.extend(self.adjustment_track(track_index, track.clips)?);
        }
        for (track_index, track) in timeline.tracks.caption.into_iter().enumerate() {
            captions.push(self.caption_track(
                track_index,
                track.style,
                track.layout,
                track.presenter,
                track.clips,
            )?);
        }

        let wire = document::TimelineDocumentEnvelopeWire {
            document: document::TimelineDocumentWire {
                canvas: document::CanvasWire {
                    width: timeline.canvas.width,
                    height: timeline.canvas.height,
                    fps: self.frame_rate.into_exact(),
                    sample_rate: 48_000,
                    channel_layout: document::ChannelLayoutWire::Stereo,
                    color_space: document::ColorSpaceWire::Srgb,
                    duration,
                },
                background: document::BackgroundWire {
                    color: timeline
                        .canvas
                        .background
                        .unwrap_or_else(|| DEFAULT_BACKGROUND.to_owned()),
                },
                visual: document::VisualCompositionWire { tracks: visual },
                audio: document::AudioCompositionWire { tracks: audio },
                adjustments,
                captions: document::CaptionCompositionWire { tracks: captions },
                camera: None,
                metadata: document::JsonObject::new(),
            },
        };
        CanonicalTimeline::try_from_wire(wire).map_err(CompileTimelineError::InvalidCanonical)
    }

    fn visual_track(
        &self,
        track_index: usize,
        clips: Vec<timeline::TimelineVisualClipWire>,
        transitions: Vec<timeline::TimelineTransitionWire>,
    ) -> Result<document::VisualTrackWire, CompileTimelineError> {
        let track_id = track_id("visual", track_index);
        let transitions: BTreeMap<_, _> = transitions
            .into_iter()
            .map(|value| (value.to, value))
            .collect();
        let mut cursor = ExactRational::ZERO;
        let mut items = Vec::new();
        for (clip_index, clip) in clips.into_iter().enumerate() {
            let clip_path = format!("/tracks/visual/{track_index}/clips/{clip_index}");
            let clip_id = clip_id("visual", track_index, clip_index);
            let start = exact_time(&clip.start);
            if let Some(transition) = transitions.get(&clip_index) {
                items.push(document::VisualItemWire::Transition(
                    document::VisualTransitionWire {
                        id: format!("{track_id}:transition:{}", transition.from),
                        duration: cursor.checked_sub(start).map_err(|_| {
                            CompileTimelineError::TimeOverflow {
                                path: clip_path.clone(),
                            }
                        })?,
                        kernel: document::TransitionKernelWire::Builtin(
                            document::BuiltinTransitionKernelWire {
                                kind: transition.kind,
                                params: transition.params.clone(),
                            },
                        ),
                    },
                ));
            } else {
                push_visual_gap(
                    &mut items,
                    track_index,
                    clip_index,
                    cursor,
                    start,
                    &clip_path,
                )?;
            }
            let duration = exact_time(&clip.duration);
            cursor = checked_end(start, duration, &clip_path)?;

            let source = self.visual_source(
                clip.source,
                &clip.duration,
                track_index,
                clip_index,
                &clip_path,
            )?;
            let layer = document::VisualLayerWire {
                transform: document::LayerTransformWire {
                    size: Some(
                        clip.size
                            .map(|value| timeline_param_to_canonical(value, &clip_id, "size"))
                            .unwrap_or_else(|| constant(self.canvas_size.map(f64::from))),
                    ),
                    position: clip
                        .position
                        .map(|value| timeline_param_to_canonical(value, &clip_id, "position"))
                        .unwrap_or_else(|| constant([0.5, 0.5])),
                    scale: clip
                        .scale
                        .map(|value| timeline_param_to_canonical(value, &clip_id, "scale"))
                        .unwrap_or_else(|| constant([1.0, 1.0])),
                    rotation: clip
                        .rotation
                        .map(|value| {
                            timeline_param_to_canonical(
                                map_param(value, f64::to_radians),
                                &clip_id,
                                "rotation",
                            )
                        })
                        .unwrap_or_else(|| constant(0.0)),
                    anchor: clip.anchor.unwrap_or([0.5, 0.5]),
                },
                opacity: clip
                    .opacity
                    .map(|value| timeline_param_to_canonical(value, &clip_id, "opacity"))
                    .unwrap_or_else(|| constant(1.0)),
                mask: None,
                filters: Vec::new(),
                blend: clip
                    .blend
                    .map(lower_blend)
                    .unwrap_or(document::BlendModeWire::Normal),
            };
            items.push(document::VisualItemWire::Clip(document::VisualClipWire {
                id: clip_id,
                duration,
                layer,
                source,
            }));
        }
        Ok(document::VisualTrackWire {
            id: track_id,
            items,
        })
    }

    fn visual_source(
        &self,
        source: timeline::TimelineVisualSourceWire,
        _clip_duration: &timeline::TimelineTimeWire,
        track_index: usize,
        clip_index: usize,
        clip_path: &str,
    ) -> Result<document::VisualSourceWire, CompileTimelineError> {
        let source_path = format!("{clip_path}/src");
        Ok(match source {
            timeline::TimelineVisualSourceWire::Video {
                src,
                gain,
                trim_start,
                rate,
                end,
                fit,
            } => document::VisualSourceWire::Video(document::VideoSourceWire {
                gain: gain.map(|value| {
                    timeline_param_to_canonical(
                        value,
                        &clip_id("visual", track_index, clip_index),
                        "gain",
                    )
                }),
                resource: self.resolve(&src, &source_path)?,
                source_start: trim_start.as_ref().map_or(ExactRational::ZERO, exact_time),
                rate: rate.as_ref().map_or(ExactRational::ONE, exact_time),
                end_behavior: end
                    .map(lower_media_end)
                    .unwrap_or(document::MediaEndBehaviorWire::Error),
                sampling: document::RasterSamplingWire {
                    fit: fit
                        .map(lower_raster_fit)
                        .unwrap_or(document::RasterFitWire::Contain),
                },
            }),
            timeline::TimelineVisualSourceWire::Image { src, fit } => {
                document::VisualSourceWire::Image(document::ImageSourceWire {
                    resource: self.resolve(&src, &source_path)?,
                    sampling: document::RasterSamplingWire {
                        fit: fit
                            .map(lower_raster_fit)
                            .unwrap_or(document::RasterFitWire::Contain),
                    },
                })
            }
            timeline::TimelineVisualSourceWire::Lottie {
                src,
                trim_start,
                rate,
                end,
                fit,
            } => document::VisualSourceWire::Lottie(document::LottieSourceWire {
                resource: self.resolve(&src, &source_path)?,
                source_start: trim_start.as_ref().map_or(ExactRational::ZERO, exact_time),
                rate: rate.as_ref().map_or(ExactRational::ONE, exact_time),
                end_behavior: end
                    .map(lower_media_end)
                    .unwrap_or(document::MediaEndBehaviorWire::Error),
                sampling: document::LottieSamplingWire {
                    fit: fit
                        .map(lower_raster_fit)
                        .unwrap_or(document::RasterFitWire::Contain),
                },
            }),
            timeline::TimelineVisualSourceWire::Motion {
                component,
                fit,
                trim_start,
                rate,
                end,
                props,
                data,
                resources,
            } => {
                let owner = clip_id("visual", track_index, clip_index);
                let props = props
                    .into_iter()
                    .enumerate()
                    .map(|(index, (name, value))| {
                        (
                            name,
                            timeline_param_to_canonical(value, &owner, &format!("prop:{index}")),
                        )
                    })
                    .collect();
                let resources = resources
                    .into_iter()
                    .map(|(slot, alias)| {
                        let path = format!("{clip_path}/resources/{slot}");
                        self.resolve(&alias, &path).map(|resource| (slot, resource))
                    })
                    .collect::<Result<_, _>>()?;
                let metadata = self.motion_sources.get(&component).ok_or_else(|| {
                    CompileTimelineError::MissingMotionMetadata {
                        path: clip_path.to_owned(),
                    }
                })?;
                let role_error = |reason: String| CompileTimelineError::MotionRole {
                    component: component.clone(),
                    role: metadata.role.name().into(),
                    path: clip_path.to_owned(),
                    reason,
                };
                if matches!(
                    metadata.role,
                    valle_timeline::MotionRole::CaptionPresenter { .. }
                ) {
                    return Err(role_error(
                        "captionPresenter belongs on a caption track".into(),
                    ));
                }
                metadata
                    .role
                    .validate(
                        metadata.duration,
                        Some(RationalTime::from_exact(exact_time(_clip_duration))),
                    )
                    .map_err(|reason| {
                        role_error(format!(
                            "{reason}; host duration {}, template duration {}",
                            exact_time(_clip_duration),
                            metadata.duration
                        ))
                    })?;
                if matches!(metadata.role, valle_timeline::MotionRole::Overlay { .. }) {
                    for (field, present) in [
                        ("fit", fit.is_some()),
                        ("trimStart", trim_start.is_some()),
                        ("rate", rate.is_some()),
                        ("end", end.is_some()),
                    ] {
                        if present {
                            return Err(role_error(format!("overlay does not admit `{field}`")));
                        }
                    }
                }
                document::VisualSourceWire::Motion(document::MotionInstanceWire {
                    component: self.resolve(&component, &format!("{clip_path}/component"))?,
                    fit: fit
                        .map(lower_raster_fit)
                        .unwrap_or(document::RasterFitWire::Contain),
                    source_start: trim_start.as_ref().map_or(ExactRational::ZERO, exact_time),
                    source_duration: metadata.duration.into_exact(),
                    role: metadata.role,
                    rate: rate.as_ref().map_or(ExactRational::ONE, exact_time),
                    end_behavior: if matches!(
                        metadata.role,
                        valle_timeline::MotionRole::Overlay { .. }
                    ) {
                        document::MediaEndBehaviorWire::Hold
                    } else {
                        end.map(lower_media_end)
                            .unwrap_or(document::MediaEndBehaviorWire::Error)
                    },
                    props,
                    data,
                    resources,
                })
            }
            timeline::TimelineVisualSourceWire::Solid { color } => {
                document::VisualSourceWire::Solid(document::SolidSourceWire { color })
            }
        })
    }

    fn audio_track(
        &self,
        track_index: usize,
        clips: Vec<timeline::TimelineAudioClipWire>,
    ) -> Result<document::AudioTrackWire, CompileTimelineError> {
        let mut cursor = ExactRational::ZERO;
        let mut items = Vec::new();
        for (clip_index, clip) in clips.into_iter().enumerate() {
            let clip_path = format!("/tracks/audio/{track_index}/clips/{clip_index}");
            let id = clip_id("audio", track_index, clip_index);
            let start = exact_time(&clip.start);
            push_audio_gap(
                &mut items,
                track_index,
                clip_index,
                cursor,
                start,
                &clip_path,
            )?;
            let duration = exact_time(&clip.duration);
            cursor = checked_end(start, duration, &clip_path)?;
            items.push(document::AudioItemWire::Clip(document::AudioClipWire {
                id: id.clone(),
                duration,
                source: document::AudioSourceWire::Media(document::AudioMediaSourceWire {
                    resource: self.resolve(&clip.src, &format!("{clip_path}/src"))?,
                    source_start: clip
                        .trim_start
                        .as_ref()
                        .map_or(ExactRational::ZERO, exact_time),
                    rate: clip.rate.as_ref().map_or(ExactRational::ONE, exact_time),
                    end_behavior: clip
                        .end
                        .map(lower_media_end)
                        .unwrap_or(document::MediaEndBehaviorWire::Error),
                }),
                gain: clip
                    .gain
                    .map(|value| timeline_param_to_canonical(value, &id, "gain"))
                    .unwrap_or_else(|| constant(1.0)),
                pan: clip
                    .pan
                    .map(|value| timeline_param_to_canonical(value, &id, "pan"))
                    .unwrap_or_else(|| constant(0.0)),
                effects: Vec::new(),
            }));
        }
        Ok(document::AudioTrackWire {
            id: track_id("audio", track_index),
            items,
        })
    }

    fn caption_track(
        &self,
        track_index: usize,
        style: timeline::TimelineCaptionStyleWire,
        track_layout: Option<timeline::TimelineCaptionLayoutWire>,
        presenter: Option<timeline::TimelineCaptionPresenterWire>,
        clips: Vec<timeline::TimelineCaptionClipWire>,
    ) -> Result<document::CaptionTrackWire, CompileTimelineError> {
        let font = self.resolve(
            &style.font,
            &format!("/tracks/caption/{track_index}/style/font"),
        )?;
        let canonical_style = document::CaptionStyleWire {
            font,
            font_size: style.font_size.unwrap_or(64.0),
            color: style
                .color
                .clone()
                .unwrap_or_else(|| "#ffffffff".to_owned()),
            shadow: style
                .shadow
                .clone()
                .map(|shadow| document::CaptionShadowWire {
                    color: shadow.color,
                    offset: shadow.offset,
                    blur_sigma: shadow.blur.unwrap_or(0.0),
                }),
        };
        let mut cursor = ExactRational::ZERO;
        let mut items = Vec::new();
        for (clip_index, clip) in clips.into_iter().enumerate() {
            let clip_path = format!("/tracks/caption/{track_index}/clips/{clip_index}");
            let id = clip_id("caption", track_index, clip_index);
            let start = exact_time(&clip.start);
            push_caption_gap(
                &mut items,
                track_index,
                clip_index,
                cursor,
                start,
                &clip_path,
            )?;
            let duration = exact_time(&clip.duration);
            cursor = checked_end(start, duration, &clip_path)?;

            if let Some(presenter) = &presenter {
                let input = crate::motion_inputs::caption_motion_input(
                    &self.resource_locators,
                    presenter,
                    &style,
                    track_layout.as_ref(),
                    &clip,
                    self.canvas_size,
                    &clip_path,
                )?;
                let key = input.key();
                let metadata = self.motion_sources.get(&key).ok_or_else(|| {
                    CompileTimelineError::MissingMotionMetadata {
                        path: clip_path.clone(),
                    }
                })?;
                let role_error = |reason: &str| CompileTimelineError::MotionRole {
                    component: presenter.component.clone(),
                    role: metadata.role.name().into(),
                    path: clip_path.clone(),
                    reason: reason.into(),
                };
                if !matches!(
                    metadata.role,
                    valle_timeline::MotionRole::CaptionPresenter { .. }
                ) {
                    return Err(role_error(
                        "a caption track presenter must declare captionPresenter",
                    ));
                }
                metadata
                    .role
                    .validate(metadata.duration, Some(RationalTime::from_exact(duration)))
                    .map_err(role_error)?;
                items.push(document::CaptionItemWire::Motion(
                    document::CaptionMotionWire {
                        id: id.clone(),
                        duration,
                        source: document::MotionInstanceWire {
                            component: self.resolve(&key, &clip_path)?,
                            role: metadata.role,
                            fit: document::RasterFitWire::Contain,
                            source_start: ExactRational::ZERO,
                            source_duration: metadata.duration.into_exact(),
                            rate: ExactRational::ONE,
                            end_behavior: document::MediaEndBehaviorWire::Hold,
                            props: presenter
                                .props
                                .clone()
                                .into_iter()
                                .map(|(name, value)| {
                                    (
                                        name.clone(),
                                        timeline_param_to_canonical(
                                            value,
                                            &id,
                                            &format!("motionProp:{name}"),
                                        ),
                                    )
                                })
                                .collect(),
                            data: input
                                .data
                                .as_object()
                                .expect("caption data object")
                                .iter()
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect(),
                            resources: input
                                .resources
                                .into_iter()
                                .map(|(slot, alias)| {
                                    self.resolve(&alias, &clip_path).map(|id| (slot, id))
                                })
                                .collect::<Result<_, _>>()?,
                        },
                    },
                ));
                continue;
            }
            let runs = caption_runs(clip.text, clip.runs, &id, &clip_path)?;
            let layout = merge_layout(track_layout.as_ref(), clip.layout.as_ref());
            let uses_preset = clip.enter.is_some() || clip.display.is_some() || clip.exit.is_some();
            if uses_preset && clip.presentation.is_some() {
                return Err(CompileTimelineError::PresetWithCustomPresentation { path: clip_path });
            }
            let presentation = if uses_preset {
                let request =
                    self.preset_request(&id, duration, clip.enter, clip.display, clip.exit);
                let expanded = expand_caption_presets(&request).map_err(|source| {
                    CompileTimelineError::CaptionPreset {
                        path: clip_path.clone(),
                        source,
                    }
                })?;
                expanded
            } else {
                clip.presentation
                    .map(|value| timeline_presentation_to_canonical(value, &id))
                    .unwrap_or_else(default_caption_presentation)
            };

            let behavior = clip.behavior.map(Self::caption_behavior);
            items.push(document::CaptionItemWire::Clip(document::CaptionWire {
                id,
                duration,
                runs,
                style: canonical_style.clone(),
                layout,
                presentation,
                behavior,
            }));
        }
        Ok(document::CaptionTrackWire {
            id: track_id("caption", track_index),
            items,
        })
    }

    fn adjustment_track(
        &self,
        track_index: usize,
        clips: Vec<timeline::TimelineAdjustmentClipWire>,
    ) -> Result<Vec<document::TimedAdjustmentWire>, CompileTimelineError> {
        let mut cursor = ExactRational::ZERO;
        let mut effects = Vec::with_capacity(clips.len());
        for (clip_index, clip) in clips.into_iter().enumerate() {
            let path = format!("/tracks/adjustment/{track_index}/clips/{clip_index}");
            let start = exact_time(&clip.start);
            let _ = require_gap(cursor, start, &path)?;
            let duration = exact_time(&clip.duration);
            cursor = checked_end(start, duration, &path)?;
            let effect = match clip.adjustment {
                timeline::TimelineAdjustmentWire::ColorGrade { temperature } => {
                    document::AdjustmentEffectWire::ColorGrade(document::ColorGradeEffectWire {
                        kind: document::ColorGradeEffectTag::ColorGrade,
                        temperature,
                    })
                }
            };
            effects.push(document::TimedAdjustmentWire {
                id: clip_id("adjustment", track_index, clip_index),
                start,
                duration,
                effect,
            });
        }
        Ok(effects)
    }

    fn preset_request(
        &self,
        owner_id: &str,
        caption_duration: ExactRational,
        enter: Option<timeline::TimelineEnterPresetWire>,
        display: Option<timeline::TimelineDisplayPresetWire>,
        exit: Option<timeline::TimelineExitPresetWire>,
    ) -> CaptionPresetRequest {
        let enter = enter.map(enter_preset);
        let display = display.map(display_preset);
        let exit = exit.map(exit_preset);
        CaptionPresetRequest {
            owner_id: owner_id.to_owned(),
            caption_duration: RationalTime::from_exact(caption_duration),
            fps: self.frame_rate,
            canvas_size: self.canvas_size,
            enter,
            display,
            exit,
        }
    }

    fn resolve(&self, alias: &str, path: &str) -> Result<String, CompileTimelineError> {
        if !self.resources.contains(alias) {
            return Err(CompileTimelineError::UnknownResourceAlias {
                alias: alias.to_owned(),
                path: path.to_owned(),
            });
        }
        Ok(format!("{RESOURCE_ID_NAMESPACE}:{alias}"))
    }

    fn caption_behavior(
        behavior: timeline::TimelineCaptionBehaviorWire,
    ) -> document::CaptionBehaviorWire {
        match behavior {
            timeline::TimelineCaptionBehaviorWire::Scroll { axis, speed } => {
                document::CaptionBehaviorWire::Scroll {
                    axis: lower_scroll_axis(axis),
                    speed,
                }
            }
            timeline::TimelineCaptionBehaviorWire::Karaoke { mode } => {
                document::CaptionBehaviorWire::Karaoke {
                    mode: lower_karaoke_mode(mode),
                }
            }
        }
    }
}

fn validate_resources(resources: &BTreeMap<String, String>) -> Result<(), CompileTimelineError> {
    for (alias, locator) in resources {
        if !valid_alias(alias) {
            return Err(CompileTimelineError::InvalidResourceAlias {
                alias: alias.clone(),
                path: format!("/resources/{alias}"),
            });
        }
        if locator.is_empty() {
            return Err(CompileTimelineError::EmptyResourceLocator {
                alias: alias.clone(),
            });
        }
    }
    Ok(())
}

fn valid_alias(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn frame_rate(value: &timeline::TimelineFrameRateWire) -> Result<FrameRate, CompileTimelineError> {
    value
        .try_to_frame_rate()
        .map_err(CompileTimelineError::InvalidFrameRate)
}

fn exact_time(value: &timeline::TimelineTimeWire) -> ExactRational {
    value.to_exact()
}

fn timeline_duration(
    timeline: &timeline::TimelineWire,
) -> Result<ExactRational, CompileTimelineError> {
    let mut maximum = ExactRational::ZERO;
    for (band, tracks) in [
        (
            "visual",
            timeline
                .tracks
                .visual
                .iter()
                .map(|track| {
                    track
                        .clips
                        .iter()
                        .map(|clip| (&clip.start, &clip.duration))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>(),
        ),
        (
            "audio",
            timeline
                .tracks
                .audio
                .iter()
                .map(|track| {
                    track
                        .clips
                        .iter()
                        .map(|clip| (&clip.start, &clip.duration))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>(),
        ),
        (
            "caption",
            timeline
                .tracks
                .caption
                .iter()
                .map(|track| {
                    track
                        .clips
                        .iter()
                        .map(|clip| (&clip.start, &clip.duration))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>(),
        ),
        (
            "adjustment",
            timeline
                .tracks
                .adjustment
                .iter()
                .map(|track| {
                    track
                        .clips
                        .iter()
                        .map(|clip| (&clip.start, &clip.duration))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>(),
        ),
    ] {
        for (track_index, clips) in tracks.into_iter().enumerate() {
            for (clip_index, (start, duration)) in clips.into_iter().enumerate() {
                let path = format!("/tracks/{band}/{track_index}/clips/{clip_index}");
                maximum = maximum.max(checked_end(exact_time(start), exact_time(duration), &path)?);
            }
        }
    }
    if maximum.is_positive() {
        Ok(maximum)
    } else {
        Err(CompileTimelineError::EmptyTimeline)
    }
}

fn checked_end(
    start: ExactRational,
    duration: ExactRational,
    path: &str,
) -> Result<ExactRational, CompileTimelineError> {
    start
        .checked_add(duration)
        .map_err(|_| CompileTimelineError::TimeOverflow {
            path: path.to_owned(),
        })
}

fn require_gap(
    cursor: ExactRational,
    start: ExactRational,
    path: &str,
) -> Result<Option<ExactRational>, CompileTimelineError> {
    if start < cursor {
        return Err(CompileTimelineError::TrackOverlap {
            path: path.to_owned(),
            start,
            previous_end: cursor,
        });
    }
    let gap = start
        .checked_sub(cursor)
        .map_err(|_| CompileTimelineError::TimeOverflow {
            path: path.to_owned(),
        })?;
    Ok(gap.is_positive().then_some(gap))
}

fn push_visual_gap(
    items: &mut Vec<document::VisualItemWire>,
    track_index: usize,
    clip_index: usize,
    cursor: ExactRational,
    start: ExactRational,
    path: &str,
) -> Result<(), CompileTimelineError> {
    if let Some(duration) = require_gap(cursor, start, path)? {
        items.push(document::VisualItemWire::Gap(document::GapWire {
            id: gap_id("visual", track_index, clip_index),
            duration,
        }));
    }
    Ok(())
}

fn push_audio_gap(
    items: &mut Vec<document::AudioItemWire>,
    track_index: usize,
    clip_index: usize,
    cursor: ExactRational,
    start: ExactRational,
    path: &str,
) -> Result<(), CompileTimelineError> {
    if let Some(duration) = require_gap(cursor, start, path)? {
        items.push(document::AudioItemWire::Gap(document::GapWire {
            id: gap_id("audio", track_index, clip_index),
            duration,
        }));
    }
    Ok(())
}

fn push_caption_gap(
    items: &mut Vec<document::CaptionItemWire>,
    track_index: usize,
    clip_index: usize,
    cursor: ExactRational,
    start: ExactRational,
    path: &str,
) -> Result<(), CompileTimelineError> {
    if let Some(duration) = require_gap(cursor, start, path)? {
        items.push(document::CaptionItemWire::Gap(document::GapWire {
            id: gap_id("caption", track_index, clip_index),
            duration,
        }));
    }
    Ok(())
}

fn track_id(band: &str, track_index: usize) -> String {
    format!("timeline:{band}-track:{track_index}")
}

fn clip_id(band: &str, track_index: usize, clip_index: usize) -> String {
    format!("timeline:{band}-track:{track_index}:clip:{clip_index}")
}

fn gap_id(band: &str, track_index: usize, clip_index: usize) -> String {
    format!("timeline:{band}-track:{track_index}:gap:{clip_index}")
}

fn timeline_param_to_canonical<T>(
    value: timeline::TimelineParamWire<T>,
    owner_id: &str,
    slot: &str,
) -> document::ParamWire<T> {
    match value {
        timeline::TimelineParamWire::Value(value) => constant(value),
        timeline::TimelineParamWire::Curve(curve) => {
            let curve_id = format!("{owner_id}:curve:{slot}");
            let keyframes = curve
                .keyframes
                .into_iter()
                .enumerate()
                .map(|(index, keyframe)| {
                    let (time, value, out_easing) = keyframe.into_parts();
                    document::KeyframeWire {
                        id: format!("{curve_id}:keyframe:{index}"),
                        time: time.to_exact(),
                        value,
                        out_easing: out_easing.map(lower_easing),
                    }
                })
                .collect();
            document::ParamWire::Curve(document::CurveWire {
                id: curve_id,
                interpolation: curve
                    .interpolation
                    .map(lower_interpolation)
                    .unwrap_or(document::InterpolationWire::Linear),
                keyframes,
                extrapolation: document::ExtrapolationWire::Clamp,
            })
        }
    }
}

fn timeline_presentation_to_canonical(
    value: timeline::TimelineCaptionPresentationWire,
    owner_id: &str,
) -> document::CaptionPresentationWire {
    document::CaptionPresentationWire {
        opacity: value
            .opacity
            .map(|value| timeline_param_to_canonical(value, owner_id, "opacity"))
            .unwrap_or_else(|| constant(1.0)),
        translation: value
            .translation
            .map(|value| timeline_param_to_canonical(value, owner_id, "translation"))
            .unwrap_or_else(|| constant([0.0, 0.0])),
        scale: value
            .scale
            .map(|value| timeline_param_to_canonical(value, owner_id, "scale"))
            .unwrap_or_else(|| constant(1.0)),
        rotation: value
            .rotation
            .map(|value| {
                timeline_param_to_canonical(map_param(value, f64::to_radians), owner_id, "rotation")
            })
            .unwrap_or_else(|| constant(0.0)),
        clip_inset: value
            .clip_inset
            .map(|value| timeline_param_to_canonical(value, owner_id, "clipInset"))
            .unwrap_or_else(|| constant([0.0, 0.0, 0.0, 0.0])),
        blur_sigma: value
            .blur
            .map(|value| timeline_param_to_canonical(value, owner_id, "blurSigma"))
            .unwrap_or_else(|| constant(0.0)),
    }
}

fn default_caption_presentation() -> document::CaptionPresentationWire {
    document::CaptionPresentationWire {
        opacity: constant(1.0),
        translation: constant([0.0, 0.0]),
        scale: constant(1.0),
        rotation: constant(0.0),
        clip_inset: constant([0.0, 0.0, 0.0, 0.0]),
        blur_sigma: constant(0.0),
    }
}

fn caption_runs(
    text: Option<String>,
    runs: Option<Vec<timeline::TimelineTextRunWire>>,
    owner_id: &str,
    path: &str,
) -> Result<Vec<document::TextRunWire>, CompileTimelineError> {
    let runs = match (text, runs) {
        (Some(text), None) => vec![timeline::TimelineTextRunWire {
            text,
            start: None,
            end: None,
            font_size: None,
            color: None,
        }],
        (None, Some(runs)) if !runs.is_empty() => runs,
        _ => {
            return Err(CompileTimelineError::InvalidCaptionContent {
                path: path.to_owned(),
            });
        }
    };
    Ok(runs
        .into_iter()
        .enumerate()
        .map(|(index, run)| {
            let style = (run.font_size.is_some() || run.color.is_some()).then_some(
                document::TextRunStyleWire {
                    font_size: run.font_size,
                    color: run.color,
                },
            );
            document::TextRunWire {
                id: format!("{owner_id}:run:{index}"),
                text: run.text,
                timing: run
                    .start
                    .zip(run.end)
                    .map(|(start, end)| document::TextRunTimingWire {
                        start: start.to_exact(),
                        end: end.to_exact(),
                    }),
                style,
            }
        })
        .collect())
}

/// Freeze one caption's text, timed runs and inherited style/layout for Motion preparation.
/// Placement start is deliberately absent; duration only validates timed input.
pub fn prepare_caption_presenter_data(
    style: &timeline::TimelineCaptionStyleWire,
    track_layout: Option<&timeline::TimelineCaptionLayoutWire>,
    clip: &timeline::TimelineCaptionClipWire,
    canvas: [u32; 2],
    path: &str,
) -> Result<valle_timeline::motion::CaptionPresenterData, CompileTimelineError> {
    use valle_timeline::motion::*;
    let error = |reason: String| CompileTimelineError::MotionPreparation {
        reason: format!("{path}: {reason}"),
    };
    for (name, present) in [
        ("enter", clip.enter.is_some()),
        ("display", clip.display.is_some()),
        ("exit", clip.exit.is_some()),
        ("presentation", clip.presentation.is_some()),
        ("behavior", clip.behavior.is_some()),
    ] {
        if present {
            return Err(error(format!("captionPresenter does not admit `{name}`")));
        }
    }
    let authored_runs = match (&clip.text, &clip.runs) {
        (Some(text), None) => vec![timeline::TimelineTextRunWire {
            text: text.clone(),
            start: None,
            end: None,
            font_size: None,
            color: None,
        }],
        (None, Some(runs)) if !runs.is_empty() => runs.clone(),
        _ => return Err(CompileTimelineError::InvalidCaptionContent { path: path.into() }),
    };
    let layout = merge_layout(track_layout, clip.layout.as_ref());
    let [x, y, width, height] = layout.region;
    if layout.region.iter().any(|value| !value.is_finite())
        || x < 0.0
        || y < 0.0
        || width <= 0.0
        || height <= 0.0
        || x + width > 1.0
        || y + height > 1.0
        || canvas.contains(&0)
    {
        return Err(error(
            "caption region must lie within a positive canvas".into(),
        ));
    }
    let data = CaptionPresenterData {
        text: authored_runs.iter().map(|run| run.text.as_str()).collect(),
        runs: authored_runs
            .into_iter()
            .map(|run| CaptionPresenterRun {
                text: run.text,
                start: run.start.map(|time| time.to_exact().as_f64()),
                end: run.end.map(|time| time.to_exact().as_f64()),
                font_size: run.font_size,
                color: run.color,
            })
            .collect(),
        style: CaptionPresenterStyle {
            font: CAPTION_FONT_URI.into(),
            font_size: style.font_size.unwrap_or(64.0),
            color: style.color.clone().unwrap_or_else(|| "#ffffffff".into()),
            shadow: style.shadow.as_ref().map(|shadow| CaptionPresenterShadow {
                color: shadow.color.clone(),
                offset: shadow.offset,
                blur: shadow.blur.unwrap_or(0.0),
            }),
        },
        region: [
            x * f64::from(canvas[0]),
            y * f64::from(canvas[1]),
            width * f64::from(canvas[0]),
            height * f64::from(canvas[1]),
        ],
        align: layout.align,
    };
    data.validate(Some(RationalTime::from_exact(clip.duration.to_exact())))
        .map_err(error)?;
    Ok(data)
}

fn merge_layout(
    track: Option<&timeline::TimelineCaptionLayoutWire>,
    clip: Option<&timeline::TimelineCaptionLayoutWire>,
) -> document::CaptionLayoutWire {
    let region = clip
        .and_then(|value| value.region)
        .or_else(|| track.and_then(|value| value.region));
    let align = clip
        .and_then(|value| value.align)
        .or_else(|| track.and_then(|value| value.align));
    document::CaptionLayoutWire {
        region: region.unwrap_or([0.1, 0.76, 0.8, 0.18]),
        align: align
            .map(lower_caption_align)
            .unwrap_or(document::CaptionAlignWire::BottomCenter),
    }
}

fn lower_blend(value: timeline::BlendModeWire) -> document::BlendModeWire {
    match value {
        timeline::BlendModeWire::Normal => document::BlendModeWire::Normal,
        timeline::BlendModeWire::Screen => document::BlendModeWire::Screen,
        timeline::BlendModeWire::Lighten => document::BlendModeWire::Lighten,
        timeline::BlendModeWire::ColorDodge => document::BlendModeWire::ColorDodge,
        timeline::BlendModeWire::Multiply => document::BlendModeWire::Multiply,
        timeline::BlendModeWire::Darken => document::BlendModeWire::Darken,
        timeline::BlendModeWire::ColorBurn => document::BlendModeWire::ColorBurn,
        timeline::BlendModeWire::LinearBurn => document::BlendModeWire::LinearBurn,
        timeline::BlendModeWire::Overlay => document::BlendModeWire::Overlay,
        timeline::BlendModeWire::SoftLight => document::BlendModeWire::SoftLight,
        timeline::BlendModeWire::HardLight => document::BlendModeWire::HardLight,
        timeline::BlendModeWire::Difference => document::BlendModeWire::Difference,
        timeline::BlendModeWire::Exclusion => document::BlendModeWire::Exclusion,
        timeline::BlendModeWire::Hue => document::BlendModeWire::Hue,
        timeline::BlendModeWire::Saturation => document::BlendModeWire::Saturation,
        timeline::BlendModeWire::Color => document::BlendModeWire::Color,
        timeline::BlendModeWire::Luminosity => document::BlendModeWire::Luminosity,
    }
}

fn lower_media_end(value: timeline::MediaEndBehaviorWire) -> document::MediaEndBehaviorWire {
    match value {
        timeline::MediaEndBehaviorWire::Error => document::MediaEndBehaviorWire::Error,
        timeline::MediaEndBehaviorWire::Hold => document::MediaEndBehaviorWire::Hold,
        timeline::MediaEndBehaviorWire::Loop => document::MediaEndBehaviorWire::Loop,
    }
}

fn lower_raster_fit(value: timeline::RasterFitWire) -> document::RasterFitWire {
    match value {
        timeline::RasterFitWire::Contain => document::RasterFitWire::Contain,
        timeline::RasterFitWire::Cover => document::RasterFitWire::Cover,
        timeline::RasterFitWire::Fill => document::RasterFitWire::Fill,
        timeline::RasterFitWire::None => document::RasterFitWire::None,
    }
}

fn lower_scroll_axis(value: timeline::ScrollAxisWire) -> document::ScrollAxisWire {
    match value {
        timeline::ScrollAxisWire::Horizontal => document::ScrollAxisWire::Horizontal,
        timeline::ScrollAxisWire::Vertical => document::ScrollAxisWire::Vertical,
    }
}

fn lower_karaoke_mode(value: timeline::KaraokeModeWire) -> document::KaraokeModeWire {
    match value {
        timeline::KaraokeModeWire::Word => document::KaraokeModeWire::Word,
        timeline::KaraokeModeWire::Line => document::KaraokeModeWire::Line,
    }
}

fn lower_interpolation(value: timeline::InterpolationWire) -> document::InterpolationWire {
    match value {
        timeline::InterpolationWire::Step => document::InterpolationWire::Step,
        timeline::InterpolationWire::Linear => document::InterpolationWire::Linear,
    }
}

fn lower_easing(value: timeline::EasingWire) -> document::EasingWire {
    match value {
        timeline::EasingWire::Named(value) => document::EasingWire::Named(match value {
            timeline::NamedEasingWire::Linear => document::NamedEasingWire::Linear,
            timeline::NamedEasingWire::Ease => document::NamedEasingWire::Ease,
            timeline::NamedEasingWire::EaseIn => document::NamedEasingWire::EaseIn,
            timeline::NamedEasingWire::EaseOut => document::NamedEasingWire::EaseOut,
            timeline::NamedEasingWire::EaseInOut => document::NamedEasingWire::EaseInOut,
        }),
        timeline::EasingWire::CubicBezier(value) => {
            document::EasingWire::CubicBezier(document::CubicBezierEasingWire {
                kind: document::CubicBezierTag::CubicBezier,
                x1: value.x1,
                y1: value.y1,
                x2: value.x2,
                y2: value.y2,
            })
        }
    }
}

fn lower_caption_align(value: timeline::CaptionAlignWire) -> document::CaptionAlignWire {
    match value {
        timeline::CaptionAlignWire::TopLeft => document::CaptionAlignWire::TopLeft,
        timeline::CaptionAlignWire::TopCenter => document::CaptionAlignWire::TopCenter,
        timeline::CaptionAlignWire::TopRight => document::CaptionAlignWire::TopRight,
        timeline::CaptionAlignWire::CenterLeft => document::CaptionAlignWire::CenterLeft,
        timeline::CaptionAlignWire::Center => document::CaptionAlignWire::Center,
        timeline::CaptionAlignWire::CenterRight => document::CaptionAlignWire::CenterRight,
        timeline::CaptionAlignWire::BottomLeft => document::CaptionAlignWire::BottomLeft,
        timeline::CaptionAlignWire::BottomCenter => document::CaptionAlignWire::BottomCenter,
        timeline::CaptionAlignWire::BottomRight => document::CaptionAlignWire::BottomRight,
    }
}

fn constant<T>(value: T) -> document::ParamWire<T> {
    document::ParamWire::Constant(document::ConstantParamWire { value })
}

fn enter_preset(value: timeline::TimelineEnterPresetWire) -> TimedCaptionPreset {
    timed_preset(value.preset.into(), value.duration)
}

fn exit_preset(value: timeline::TimelineExitPresetWire) -> TimedCaptionPreset {
    timed_preset(value.preset.into(), value.duration)
}

fn timed_preset(
    preset: CaptionPreset,
    duration: Option<timeline::TimelineTimeWire>,
) -> TimedCaptionPreset {
    let duration = duration.map_or_else(
        || {
            preset
                .descriptor()
                .recommended_duration
                .expect("enter and exit presets define a recommended duration")
        },
        |duration| RationalTime::from_exact(exact_time(&duration)),
    );
    TimedCaptionPreset { preset, duration }
}

fn display_preset(value: timeline::TimelineDisplayPresetWire) -> DisplayCaptionPreset {
    let preset: CaptionPreset = value.preset.into();
    let rate = value.rate;
    DisplayCaptionPreset {
        preset,
        rate: rate
            .or_else(|| preset.descriptor().recommended_rate)
            .unwrap_or(1.0),
    }
}

fn map_param<T, U>(
    value: timeline::TimelineParamWire<T>,
    map: impl Fn(T) -> U,
) -> timeline::TimelineParamWire<U> {
    match value {
        timeline::TimelineParamWire::Value(value) => timeline::TimelineParamWire::Value(map(value)),
        timeline::TimelineParamWire::Curve(curve) => {
            timeline::TimelineParamWire::Curve(timeline::TimelineCurveWire {
                interpolation: curve.interpolation,
                keyframes: curve
                    .keyframes
                    .into_iter()
                    .map(|keyframe| match keyframe {
                        timeline::TimelineKeyframeWire::Plain((time, value)) => {
                            timeline::TimelineKeyframeWire::Plain((time, map(value)))
                        }
                        timeline::TimelineKeyframeWire::Eased((time, value, easing)) => {
                            timeline::TimelineKeyframeWire::Eased((time, map(value), easing))
                        }
                    })
                    .collect(),
            })
        }
    }
}
