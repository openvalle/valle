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
    CanonicalTimeline, ExactRational, FrameRate, LocalInvariantReport, RationalTime,
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
    #[error("normalized Timeline violates canonical local invariants")]
    InvalidCanonical(LocalInvariantReport),
}

/// Compile one complete sparse Timeline into Valle's existing internal canonical document.
///
/// This function performs no I/O. URLs stay in the persisted Timeline; Canonical resource
/// references are deterministic `resource:<alias>` IDs that can be resolved by the project layer.
pub fn compile_timeline(timeline: Timeline) -> Result<CanonicalTimeline, CompileTimelineError> {
    let timeline = timeline.into_wire();
    TimelineNormalizer::new(&timeline)?.compile(NormalizationInput { timeline })
}

/// Private normalization carrier. It intentionally has no serde/schema surface.
struct NormalizationInput {
    timeline: timeline::TimelineWire,
}

struct TimelineNormalizer {
    resources: BTreeSet<String>,
    frame_rate: FrameRate,
    canvas_size: [u32; 2],
}

impl TimelineNormalizer {
    fn new(timeline: &timeline::TimelineWire) -> Result<Self, CompileTimelineError> {
        validate_resources(&timeline.resources)?;
        let frame_rate = frame_rate(&timeline.canvas.fps)?;
        Ok(Self {
            resources: timeline.resources.keys().cloned().collect(),
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
            visual.push(self.visual_track(track_index, track.clips)?);
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
    ) -> Result<document::VisualTrackWire, CompileTimelineError> {
        let track_id = track_id("visual", track_index);
        let mut cursor = ExactRational::ZERO;
        let mut items = Vec::new();
        for (clip_index, clip) in clips.into_iter().enumerate() {
            let clip_path = format!("/tracks/visual/{track_index}/clips/{clip_index}");
            let clip_id = clip_id("visual", track_index, clip_index);
            let start = exact_time(&clip.start);
            push_visual_gap(
                &mut items,
                track_index,
                clip_index,
                cursor,
                start,
                &clip_path,
            )?;
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
                        .map(|value| timeline_param_to_canonical(value, &clip_id, "rotation"))
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
        clip_duration: &timeline::TimelineTimeWire,
        track_index: usize,
        clip_index: usize,
        clip_path: &str,
    ) -> Result<document::VisualSourceWire, CompileTimelineError> {
        let source_path = format!("{clip_path}/src");
        Ok(match source {
            timeline::TimelineVisualSourceWire::Video {
                src,
                trim_start,
                rate,
                end,
                fit,
            } => document::VisualSourceWire::Video(document::VideoSourceWire {
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
                trim_start,
                source_duration,
                rate,
                end,
                props,
                cues,
                resources,
                phases,
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
                let cues = cues
                    .into_iter()
                    .map(|(name, cue)| (name, timeline_motion_cue_to_canonical(cue)))
                    .collect();
                document::VisualSourceWire::Motion(document::MotionInstanceWire {
                    component: self.resolve(&component, &format!("{clip_path}/component"))?,
                    source_start: trim_start.as_ref().map_or(ExactRational::ZERO, exact_time),
                    source_duration: source_duration
                        .unwrap_or_else(|| clip_duration.clone())
                        .to_exact(),
                    rate: rate.as_ref().map_or(ExactRational::ONE, exact_time),
                    end_behavior: end
                        .map(lower_media_end)
                        .unwrap_or(document::MediaEndBehaviorWire::Error),
                    props,
                    cues,
                    resources,
                    phases: phases.map_or(
                        document::MotionPhaseOverridesWire {
                            enter_duration: None,
                            exit_duration: None,
                        },
                        timeline_motion_phases_to_canonical,
                    ),
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
        clips: Vec<timeline::TimelineCaptionClipWire>,
    ) -> Result<document::CaptionTrackWire, CompileTimelineError> {
        let font = self.resolve(
            &style.font,
            &format!("/tracks/caption/{track_index}/style/font"),
        )?;
        let style = document::CaptionStyleWire {
            font,
            font_size: style.font_size.unwrap_or(64.0),
            color: style.color.unwrap_or_else(|| "#ffffffff".to_owned()),
            shadow: style.shadow.map(|shadow| document::CaptionShadowWire {
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
                style: style.clone(),
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

fn timeline_motion_cue_to_canonical(
    cue: timeline::TimelineMotionCueBindingWire,
) -> document::MotionCueBindingWire {
    match cue {
        timeline::TimelineMotionCueBindingWire::SourceRange {
            start,
            end,
            enter_duration,
            exit_duration,
        } => document::MotionCueBindingWire::SourceRange {
            start: start.to_exact(),
            end: end.to_exact(),
            enter_duration: enter_duration
                .as_ref()
                .map_or(ExactRational::ZERO, exact_time),
            exit_duration: exit_duration
                .as_ref()
                .map_or(ExactRational::ZERO, exact_time),
        },
    }
}

fn timeline_motion_phases_to_canonical(
    phases: timeline::TimelineMotionPhaseOverridesWire,
) -> document::MotionPhaseOverridesWire {
    document::MotionPhaseOverridesWire {
        enter_duration: phases.enter_duration.as_ref().map(exact_time),
        exit_duration: phases.exit_duration.as_ref().map(exact_time),
    }
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
            .map(|value| timeline_param_to_canonical(value, owner_id, "rotation"))
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
    let (preset, duration) = match value {
        timeline::TimelineEnterPresetWire::Name(name) => (name.into(), None),
        timeline::TimelineEnterPresetWire::Options(options) => {
            (options.preset.into(), options.duration)
        }
    };
    timed_preset(preset, duration)
}

fn exit_preset(value: timeline::TimelineExitPresetWire) -> TimedCaptionPreset {
    let (preset, duration) = match value {
        timeline::TimelineExitPresetWire::Name(name) => (name.into(), None),
        timeline::TimelineExitPresetWire::Options(options) => {
            (options.preset.into(), options.duration)
        }
    };
    timed_preset(preset, duration)
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
    let (preset, rate) = match value {
        timeline::TimelineDisplayPresetWire::Name(name) => (name.into(), None),
        timeline::TimelineDisplayPresetWire::Options(options) => {
            (options.preset.into(), options.rate)
        }
    };
    DisplayCaptionPreset {
        preset,
        rate: rate
            .or_else(|| preset.descriptor().recommended_rate)
            .unwrap_or(1.0),
    }
}
