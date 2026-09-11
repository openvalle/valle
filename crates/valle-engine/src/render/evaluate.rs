use std::collections::BTreeMap;

use valle_timeline::internal::quantize::quantize_sample_boundary;
use valle_timeline::{FrameRate, RationalTime};

use super::{
    CompiledAudioClip, CompiledAudioItem, CompiledAudioProgram, CompiledCameraProgram,
    CompiledCaptionItem, CompiledCaptionProgram, CompiledEndBehavior, CompiledMask, CompiledSource,
    CompiledSourceCatalog, CompiledVisualClip, CompiledVisualItem, CompiledVisualProgram,
    EvaluatedAudioEndpoint, EvaluatedAudioSample, EvaluatedAudioTrack, EvaluatedCamera,
    EvaluatedCaption, EvaluatedCaptionPresentation, EvaluatedResourceRef, EvaluatedSourceRef,
    EvaluatedVisualClip, EvaluatedVisualLayer, EvaluatedVisualMask, EvaluatedVisualOperation,
    EvaluatedVisualTransition, EvaluationClocks, FrameRange, MappedSourceTime,
    ResolvedResourceSnapshot, RuntimeFault, SampleRange, audio_gain,
};

pub(super) fn cubic_bezier_progress(progress: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    if progress <= 0.0 {
        return 0.0;
    }
    if progress >= 1.0 {
        return 1.0;
    }
    fn coordinate(t: f64, first: f64, second: f64) -> f64 {
        let inverse = 1.0 - t;
        3.0 * inverse * inverse * t * first + 3.0 * inverse * t * t * second + t * t * t
    }
    // A fixed iteration count is deterministic across preview/export and does
    // not make convergence part of the authoring document.
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..32 {
        let midpoint = (low + high) * 0.5;
        if coordinate(midpoint, x1, x2) < progress {
            low = midpoint;
        } else {
            high = midpoint;
        }
    }
    coordinate((low + high) * 0.5, y1, y2).clamp(0.0, 1.0)
}

pub(super) fn frame_sample_time(
    frame: i64,
    frame_rate: FrameRate,
) -> Result<RationalTime, RuntimeFault> {
    RationalTime::new(frame, 1)
        .and_then(|frame| {
            frame.checked_div(RationalTime::new(
                frame_rate.numerator(),
                frame_rate.denominator(),
            )?)
        })
        .map_err(|_| RuntimeFault::ExactTimeOverflow)
}

pub(super) fn sample_time(sample: i64, sample_rate: u32) -> Result<RationalTime, RuntimeFault> {
    RationalTime::new(sample, sample_rate).map_err(|_| RuntimeFault::ExactTimeOverflow)
}

pub(super) fn euclidean_source_modulo(
    value: RationalTime,
    modulus: RationalTime,
) -> Result<RationalTime, ()> {
    if !modulus.is_positive() {
        return Err(());
    }
    let numerator = (value.numerator() as i128)
        .checked_mul(modulus.denominator() as i128)
        .ok_or(())?;
    let denominator = (value.denominator() as i128)
        .checked_mul(modulus.numerator() as i128)
        .ok_or(())?;
    let quotient = numerator.div_euclid(denominator);
    let quotient = i64::try_from(quotient).map_err(|_| ())?;
    value
        .checked_sub(
            modulus
                .checked_mul(RationalTime::new(quotient, 1).map_err(|_| ())?)
                .map_err(|_| ())?,
        )
        .map_err(|_| ())
}

pub(super) fn transition_progress(frame: i64, window: FrameRange) -> Option<RationalTime> {
    if !window.contains(frame) {
        return None;
    }
    if window.len() == 1 {
        return RationalTime::new(1, 2).ok();
    }
    RationalTime::new(frame - window.start, (window.len() - 1) as u32).ok()
}

pub(super) fn crossfade_progress(sample: i64, window: SampleRange) -> Option<RationalTime> {
    if !window.contains(sample) {
        return None;
    }
    if window.len() == 1 {
        return RationalTime::new(1, 2).ok();
    }
    RationalTime::new(sample - window.start, (window.len() - 1) as u32).ok()
}

pub(super) fn evaluate_camera(
    camera: &CompiledCameraProgram,
    composition: RationalTime,
) -> EvaluatedCamera {
    let clocks = EvaluationClocks {
        composition,
        clip: composition,
        motion: composition,
    };
    EvaluatedCamera {
        center_x: camera.center_x.evaluate(clocks),
        center_y: camera.center_y.evaluate(clocks),
        zoom: camera.zoom.evaluate(clocks),
        rotation: camera.rotation.evaluate(clocks),
    }
}

pub(super) fn evaluate_visual_program(
    program: &CompiledVisualProgram,
    sources: &CompiledSourceCatalog,
    snapshot: &ResolvedResourceSnapshot,
    frame: i64,
    composition_time: RationalTime,
    resources: &mut Vec<EvaluatedResourceRef>,
) -> Result<Vec<EvaluatedVisualOperation>, RuntimeFault> {
    let mut operations = Vec::new();
    for track in &program.tracks {
        let active_transition = track.items.iter().find_map(|item| match item {
            CompiledVisualItem::Transition { transition } if transition.window.contains(frame) => {
                Some(transition)
            }
            _ => None,
        });
        if let Some(transition) = active_transition {
            let from_clip = visual_clip_at(&track.items, transition.from_item)
                .expect("admitted transition left endpoint exists");
            let to_clip = visual_clip_at(&track.items, transition.to_item)
                .expect("admitted transition right endpoint exists");
            let progress = transition
                .progress_at(frame)
                .expect("active admitted transition has progress");
            let from = evaluate_source_ref(
                sources,
                snapshot,
                transition.from_source,
                composition_time,
                &format!("visual/{}/transition/from", track.order),
                resources,
            )?;
            let to = evaluate_source_ref(
                sources,
                snapshot,
                transition.to_source,
                composition_time,
                &format!("visual/{}/transition/to", track.order),
                resources,
            )?;
            let progress_f64 = progress.as_f64();
            operations.push(EvaluatedVisualOperation::Transition(
                EvaluatedVisualTransition {
                    track_id: track.id.clone(),
                    from_clip_id: from_clip.id.clone(),
                    to_clip_id: to_clip.id.clone(),
                    track_order: track.order,
                    window: transition.window,
                    progress,
                    from_weight: 1.0 - progress_f64,
                    to_weight: progress_f64,
                    from,
                    to,
                    from_layer: evaluate_visual_layer(from_clip, composition_time)?,
                    to_layer: evaluate_visual_layer(to_clip, composition_time)?,
                    kernel: transition.kernel.clone(),
                },
            ));
            continue;
        }
        if let Some(clip) = track.items.iter().find_map(|item| match item {
            CompiledVisualItem::Clip { clip } if clip.range.contains(frame) => Some(clip),
            _ => None,
        }) {
            operations.push(EvaluatedVisualOperation::Clip(EvaluatedVisualClip {
                clip_id: clip.id.clone(),
                track_id: track.id.clone(),
                track_order: track.order,
                source: evaluate_source_ref(
                    sources,
                    snapshot,
                    clip.source,
                    composition_time,
                    &format!("visual/{}/clip", track.order),
                    resources,
                )?,
                layer: evaluate_visual_layer(clip, composition_time)?,
            }));
        }
    }
    Ok(operations)
}

pub(super) fn visual_clip_at(
    items: &[CompiledVisualItem],
    index: u32,
) -> Option<&CompiledVisualClip> {
    match items.get(index as usize)? {
        CompiledVisualItem::Clip { clip } => Some(clip),
        _ => None,
    }
}

fn evaluate_visual_layer(
    clip: &CompiledVisualClip,
    composition_time: RationalTime,
) -> Result<EvaluatedVisualLayer, RuntimeFault> {
    let clip_time = composition_time
        .checked_sub(clip.exact_start)
        .map_err(|_| RuntimeFault::ExactTimeOverflow)?;
    let clocks = EvaluationClocks {
        composition: composition_time,
        clip: clip_time,
        motion: clip_time,
    };
    let mask = clip.layer.mask.as_ref().map(|mask| match mask {
        CompiledMask::Rect {
            rect,
            feather,
            invert,
        } => EvaluatedVisualMask::Rect {
            rect: rect.evaluate(clocks),
            feather: feather.evaluate(clocks),
            invert: *invert,
        },
        CompiledMask::Ellipse {
            rect,
            feather,
            invert,
        } => EvaluatedVisualMask::Ellipse {
            rect: rect.evaluate(clocks),
            feather: feather.evaluate(clocks),
            invert: *invert,
        },
    });
    Ok(EvaluatedVisualLayer {
        size: clip.layer.size.as_ref().map(|size| size.evaluate(clocks)),
        position: clip.layer.position.evaluate(clocks),
        scale: clip.layer.scale.evaluate(clocks),
        rotation: clip.layer.rotation.evaluate(clocks),
        anchor: clip.layer.anchor,
        opacity: clip.layer.opacity.evaluate(clocks),
        blend: clip.layer.blend,
        mask,
        filters: clip.layer.filters.clone(),
    })
}

fn evaluate_source_ref(
    sources: &CompiledSourceCatalog,
    snapshot: &ResolvedResourceSnapshot,
    source_index: u32,
    composition_time: RationalTime,
    role: &str,
    evaluated_resources: &mut Vec<EvaluatedResourceRef>,
) -> Result<EvaluatedSourceRef, RuntimeFault> {
    let source = sources
        .source(source_index)
        .expect("admitted source index exists");
    let mapped_time = source.map_time(composition_time)?;
    let sample_time = match mapped_time {
        MappedSourceTime::Exact(time) => time,
        MappedSourceTime::Static | MappedSourceTime::HoldStart => RationalTime::ZERO,
        MappedSourceTime::HoldEnd => source
            .hold_end_time
            .ok_or(RuntimeFault::VisualSourceHoldEndMissing { source_index })?,
    };
    let motion_props = source.motion().map(|motion| {
        motion.evaluate_props(
            mapped_time,
            source
                .source_duration
                .expect("admitted Motion sources have a finite duration"),
        )
    });
    let mut motion_resources = BTreeMap::new();
    let mut motion_artifact_dependencies = Vec::new();
    if let Some(motion) = source.motion() {
        for (name, target) in &motion.resources {
            let evaluated =
                evaluate_resource_ref(snapshot, *target, &format!("{role}/motion-resource/{name}"));
            evaluated_resources.push(evaluated.clone());
            motion_resources.insert(name.clone(), evaluated);
        }
        for dependency in &motion.artifact_dependencies {
            let evaluated = evaluate_resource_ref(
                snapshot,
                dependency.target,
                &format!("{role}/motion-artifact/{}", dependency.role),
            );
            evaluated_resources.push(evaluated.clone());
            motion_artifact_dependencies.push(evaluated);
        }
    }
    let resource = source.resource_target.map(|target| {
        let evaluated = evaluate_resource_ref(snapshot, target, role);
        evaluated_resources.push(evaluated.clone());
        evaluated
    });
    Ok(EvaluatedSourceRef {
        source_index,
        kind: source.kind,
        mapped_time,
        sample_time,
        resource,
        motion_props,
        motion_resources,
        motion_artifact_dependencies,
    })
}

fn evaluate_resource_ref(
    snapshot: &ResolvedResourceSnapshot,
    target: u32,
    role: &str,
) -> EvaluatedResourceRef {
    let resource = snapshot
        .resources
        .get(target as usize)
        .expect("admitted resource target exists");
    EvaluatedResourceRef {
        target,
        resource_id: resource.resource_id.clone(),
        role: role.to_owned(),
        kind: resource.kind,
        digest: resource.digest.clone(),
        handle: resource.handle,
        facts: resource.facts.clone(),
    }
}

pub(super) fn evaluate_caption_program(
    program: &CompiledCaptionProgram,
    snapshot: &ResolvedResourceSnapshot,
    frame: i64,
    composition_time: RationalTime,
    frame_rate: FrameRate,
    resources: &mut Vec<EvaluatedResourceRef>,
) -> Result<Vec<EvaluatedCaption>, RuntimeFault> {
    let mut evaluated = Vec::new();
    for track in &program.tracks {
        for item in &track.items {
            let CompiledCaptionItem::Clip { clip: caption } = item else {
                continue;
            };
            if !caption.range.contains(frame) {
                continue;
            }
            let font = evaluate_resource_ref(snapshot, caption.font_target, "caption/font");
            resources.push(font.clone());
            let local_frame = frame
                .checked_sub(caption.range.start())
                .ok_or(RuntimeFault::ExactTimeOverflow)?;
            let local_time = frame_sample_time(local_frame, frame_rate)?;
            let clocks = EvaluationClocks {
                composition: composition_time,
                clip: local_time,
                motion: local_time,
            };
            evaluated.push(EvaluatedCaption {
                track_order: track.order,
                range: caption.range,
                local_time,
                runs: caption.runs.clone(),
                run_timings: caption.run_timings.clone(),
                run_styles: caption.run_styles.clone(),
                font,
                shadow: caption.shadow.clone(),
                region: caption.region,
                align: caption.align,
                presentation: EvaluatedCaptionPresentation {
                    opacity: caption.presentation.opacity.evaluate(clocks),
                    translation: caption.presentation.translation.evaluate(clocks),
                    scale: caption.presentation.scale.evaluate(clocks),
                    rotation: caption.presentation.rotation.evaluate(clocks),
                    clip_inset: caption.presentation.clip_inset.evaluate(clocks),
                    blur_sigma: caption.presentation.blur_sigma.evaluate(clocks),
                },
                behavior: caption.behavior.clone(),
            });
        }
    }
    Ok(evaluated)
}

pub(super) fn evaluate_audio_sample(
    program: &CompiledAudioProgram,
    sample: i64,
) -> Result<EvaluatedAudioSample, RuntimeFault> {
    let composition_time = sample_time(sample, program.sample_rate)?;
    let mut tracks = Vec::new();
    for track in &program.tracks {
        let active_crossfade = track.items.iter().find_map(|item| match item {
            CompiledAudioItem::Crossfade { crossfade } if crossfade.window.contains(sample) => {
                Some(crossfade)
            }
            _ => None,
        });
        let endpoints = if let Some(crossfade) = active_crossfade {
            let from = audio_clip_at(&track.items, crossfade.from_item)
                .expect("admitted crossfade left endpoint exists");
            let to = audio_clip_at(&track.items, crossfade.to_item)
                .expect("admitted crossfade right endpoint exists");
            let progress = crossfade
                .progress_at(sample)
                .expect("active admitted crossfade has progress")
                .as_f64();
            vec![
                evaluate_audio_endpoint(program, from, composition_time, 1.0 - progress)?,
                evaluate_audio_endpoint(program, to, composition_time, progress)?,
            ]
        } else if let Some(clip) = track.items.iter().find_map(|item| match item {
            CompiledAudioItem::Clip { clip } if clip.range.contains(sample) => Some(clip),
            _ => None,
        }) {
            vec![evaluate_audio_endpoint(
                program,
                clip,
                composition_time,
                1.0,
            )?]
        } else {
            Vec::new()
        };
        if !endpoints.is_empty() {
            tracks.push(EvaluatedAudioTrack {
                track_order: track.order,
                endpoints,
            });
        }
    }
    Ok(EvaluatedAudioSample {
        sample,
        sample_time: composition_time,
        tracks,
    })
}

fn audio_clip_at(items: &[CompiledAudioItem], index: u32) -> Option<&CompiledAudioClip> {
    match items.get(index as usize)? {
        CompiledAudioItem::Clip { clip } => Some(clip),
        _ => None,
    }
}

fn evaluate_audio_endpoint(
    program: &CompiledAudioProgram,
    clip: &CompiledAudioClip,
    composition_time: RationalTime,
    crossfade_gain: f64,
) -> Result<EvaluatedAudioEndpoint, RuntimeFault> {
    let source = program
        .sources
        .source(clip.source)
        .expect("admitted audio source exists");
    let clip_time = composition_time
        .checked_sub(clip.exact_start)
        .map_err(|_| RuntimeFault::ExactTimeOverflow)?;
    let clocks = EvaluationClocks {
        composition: composition_time,
        clip: clip_time,
        motion: clip_time,
    };
    let gain = clip.gain.evaluate(clocks);
    let pan = clip.pan.evaluate(clocks);
    // Common audio ABI stage order is fixed: sample-local effects, authored
    // gain, constant-power-free linear pan, then transition crossfade. Keep
    // all intermediates in f64; platform mixers quantize only their terminal
    // PCM sample.
    let after_effect_and_gain = audio_gain::apply_multiplier(clip.effect_multiplier, gain);
    let left_after_pan = after_effect_and_gain * (1.0 - pan.max(0.0));
    let right_after_pan = after_effect_and_gain * (1.0 + pan.min(0.0));
    let mapped_time = source.map_time(composition_time)?;
    let source_sample_index = evaluate_source_sample_index(source, mapped_time)?;
    let sample_time = RationalTime::new(
        source_sample_index,
        source
            .audio_source_sample_rate()
            .expect("admitted audio source has a sample rate"),
    )
    .map_err(|_| RuntimeFault::ExactTimeOverflow)?;
    let resource = source
        .resource_target
        .map(|target| evaluate_resource_ref(&program.resources, target, "audio/clip"));
    Ok(EvaluatedAudioEndpoint {
        source: EvaluatedSourceRef {
            source_index: clip.source,
            kind: source.kind,
            mapped_time,
            sample_time,
            resource,
            motion_props: None,
            motion_resources: BTreeMap::new(),
            motion_artifact_dependencies: Vec::new(),
        },
        source_sample_index,
        crossfade_gain,
        effect_multiplier: clip.effect_multiplier,
        gain,
        pan,
        left_gain: left_after_pan * crossfade_gain,
        right_gain: right_after_pan * crossfade_gain,
    })
}

pub(super) fn evaluate_source_sample_index(
    source: &CompiledSource,
    mapped_time: MappedSourceTime,
) -> Result<i64, RuntimeFault> {
    let sample_rate = source
        .audio_source_sample_rate()
        .expect("admitted audio source carries its source clock");
    let sample_count = source
        .audio_source_sample_count()
        .expect("admitted audio source carries its source extent");
    debug_assert!(sample_count > 0);
    let selected = match mapped_time {
        MappedSourceTime::Static | MappedSourceTime::HoldStart => 0,
        MappedSourceTime::HoldEnd => sample_count - 1,
        MappedSourceTime::Exact(time) => {
            let sample = quantize_sample_boundary(time, sample_rate)
                .map_err(|_| RuntimeFault::ExactTimeOverflow)?;
            match source.end_behavior {
                CompiledEndBehavior::Loop => sample.rem_euclid(sample_count),
                CompiledEndBehavior::Hold => sample.clamp(0, sample_count - 1),
                CompiledEndBehavior::Error | CompiledEndBehavior::Static => sample,
            }
        }
    };
    if selected < 0 || selected >= sample_count {
        return Err(RuntimeFault::SourceSampleOutOfRange {
            sample: selected,
            sample_count,
        });
    }
    Ok(selected)
}
