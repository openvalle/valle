use super::*;

pub(super) fn compile_admitted_timeline(
    admitted: AdmittedTimeline,
    canvas: CompiledCanvas,
    resources: Arc<ResolvedResourceSnapshot>,
) -> CompiledPrograms {
    fn compile_owner_clock(clock: AdmittedOwnerClock) -> CompiledOwnerClock {
        match clock {
            AdmittedOwnerClock::CompositionGlobal => CompiledOwnerClock::CompositionGlobal,
            AdmittedOwnerClock::VisualClipLocal => CompiledOwnerClock::VisualClipLocal,
            AdmittedOwnerClock::AudioClipLocal => CompiledOwnerClock::AudioClipLocal,
            AdmittedOwnerClock::CaptionLocal => CompiledOwnerClock::CaptionLocal,
            AdmittedOwnerClock::MotionSource => CompiledOwnerClock::MotionSource,
        }
    }

    fn compile_easing(easing: AdmittedEasing) -> CompiledEasing {
        match easing {
            AdmittedEasing::Linear => CompiledEasing::Linear,
            AdmittedEasing::CubicBezier { x1, y1, x2, y2 } => {
                CompiledEasing::CubicBezier { x1, y1, x2, y2 }
            }
        }
    }

    fn compile_param<T>(param: AdmittedParam<T>) -> CompiledParam<T> {
        let value = match param.value {
            AdmittedParamValue::Constant { value } => CompiledParamValue::Constant { value },
            AdmittedParamValue::Curve { curve } => CompiledParamValue::Curve {
                curve: CompiledCurve {
                    interpolation: match curve.interpolation {
                        AdmittedInterpolation::Step => CompiledInterpolation::Step,
                        AdmittedInterpolation::Linear => CompiledInterpolation::Linear,
                    },
                    keyframes: curve
                        .keyframes
                        .into_iter()
                        .map(|keyframe| CompiledKeyframe {
                            time: keyframe.time,
                            value: keyframe.value,
                            easing: compile_easing(keyframe.easing),
                        })
                        .collect(),
                },
            },
        };
        CompiledParam {
            owner_clock: compile_owner_clock(param.owner_clock),
            value,
        }
    }

    fn compile_kernel_call(call: AdmittedKernelCall) -> CompiledKernelCall {
        CompiledKernelCall {
            kernel: call.kernel,
            parameters: call.parameters,
        }
    }

    fn compile_motion_param(param: AdmittedMotionParam) -> CompiledMotionParam {
        match param {
            AdmittedMotionParam::Scalar { param } => CompiledMotionParam::Scalar {
                param: compile_param(param),
            },
            AdmittedMotionParam::Vec2 { param } => CompiledMotionParam::Vec2 {
                param: compile_param(param),
            },
            AdmittedMotionParam::Vec4 { param } => CompiledMotionParam::Vec4 {
                param: compile_param(param),
            },
            AdmittedMotionParam::Boolean { value } => CompiledMotionParam::Boolean { value },
            AdmittedMotionParam::String { value } => CompiledMotionParam::String { value },
        }
    }

    fn compile_motion_cue(cue: AdmittedMotionCue) -> CompiledMotionCue {
        match cue {
            AdmittedMotionCue::SourceRange {
                start,
                end,
                enter_duration,
                exit_duration,
            } => CompiledMotionCue::SourceRange {
                start,
                end,
                enter_duration,
                exit_duration,
            },
        }
    }

    fn compile_motion_instance(instance: AdmittedMotionInstance) -> CompiledMotionInstance {
        CompiledMotionInstance {
            component_target: instance.component_target,
            reads_destination: instance.reads_destination,
            props: instance
                .props
                .into_iter()
                .map(|(name, param)| (name, compile_motion_param(param)))
                .collect(),
            cues: instance
                .cues
                .into_iter()
                .map(|(name, cue)| (name, compile_motion_cue(cue)))
                .collect(),
            resources: instance.resources,
            artifact_dependencies: instance
                .artifact_dependencies
                .into_iter()
                .map(|dependency| CompiledMotionArtifactDependency {
                    role: dependency.role,
                    target: dependency.target,
                })
                .collect(),
            phases: CompiledMotionPhases {
                duration_frames: instance.phases.duration_frames,
                enter_frames: instance.phases.enter_frames,
                hold_frames: instance.phases.hold_frames,
                exit_frames: instance.phases.exit_frames,
                hold_cycle_frames: instance.phases.hold_cycle_frames,
            },
            artifact: instance.artifact,
        }
    }

    fn compile_source(source: AdmittedSource) -> CompiledSource {
        let kind = match source.kind {
            AdmittedSourceKind::Video => CompiledSourceKind::Video,
            AdmittedSourceKind::Image => CompiledSourceKind::Image,
            AdmittedSourceKind::Lottie => CompiledSourceKind::Lottie,
            AdmittedSourceKind::Motion => CompiledSourceKind::Motion,
            AdmittedSourceKind::Solid => CompiledSourceKind::Solid,
            AdmittedSourceKind::Audio => CompiledSourceKind::Audio,
        };
        let end_behavior = match source.end_behavior {
            AdmittedEndBehavior::Static => CompiledEndBehavior::Static,
            AdmittedEndBehavior::Error => CompiledEndBehavior::Error,
            AdmittedEndBehavior::Hold => CompiledEndBehavior::Hold,
            AdmittedEndBehavior::Loop => CompiledEndBehavior::Loop,
        };
        let dependency_range = match source.dependency_range {
            AdmittedDependencyRange::Frames { range } => CompiledDependencyRange::Frames { range },
            AdmittedDependencyRange::Samples { range } => {
                CompiledDependencyRange::Samples { range }
            }
            AdmittedDependencyRange::Static => CompiledDependencyRange::Static,
        };
        let compile_fit = |fit| match fit {
            AdmittedRasterFit::Contain => CompiledRasterFit::Contain,
            AdmittedRasterFit::Cover => CompiledRasterFit::Cover,
            AdmittedRasterFit::Fill => CompiledRasterFit::Fill,
            AdmittedRasterFit::None => CompiledRasterFit::None,
        };
        let payload = match source.payload {
            AdmittedSourcePayload::Video { fit } => CompiledSourcePayload::Video {
                fit: compile_fit(fit),
            },
            AdmittedSourcePayload::Image { fit } => CompiledSourcePayload::Image {
                fit: compile_fit(fit),
            },
            AdmittedSourcePayload::Lottie { fit } => CompiledSourcePayload::Lottie {
                fit: compile_fit(fit),
            },
            AdmittedSourcePayload::Motion { instance } => CompiledSourcePayload::Motion {
                instance: compile_motion_instance(instance),
            },
            AdmittedSourcePayload::Solid { color } => CompiledSourcePayload::Solid { color },
            AdmittedSourcePayload::Audio {
                channel_map,
                source_sample_rate,
                source_sample_count,
            } => CompiledSourcePayload::Audio {
                channel_map: match channel_map {
                    AdmittedAudioChannelMap::StereoIdentity => {
                        CompiledAudioChannelMap::StereoIdentity
                    }
                    AdmittedAudioChannelMap::MonoToStereo => CompiledAudioChannelMap::MonoToStereo,
                },
                source_sample_rate,
                source_sample_count,
            },
        };
        CompiledSource {
            kind,
            resource_target: source.resource_target,
            placement_start: source.placement_start,
            source_start: source.source_start,
            source_duration: source.source_duration,
            hold_end_time: source.hold_end_time,
            rate: source.rate,
            end_behavior,
            dependency_range,
            payload,
        }
    }

    fn compile_blend(blend: AdmittedBlendMode) -> CompiledBlendMode {
        match blend {
            AdmittedBlendMode::Normal => CompiledBlendMode::Normal,
            AdmittedBlendMode::Screen => CompiledBlendMode::Screen,
            AdmittedBlendMode::Lighten => CompiledBlendMode::Lighten,
            AdmittedBlendMode::ColorDodge => CompiledBlendMode::ColorDodge,
            AdmittedBlendMode::Multiply => CompiledBlendMode::Multiply,
            AdmittedBlendMode::Darken => CompiledBlendMode::Darken,
            AdmittedBlendMode::ColorBurn => CompiledBlendMode::ColorBurn,
            AdmittedBlendMode::LinearBurn => CompiledBlendMode::LinearBurn,
            AdmittedBlendMode::Overlay => CompiledBlendMode::Overlay,
            AdmittedBlendMode::SoftLight => CompiledBlendMode::SoftLight,
            AdmittedBlendMode::HardLight => CompiledBlendMode::HardLight,
            AdmittedBlendMode::Difference => CompiledBlendMode::Difference,
            AdmittedBlendMode::Exclusion => CompiledBlendMode::Exclusion,
            AdmittedBlendMode::Hue => CompiledBlendMode::Hue,
            AdmittedBlendMode::Saturation => CompiledBlendMode::Saturation,
            AdmittedBlendMode::Color => CompiledBlendMode::Color,
            AdmittedBlendMode::Luminosity => CompiledBlendMode::Luminosity,
        }
    }

    fn compile_mask(mask: AdmittedMask) -> CompiledMask {
        match mask {
            AdmittedMask::Rect {
                rect,
                feather,
                invert,
            } => CompiledMask::Rect {
                rect: compile_param(rect),
                feather: compile_param(feather),
                invert,
            },
            AdmittedMask::Ellipse {
                rect,
                feather,
                invert,
            } => CompiledMask::Ellipse {
                rect: compile_param(rect),
                feather: compile_param(feather),
                invert,
            },
        }
    }

    fn compile_visual_layer(layer: AdmittedVisualLayer) -> CompiledVisualLayer {
        CompiledVisualLayer {
            position: compile_param(layer.position),
            scale: compile_param(layer.scale),
            rotation: compile_param(layer.rotation),
            anchor: layer.anchor,
            opacity: compile_param(layer.opacity),
            mask: layer.mask.map(compile_mask),
            filters: layer.filters.into_iter().map(compile_kernel_call).collect(),
            blend: compile_blend(layer.blend),
            footprint: layer.footprint,
        }
    }

    fn compile_visual_item(item: AdmittedVisualItem) -> CompiledVisualItem {
        match item {
            AdmittedVisualItem::Clip { clip } => CompiledVisualItem::Clip {
                clip: CompiledVisualClip {
                    id: clip.id,
                    range: clip.range,
                    exact_start: clip.exact_start,
                    source: clip.source,
                    layer: compile_visual_layer(clip.layer),
                },
            },
            AdmittedVisualItem::Gap { gap } => CompiledVisualItem::Gap {
                gap: CompiledVisualGap { range: gap.range },
            },
            AdmittedVisualItem::Transition { transition } => CompiledVisualItem::Transition {
                transition: CompiledVisualTransition {
                    window: transition.window,
                    cut_frame: transition.cut_frame,
                    left_frames: transition.left_frames,
                    right_frames: transition.right_frames,
                    from_item: transition.from_item,
                    to_item: transition.to_item,
                    from_source: transition.from_source,
                    to_source: transition.to_source,
                    kernel: match transition.kernel {
                        AdmittedTransitionKernel::CrossFade => CompiledTransitionKernel::CrossFade,
                        AdmittedTransitionKernel::Extension { call } => {
                            CompiledTransitionKernel::Extension {
                                call: compile_kernel_call(call),
                            }
                        }
                    },
                    footprint: transition.footprint,
                },
            },
        }
    }

    fn compile_visual_program(program: AdmittedVisualProgram) -> CompiledVisualProgram {
        CompiledVisualProgram {
            tracks: program
                .tracks
                .into_iter()
                .map(|track| CompiledVisualTrack {
                    id: track.id,
                    order: track.order,
                    items: track.items.into_iter().map(compile_visual_item).collect(),
                })
                .collect(),
        }
    }

    fn compile_adjustment_program(program: AdmittedAdjustmentProgram) -> CompiledAdjustmentProgram {
        CompiledAdjustmentProgram {
            clips: program
                .clips
                .into_iter()
                .map(|clip| CompiledAdjustmentClip {
                    range: clip.range,
                    effect: match clip.effect {
                        AdmittedAdjustmentEffect::ColorGrade { temperature } => {
                            CompiledAdjustmentEffect::ColorGrade { temperature }
                        }
                        AdmittedAdjustmentEffect::Extension { call } => {
                            CompiledAdjustmentEffect::Extension {
                                call: compile_kernel_call(call),
                            }
                        }
                    },
                })
                .collect(),
        }
    }

    fn compile_caption_align(align: AdmittedCaptionAlign) -> CompiledCaptionAlign {
        match align {
            AdmittedCaptionAlign::TopLeft => CompiledCaptionAlign::TopLeft,
            AdmittedCaptionAlign::TopCenter => CompiledCaptionAlign::TopCenter,
            AdmittedCaptionAlign::TopRight => CompiledCaptionAlign::TopRight,
            AdmittedCaptionAlign::CenterLeft => CompiledCaptionAlign::CenterLeft,
            AdmittedCaptionAlign::Center => CompiledCaptionAlign::Center,
            AdmittedCaptionAlign::CenterRight => CompiledCaptionAlign::CenterRight,
            AdmittedCaptionAlign::BottomLeft => CompiledCaptionAlign::BottomLeft,
            AdmittedCaptionAlign::BottomCenter => CompiledCaptionAlign::BottomCenter,
            AdmittedCaptionAlign::BottomRight => CompiledCaptionAlign::BottomRight,
        }
    }

    fn compile_caption_program(program: AdmittedCaptionProgram) -> CompiledCaptionProgram {
        CompiledCaptionProgram {
            tracks: program
                .tracks
                .into_iter()
                .map(|track| CompiledCaptionTrack {
                    order: track.order,
                    items: track
                        .items
                        .into_iter()
                        .map(|item| match item {
                            AdmittedCaptionItem::Clip { clip: caption } => {
                                CompiledCaptionItem::Clip {
                                    clip: CompiledCaptionClip {
                                        range: caption.range,
                                        runs: caption.runs,
                                        run_timings: caption
                                            .run_timings
                                            .into_iter()
                                            .map(|timing| {
                                                timing.map(|timing| CompiledCaptionRunTiming {
                                                    start: timing.start,
                                                    end: timing.end,
                                                })
                                            })
                                            .collect(),
                                        run_styles: caption
                                            .run_styles
                                            .into_iter()
                                            .map(|style| CompiledCaptionRunStyle {
                                                font_size: style.font_size,
                                                color: style.color,
                                                font_weight: style.font_weight,
                                            })
                                            .collect(),
                                        font_target: caption.font_target,
                                        shadow: caption.shadow.map(|shadow| {
                                            CompiledCaptionShadow {
                                                color: shadow.color,
                                                offset: shadow.offset,
                                                blur_sigma: shadow.blur_sigma,
                                            }
                                        }),
                                        region: caption.region,
                                        align: compile_caption_align(caption.align),
                                        presentation: CompiledCaptionPresentation {
                                            opacity: compile_param(caption.presentation.opacity),
                                            translation: compile_param(
                                                caption.presentation.translation,
                                            ),
                                            scale: compile_param(caption.presentation.scale),
                                            rotation: compile_param(caption.presentation.rotation),
                                            clip_inset: compile_param(
                                                caption.presentation.clip_inset,
                                            ),
                                            blur_sigma: compile_param(
                                                caption.presentation.blur_sigma,
                                            ),
                                        },
                                        behavior: caption.behavior.map(|behavior| match behavior {
                                            AdmittedCaptionBehavior::Scroll {
                                                horizontal,
                                                speed,
                                            } => CompiledCaptionBehavior::Scroll {
                                                horizontal,
                                                speed,
                                            },
                                            AdmittedCaptionBehavior::Karaoke { line_mode } => {
                                                CompiledCaptionBehavior::Karaoke { line_mode }
                                            }
                                        }),
                                    },
                                }
                            }
                            AdmittedCaptionItem::Gap { gap } => CompiledCaptionItem::Gap {
                                gap: CompiledCaptionGap { range: gap.range },
                            },
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    fn compile_audio_item(item: AdmittedAudioItem) -> CompiledAudioItem {
        match item {
            AdmittedAudioItem::Clip { clip } => CompiledAudioItem::Clip {
                clip: CompiledAudioClip {
                    range: clip.range,
                    exact_start: clip.exact_start,
                    source: clip.source,
                    effect_multiplier: clip.effect_multiplier,
                    gain: compile_param(clip.gain),
                    pan: compile_param(clip.pan),
                    effects: clip.effects.into_iter().map(compile_kernel_call).collect(),
                    footprint: clip.footprint,
                },
            },
            AdmittedAudioItem::Gap { gap } => CompiledAudioItem::Gap {
                gap: CompiledAudioGap { range: gap.range },
            },
            AdmittedAudioItem::Crossfade { crossfade } => CompiledAudioItem::Crossfade {
                crossfade: CompiledAudioCrossfade {
                    window: crossfade.window,
                    cut_sample: crossfade.cut_sample,
                    left_samples: crossfade.left_samples,
                    right_samples: crossfade.right_samples,
                    from_item: crossfade.from_item,
                    to_item: crossfade.to_item,
                    from_source: crossfade.from_source,
                    to_source: crossfade.to_source,
                },
            },
        }
    }

    fn compile_camera(camera: AdmittedCameraProgram) -> CompiledCameraProgram {
        CompiledCameraProgram {
            center_x: compile_param(camera.center_x),
            center_y: compile_param(camera.center_y),
            zoom: compile_param(camera.zoom),
            rotation: compile_param(camera.rotation),
        }
    }

    let AdmittedTimeline {
        sources: admitted_sources,
        visual,
        adjustments,
        captions,
        audio,
        camera,
    } = admitted;
    let sources = Arc::new(CompiledSourceCatalog {
        entries: admitted_sources.into_iter().map(compile_source).collect(),
    });
    let audio = CompiledAudioProgram {
        tracks: audio
            .tracks
            .into_iter()
            .map(|track| CompiledAudioTrack {
                order: track.order,
                items: track.items.into_iter().map(compile_audio_item).collect(),
            })
            .collect(),
        sample_rate: canvas.sample_rate,
        sample_count: canvas.sample_count,
        sources: Arc::clone(&sources),
        resources,
    };
    CompiledPrograms {
        sources,
        visual: compile_visual_program(visual),
        adjustments: compile_adjustment_program(adjustments),
        captions: compile_caption_program(captions),
        audio,
        camera: camera.map(compile_camera),
    }
}
