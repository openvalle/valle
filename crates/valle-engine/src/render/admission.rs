use super::*;

mod compile;

pub(super) fn open_engine_render(
    timeline: &CanonicalTimeline,
    manifest: &ResourceManifest,
    bindings: &ResourceBindings,
    capabilities: &Capabilities,
    profile: &ExecutionProfile,
) -> Result<CompiledRender, EngineOpenReport> {
    let mut diagnostics = Vec::new();
    let (root_uses, kernel_requirements) = collect_document_requirements(timeline);
    let limits = profile.limits();

    if timeline.document().camera.is_some() && !capabilities.camera {
        diagnostics.push(diagnostic(
            EngineOpenDiagnosticCode::UnsupportedCamera,
            "/document/camera",
            EngineOpenPhase::Admission,
            None,
            BTreeMap::new(),
        ));
    }

    // `common` has one closed output mix layout. This is a canvas/profile
    // invariant even when the document happens to contain no audio clips.
    if timeline.document().canvas.channel_layout != ChannelLayout::Stereo {
        diagnostics.push(diagnostic(
            EngineOpenDiagnosticCode::UnsupportedAudioChannelLayout,
            "/document/canvas/channelLayout",
            EngineOpenPhase::Admission,
            None,
            details([
                ("actual", "mono".to_owned()),
                ("required", "stereo".to_owned()),
                ("reason", "common-canvas-layout".to_owned()),
            ]),
        ));
    }

    if !timeline.document().captions.tracks.is_empty()
        && (profile.caption_abi() != CAPTION_GLYPH_RUN_ABI || !cfg!(feature = "text"))
    {
        diagnostics.push(diagnostic(
            EngineOpenDiagnosticCode::UnsupportedCaptionBackend,
            "/document/captions",
            EngineOpenPhase::Admission,
            None,
            details([
                ("profileAbi", profile.caption_abi().to_owned()),
                ("requiredAbi", CAPTION_GLYPH_RUN_ABI.to_owned()),
            ]),
        ));
    }

    if kernel_requirements.len() > limits.max_extension_kernels as usize {
        diagnostics.push(diagnostic(
            EngineOpenDiagnosticCode::ExtensionKernelBudgetExceeded,
            "/document",
            EngineOpenPhase::Admission,
            None,
            details([("limit", limits.max_extension_kernels.to_string())]),
        ));
    }
    let mut kernels = Vec::with_capacity(kernel_requirements.len());
    for kernel in &kernel_requirements {
        let Some(capability) = capabilities.extension_kernel(&kernel.kind) else {
            diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::UnsupportedExtensionKernel,
                &kernel.role,
                EngineOpenPhase::Admission,
                None,
                details([("kernel", kernel.kind.clone())]),
            ));
            continue;
        };
        let supported_abi = match kernel.domain {
            CompiledKernelDomain::VisualFilter | CompiledKernelDomain::AdjustmentEffect => {
                Some(EXTENSION_COLOR_GAIN_ABI)
            }
            CompiledKernelDomain::VisualTransition => Some(EXTENSION_CROSS_FADE_ABI),
            CompiledKernelDomain::AudioEffect if kernel.kind.as_str() == AUDIO_GAIN_EFFECT_KIND => {
                Some(AUDIO_GAIN_EFFECT_ABI)
            }
            CompiledKernelDomain::AudioEffect => None,
        };
        if supported_abi != Some(capability.abi.as_str()) {
            diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::UnsupportedExtensionKernel,
                &kernel.role,
                EngineOpenPhase::Admission,
                None,
                details([
                    ("kernel", kernel.kind.clone()),
                    ("abi", capability.abi.clone()),
                    ("supportedAbi", supported_abi.unwrap_or("none").to_owned()),
                ]),
            ));
            continue;
        }
        let expected_implementation = engine_owned_kernel_implementation_digest(&capability.abi)
            .expect("every supported engine-owned ABI has an implementation artifact");
        if capability.implementation_digest != expected_implementation {
            diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::UnsupportedExtensionKernel,
                &kernel.role,
                EngineOpenPhase::Admission,
                None,
                details([
                    ("kernel", kernel.kind.clone()),
                    ("abi", capability.abi.clone()),
                    (
                        "implementationDigest",
                        capability.implementation_digest.to_string(),
                    ),
                    (
                        "expectedImplementationDigest",
                        expected_implementation.to_string(),
                    ),
                ]),
            ));
            continue;
        }
        if !extension_parameters_valid(&capability.abi, &kernel.parameters) {
            diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::InvalidExtensionParameters,
                &kernel.role,
                EngineOpenPhase::Admission,
                None,
                details([
                    ("kernel", kernel.kind.clone()),
                    ("abi", capability.abi.clone()),
                ]),
            ));
            continue;
        }
        kernels.push(AdmittedKernel {
            role: kernel.role.clone(),
            kind: kernel.kind.clone(),
            domain: kernel.domain,
            abi: capability.abi.clone(),
            implementation_digest: capability.implementation_digest.clone(),
            visual_footprint: capability.visual_footprint,
            audio_footprint: capability.audio_footprint,
        });
    }

    let mut resolver = Resolver::new(manifest, bindings, capabilities, limits);
    let mut roots = Vec::with_capacity(root_uses.len());
    for resource_use in &root_uses {
        if let Some(target) = resolver.resolve(
            &resource_use.resource_id,
            resource_use.expected_kind,
            &resource_use.path,
            0,
        ) {
            roots.push(ResolvedRootProjection {
                role: resource_use.role.clone(),
                target,
            });
        }
    }
    diagnostics.append(&mut resolver.diagnostics);
    if profile.audio_abi() != COMMON_AUDIO_ABI
        && (!timeline.document().audio.tracks.is_empty()
            || resolver
                .resources
                .iter()
                .any(|resource| resource.kind == ResourceKind::Audio))
    {
        diagnostics.push(diagnostic(
            EngineOpenDiagnosticCode::UnsupportedAudioExecutionProfile,
            "/document/audio",
            EngineOpenPhase::Admission,
            None,
            details([
                ("profileAbi", profile.audio_abi().to_owned()),
                ("requiredAbi", COMMON_AUDIO_ABI.to_owned()),
            ]),
        ));
    }

    let canvas = admit_canvas(timeline, &mut diagnostics);
    if let Some(canvas) = canvas {
        validate_quantized_intervals(timeline, canvas, &mut diagnostics);
    }
    validate_common_font_face_indices(&resolver.resources, &mut diagnostics);
    validate_resolved_admission(
        timeline.document(),
        &resolver.index_by_id,
        &resolver.resources,
        &mut diagnostics,
    );
    if !diagnostics.is_empty() {
        return Err(EngineOpenReport { diagnostics });
    }
    let canvas = canvas.expect("a diagnostic is emitted when canvas compilation fails");

    let kernel_by_role = kernels
        .iter()
        .enumerate()
        .map(|(index, kernel)| (kernel.role.clone(), index as u32))
        .collect();
    validate_kernel_footprints_admission(
        timeline.document(),
        &kernel_by_role,
        &kernels,
        &mut diagnostics,
    );
    if !diagnostics.is_empty() {
        return Err(EngineOpenReport { diagnostics });
    }

    let admitted = admit_timeline_programs(
        timeline.document(),
        canvas,
        &resolver.index_by_id,
        &resolver.resources,
        &kernel_by_role,
        &kernels,
        &mut diagnostics,
    );
    if !diagnostics.is_empty() {
        return Err(EngineOpenReport { diagnostics });
    }
    let admitted = admitted
        .ok_or_else(|| internal_compile_report("/document", "timeline-admission-lowering"))?;
    let snapshot = Arc::new(ResolvedResourceSnapshot {
        resources: resolver.resources,
        roots,
    });
    let compiled = compile::compile_admitted_timeline(admitted, canvas, Arc::clone(&snapshot));
    let render_projection = RenderProjection {
        profile: RenderProfileProjection {
            numeric_abi: &profile.projection.numeric_abi,
            audio_abi: &profile.projection.audio_abi,
            motion_abi: &profile.projection.motion_abi,
            caption_abi: &profile.projection.caption_abi,
        },
        resources: SnapshotProjection {
            roots: &snapshot.roots,
            resources: snapshot
                .resources
                .iter()
                .map(|resource| ResolvedResourceProjection {
                    kind: resource.kind,
                    digest: &resource.digest,
                    facts: &resource.facts,
                    entry: &resource.entry,
                    dependencies: &resource.dependencies,
                })
                .collect(),
        },
        canvas,
        sources: &compiled.sources,
        visual: &compiled.visual,
        adjustments: &compiled.adjustments,
        captions: &compiled.captions,
        audio: &compiled.audio,
        camera: &compiled.camera,
        kernels: &kernels,
    };
    let render_bytes = render_projection_bytes(&render_projection)
        .map_err(|_| internal_compile_report("/document", "render-identity-projection"))?;
    let render_id = RenderId::from_canonical_bytes(&render_bytes);

    Ok(CompiledRender {
        render_id,
        canvas,
        sources: compiled.sources,
        visual: compiled.visual,
        adjustments: compiled.adjustments,
        captions: compiled.captions,
        audio: compiled.audio,
        camera: compiled.camera,
        kernels,
        resources: snapshot,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SnapshotProjection<'a> {
    roots: &'a [ResolvedRootProjection],
    resources: Vec<ResolvedResourceProjection<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenderProjection<'a> {
    profile: RenderProfileProjection<'a>,
    resources: SnapshotProjection<'a>,
    canvas: CompiledCanvas,
    sources: &'a CompiledSourceCatalog,
    visual: &'a CompiledVisualProgram,
    adjustments: &'a CompiledAdjustmentProgram,
    captions: &'a CompiledCaptionProgram,
    audio: &'a CompiledAudioProgram,
    camera: &'a Option<CompiledCameraProgram>,
    kernels: &'a [AdmittedKernel],
}

/// Only execution semantics belong to RenderId. Producer-facing profile names
/// and admission budgets can change whether an input is accepted, but cannot
/// change the pixels or samples of an already accepted input.
#[derive(Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenderProfileProjection<'a> {
    numeric_abi: &'a str,
    audio_abi: &'a str,
    motion_abi: &'a str,
    caption_abi: &'a str,
}

fn render_projection_bytes(
    projection: &RenderProjection<'_>,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_jcs::to_vec(projection)
}

/// The initial execution profile is shared by Native and CanvasKit. CanvasKit 0.41 exposes only
/// `MakeFreeTypeFaceFromData(bytes)` and therefore cannot select a non-zero face from a TTC/OTC
/// collection. Keep the common contract honest by rejecting such a resource before any compiled
/// program can name it; silently decoding face zero would render a different concrete font on Web.
fn validate_common_font_face_indices(
    resources: &[ResolvedResource],
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) {
    for resource in resources {
        let VerifiedResourceFacts::Font { descriptor, .. } = &resource.facts else {
            continue;
        };
        if descriptor.face_index == 0 {
            continue;
        }
        diagnostics.push(diagnostic(
            EngineOpenDiagnosticCode::UnsupportedFontFaceIndex,
            format!(
                "/resources/{}/descriptor/faceIndex",
                json_pointer_segment(&resource.resource_id)
            ),
            EngineOpenPhase::Admission,
            Some(&resource.resource_id),
            details([
                ("actual", descriptor.face_index.to_string()),
                ("supported", "0".to_owned()),
                ("reason", "native-web-common-profile".to_owned()),
            ]),
        ));
    }
}

fn json_pointer_segment(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

/// Closes resource-dependent authoring checks before program admission. Once this function and
/// `admit_timeline_programs` succeed, the typed compiler boundary cannot produce a late
/// schema/capability rejection.
fn validate_resolved_admission(
    document: &TimelineDocument,
    resource_targets: &BTreeMap<String, u32>,
    resources: &[ResolvedResource],
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) {
    for (effect_index, effect) in document.adjustments.iter().enumerate() {
        if let AdjustmentEffect::ColorGrade(effect) = &effect.effect
            && !(-1.0..=1.0).contains(&effect.temperature)
        {
            diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::InvalidAdjustmentEffectParameters,
                format!("/document/adjustments/{effect_index}/effect/temperature"),
                EngineOpenPhase::Admission,
                None,
                details([("reason", "temperature-out-of-range".to_owned())]),
            ));
        }
    }

    for (track_index, track) in document.visual.tracks.iter().enumerate() {
        for (item_index, item) in track.items.iter().enumerate() {
            let VisualItem::Clip(clip) = item else {
                continue;
            };
            let VisualSource::Lottie(source) = &clip.source else {
                continue;
            };
            if source.end_behavior != MediaEndBehavior::Hold {
                continue;
            }
            let Some(target) = resource_targets.get(&source.resource).copied() else {
                continue;
            };
            let Some(VerifiedResourceFacts::Lottie { descriptor, .. }) = resources
                .get(target as usize)
                .map(|resource| &resource.facts)
            else {
                continue;
            };
            if lottie_hold_end_boundary(descriptor).is_err() {
                diagnostics.push(diagnostic(
                    EngineOpenDiagnosticCode::InvalidIntervalQuantization,
                    format!(
                        "/document/visual/tracks/{track_index}/items/{item_index}/source/resource"
                    ),
                    EngineOpenPhase::Admission,
                    Some(&source.resource),
                    details([
                        (
                            "reason",
                            "hold-end-descriptor-boundary-clock-overflow".to_owned(),
                        ),
                        ("duration", descriptor.duration.to_string()),
                        ("timeBase", descriptor.time_base.to_string()),
                        ("boundarySampling", "left-limit".to_owned()),
                    ]),
                ));
            }
        }
    }

    for (track_index, track) in document.captions.tracks.iter().enumerate() {
        for (item_index, item) in track.items.iter().enumerate() {
            let CaptionItem::Clip(caption) = item else {
                continue;
            };
            let caption_path =
                format!("/document/captions/tracks/{track_index}/items/{item_index}");
            if let Some(target) = resource_targets.get(&caption.style.font).copied() {
                validate_caption_font_admission(
                    caption,
                    target,
                    resources,
                    &caption_path,
                    diagnostics,
                );
            }
        }
    }

    for (track_index, track) in document.audio.tracks.iter().enumerate() {
        for (item_index, item) in track.items.iter().enumerate() {
            let AudioItem::Clip(clip) = item else {
                continue;
            };
            let AudioSource::Media(media) = &clip.source;
            if let Some(target) = resource_targets.get(&media.resource).copied()
                && let Some(ResolvedResource {
                    facts: VerifiedResourceFacts::Audio { descriptor, .. },
                    ..
                }) = resources.get(target as usize)
            {
                let reason =
                    match quantize_sample_boundary(descriptor.duration, descriptor.sample_rate) {
                        Ok(count) if count <= 0 => Some("source-sample-count-quantized-to-zero"),
                        Ok(count) if count > MAX_SAFE_JSON_INTEGER => {
                            Some("source-sample-count-outside-safe-range")
                        }
                        Ok(_) => None,
                        Err(_) => Some("source-sample-count-overflow"),
                    };
                if let Some(reason) = reason {
                    diagnostics.push(diagnostic(
                        EngineOpenDiagnosticCode::InvalidIntervalQuantization,
                        format!(
                            "/document/audio/tracks/{track_index}/items/{item_index}/source/resource"
                        ),
                        EngineOpenPhase::Admission,
                        Some(&media.resource),
                        details([
                            ("reason", reason.to_owned()),
                            ("sampleRate", descriptor.sample_rate.to_string()),
                            ("duration", descriptor.duration.to_string()),
                        ]),
                    ));
                }
            }
            let mut combined = 1.0_f64;
            for (effect_index, effect) in clip.effects.iter().enumerate() {
                let Some(multiplier) = effect
                    .parameters
                    .get("multiplier")
                    .and_then(JsonValue::as_f64)
                else {
                    continue;
                };
                let Some(next) = audio_gain::combine_multiplier(combined, multiplier) else {
                    diagnostics.push(diagnostic(
                        EngineOpenDiagnosticCode::InvalidExtensionParameters,
                        format!(
                            "/document/audio/tracks/{track_index}/items/{item_index}/effects/{effect_index}"
                        ),
                        EngineOpenPhase::Admission,
                        None,
                        details([
                            ("kernel", effect.kind.as_str().to_owned()),
                            ("reason", "composed-multiplier-not-finite".to_owned()),
                        ]),
                    ));
                    break;
                };
                combined = next;
            }
        }
    }
}

/// Validates the actual sequential kernel chains before the compiler may consume them. Individual
/// capability footprints are valid in isolation, but their Minkowski sum can overflow the frozen
/// visual representation or the interoperable audio safe-integer range.
fn validate_kernel_footprints_admission(
    document: &TimelineDocument,
    kernel_targets: &BTreeMap<String, u32>,
    kernels: &[AdmittedKernel],
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) {
    for (track_index, track) in document.visual.tracks.iter().enumerate() {
        let mut clip_footprints = vec![None; track.items.len()];
        for (item_index, item) in track.items.iter().enumerate() {
            let VisualItem::Clip(clip) = item else {
                continue;
            };
            let mut footprint = VisualFootprint::default();
            let mut valid = true;
            for (filter_index, _) in clip.layer.filters.iter().enumerate() {
                let role = format!(
                    "/document/visual/tracks/{track_index}/items/{item_index}/layer/filters/{filter_index}"
                );
                let Some(kernel) = admitted_kernel_for_role(&role, kernel_targets, kernels) else {
                    continue;
                };
                let Some(combined) =
                    checked_add_visual_footprints(footprint, kernel.visual_footprint)
                else {
                    diagnostics.push(kernel_footprint_admission_diagnostic(
                        &role,
                        "visual-temporal-footprint-overflow",
                        None,
                    ));
                    valid = false;
                    break;
                };
                footprint = combined;
            }
            if valid {
                clip_footprints[item_index] = Some(footprint);
            }
        }

        for (item_index, item) in track.items.iter().enumerate() {
            let VisualItem::Transition(transition) = item else {
                continue;
            };
            let TransitionKernel::Extension(_) = &transition.kernel else {
                continue;
            };
            let role = format!("/document/visual/tracks/{track_index}/items/{item_index}/kernel");
            let Some(kernel) = admitted_kernel_for_role(&role, kernel_targets, kernels) else {
                continue;
            };
            for (endpoint, endpoint_index) in [
                ("left", item_index.checked_sub(1)),
                ("right", item_index.checked_add(1)),
            ] {
                let Some(layer_footprint) = endpoint_index
                    .and_then(|index| clip_footprints.get(index))
                    .copied()
                    .flatten()
                else {
                    continue;
                };
                if checked_add_visual_footprints(layer_footprint, kernel.visual_footprint).is_none()
                {
                    diagnostics.push(kernel_footprint_admission_diagnostic(
                        &role,
                        "visual-transition-temporal-footprint-overflow",
                        Some(endpoint),
                    ));
                }
            }
        }
    }

    for (track_index, track) in document.audio.tracks.iter().enumerate() {
        for (item_index, item) in track.items.iter().enumerate() {
            let AudioItem::Clip(clip) = item else {
                continue;
            };
            let mut footprint = AudioFootprint::default();
            for (effect_index, _) in clip.effects.iter().enumerate() {
                let role = format!(
                    "/document/audio/tracks/{track_index}/items/{item_index}/effects/{effect_index}"
                );
                let Some(kernel) = admitted_kernel_for_role(&role, kernel_targets, kernels) else {
                    continue;
                };
                let Some(combined) =
                    checked_add_audio_footprints(footprint, kernel.audio_footprint)
                else {
                    diagnostics.push(kernel_footprint_admission_diagnostic(
                        &role,
                        "audio-temporal-footprint-overflow",
                        None,
                    ));
                    break;
                };
                footprint = combined;
            }
        }
    }
}

fn admitted_kernel_for_role<'a>(
    role: &str,
    kernel_targets: &BTreeMap<String, u32>,
    kernels: &'a [AdmittedKernel],
) -> Option<&'a AdmittedKernel> {
    kernel_targets
        .get(role)
        .and_then(|target| kernels.get(*target as usize))
}

fn kernel_footprint_admission_diagnostic(
    path: &str,
    reason: &str,
    endpoint: Option<&str>,
) -> EngineOpenDiagnostic {
    let mut diagnostic_details = details([("reason", reason.to_owned())]);
    if let Some(endpoint) = endpoint {
        diagnostic_details.insert("endpoint".to_owned(), endpoint.to_owned());
    }
    diagnostic(
        EngineOpenDiagnosticCode::ExtensionKernelBudgetExceeded,
        path,
        EngineOpenPhase::Admission,
        None,
        diagnostic_details,
    )
}

struct CompiledPrograms {
    sources: Arc<CompiledSourceCatalog>,
    visual: CompiledVisualProgram,
    adjustments: CompiledAdjustmentProgram,
    captions: CompiledCaptionProgram,
    audio: CompiledAudioProgram,
    camera: Option<CompiledCameraProgram>,
}

fn motion_hold_end_frame_time(
    duration: RationalTime,
    frame_rate: FrameRate,
) -> Result<RationalTime, ()> {
    let frame_count = quantized_motion_source_frame_count(duration, frame_rate)?;
    frame_sample_time(frame_count - 1, frame_rate).map_err(|_| ())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmittedOwnerClock {
    CompositionGlobal,
    VisualClipLocal,
    AudioClipLocal,
    CaptionLocal,
    MotionSource,
}

#[derive(Debug, Clone)]
struct AdmittedParam<T> {
    owner_clock: AdmittedOwnerClock,
    value: AdmittedParamValue<T>,
}

#[derive(Debug, Clone)]
enum AdmittedParamValue<T> {
    Constant { value: T },
    Curve { curve: AdmittedCurve<T> },
}

#[derive(Debug, Clone)]
struct AdmittedCurve<T> {
    interpolation: AdmittedInterpolation,
    keyframes: Vec<AdmittedKeyframe<T>>,
}

#[derive(Debug, Clone)]
struct AdmittedKeyframe<T> {
    time: RationalTime,
    value: T,
    easing: AdmittedEasing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmittedInterpolation {
    Step,
    Linear,
}

#[derive(Debug, Clone, Copy)]
enum AdmittedEasing {
    Linear,
    CubicBezier { x1: f64, y1: f64, x2: f64, y2: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmittedEndBehavior {
    Static,
    Error,
    Hold,
    Loop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmittedRasterFit {
    Contain,
    Cover,
    Fill,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmittedAudioChannelMap {
    StereoIdentity,
    MonoToStereo,
}

#[derive(Debug, Clone)]
enum AdmittedMotionParam {
    Scalar { param: AdmittedParam<f64> },
    Vec2 { param: AdmittedParam<[f64; 2]> },
    Vec4 { param: AdmittedParam<[f64; 4]> },
    Boolean { value: bool },
    String { value: String },
}

#[derive(Debug, Clone)]
enum AdmittedMotionCue {
    SourceRange {
        start: RationalTime,
        end: RationalTime,
        enter_duration: RationalTime,
        exit_duration: RationalTime,
    },
}

#[derive(Debug, Clone, Copy)]
struct AdmittedMotionPhases {
    duration_frames: u32,
    enter_frames: u32,
    hold_frames: u32,
    exit_frames: u32,
    hold_cycle_frames: Option<u32>,
}

impl AdmittedMotionPhases {
    fn from_layout(layout: valle_motion::PhaseLayout) -> Self {
        Self {
            duration_frames: layout.duration_frames,
            enter_frames: layout.enter_frames,
            hold_frames: layout.hold_frames,
            exit_frames: layout.exit_frames,
            hold_cycle_frames: layout.hold_cycle_frames,
        }
    }

    const fn duration_frames(self) -> u32 {
        self.duration_frames
    }
}

#[derive(Debug, Clone)]
struct AdmittedMotionArtifactDependency {
    role: String,
    target: u32,
}

#[derive(Debug, Clone)]
struct AdmittedMotionInstance {
    component_target: u32,
    reads_destination: bool,
    props: BTreeMap<String, AdmittedMotionParam>,
    cues: BTreeMap<String, AdmittedMotionCue>,
    resources: BTreeMap<String, u32>,
    artifact_dependencies: Vec<AdmittedMotionArtifactDependency>,
    phases: AdmittedMotionPhases,
    artifact: Arc<SceneArtifact>,
}

impl AdmittedMotionInstance {
    const fn phases(&self) -> AdmittedMotionPhases {
        self.phases
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmittedSourceKind {
    Video,
    Image,
    Lottie,
    Motion,
    Solid,
    Audio,
}

#[derive(Debug, Clone)]
enum AdmittedSourcePayload {
    Video {
        fit: AdmittedRasterFit,
    },
    Image {
        fit: AdmittedRasterFit,
    },
    Lottie {
        fit: AdmittedRasterFit,
    },
    Motion {
        instance: AdmittedMotionInstance,
    },
    Solid {
        color: String,
    },
    Audio {
        channel_map: AdmittedAudioChannelMap,
        source_sample_rate: u32,
        source_sample_count: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmittedDependencyRange {
    Frames { range: FrameRange },
    Samples { range: SampleRange },
    Static,
}

#[derive(Debug, Clone)]
struct AdmittedSource {
    kind: AdmittedSourceKind,
    resource_target: Option<u32>,
    placement_start: RationalTime,
    source_start: RationalTime,
    source_duration: Option<RationalTime>,
    hold_end_time: Option<RationalTime>,
    rate: RationalRate,
    end_behavior: AdmittedEndBehavior,
    dependency_range: AdmittedDependencyRange,
    payload: AdmittedSourcePayload,
}

impl AdmittedSource {
    fn audio_source_sample_rate(&self) -> Option<u32> {
        match &self.payload {
            AdmittedSourcePayload::Audio {
                source_sample_rate, ..
            } => Some(*source_sample_rate),
            _ => None,
        }
    }

    fn audio_source_sample_count(&self) -> Option<i64> {
        match &self.payload {
            AdmittedSourcePayload::Audio {
                source_sample_count,
                ..
            } => Some(*source_sample_count),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct AdmittedKernelCall {
    kernel: u32,
    parameters: JsonObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmittedBlendMode {
    Normal,
    Screen,
    Lighten,
    ColorDodge,
    Multiply,
    Darken,
    ColorBurn,
    LinearBurn,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

#[derive(Debug, Clone)]
enum AdmittedMask {
    Rect {
        rect: AdmittedParam<[f64; 4]>,
        feather: AdmittedParam<f64>,
        invert: bool,
    },
    Ellipse {
        rect: AdmittedParam<[f64; 4]>,
        feather: AdmittedParam<f64>,
        invert: bool,
    },
}

#[derive(Debug, Clone)]
struct AdmittedVisualLayer {
    position: AdmittedParam<[f64; 2]>,
    scale: AdmittedParam<[f64; 2]>,
    rotation: AdmittedParam<f64>,
    anchor: [f64; 2],
    opacity: AdmittedParam<f64>,
    mask: Option<AdmittedMask>,
    filters: Vec<AdmittedKernelCall>,
    blend: AdmittedBlendMode,
    footprint: VisualFootprint,
}

#[derive(Debug, Clone)]
struct AdmittedVisualClip {
    id: String,
    range: FrameRange,
    exact_start: RationalTime,
    source: u32,
    layer: AdmittedVisualLayer,
}

#[derive(Debug, Clone, Copy)]
struct AdmittedVisualGap {
    range: FrameRange,
}

#[derive(Debug, Clone)]
enum AdmittedTransitionKernel {
    CrossFade,
    Extension { call: AdmittedKernelCall },
}

#[derive(Debug, Clone)]
struct AdmittedVisualTransition {
    window: FrameRange,
    cut_frame: i64,
    left_frames: i64,
    right_frames: i64,
    from_item: u32,
    to_item: u32,
    from_source: u32,
    to_source: u32,
    kernel: AdmittedTransitionKernel,
    footprint: VisualFootprint,
}

#[derive(Debug, Clone)]
enum AdmittedVisualItem {
    Clip {
        clip: AdmittedVisualClip,
    },
    Gap {
        gap: AdmittedVisualGap,
    },
    Transition {
        transition: AdmittedVisualTransition,
    },
}

#[derive(Debug, Clone)]
struct AdmittedVisualTrack {
    id: String,
    order: u32,
    items: Vec<AdmittedVisualItem>,
}

#[derive(Debug, Clone)]
struct AdmittedVisualProgram {
    tracks: Vec<AdmittedVisualTrack>,
}

#[derive(Debug, Clone)]
enum AdmittedAdjustmentEffect {
    ColorGrade { temperature: f64 },
    Extension { call: AdmittedKernelCall },
}

#[derive(Debug, Clone)]
struct AdmittedAdjustmentClip {
    range: FrameRange,
    effect: AdmittedAdjustmentEffect,
}

#[derive(Debug, Clone)]
struct AdmittedAdjustmentProgram {
    clips: Vec<AdmittedAdjustmentClip>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmittedCaptionAlign {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

#[derive(Debug, Clone)]
enum AdmittedCaptionBehavior {
    Scroll { horizontal: bool, speed: f64 },
    Karaoke { line_mode: bool },
}

#[derive(Debug, Clone)]
struct AdmittedCaptionClip {
    range: FrameRange,
    runs: Vec<String>,
    run_timings: Vec<Option<AdmittedCaptionRunTiming>>,
    run_styles: Vec<AdmittedCaptionRunStyle>,
    font_target: u32,
    shadow: Option<AdmittedCaptionShadow>,
    region: [f64; 4],
    align: AdmittedCaptionAlign,
    presentation: AdmittedCaptionPresentation,
    behavior: Option<AdmittedCaptionBehavior>,
}

#[derive(Debug, Clone, Copy)]
struct AdmittedCaptionRunTiming {
    start: RationalTime,
    end: RationalTime,
}

#[derive(Debug, Clone)]
struct AdmittedCaptionRunStyle {
    font_size: f64,
    color: String,
    font_weight: u16,
}

#[derive(Debug, Clone)]
struct AdmittedCaptionShadow {
    color: String,
    offset: [f64; 2],
    blur_sigma: f64,
}

#[derive(Debug, Clone)]
struct AdmittedCaptionPresentation {
    opacity: AdmittedParam<f64>,
    translation: AdmittedParam<[f64; 2]>,
    scale: AdmittedParam<f64>,
    rotation: AdmittedParam<f64>,
    clip_inset: AdmittedParam<[f64; 4]>,
    blur_sigma: AdmittedParam<f64>,
}

#[derive(Debug, Clone, Copy)]
struct AdmittedCaptionGap {
    range: FrameRange,
}

#[derive(Debug, Clone)]
enum AdmittedCaptionItem {
    Clip { clip: AdmittedCaptionClip },
    Gap { gap: AdmittedCaptionGap },
}

#[derive(Debug, Clone)]
struct AdmittedCaptionTrack {
    order: u32,
    items: Vec<AdmittedCaptionItem>,
}

#[derive(Debug, Clone)]
struct AdmittedCaptionProgram {
    tracks: Vec<AdmittedCaptionTrack>,
}

#[derive(Debug, Clone)]
struct AdmittedAudioClip {
    range: SampleRange,
    exact_start: RationalTime,
    source: u32,
    effect_multiplier: f64,
    gain: AdmittedParam<f64>,
    pan: AdmittedParam<f64>,
    effects: Vec<AdmittedKernelCall>,
    footprint: AudioFootprint,
}

#[derive(Debug, Clone, Copy)]
struct AdmittedAudioGap {
    range: SampleRange,
}

#[derive(Debug, Clone, Copy)]
struct AdmittedAudioCrossfade {
    window: SampleRange,
    cut_sample: i64,
    left_samples: i64,
    right_samples: i64,
    from_item: u32,
    to_item: u32,
    from_source: u32,
    to_source: u32,
}

#[derive(Debug, Clone)]
enum AdmittedAudioItem {
    Clip { clip: AdmittedAudioClip },
    Gap { gap: AdmittedAudioGap },
    Crossfade { crossfade: AdmittedAudioCrossfade },
}

#[derive(Debug, Clone)]
struct AdmittedAudioTrack {
    order: u32,
    items: Vec<AdmittedAudioItem>,
}

#[derive(Debug, Clone)]
struct AdmittedAudioProgram {
    tracks: Vec<AdmittedAudioTrack>,
}

#[derive(Debug, Clone)]
struct AdmittedCameraProgram {
    center_x: AdmittedParam<f64>,
    center_y: AdmittedParam<f64>,
    zoom: AdmittedParam<f64>,
    rotation: AdmittedParam<f64>,
}

/// Engine-private type-state boundary. Every authoring program has an independent admitted IR;
/// no `Compiled*Program` can exist until the final linker consumes this complete value. Admission
/// resolves every resource/capability reference and materializes discrete ranges, while compile
/// performs the one-way conversion into the immutable execution graph.
struct AdmittedTimeline {
    sources: Vec<AdmittedSource>,
    visual: AdmittedVisualProgram,
    adjustments: AdmittedAdjustmentProgram,
    captions: AdmittedCaptionProgram,
    audio: AdmittedAudioProgram,
    camera: Option<AdmittedCameraProgram>,
}

fn admit_timeline_programs(
    document: &TimelineDocument,
    canvas: CompiledCanvas,
    resource_targets: &BTreeMap<String, u32>,
    resources: &[ResolvedResource],
    kernel_targets: &BTreeMap<String, u32>,
    kernels: &[AdmittedKernel],
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> Option<AdmittedTimeline> {
    let mut sources = Vec::new();
    let visual = admit_visual_program(
        &document.visual,
        canvas,
        resource_targets,
        resources,
        kernel_targets,
        kernels,
        &mut sources,
        diagnostics,
    )?;
    let adjustments = admit_adjustment_program(
        &document.adjustments,
        canvas,
        kernel_targets,
        kernels,
        diagnostics,
    )?;
    let captions = admit_caption_program(
        &document.captions,
        canvas,
        resource_targets,
        resources,
        diagnostics,
    )?;
    let audio = admit_audio_program(
        &document.audio,
        canvas,
        resource_targets,
        resources,
        kernel_targets,
        kernels,
        &mut sources,
        diagnostics,
    )?;
    let camera = document
        .camera
        .as_ref()
        .map(|camera| admit_camera_program(camera, diagnostics));

    diagnostics.is_empty().then_some(AdmittedTimeline {
        sources,
        visual,
        adjustments,
        captions,
        audio: AdmittedAudioProgram { tracks: audio },
        camera,
    })
}

fn admit_param<T: Clone>(
    param: &Param<T>,
    owner_clock: AdmittedOwnerClock,
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> AdmittedParam<T> {
    admit_param_mapped(param, owner_clock, path, diagnostics, Clone::clone)
}

fn admit_param_mapped<T, U>(
    param: &Param<T>,
    owner_clock: AdmittedOwnerClock,
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
    mut map_value: impl FnMut(&T) -> U,
) -> AdmittedParam<U> {
    let value = match param {
        Param::Constant(constant) => AdmittedParamValue::Constant {
            value: map_value(&constant.value),
        },
        Param::Curve(curve) => {
            if curve.keyframes.is_empty() {
                diagnostics.push(diagnostic(
                    EngineOpenDiagnosticCode::InternalCompileFault,
                    path,
                    EngineOpenPhase::Compile,
                    None,
                    details([("reason", "empty-keyframes".to_owned())]),
                ));
            }
            let interpolation = match curve.interpolation {
                Interpolation::Step => AdmittedInterpolation::Step,
                Interpolation::Linear => AdmittedInterpolation::Linear,
            };
            let keyframes = curve
                .keyframes
                .iter()
                .map(|keyframe| AdmittedKeyframe {
                    time: keyframe.time,
                    value: map_value(&keyframe.value),
                    easing: admit_easing(keyframe.out_easing.as_ref()),
                })
                .collect();
            AdmittedParamValue::Curve {
                curve: AdmittedCurve {
                    interpolation,
                    keyframes,
                },
            }
        }
    };
    AdmittedParam { owner_clock, value }
}

fn admit_easing(easing: Option<&Easing>) -> AdmittedEasing {
    match easing {
        None | Some(Easing::Named(NamedEasing::Linear)) => AdmittedEasing::Linear,
        Some(Easing::Named(NamedEasing::Ease)) => AdmittedEasing::CubicBezier {
            x1: 0.25,
            y1: 0.1,
            x2: 0.25,
            y2: 1.0,
        },
        Some(Easing::Named(NamedEasing::EaseIn)) => AdmittedEasing::CubicBezier {
            x1: 0.42,
            y1: 0.0,
            x2: 1.0,
            y2: 1.0,
        },
        Some(Easing::Named(NamedEasing::EaseOut)) => AdmittedEasing::CubicBezier {
            x1: 0.0,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        },
        Some(Easing::Named(NamedEasing::EaseInOut)) => AdmittedEasing::CubicBezier {
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        },
        Some(Easing::CubicBezier(bezier)) => AdmittedEasing::CubicBezier {
            x1: bezier.x1,
            y1: bezier.y1,
            x2: bezier.x2,
            y2: bezier.y2,
        },
    }
}

fn extension_parameters_valid(abi: &str, parameters: &JsonObject) -> bool {
    match abi {
        EXTENSION_COLOR_GAIN_ABI => {
            parameters.len() == 1
                && parameters
                    .get("gain")
                    .and_then(JsonValue::as_f64)
                    .is_some_and(|gain| gain.is_finite() && (0.0..=16.0).contains(&gain))
        }
        EXTENSION_CROSS_FADE_ABI => parameters.is_empty(),
        AUDIO_GAIN_EFFECT_ABI => {
            parameters.len() == 1
                && parameters
                    .get("multiplier")
                    .and_then(JsonValue::as_f64)
                    .is_some_and(audio_gain::valid_multiplier)
        }
        _ => false,
    }
}

fn admit_camera_program(
    camera: &CameraTrack,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> AdmittedCameraProgram {
    AdmittedCameraProgram {
        center_x: admit_param(
            &camera.center_x,
            AdmittedOwnerClock::CompositionGlobal,
            "/document/camera/centerX",
            diagnostics,
        ),
        center_y: admit_param(
            &camera.center_y,
            AdmittedOwnerClock::CompositionGlobal,
            "/document/camera/centerY",
            diagnostics,
        ),
        zoom: admit_param(
            &camera.zoom,
            AdmittedOwnerClock::CompositionGlobal,
            "/document/camera/zoom",
            diagnostics,
        ),
        rotation: admit_param(
            &camera.rotation,
            AdmittedOwnerClock::CompositionGlobal,
            "/document/camera/rotation",
            diagnostics,
        ),
    }
}

fn admit_kernel_call(
    role: &str,
    parameters: &JsonObject,
    kernel_targets: &BTreeMap<String, u32>,
    kernels: &[AdmittedKernel],
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> AdmittedKernelCall {
    let kernel = kernel_targets.get(role).copied().unwrap_or_else(|| {
        diagnostics.push(diagnostic(
            EngineOpenDiagnosticCode::InternalCompileFault,
            role,
            EngineOpenPhase::Compile,
            None,
            details([("reason", "missing-admitted-kernel".to_owned())]),
        ));
        0
    });
    if let Some(descriptor) = kernels.get(kernel as usize) {
        debug_assert!(extension_parameters_valid(&descriptor.abi, parameters));
    }
    AdmittedKernelCall {
        kernel,
        parameters: parameters.clone(),
    }
}

fn admit_visual_program(
    visual: &VisualComposition,
    canvas: CompiledCanvas,
    resource_targets: &BTreeMap<String, u32>,
    resources: &[ResolvedResource],
    kernel_targets: &BTreeMap<String, u32>,
    kernels: &[AdmittedKernel],
    sources: &mut Vec<AdmittedSource>,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> Option<AdmittedVisualProgram> {
    let mut tracks = Vec::with_capacity(visual.tracks.len());
    let mut transition_sources = BTreeSet::new();

    for (track_index, track) in visual.tracks.iter().enumerate() {
        let mut cursor = RationalTime::ZERO;
        let mut cut_times = Vec::with_capacity(track.items.len());
        let mut items: Vec<Option<AdmittedVisualItem>> = Vec::with_capacity(track.items.len());
        for (item_index, item) in track.items.iter().enumerate() {
            cut_times.push(cursor);
            let path = format!("/document/visual/tracks/{track_index}/items/{item_index}");
            match item {
                VisualItem::Clip(clip) => {
                    let duration = clip.duration;
                    let interval = quantize_frame_interval(cursor, duration, canvas.frame_rate)
                        .map_err(|_| ())
                        .ok()?;
                    let range = FrameRange::new(interval.start_frame, interval.end_frame);
                    let source = admit_visual_source(
                        &clip.source,
                        cursor,
                        range,
                        canvas.frame_rate,
                        resource_targets,
                        resources,
                        sources,
                        &format!("{path}/source"),
                        diagnostics,
                    );
                    let layer = admit_visual_layer(
                        &clip.layer,
                        track_index,
                        item_index,
                        kernel_targets,
                        kernels,
                        diagnostics,
                    );
                    if let Some(expanded) = expand_frame_range(range, layer.footprint) {
                        extend_visual_dependency(sources, source, expanded);
                    } else {
                        diagnostics.push(admit_fault(&path, "visual-filter-footprint-overflow"));
                    }
                    items.push(Some(AdmittedVisualItem::Clip {
                        clip: AdmittedVisualClip {
                            id: clip.id.clone(),
                            range,
                            exact_start: cursor,
                            source,
                            layer,
                        },
                    }));
                    cursor = cursor.checked_add(duration).ok()?;
                }
                VisualItem::Gap(gap) => {
                    let duration = gap.duration;
                    let interval = quantize_frame_interval(cursor, duration, canvas.frame_rate)
                        .map_err(|_| ())
                        .ok()?;
                    items.push(Some(AdmittedVisualItem::Gap {
                        gap: AdmittedVisualGap {
                            range: FrameRange::new(interval.start_frame, interval.end_frame),
                        },
                    }));
                    cursor = cursor.checked_add(duration).ok()?;
                }
                VisualItem::Transition(_) => items.push(None),
            }
        }

        let mut endpoint_windows: BTreeMap<usize, Vec<FrameRange>> = BTreeMap::new();
        for (item_index, item) in track.items.iter().enumerate() {
            let VisualItem::Transition(transition) = item else {
                continue;
            };
            let path = format!("/document/visual/tracks/{track_index}/items/{item_index}");
            let Some(from_index) = item_index.checked_sub(1) else {
                diagnostics.push(admit_fault(&path, "transition-left-endpoint"));
                continue;
            };
            let to_index = item_index + 1;
            let Some(AdmittedVisualItem::Clip { clip: from_clip }) =
                items.get(from_index).and_then(Option::as_ref).cloned()
            else {
                diagnostics.push(admit_fault(&path, "transition-left-endpoint"));
                continue;
            };
            let Some(AdmittedVisualItem::Clip { clip: to_clip }) =
                items.get(to_index).and_then(Option::as_ref).cloned()
            else {
                diagnostics.push(admit_fault(&path, "transition-right-endpoint"));
                continue;
            };
            let duration = transition.duration;
            let n = quantize_frame_interval(RationalTime::ZERO, duration, canvas.frame_rate)
                .ok()?
                .duration_frames;
            let cut_frame =
                quantize_frame_boundary(cut_times[item_index], canvas.frame_rate).ok()?;
            let left_frames = n / 2;
            let right_frames = n - left_frames;
            let window = FrameRange::new(
                cut_frame.checked_sub(left_frames)?,
                cut_frame.checked_add(right_frames)?,
            );
            if window.start < 0 || window.end > canvas.frame_count {
                diagnostics.push(diagnostic(
                    EngineOpenDiagnosticCode::TransitionWindowOutOfCanvas,
                    &path,
                    EngineOpenPhase::Admission,
                    None,
                    range_details(window.start, window.end),
                ));
            }
            if left_frames > from_clip.range.len() || right_frames > to_clip.range.len() {
                diagnostics.push(diagnostic(
                    EngineOpenDiagnosticCode::TransitionInsufficientHandle,
                    &path,
                    EngineOpenPhase::Admission,
                    None,
                    details([
                        ("leftFrames", left_frames.to_string()),
                        ("rightFrames", right_frames.to_string()),
                    ]),
                ));
            }
            if from_clip.layer.blend != AdmittedBlendMode::Normal
                || to_clip.layer.blend != AdmittedBlendMode::Normal
            {
                diagnostics.push(diagnostic(
                    EngineOpenDiagnosticCode::TransitionEndpointLayerUnsupported,
                    &path,
                    EngineOpenPhase::Admission,
                    None,
                    BTreeMap::new(),
                ));
            }
            for endpoint in [from_index, to_index] {
                if endpoint_windows
                    .get(&endpoint)
                    .is_some_and(|ranges| ranges.iter().any(|range| range.intersects(window)))
                {
                    diagnostics.push(diagnostic(
                        EngineOpenDiagnosticCode::OverlappingTransitionEndpoint,
                        &path,
                        EngineOpenPhase::Admission,
                        None,
                        details([("endpointItem", endpoint.to_string())]),
                    ));
                }
                endpoint_windows.entry(endpoint).or_default().push(window);
            }

            let kernel = match &transition.kernel {
                TransitionKernel::CrossFade => AdmittedTransitionKernel::CrossFade,
                TransitionKernel::Extension(extension) => AdmittedTransitionKernel::Extension {
                    call: admit_kernel_call(
                        &format!("{path}/kernel"),
                        &extension.parameters,
                        kernel_targets,
                        kernels,
                        diagnostics,
                    ),
                },
            };
            let transition_footprint = match &kernel {
                AdmittedTransitionKernel::CrossFade => VisualFootprint::default(),
                AdmittedTransitionKernel::Extension { call } => kernels
                    .get(call.kernel as usize)
                    .map(|kernel| kernel.visual_footprint)
                    .unwrap_or_default(),
            };
            for (clip, source_index) in [(&from_clip, from_clip.source), (&to_clip, to_clip.source)]
            {
                let endpoint_footprint = add_visual_footprints(
                    clip.layer.footprint,
                    transition_footprint,
                    &path,
                    diagnostics,
                );
                if let Some(expanded) = expand_frame_range(window, endpoint_footprint) {
                    extend_visual_dependency(sources, source_index, expanded);
                } else {
                    diagnostics.push(admit_fault(&path, "transition-footprint-overflow"));
                }
                transition_sources.insert(source_index);
            }
            items[item_index] = Some(AdmittedVisualItem::Transition {
                transition: AdmittedVisualTransition {
                    window,
                    cut_frame,
                    left_frames,
                    right_frames,
                    from_item: from_index as u32,
                    to_item: to_index as u32,
                    from_source: from_clip.source,
                    to_source: to_clip.source,
                    kernel,
                    footprint: transition_footprint,
                },
            });
        }
        let items = items.into_iter().collect::<Option<Vec<_>>>()?;
        tracks.push(AdmittedVisualTrack {
            id: track.id.clone(),
            order: track_index as u32,
            items,
        });
    }

    finalize_visual_dependencies(sources, resources, canvas, &transition_sources, diagnostics);

    Some(AdmittedVisualProgram { tracks })
}

fn admit_visual_layer(
    layer: &VisualLayer,
    track_index: usize,
    item_index: usize,
    kernel_targets: &BTreeMap<String, u32>,
    kernels: &[AdmittedKernel],
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> AdmittedVisualLayer {
    let path = format!("/document/visual/tracks/{track_index}/items/{item_index}/layer");
    let transform = &layer.transform;
    let mask = layer.mask.as_ref().map(|mask| match mask {
        LayerMask::Rect {
            rect,
            feather,
            invert,
        } => AdmittedMask::Rect {
            rect: admit_param_mapped(
                rect,
                AdmittedOwnerClock::VisualClipLocal,
                &format!("{path}/mask/rect"),
                diagnostics,
                |rect| *rect.as_array(),
            ),
            feather: admit_param(
                feather,
                AdmittedOwnerClock::VisualClipLocal,
                &format!("{path}/mask/feather"),
                diagnostics,
            ),
            invert: *invert,
        },
        LayerMask::Ellipse {
            rect,
            feather,
            invert,
        } => AdmittedMask::Ellipse {
            rect: admit_param_mapped(
                rect,
                AdmittedOwnerClock::VisualClipLocal,
                &format!("{path}/mask/rect"),
                diagnostics,
                |rect| *rect.as_array(),
            ),
            feather: admit_param(
                feather,
                AdmittedOwnerClock::VisualClipLocal,
                &format!("{path}/mask/feather"),
                diagnostics,
            ),
            invert: *invert,
        },
    });
    let filters: Vec<AdmittedKernelCall> = layer
        .filters
        .iter()
        .enumerate()
        .map(|(filter_index, filter)| {
            admit_kernel_call(
                &format!("{path}/filters/{filter_index}"),
                &filter.parameters,
                kernel_targets,
                kernels,
                diagnostics,
            )
        })
        .collect();
    let footprint = combined_visual_footprint(&filters, kernels, &path, diagnostics);
    AdmittedVisualLayer {
        position: admit_param(
            &transform.position,
            AdmittedOwnerClock::VisualClipLocal,
            &format!("{path}/transform/position"),
            diagnostics,
        ),
        scale: admit_param(
            &transform.scale,
            AdmittedOwnerClock::VisualClipLocal,
            &format!("{path}/transform/scale"),
            diagnostics,
        ),
        rotation: admit_param(
            &transform.rotation,
            AdmittedOwnerClock::VisualClipLocal,
            &format!("{path}/transform/rotation"),
            diagnostics,
        ),
        anchor: transform.anchor,
        opacity: admit_param(
            &layer.opacity,
            AdmittedOwnerClock::VisualClipLocal,
            &format!("{path}/opacity"),
            diagnostics,
        ),
        mask,
        filters,
        blend: admit_blend(layer.blend),
        footprint,
    }
}

fn admit_blend(blend: BlendMode) -> AdmittedBlendMode {
    match blend {
        BlendMode::Normal => AdmittedBlendMode::Normal,
        BlendMode::Screen => AdmittedBlendMode::Screen,
        BlendMode::Lighten => AdmittedBlendMode::Lighten,
        BlendMode::ColorDodge => AdmittedBlendMode::ColorDodge,
        BlendMode::Multiply => AdmittedBlendMode::Multiply,
        BlendMode::Darken => AdmittedBlendMode::Darken,
        BlendMode::ColorBurn => AdmittedBlendMode::ColorBurn,
        BlendMode::LinearBurn => AdmittedBlendMode::LinearBurn,
        BlendMode::Overlay => AdmittedBlendMode::Overlay,
        BlendMode::SoftLight => AdmittedBlendMode::SoftLight,
        BlendMode::HardLight => AdmittedBlendMode::HardLight,
        BlendMode::Difference => AdmittedBlendMode::Difference,
        BlendMode::Exclusion => AdmittedBlendMode::Exclusion,
        BlendMode::Hue => AdmittedBlendMode::Hue,
        BlendMode::Saturation => AdmittedBlendMode::Saturation,
        BlendMode::Color => AdmittedBlendMode::Color,
        BlendMode::Luminosity => AdmittedBlendMode::Luminosity,
    }
}

#[allow(clippy::too_many_arguments)]
fn admit_visual_source(
    source: &VisualSource,
    placement_start: RationalTime,
    range: FrameRange,
    frame_rate: FrameRate,
    resource_targets: &BTreeMap<String, u32>,
    resources: &[ResolvedResource],
    sources: &mut Vec<AdmittedSource>,
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> u32 {
    let compiled = match source {
        VisualSource::Video(source) => {
            let target =
                required_resource_target(resource_targets, &source.resource, path, diagnostics);
            let source_duration = visual_resource_duration(resources, target);
            AdmittedSource {
                kind: AdmittedSourceKind::Video,
                resource_target: Some(target),
                placement_start,
                source_start: source.source_start,
                source_duration,
                hold_end_time: source_duration,
                rate: source.rate,
                end_behavior: admit_end_behavior(source.end_behavior),
                dependency_range: AdmittedDependencyRange::Frames { range },
                payload: AdmittedSourcePayload::Video {
                    fit: admit_raster_fit(source.sampling.fit),
                },
            }
        }
        VisualSource::Image(source) => {
            let target =
                required_resource_target(resource_targets, &source.resource, path, diagnostics);
            AdmittedSource {
                kind: AdmittedSourceKind::Image,
                resource_target: Some(target),
                placement_start,
                source_start: RationalTime::ZERO,
                source_duration: None,
                hold_end_time: None,
                rate: RationalRate::ONE,
                end_behavior: AdmittedEndBehavior::Static,
                dependency_range: AdmittedDependencyRange::Static,
                payload: AdmittedSourcePayload::Image {
                    fit: admit_raster_fit(source.sampling.fit),
                },
            }
        }
        VisualSource::Lottie(source) => {
            let target =
                required_resource_target(resource_targets, &source.resource, path, diagnostics);
            let descriptor = lottie_resource_descriptor(resources, target);
            let source_duration = descriptor.map(|descriptor| descriptor.duration);
            let hold_end_time = (source.end_behavior == MediaEndBehavior::Hold)
                .then(|| {
                    descriptor.and_then(|descriptor| {
                        lottie_hold_end_boundary(descriptor)
                            .map_err(|_| {
                                diagnostics.push(admit_fault(path, "lottie-hold-end-boundary"));
                            })
                            .ok()
                    })
                })
                .flatten();
            AdmittedSource {
                kind: AdmittedSourceKind::Lottie,
                resource_target: Some(target),
                placement_start,
                source_start: source.source_start,
                source_duration,
                hold_end_time,
                rate: source.rate,
                end_behavior: admit_end_behavior(source.end_behavior),
                dependency_range: AdmittedDependencyRange::Frames { range },
                payload: AdmittedSourcePayload::Lottie {
                    fit: admit_raster_fit(source.sampling.fit),
                },
            }
        }
        VisualSource::Motion(source) => {
            let target =
                required_resource_target(resource_targets, &source.component, path, diagnostics);
            let instance = admit_motion_instance(
                source,
                target,
                frame_rate,
                resource_targets,
                resources,
                path,
                diagnostics,
            );
            let source_duration = source.source_duration;
            let hold_end_time = (source.end_behavior == MediaEndBehavior::Hold
                && instance.phases().duration_frames() > 0)
                .then(|| {
                    motion_hold_end_frame_time(source_duration, frame_rate)
                        .map_err(|_| {
                            diagnostics.push(admit_fault(path, "motion-hold-end-frame-time"));
                        })
                        .ok()
                })
                .flatten();
            AdmittedSource {
                kind: AdmittedSourceKind::Motion,
                resource_target: Some(target),
                placement_start,
                source_start: source.source_start,
                source_duration: Some(source_duration),
                hold_end_time,
                rate: source.rate,
                end_behavior: admit_end_behavior(source.end_behavior),
                dependency_range: AdmittedDependencyRange::Frames { range },
                payload: AdmittedSourcePayload::Motion { instance },
            }
        }
        VisualSource::Solid(source) => AdmittedSource {
            kind: AdmittedSourceKind::Solid,
            resource_target: None,
            placement_start,
            source_start: RationalTime::ZERO,
            source_duration: None,
            hold_end_time: None,
            rate: RationalRate::ONE,
            end_behavior: AdmittedEndBehavior::Static,
            dependency_range: AdmittedDependencyRange::Static,
            payload: AdmittedSourcePayload::Solid {
                color: source.color.clone(),
            },
        },
    };
    let index = sources.len() as u32;
    sources.push(compiled);
    index
}

fn admit_motion_instance(
    source: &MotionInstance,
    component_target: u32,
    frame_rate: FrameRate,
    resource_targets: &BTreeMap<String, u32>,
    resources: &[ResolvedResource],
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> AdmittedMotionInstance {
    let (descriptor, artifact) = match resources
        .get(component_target as usize)
        .map(|resource| &resource.facts)
    {
        Some(VerifiedResourceFacts::MotionArtifact {
            descriptor,
            artifact,
            ..
        }) => (descriptor, Arc::clone(artifact)),
        _ => unreachable!("resolver admitted a Motion component without Motion facts"),
    };

    if !artifact.controls.camera.values.is_empty() {
        diagnostics.push(motion_schema_diagnostic(
            EngineOpenDiagnosticCode::MotionControlSchemaMismatch,
            &format!("{path}/component/controls/camera"),
            "camera-controls-not-supported",
        ));
    }

    let mut props = BTreeMap::new();
    for (name, control) in &artifact.controls.props {
        if control.required && !source.props.contains_key(name) {
            diagnostics.push(motion_schema_diagnostic(
                EngineOpenDiagnosticCode::MotionControlSchemaMismatch,
                &format!("{path}/props/{name}"),
                "missing-required-prop",
            ));
        }
    }
    for (name, param) in &source.props {
        let prop_path = format!("{path}/props/{name}");
        let Some(control) = artifact.controls.props.get(name) else {
            diagnostics.push(motion_schema_diagnostic(
                EngineOpenDiagnosticCode::MotionControlSchemaMismatch,
                &prop_path,
                "unknown-prop",
            ));
            continue;
        };
        if !motion_param_matches_artifact_control(param, &control.control) {
            diagnostics.push(motion_schema_diagnostic(
                EngineOpenDiagnosticCode::MotionControlSchemaMismatch,
                &prop_path,
                "prop-value-outside-artifact-schema",
            ));
            continue;
        }
        if let Some(param) = admit_motion_param(param, &control.control, &prop_path, diagnostics) {
            props.insert(name.clone(), param);
        }
    }

    let mut cues = BTreeMap::new();
    for (name, control) in &artifact.controls.cues {
        if control.required && !source.cues.contains_key(name) {
            diagnostics.push(motion_schema_diagnostic(
                EngineOpenDiagnosticCode::MotionCueSchemaMismatch,
                &format!("{path}/cues/{name}"),
                "missing-required-cue",
            ));
        }
    }
    for (name, cue) in &source.cues {
        let cue_path = format!("{path}/cues/{name}");
        let Some(_control) = artifact.controls.cues.get(name) else {
            diagnostics.push(motion_schema_diagnostic(
                EngineOpenDiagnosticCode::MotionCueSchemaMismatch,
                &cue_path,
                "unknown-cue",
            ));
            continue;
        };
        let MotionCueBinding::SourceRange {
            start,
            end,
            enter_duration,
            exit_duration,
        } = cue;
        let compiled = Some(AdmittedMotionCue::SourceRange {
            start: *start,
            end: *end,
            enter_duration: *enter_duration,
            exit_duration: *exit_duration,
        });
        if let Some(compiled) = compiled {
            cues.insert(name.clone(), compiled);
        }
    }

    let mut compiled_resources = BTreeMap::new();
    for (name, control) in &artifact.controls.assets {
        if artifact_asset_kind(control.kind).is_none() {
            diagnostics.push(motion_schema_diagnostic(
                EngineOpenDiagnosticCode::MotionResourceSchemaMismatch,
                &format!("{path}/resources/{name}"),
                "unsupported-model3d-resource-kind",
            ));
        }
        if control.required && !source.resources.contains_key(name) {
            diagnostics.push(motion_schema_diagnostic(
                EngineOpenDiagnosticCode::MotionResourceSchemaMismatch,
                &format!("{path}/resources/{name}"),
                "missing-required-resource",
            ));
        }
    }
    for (name, resource_id) in &source.resources {
        let resource_path = format!("{path}/resources/{name}");
        let Some(control) = artifact.controls.assets.get(name) else {
            diagnostics.push(motion_schema_diagnostic(
                EngineOpenDiagnosticCode::MotionResourceSchemaMismatch,
                &resource_path,
                "unknown-resource-slot",
            ));
            continue;
        };
        let target =
            required_resource_target(resource_targets, resource_id, &resource_path, diagnostics);
        let Some(expected) = artifact_asset_kind(control.kind) else {
            continue;
        };
        if resources
            .get(target as usize)
            .is_some_and(|resource| resource.kind != expected)
        {
            diagnostics.push(motion_schema_diagnostic(
                EngineOpenDiagnosticCode::MotionResourceSchemaMismatch,
                &resource_path,
                "resource-kind-mismatch",
            ));
            continue;
        }
        compiled_resources.insert(name.clone(), target);
    }

    let artifact_dependencies = resources
        .get(component_target as usize)
        .map(|resource| {
            resource
                .dependencies
                .iter()
                .map(|dependency| AdmittedMotionArtifactDependency {
                    role: dependency.role.clone(),
                    target: dependency.target,
                })
                .collect()
        })
        .unwrap_or_default();

    let phases = admit_motion_phases(source, artifact.as_ref(), frame_rate, path, diagnostics);

    AdmittedMotionInstance {
        component_target,
        reads_destination: descriptor.reads_destination,
        props,
        cues,
        resources: compiled_resources,
        artifact_dependencies,
        phases,
        artifact,
    }
}

/// Resolve author phase durations against the executable Artifact exactly once, while the
/// document clock and verified Artifact controls are both available. Prepare consumes the frozen
/// layout and therefore cannot rediscover a timing rejection from author input.
fn admit_motion_phases(
    source: &MotionInstance,
    artifact: &SceneArtifact,
    frame_rate: FrameRate,
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> AdmittedMotionPhases {
    let duration_frames = admit_motion_frame_count(
        source.source_duration,
        frame_rate,
        false,
        &format!("{path}/sourceDuration"),
        "source-duration",
        diagnostics,
    );
    let enter_frames = source.phases.enter_duration.map(|duration| {
        admit_motion_frame_count(
            duration,
            frame_rate,
            true,
            &format!("{path}/phases/enterDuration"),
            "enter-duration",
            diagnostics,
        )
    });
    let exit_frames = source.phases.exit_duration.map(|duration| {
        admit_motion_frame_count(
            duration,
            frame_rate,
            true,
            &format!("{path}/phases/exitDuration"),
            "exit-duration",
            diagnostics,
        )
    });

    let resolved = match (
        duration_frames,
        enter_frames.transpose(),
        exit_frames.transpose(),
    ) {
        (Ok(duration_frames), Ok(enter_frames), Ok(exit_frames)) => match artifact
            .controls
            .phase_spec_with_overrides(enter_frames, exit_frames)
        {
            Ok(spec) => Some(valle_motion::phase_windows(&spec, duration_frames)),
            Err(error) => {
                let (field_path, duration) = if error.field == "enterFrames" {
                    (
                        format!("{path}/phases/enterDuration"),
                        source.phases.enter_duration,
                    )
                } else {
                    (
                        format!("{path}/phases/exitDuration"),
                        source.phases.exit_duration,
                    )
                };
                let mut diagnostic_details = details([
                    (
                        "reason",
                        "phase-override-outside-artifact-controls".to_owned(),
                    ),
                    ("field", error.field.to_owned()),
                    ("valueFrames", error.value.to_string()),
                    ("minFrames", error.min.to_string()),
                    (
                        "fps",
                        format!("{}/{}", frame_rate.numerator(), frame_rate.denominator()),
                    ),
                ]);
                if let Some(max) = error.max {
                    diagnostic_details.insert("maxFrames".to_owned(), max.to_string());
                }
                if let Some(duration) = duration {
                    diagnostic_details.insert("duration".to_owned(), duration.to_string());
                }
                diagnostics.push(diagnostic(
                    EngineOpenDiagnosticCode::MotionTimingMismatch,
                    field_path,
                    EngineOpenPhase::Admission,
                    None,
                    diagnostic_details,
                ));
                None
            }
        },
        _ => None,
    };

    AdmittedMotionPhases::from_layout(resolved.unwrap_or(valle_motion::PhaseLayout {
        duration_frames: 0,
        enter_frames: 0,
        hold_frames: 0,
        exit_frames: 0,
        hold_cycle_frames: None,
    }))
}

fn admit_motion_frame_count(
    duration: RationalTime,
    frame_rate: FrameRate,
    allow_zero: bool,
    path: &str,
    field: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> Result<u32, ()> {
    let result = quantize_frame_interval(RationalTime::ZERO, duration, frame_rate);
    let (frame_count, reason) = match result {
        Ok(interval) if interval.duration_frames == 0 && allow_zero => (Some(0), None),
        Ok(interval) if interval.duration_frames <= 0 => {
            (None, Some("duration-quantized-to-zero-frames"))
        }
        Ok(interval) => match u32::try_from(interval.duration_frames) {
            Ok(value) => (Some(value), None),
            Err(_) => (None, Some("frame-count-outside-u32")),
        },
        Err(_) => (None, Some("frame-count-quantization-overflow")),
    };
    if let Some(frame_count) = frame_count {
        return Ok(frame_count);
    }

    diagnostics.push(diagnostic(
        EngineOpenDiagnosticCode::MotionTimingMismatch,
        path,
        EngineOpenPhase::Admission,
        None,
        details([
            (
                "reason",
                reason.expect("failed quantization has a reason").to_owned(),
            ),
            ("field", field.to_owned()),
            ("duration", duration.to_string()),
            (
                "fps",
                format!("{}/{}", frame_rate.numerator(), frame_rate.denominator()),
            ),
        ]),
    ));
    Err(())
}

fn motion_param_matches_artifact_control(param: &Param<JsonValue>, control: &ControlType) -> bool {
    let accepts = |value: &JsonValue| motion_json_matches_artifact_control(value, control);
    match param {
        Param::Constant(constant) => accepts(&constant.value),
        Param::Curve(curve) => curve
            .keyframes
            .iter()
            .all(|keyframe| accepts(&keyframe.value)),
    }
}

fn motion_json_matches_artifact_control(value: &JsonValue, control: &ControlType) -> bool {
    let finite_number = |value: &JsonValue| value.as_f64().is_some_and(f64::is_finite);
    let finite_array = |value: &JsonValue, len: usize| {
        value.as_array().is_some_and(|values| {
            values.len() == len && values.iter().all(|value| finite_number(value))
        })
    };
    match control {
        ControlType::Number { min, max, .. } => value.as_f64().is_some_and(|value| {
            value.is_finite()
                && min.is_none_or(|min| value >= min)
                && max.is_none_or(|max| value <= max)
        }),
        ControlType::Length | ControlType::Angle => finite_number(value),
        ControlType::Point => finite_array(value, 2),
        ControlType::Color | ControlType::Rect => finite_array(value, 4),
        ControlType::Bool => value.is_boolean(),
        ControlType::String | ControlType::NodeTarget => value.is_string(),
        ControlType::Select { values } => value
            .as_str()
            .is_some_and(|value| values.iter().any(|allowed| allowed == value)),
        ControlType::PathData => false,
    }
}

fn admit_motion_param(
    param: &Param<JsonValue>,
    control: &ControlType,
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> Option<AdmittedMotionParam> {
    match control {
        ControlType::Number { .. } | ControlType::Length | ControlType::Angle => {
            admit_motion_typed_param(param, JsonValue::as_f64)
                .map(|param| AdmittedMotionParam::Scalar { param })
                .or_else(|| motion_param_type_error(path, diagnostics))
        }
        ControlType::Point => admit_motion_typed_param(param, |value| {
            let values = value.as_array()?;
            Some([values.first()?.as_f64()?, values.get(1)?.as_f64()?])
                .filter(|_| values.len() == 2)
        })
        .map(|param| AdmittedMotionParam::Vec2 { param })
        .or_else(|| motion_param_type_error(path, diagnostics)),
        ControlType::Color | ControlType::Rect => admit_motion_typed_param(param, |value| {
            let values = value.as_array()?;
            Some([
                values.first()?.as_f64()?,
                values.get(1)?.as_f64()?,
                values.get(2)?.as_f64()?,
                values.get(3)?.as_f64()?,
            ])
            .filter(|_| values.len() == 4)
        })
        .map(|param| AdmittedMotionParam::Vec4 { param })
        .or_else(|| motion_param_type_error(path, diagnostics)),
        ControlType::Bool => match param {
            Param::Constant(constant) => constant
                .value
                .as_bool()
                .map(|value| AdmittedMotionParam::Boolean { value })
                .or_else(|| motion_param_type_error(path, diagnostics)),
            Param::Curve(_) => motion_param_type_error(path, diagnostics),
        },
        ControlType::String | ControlType::NodeTarget | ControlType::Select { .. } => match param {
            Param::Constant(constant) => constant
                .value
                .as_str()
                .map(|value| AdmittedMotionParam::String {
                    value: value.to_owned(),
                })
                .or_else(|| motion_param_type_error(path, diagnostics)),
            Param::Curve(_) => motion_param_type_error(path, diagnostics),
        },
        ControlType::PathData => motion_param_type_error(path, diagnostics),
    }
}

fn admit_motion_typed_param<T: Clone>(
    param: &Param<JsonValue>,
    convert: impl Fn(&JsonValue) -> Option<T>,
) -> Option<AdmittedParam<T>> {
    let value = match param {
        Param::Constant(constant) => AdmittedParamValue::Constant {
            value: convert(&constant.value)?,
        },
        Param::Curve(curve) => AdmittedParamValue::Curve {
            curve: AdmittedCurve {
                interpolation: match curve.interpolation {
                    Interpolation::Step => AdmittedInterpolation::Step,
                    Interpolation::Linear => AdmittedInterpolation::Linear,
                },
                keyframes: curve
                    .keyframes
                    .iter()
                    .map(|keyframe| {
                        Some(AdmittedKeyframe {
                            time: keyframe.time,
                            value: convert(&keyframe.value)?,
                            easing: admit_easing(keyframe.out_easing.as_ref()),
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            },
        },
    };
    Some(AdmittedParam {
        owner_clock: AdmittedOwnerClock::MotionSource,
        value,
    })
}

fn motion_param_type_error<T>(
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> Option<T> {
    diagnostics.push(motion_schema_diagnostic(
        EngineOpenDiagnosticCode::MotionControlSchemaMismatch,
        path,
        "prop-type-mismatch",
    ));
    None
}

fn motion_schema_diagnostic(
    code: EngineOpenDiagnosticCode,
    path: &str,
    reason: &str,
) -> EngineOpenDiagnostic {
    diagnostic(
        code,
        path,
        EngineOpenPhase::Admission,
        None,
        details([("reason", reason.to_owned())]),
    )
}

fn admit_end_behavior(behavior: MediaEndBehavior) -> AdmittedEndBehavior {
    match behavior {
        MediaEndBehavior::Error => AdmittedEndBehavior::Error,
        MediaEndBehavior::Hold => AdmittedEndBehavior::Hold,
        MediaEndBehavior::Loop => AdmittedEndBehavior::Loop,
    }
}

fn admit_raster_fit(fit: RasterFit) -> AdmittedRasterFit {
    match fit {
        RasterFit::Contain => AdmittedRasterFit::Contain,
        RasterFit::Cover => AdmittedRasterFit::Cover,
        RasterFit::Fill => AdmittedRasterFit::Fill,
        RasterFit::None => AdmittedRasterFit::None,
    }
}

fn required_resource_target(
    resource_targets: &BTreeMap<String, u32>,
    resource_id: &str,
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> u32 {
    resource_targets
        .get(resource_id)
        .copied()
        .unwrap_or_else(|| {
            diagnostics.push(admit_fault(path, "missing-resolved-resource"));
            0
        })
}

fn visual_resource_duration(resources: &[ResolvedResource], target: u32) -> Option<RationalTime> {
    match resources
        .get(target as usize)
        .map(|resource| &resource.facts)
    {
        Some(VerifiedResourceFacts::Video { descriptor, .. }) => Some(descriptor.duration),
        Some(VerifiedResourceFacts::Lottie { descriptor, .. }) => Some(descriptor.duration),
        _ => None,
    }
}

fn lottie_resource_descriptor(
    resources: &[ResolvedResource],
    target: u32,
) -> Option<&LottieResourceDescriptorWire> {
    match resources
        .get(target as usize)
        .map(|resource| &resource.facts)
    {
        Some(VerifiedResourceFacts::Lottie { descriptor, .. }) => Some(descriptor),
        _ => None,
    }
}

/// Resolves the Lottie ABI's `t -> D-` sentinel onto the descriptor clock. The selected tick is
/// `ceil(D / timeBase) - 1`, which is the final deterministic descriptor boundary strictly below
/// `D`; this is exact integer arithmetic, not an epsilon approximation or a canvas-frame sample.
fn lottie_hold_end_boundary(descriptor: &LottieResourceDescriptorWire) -> Result<RationalTime, ()> {
    if descriptor.boundary_sampling != ContinuousBoundarySamplingWire::LeftLimit {
        return Err(());
    }
    let duration = descriptor.duration;
    let time_base = RationalTime::from_exact(descriptor.time_base);
    let duration_numerator = u128::try_from(duration.numerator()).map_err(|_| ())?;
    let time_base_numerator = u128::try_from(time_base.numerator()).map_err(|_| ())?;
    if duration_numerator == 0 || time_base_numerator == 0 {
        return Err(());
    }
    let scaled_duration = duration_numerator
        .checked_mul(u128::from(time_base.denominator()))
        .ok_or(())?;
    let scaled_tick = u128::from(duration.denominator())
        .checked_mul(time_base_numerator)
        .ok_or(())?;
    let tick_count = (scaled_duration / scaled_tick)
        .checked_add(u128::from(!scaled_duration.is_multiple_of(scaled_tick)))
        .ok_or(())?;
    let last_tick = tick_count.checked_sub(1).ok_or(())?;
    let sample_numerator = last_tick.checked_mul(time_base_numerator).ok_or(())?;
    let sample_denominator = u128::from(time_base.denominator());
    let divisor = gcd_u128(sample_numerator, sample_denominator);
    RationalTime::new(
        i64::try_from(sample_numerator / divisor).map_err(|_| ())?,
        u32::try_from(sample_denominator / divisor).map_err(|_| ())?,
    )
    .map_err(|_| ())
}

fn gcd_u128(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn visual_footprint(resources: &[ResolvedResource], target: u32) -> VisualFootprint {
    match resources
        .get(target as usize)
        .map(|resource| &resource.facts)
    {
        Some(VerifiedResourceFacts::Video {
            temporal_footprint, ..
        })
        | Some(VerifiedResourceFacts::Image {
            temporal_footprint, ..
        })
        | Some(VerifiedResourceFacts::Lottie {
            temporal_footprint, ..
        })
        | Some(VerifiedResourceFacts::MotionArtifact {
            temporal_footprint, ..
        }) => *temporal_footprint,
        _ => VisualFootprint::default(),
    }
}

fn combined_visual_footprint(
    calls: &[AdmittedKernelCall],
    kernels: &[AdmittedKernel],
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> VisualFootprint {
    calls
        .iter()
        .fold(VisualFootprint::default(), |total, call| {
            let footprint = kernels
                .get(call.kernel as usize)
                .map(|kernel| kernel.visual_footprint)
                .unwrap_or_default();
            add_visual_footprints(total, footprint, path, diagnostics)
        })
}

fn add_visual_footprints(
    left: VisualFootprint,
    right: VisualFootprint,
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> VisualFootprint {
    let Some(combined) = checked_add_visual_footprints(left, right) else {
        diagnostics.push(admit_fault(path, "visual-footprint-overflow"));
        return VisualFootprint::default();
    };
    combined
}

fn checked_add_visual_footprints(
    left: VisualFootprint,
    right: VisualFootprint,
) -> Option<VisualFootprint> {
    Some(VisualFootprint::new(
        left.past_frames.checked_add(right.past_frames)?,
        left.future_frames.checked_add(right.future_frames)?,
    ))
}

fn expand_frame_range(range: FrameRange, footprint: VisualFootprint) -> Option<FrameRange> {
    Some(FrameRange::new(
        range.start.checked_sub(i64::from(footprint.past_frames))?,
        range.end.checked_add(i64::from(footprint.future_frames))?,
    ))
}

fn extend_visual_dependency(
    sources: &mut [AdmittedSource],
    source: u32,
    range: FrameRange,
) -> bool {
    let Some(source) = sources.get_mut(source as usize) else {
        return false;
    };
    if let AdmittedDependencyRange::Frames { range: current } = &mut source.dependency_range {
        let union = current.union(range);
        let changed = union != *current;
        *current = union;
        return changed;
    }
    false
}

fn finalize_visual_dependencies(
    sources: &mut [AdmittedSource],
    resources: &[ResolvedResource],
    canvas: CompiledCanvas,
    transition_sources: &BTreeSet<u32>,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) {
    for (source_index, source) in sources.iter_mut().enumerate() {
        let AdmittedDependencyRange::Frames { range } = source.dependency_range else {
            continue;
        };
        let footprint = source
            .resource_target
            .map(|target| visual_footprint(resources, target))
            .unwrap_or_default();
        let Some(start) = range.start.checked_sub(i64::from(footprint.past_frames())) else {
            diagnostics.push(admit_fault("/document/visual", "visual-footprint-overflow"));
            continue;
        };
        let Some(end) = range.end.checked_add(i64::from(footprint.future_frames())) else {
            diagnostics.push(admit_fault("/document/visual", "visual-footprint-overflow"));
            continue;
        };
        let expanded = FrameRange::new(start, end);
        source.dependency_range = AdmittedDependencyRange::Frames { range: expanded };
        if source.end_behavior == AdmittedEndBehavior::Error
            && !source_range_is_valid(source, expanded.start, expanded.end, canvas.frame_rate)
        {
            let code = if transition_sources.contains(&(source_index as u32)) {
                EngineOpenDiagnosticCode::TransitionInsufficientHandle
            } else {
                EngineOpenDiagnosticCode::SourceRangeOutOfBounds
            };
            let resource_digest = source.resource_target.and_then(|target| {
                resources
                    .get(target as usize)
                    .map(|resource| resource.digest.to_wire())
            });
            diagnostics.push(diagnostic(
                code,
                "/document/visual",
                EngineOpenPhase::Admission,
                resource_digest.as_deref(),
                range_details(expanded.start, expanded.end),
            ));
        }
    }
}

fn source_range_is_valid(
    source: &AdmittedSource,
    start: i64,
    end: i64,
    frame_rate: FrameRate,
) -> bool {
    if start >= end {
        return true;
    }
    [start, end - 1].into_iter().all(|frame| {
        frame_sample_time(frame, frame_rate)
            .ok()
            .and_then(|time| admitted_raw_source_time(source, time).ok())
            .is_some_and(|raw| {
                source
                    .source_duration
                    .is_some_and(|duration| !raw.is_negative() && raw < duration)
            })
    })
}

fn admitted_raw_source_time(
    source: &AdmittedSource,
    composition_time: RationalTime,
) -> Result<RationalTime, RuntimeFault> {
    composition_time
        .checked_sub(source.placement_start)
        .and_then(|local| local.checked_scale(source.rate))
        .and_then(|offset| source.source_start.checked_add(offset))
        .map_err(|_| RuntimeFault::ExactTimeOverflow)
}

fn admit_adjustment_program(
    clips: &[TimedAdjustment],
    canvas: CompiledCanvas,
    kernel_targets: &BTreeMap<String, u32>,
    kernels: &[AdmittedKernel],
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> Option<AdmittedAdjustmentProgram> {
    let clips = clips
        .iter()
        .enumerate()
        .map(|(index, effect)| {
            let start = effect.start;
            let duration = effect.duration;
            let interval = quantize_frame_interval(start, duration, canvas.frame_rate).ok()?;
            let path = format!("/document/adjustments/{index}/effect");
            let effect = match &effect.effect {
                AdjustmentEffect::ColorGrade(effect) => AdmittedAdjustmentEffect::ColorGrade {
                    temperature: effect.temperature,
                },
                AdjustmentEffect::Extension(extension) => AdmittedAdjustmentEffect::Extension {
                    call: admit_kernel_call(
                        &path,
                        &extension.parameters,
                        kernel_targets,
                        kernels,
                        diagnostics,
                    ),
                },
            };
            Some(AdmittedAdjustmentClip {
                range: FrameRange::new(interval.start_frame, interval.end_frame),
                effect,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(AdmittedAdjustmentProgram { clips })
}

fn admit_caption_program(
    captions: &CaptionComposition,
    canvas: CompiledCanvas,
    resource_targets: &BTreeMap<String, u32>,
    resources: &[ResolvedResource],
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> Option<AdmittedCaptionProgram> {
    let mut tracks = Vec::with_capacity(captions.tracks.len());
    for (track_index, track) in captions.tracks.iter().enumerate() {
        let mut cursor = RationalTime::ZERO;
        let mut items = Vec::with_capacity(track.items.len());
        for (item_index, item) in track.items.iter().enumerate() {
            let path = format!("/document/captions/tracks/{track_index}/items/{item_index}");
            match item {
                CaptionItem::Clip(caption) => {
                    let duration = caption.duration;
                    let interval =
                        quantize_frame_interval(cursor, duration, canvas.frame_rate).ok()?;
                    let font_target = required_resource_target(
                        resource_targets,
                        &caption.style.font,
                        &format!("{path}/style/font"),
                        diagnostics,
                    );
                    let font_weight = resolved_caption_face_weight(font_target, resources)?;
                    let behavior = caption.behavior.as_ref().map(|behavior| match behavior {
                        CaptionBehavior::Scroll { axis, speed } => {
                            AdmittedCaptionBehavior::Scroll {
                                horizontal: *axis == ScrollAxis::Horizontal,
                                speed: *speed,
                            }
                        }
                        CaptionBehavior::Karaoke { mode } => AdmittedCaptionBehavior::Karaoke {
                            line_mode: *mode == KaraokeMode::Line,
                        },
                    });
                    items.push(AdmittedCaptionItem::Clip {
                        clip: AdmittedCaptionClip {
                            range: FrameRange::new(interval.start_frame, interval.end_frame),
                            runs: caption.runs.iter().map(|run| run.text.clone()).collect(),
                            run_timings: caption
                                .runs
                                .iter()
                                .map(|run| {
                                    run.timing.as_ref().map(|timing| AdmittedCaptionRunTiming {
                                        start: timing.start,
                                        end: timing.end,
                                    })
                                })
                                .collect(),
                            run_styles: caption
                                .runs
                                .iter()
                                .map(|run| AdmittedCaptionRunStyle {
                                    font_size: run
                                        .style
                                        .as_ref()
                                        .and_then(|style| style.font_size)
                                        .unwrap_or(caption.style.font_size),
                                    color: run
                                        .style
                                        .as_ref()
                                        .and_then(|style| style.color.as_ref())
                                        .unwrap_or(&caption.style.color)
                                        .clone(),
                                    font_weight,
                                })
                                .collect(),
                            font_target,
                            shadow: caption.style.shadow.as_ref().map(|shadow| {
                                AdmittedCaptionShadow {
                                    color: shadow.color.clone(),
                                    offset: shadow.offset,
                                    blur_sigma: shadow.blur_sigma,
                                }
                            }),
                            region: *caption.layout.region.as_array(),
                            align: admit_caption_align(caption.layout.align),
                            presentation: AdmittedCaptionPresentation {
                                opacity: admit_param(
                                    &caption.presentation.opacity,
                                    AdmittedOwnerClock::CaptionLocal,
                                    &format!("{path}/presentation/opacity"),
                                    diagnostics,
                                ),
                                translation: admit_param(
                                    &caption.presentation.translation,
                                    AdmittedOwnerClock::CaptionLocal,
                                    &format!("{path}/presentation/translation"),
                                    diagnostics,
                                ),
                                scale: admit_param(
                                    &caption.presentation.scale,
                                    AdmittedOwnerClock::CaptionLocal,
                                    &format!("{path}/presentation/scale"),
                                    diagnostics,
                                ),
                                rotation: admit_param(
                                    &caption.presentation.rotation,
                                    AdmittedOwnerClock::CaptionLocal,
                                    &format!("{path}/presentation/rotation"),
                                    diagnostics,
                                ),
                                clip_inset: admit_param(
                                    &caption.presentation.clip_inset,
                                    AdmittedOwnerClock::CaptionLocal,
                                    &format!("{path}/presentation/clipInset"),
                                    diagnostics,
                                ),
                                blur_sigma: admit_param(
                                    &caption.presentation.blur_sigma,
                                    AdmittedOwnerClock::CaptionLocal,
                                    &format!("{path}/presentation/blurSigma"),
                                    diagnostics,
                                ),
                            },
                            behavior,
                        },
                    });
                    cursor = cursor.checked_add(duration).ok()?;
                }
                CaptionItem::Gap(gap) => {
                    let duration = gap.duration;
                    let interval =
                        quantize_frame_interval(cursor, duration, canvas.frame_rate).ok()?;
                    items.push(AdmittedCaptionItem::Gap {
                        gap: AdmittedCaptionGap {
                            range: FrameRange::new(interval.start_frame, interval.end_frame),
                        },
                    });
                    cursor = cursor.checked_add(duration).ok()?;
                }
            }
        }
        tracks.push(AdmittedCaptionTrack {
            order: track_index as u32,
            items,
        });
    }
    Some(AdmittedCaptionProgram { tracks })
}

fn validate_caption_font_admission(
    caption: &Caption,
    target: u32,
    resources: &[ResolvedResource],
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) {
    let Some(resource) = resources.get(target as usize) else {
        return;
    };
    let VerifiedResourceFacts::Font { descriptor, bytes } = &resource.facts else {
        return;
    };
    let Ok(face) = ttf_parser::Face::parse(bytes, descriptor.face_index) else {
        // Resource resolution already reports an invalid concrete face.
        return;
    };

    for (run_index, run) in caption.runs.iter().enumerate() {
        for (scalar_index, scalar) in run.text.chars().enumerate() {
            if caption_scalar_requires_glyph(scalar) && face.glyph_index(scalar).is_none() {
                diagnostics.push(diagnostic(
                    EngineOpenDiagnosticCode::MissingFontGlyph,
                    format!("{path}/runs/{run_index}/text"),
                    EngineOpenPhase::Admission,
                    Some(&resource.resource_id),
                    details([
                        ("codePoint", format!("U+{:04X}", scalar as u32)),
                        ("scalarIndex", scalar_index.to_string()),
                        ("faceIndex", descriptor.face_index.to_string()),
                    ]),
                ));
                return;
            }
        }
    }
}

/// Captions use the default instance of the exact descriptor-selected face. Timeline does not
/// expose a synthetic or variable-font weight selector, so shaping semantics must be derived from
/// that concrete face instead of trusting an author-authored number.
fn resolved_caption_face_weight(target: u32, resources: &[ResolvedResource]) -> Option<u16> {
    let resource = resources.get(target as usize)?;
    let VerifiedResourceFacts::Font { descriptor, bytes } = &resource.facts else {
        return None;
    };
    let face = ttf_parser::Face::parse(bytes, descriptor.face_index).ok()?;
    Some(face.weight().to_number())
}

fn caption_scalar_requires_glyph(scalar: char) -> bool {
    !scalar.is_control()
        && !matches!(
            scalar,
            '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{E0100}'..='\u{E01EF}'
        )
}

fn admit_caption_align(align: CaptionAlign) -> AdmittedCaptionAlign {
    match align {
        CaptionAlign::TopLeft => AdmittedCaptionAlign::TopLeft,
        CaptionAlign::TopCenter => AdmittedCaptionAlign::TopCenter,
        CaptionAlign::TopRight => AdmittedCaptionAlign::TopRight,
        CaptionAlign::CenterLeft => AdmittedCaptionAlign::CenterLeft,
        CaptionAlign::Center => AdmittedCaptionAlign::Center,
        CaptionAlign::CenterRight => AdmittedCaptionAlign::CenterRight,
        CaptionAlign::BottomLeft => AdmittedCaptionAlign::BottomLeft,
        CaptionAlign::BottomCenter => AdmittedCaptionAlign::BottomCenter,
        CaptionAlign::BottomRight => AdmittedCaptionAlign::BottomRight,
    }
}

#[allow(clippy::too_many_arguments)]
fn admit_audio_program(
    audio: &AudioComposition,
    canvas: CompiledCanvas,
    resource_targets: &BTreeMap<String, u32>,
    resources: &[ResolvedResource],
    kernel_targets: &BTreeMap<String, u32>,
    kernels: &[AdmittedKernel],
    sources: &mut Vec<AdmittedSource>,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> Option<Vec<AdmittedAudioTrack>> {
    let mut tracks = Vec::with_capacity(audio.tracks.len());
    let mut crossfade_sources = BTreeSet::new();
    let first_audio_source = sources.len();

    for (track_index, track) in audio.tracks.iter().enumerate() {
        let mut cursor = RationalTime::ZERO;
        let mut cut_times = Vec::with_capacity(track.items.len());
        let mut items: Vec<Option<AdmittedAudioItem>> = Vec::with_capacity(track.items.len());
        for (item_index, item) in track.items.iter().enumerate() {
            cut_times.push(cursor);
            let path = format!("/document/audio/tracks/{track_index}/items/{item_index}");
            match item {
                AudioItem::Clip(clip) => {
                    let duration = clip.duration;
                    let interval =
                        quantize_sample_interval(cursor, duration, canvas.sample_rate).ok()?;
                    let range = SampleRange::admitted(interval.start_sample, interval.end_sample);
                    let AudioSource::Media(media) = &clip.source;
                    let target = required_resource_target(
                        resource_targets,
                        &media.resource,
                        &format!("{path}/source/resource"),
                        diagnostics,
                    );
                    let (channel_map, source_sample_rate, source_sample_count) =
                        validate_audio_descriptor(
                            resources,
                            target,
                            canvas,
                            &format!("{path}/source/resource"),
                            diagnostics,
                        );
                    let source = sources.len() as u32;
                    sources.push(AdmittedSource {
                        kind: AdmittedSourceKind::Audio,
                        resource_target: Some(target),
                        placement_start: cursor,
                        source_start: media.source_start,
                        source_duration: audio_resource_duration(resources, target),
                        hold_end_time: None,
                        rate: media.rate,
                        end_behavior: admit_end_behavior(media.end_behavior),
                        dependency_range: AdmittedDependencyRange::Samples { range },
                        payload: AdmittedSourcePayload::Audio {
                            channel_map,
                            source_sample_rate,
                            source_sample_count,
                        },
                    });
                    let effects: Vec<AdmittedKernelCall> = clip
                        .effects
                        .iter()
                        .enumerate()
                        .map(|(effect_index, effect)| {
                            admit_kernel_call(
                                &format!("{path}/effects/{effect_index}"),
                                &effect.parameters,
                                kernel_targets,
                                kernels,
                                diagnostics,
                            )
                        })
                        .collect();
                    let footprint = combined_audio_footprint(&effects, kernels, &path, diagnostics);
                    let effect_multiplier =
                        compiled_audio_effect_multiplier(&effects, kernels, &path, diagnostics);
                    if let Some(expanded) = expand_sample_range(range, footprint) {
                        extend_audio_dependency(sources, source, expanded);
                    } else {
                        diagnostics.push(admit_fault(&path, "audio-effect-footprint-overflow"));
                    }
                    items.push(Some(AdmittedAudioItem::Clip {
                        clip: AdmittedAudioClip {
                            range,
                            exact_start: cursor,
                            source,
                            effect_multiplier,
                            gain: admit_param(
                                &clip.gain,
                                AdmittedOwnerClock::AudioClipLocal,
                                &format!("{path}/gain"),
                                diagnostics,
                            ),
                            pan: admit_param(
                                &clip.pan,
                                AdmittedOwnerClock::AudioClipLocal,
                                &format!("{path}/pan"),
                                diagnostics,
                            ),
                            effects,
                            footprint,
                        },
                    }));
                    cursor = cursor.checked_add(duration).ok()?;
                }
                AudioItem::Gap(gap) => {
                    let duration = gap.duration;
                    let interval =
                        quantize_sample_interval(cursor, duration, canvas.sample_rate).ok()?;
                    items.push(Some(AdmittedAudioItem::Gap {
                        gap: AdmittedAudioGap {
                            range: SampleRange::admitted(
                                interval.start_sample,
                                interval.end_sample,
                            ),
                        },
                    }));
                    cursor = cursor.checked_add(duration).ok()?;
                }
                AudioItem::Crossfade(_) => items.push(None),
            }
        }

        let mut endpoint_windows: BTreeMap<usize, Vec<SampleRange>> = BTreeMap::new();
        for (item_index, item) in track.items.iter().enumerate() {
            let AudioItem::Crossfade(crossfade) = item else {
                continue;
            };
            let path = format!("/document/audio/tracks/{track_index}/items/{item_index}");
            let Some(from_index) = item_index.checked_sub(1) else {
                diagnostics.push(admit_fault(&path, "crossfade-left-endpoint"));
                continue;
            };
            let to_index = item_index + 1;
            let Some(AdmittedAudioItem::Clip { clip: from_clip }) =
                items.get(from_index).and_then(Option::as_ref).cloned()
            else {
                diagnostics.push(admit_fault(&path, "crossfade-left-endpoint"));
                continue;
            };
            let Some(AdmittedAudioItem::Clip { clip: to_clip }) =
                items.get(to_index).and_then(Option::as_ref).cloned()
            else {
                diagnostics.push(admit_fault(&path, "crossfade-right-endpoint"));
                continue;
            };
            let duration = crossfade.duration;
            let n = quantize_sample_interval(RationalTime::ZERO, duration, canvas.sample_rate)
                .ok()?
                .duration_samples;
            let cut_sample =
                quantize_sample_boundary(cut_times[item_index], canvas.sample_rate).ok()?;
            let left_samples = n / 2;
            let right_samples = n - left_samples;
            let window = SampleRange::admitted(
                cut_sample.checked_sub(left_samples)?,
                cut_sample.checked_add(right_samples)?,
            );
            if window.start < 0 || window.end > canvas.sample_count {
                diagnostics.push(diagnostic(
                    EngineOpenDiagnosticCode::CrossfadeWindowOutOfCanvas,
                    &path,
                    EngineOpenPhase::Admission,
                    None,
                    range_details(window.start, window.end),
                ));
            }
            if left_samples > from_clip.range.len() || right_samples > to_clip.range.len() {
                diagnostics.push(diagnostic(
                    EngineOpenDiagnosticCode::CrossfadeInsufficientHandle,
                    &path,
                    EngineOpenPhase::Admission,
                    None,
                    details([
                        ("leftSamples", left_samples.to_string()),
                        ("rightSamples", right_samples.to_string()),
                    ]),
                ));
            }
            for endpoint in [from_index, to_index] {
                if endpoint_windows
                    .get(&endpoint)
                    .is_some_and(|ranges| ranges.iter().any(|range| range.intersects(window)))
                {
                    diagnostics.push(diagnostic(
                        EngineOpenDiagnosticCode::OverlappingCrossfadeEndpoint,
                        &path,
                        EngineOpenPhase::Admission,
                        None,
                        details([("endpointItem", endpoint.to_string())]),
                    ));
                }
                endpoint_windows.entry(endpoint).or_default().push(window);
            }
            for clip in [&from_clip, &to_clip] {
                if let Some(expanded) = expand_sample_range(window, clip.footprint) {
                    extend_audio_dependency(sources, clip.source, expanded);
                } else {
                    diagnostics.push(admit_fault(&path, "crossfade-footprint-overflow"));
                }
            }
            crossfade_sources.insert(from_clip.source);
            crossfade_sources.insert(to_clip.source);
            items[item_index] = Some(AdmittedAudioItem::Crossfade {
                crossfade: AdmittedAudioCrossfade {
                    window,
                    cut_sample,
                    left_samples,
                    right_samples,
                    from_item: from_index as u32,
                    to_item: to_index as u32,
                    from_source: from_clip.source,
                    to_source: to_clip.source,
                },
            });
        }
        tracks.push(AdmittedAudioTrack {
            order: track_index as u32,
            items: items.into_iter().collect::<Option<Vec<_>>>()?,
        });
    }

    finalize_audio_dependencies(
        &mut sources[first_audio_source..],
        first_audio_source,
        resources,
        canvas,
        &crossfade_sources,
        diagnostics,
    );
    Some(tracks)
}

fn validate_audio_descriptor(
    resources: &[ResolvedResource],
    target: u32,
    canvas: CompiledCanvas,
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> (AdmittedAudioChannelMap, u32, i64) {
    let Some(VerifiedResourceFacts::Audio { descriptor, .. }) = resources
        .get(target as usize)
        .map(|resource| &resource.facts)
    else {
        diagnostics.push(admit_fault(path, "audio-facts"));
        return (
            AdmittedAudioChannelMap::StereoIdentity,
            canvas.sample_rate,
            1,
        );
    };
    let channel_map = match descriptor.channel_layout {
        AudioChannelLayoutWire::Stereo => AdmittedAudioChannelMap::StereoIdentity,
        AudioChannelLayoutWire::Mono => AdmittedAudioChannelMap::MonoToStereo,
        AudioChannelLayoutWire::Surround51 | AudioChannelLayoutWire::Surround71 => {
            diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::UnsupportedAudioChannelLayout,
                path,
                EngineOpenPhase::Admission,
                None,
                details([("required", "mono-or-stereo".to_owned())]),
            ));
            AdmittedAudioChannelMap::StereoIdentity
        }
    };
    if descriptor.sample_rate != canvas.sample_rate {
        diagnostics.push(diagnostic(
            EngineOpenDiagnosticCode::UnprovenAudioResampler,
            path,
            EngineOpenPhase::Admission,
            None,
            details([
                ("sourceSampleRate", descriptor.sample_rate.to_string()),
                ("canvasSampleRate", canvas.sample_rate.to_string()),
            ]),
        ));
    }
    let source_sample_count =
        match quantize_sample_boundary(descriptor.duration, descriptor.sample_rate) {
            Ok(count) if (1..=MAX_SAFE_JSON_INTEGER).contains(&count) => count,
            _ => {
                diagnostics.push(admit_fault(path, "invalid-audio-source-sample-count"));
                1
            }
        };
    (channel_map, descriptor.sample_rate, source_sample_count)
}

fn audio_resource_duration(resources: &[ResolvedResource], target: u32) -> Option<RationalTime> {
    match resources
        .get(target as usize)
        .map(|resource| &resource.facts)
    {
        Some(VerifiedResourceFacts::Audio { descriptor, .. }) => Some(descriptor.duration),
        _ => None,
    }
}

fn audio_footprint(resources: &[ResolvedResource], target: u32) -> AudioFootprint {
    match resources
        .get(target as usize)
        .map(|resource| &resource.facts)
    {
        Some(VerifiedResourceFacts::Audio {
            temporal_footprint, ..
        }) => *temporal_footprint,
        _ => AudioFootprint::default(),
    }
}

fn combined_audio_footprint(
    calls: &[AdmittedKernelCall],
    kernels: &[AdmittedKernel],
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> AudioFootprint {
    calls.iter().fold(AudioFootprint::default(), |total, call| {
        let footprint = kernels
            .get(call.kernel as usize)
            .map(|kernel| kernel.audio_footprint)
            .unwrap_or_default();
        let Some(combined) = checked_add_audio_footprints(total, footprint) else {
            diagnostics.push(admit_fault(path, "audio-footprint-overflow"));
            return AudioFootprint::default();
        };
        combined
    })
}

fn checked_add_audio_footprints(
    left: AudioFootprint,
    right: AudioFootprint,
) -> Option<AudioFootprint> {
    AudioFootprint::new(
        left.past_samples.checked_add(right.past_samples)?,
        left.future_samples.checked_add(right.future_samples)?,
    )
    .ok()
}

fn compiled_audio_effect_multiplier(
    calls: &[AdmittedKernelCall],
    kernels: &[AdmittedKernel],
    path: &str,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> f64 {
    let mut combined = 1.0_f64;
    for (effect_index, call) in calls.iter().enumerate() {
        let effect_path = format!("{path}/effects/{effect_index}");
        let Some(kernel) = kernels.get(call.kernel as usize) else {
            diagnostics.push(admit_fault(&effect_path, "missing-admitted-audio-effect"));
            continue;
        };
        if kernel.domain != CompiledKernelDomain::AudioEffect
            || kernel.kind != AUDIO_GAIN_EFFECT_KIND
            || kernel.abi != AUDIO_GAIN_EFFECT_ABI
        {
            diagnostics.push(admit_fault(
                &effect_path,
                "unsupported-admitted-audio-effect",
            ));
            continue;
        }
        let Some(multiplier) = call
            .parameters
            .get("multiplier")
            .and_then(JsonValue::as_f64)
        else {
            diagnostics.push(admit_fault(
                &effect_path,
                "admitted-audio-multiplier-missing",
            ));
            continue;
        };
        let Some(next) = audio_gain::combine_multiplier(combined, multiplier) else {
            diagnostics.push(admit_fault(
                &effect_path,
                "admitted-audio-multiplier-overflow",
            ));
            return 1.0;
        };
        combined = next;
    }
    combined
}

fn expand_sample_range(range: SampleRange, footprint: AudioFootprint) -> Option<SampleRange> {
    let past = i64::try_from(footprint.past_samples).ok()?;
    let future = i64::try_from(footprint.future_samples).ok()?;
    Some(SampleRange::admitted(
        range.start.checked_sub(past)?,
        range.end.checked_add(future)?,
    ))
}

fn extend_audio_dependency(sources: &mut [AdmittedSource], source: u32, range: SampleRange) {
    let Some(source) = sources.get_mut(source as usize) else {
        return;
    };
    if let AdmittedDependencyRange::Samples { range: current } = &mut source.dependency_range {
        *current = current.union(range);
    }
}

fn finalize_audio_dependencies(
    sources: &mut [AdmittedSource],
    source_offset: usize,
    resources: &[ResolvedResource],
    canvas: CompiledCanvas,
    crossfade_sources: &BTreeSet<u32>,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) {
    for (local_index, source) in sources.iter_mut().enumerate() {
        let AdmittedDependencyRange::Samples { range } = source.dependency_range else {
            continue;
        };
        let footprint = source
            .resource_target
            .map(|target| audio_footprint(resources, target))
            .unwrap_or_default();
        let past = i64::try_from(footprint.past_samples()).ok();
        let future = i64::try_from(footprint.future_samples()).ok();
        let Some(start) = past.and_then(|past| range.start.checked_sub(past)) else {
            diagnostics.push(admit_fault("/document/audio", "audio-footprint-overflow"));
            continue;
        };
        let Some(end) = future.and_then(|future| range.end.checked_add(future)) else {
            diagnostics.push(admit_fault("/document/audio", "audio-footprint-overflow"));
            continue;
        };
        let expanded = SampleRange::admitted(start, end);
        source.dependency_range = AdmittedDependencyRange::Samples { range: expanded };
        if source.end_behavior == AdmittedEndBehavior::Error
            && !audio_source_range_is_valid(
                source,
                expanded.start,
                expanded.end,
                canvas.sample_rate,
            )
        {
            let source_index = (source_offset + local_index) as u32;
            let code = if crossfade_sources.contains(&source_index) {
                EngineOpenDiagnosticCode::CrossfadeInsufficientHandle
            } else {
                EngineOpenDiagnosticCode::SourceRangeOutOfBounds
            };
            diagnostics.push(diagnostic(
                code,
                "/document/audio",
                EngineOpenPhase::Admission,
                None,
                range_details(expanded.start, expanded.end),
            ));
        }
    }
}

fn audio_source_range_is_valid(
    source: &AdmittedSource,
    start: i64,
    end: i64,
    sample_rate: u32,
) -> bool {
    if start >= end {
        return true;
    }
    let Some(source_sample_rate) = source.audio_source_sample_rate() else {
        return false;
    };
    let Some(source_sample_count) = source.audio_source_sample_count() else {
        return false;
    };
    [start, end - 1].into_iter().all(|sample| {
        sample_time(sample, sample_rate)
            .ok()
            .and_then(|time| admitted_raw_source_time(source, time).ok())
            .is_some_and(|raw| {
                source
                    .source_duration
                    .is_some_and(|duration| !raw.is_negative() && raw < duration)
                    && quantize_sample_boundary(raw, source_sample_rate)
                        .ok()
                        .is_some_and(|sample| sample >= 0 && sample < source_sample_count)
            })
    })
}

fn range_details(start: i64, end: i64) -> BTreeMap<String, String> {
    details([("start", start.to_string()), ("end", end.to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_render_projection_has_exact_canonical_bytes_and_domain_hash() {
        let profile = ExecutionProfile::new(
            "common",
            "valle.numeric/common@1",
            COMMON_AUDIO_ABI,
            "valle.motion/eval@1",
            CAPTION_GLYPH_RUN_ABI,
            ExecutionLimits::new(64, 8, 32).unwrap(),
        )
        .unwrap();
        let profile_bytes = execution_profile_projection_bytes(&profile.projection).unwrap();

        let (timeline, manifest, bundle, execution_profile) =
            crate::fixed_package::empty_fixed_package_fixtures();
        let files = crate::fixed_package::fixed_package_files(
            &timeline,
            &manifest,
            &bundle,
            &execution_profile,
        );
        let package_manifest =
            crate::fixed_package::canonical_fixed_package_manifest(&files).unwrap();
        let opened =
            crate::fixed_package::open_verified_fixed_package(&package_manifest, &files).unwrap();
        let compiled = opened.compiled();
        let snapshot_projection = SnapshotProjection {
            roots: &compiled.resources.roots,
            resources: compiled
                .resources
                .resources
                .iter()
                .map(|resource| ResolvedResourceProjection {
                    kind: resource.kind,
                    digest: &resource.digest,
                    facts: &resource.facts,
                    entry: &resource.entry,
                    dependencies: &resource.dependencies,
                })
                .collect(),
        };
        let snapshot_bytes = serde_jcs::to_vec(&snapshot_projection).unwrap();
        let render_projection = RenderProjection {
            profile: RenderProfileProjection {
                numeric_abi: &profile.projection.numeric_abi,
                audio_abi: &profile.projection.audio_abi,
                motion_abi: &profile.projection.motion_abi,
                caption_abi: &profile.projection.caption_abi,
            },
            resources: snapshot_projection,
            canvas: compiled.canvas,
            sources: &compiled.sources,
            visual: &compiled.visual,
            adjustments: &compiled.adjustments,
            captions: &compiled.captions,
            audio: &compiled.audio,
            camera: &compiled.camera,
            kernels: &compiled.kernels,
        };
        let render_bytes = render_projection_bytes(&render_projection).unwrap();

        assert_eq!(
            profile_bytes,
            br#"{"audioAbi":"valle.audio/common@1","captionAbi":"valle.caption/cosmic-text-glyph-run@1","limits":{"maxDependencyDepth":8,"maxExtensionKernels":32,"maxResources":64},"motionAbi":"valle.motion/eval@1","name":"common","numericAbi":"valle.numeric/common@1"}"#
        );
        assert_eq!(snapshot_bytes, br#"{"resources":[],"roots":[]}"#);
        assert_eq!(
            render_bytes,
            br#"{"adjustments":{"clips":[]},"audio":{"sampleCount":48000,"sampleRate":48000,"tracks":[]},"camera":null,"canvas":{"backgroundRgba":[0,0,0,255],"channelLayout":"stereo","colorSpace":"srgb","duration":"1/1","frameCount":30,"frameRate":"30/1","height":180,"sampleCount":48000,"sampleRate":48000,"width":320},"captions":{"tracks":[]},"kernels":[],"profile":{"audioAbi":"valle.audio/common@1","captionAbi":"valle.caption/cosmic-text-glyph-run@1","motionAbi":"valle.motion/eval@1","numericAbi":"valle.numeric/common@1"},"resources":{"resources":[],"roots":[]},"sources":{"entries":[]},"visual":{"tracks":[]}}"#
        );
        assert_eq!(
            compiled.render_id,
            RenderId::from_canonical_bytes(&render_bytes)
        );
    }

    #[test]
    fn render_id_tracks_profile_semantics_but_ignores_labels_and_admission_budgets() {
        let (timeline_json, manifest_json, _, _) =
            crate::fixed_package::empty_fixed_package_fixtures();
        let timeline = valle_timeline::internal::decode_canonical(&timeline_json).unwrap();
        let manifest =
            valle_timeline::internal::decode_resource_manifest(manifest_json.as_bytes()).unwrap();
        let compile_with = |profile: &ExecutionProfile| {
            open_engine_render(
                &timeline,
                &manifest,
                &ResourceBindings::new(),
                &Capabilities::new(),
                profile,
            )
            .unwrap()
        };
        let baseline = ExecutionProfile::new(
            "common",
            "valle.numeric/common@1",
            COMMON_AUDIO_ABI,
            "valle.motion/eval@1",
            CAPTION_GLYPH_RUN_ABI,
            ExecutionLimits::new(64, 8, 32).unwrap(),
        )
        .unwrap();
        let nonsemantic = ExecutionProfile::new(
            "renamed-producer-policy",
            "valle.numeric/common@1",
            COMMON_AUDIO_ABI,
            "valle.motion/eval@1",
            CAPTION_GLYPH_RUN_ABI,
            ExecutionLimits::new(128, 16, 64).unwrap(),
        )
        .unwrap();
        let changed_numeric_abi = ExecutionProfile::new(
            "common",
            "valle.numeric/alternate@1",
            COMMON_AUDIO_ABI,
            "valle.motion/eval@1",
            CAPTION_GLYPH_RUN_ABI,
            ExecutionLimits::new(64, 8, 32).unwrap(),
        )
        .unwrap();

        assert_eq!(
            compile_with(&baseline).render_id(),
            compile_with(&nonsemantic).render_id()
        );
        assert_ne!(
            compile_with(&baseline).render_id(),
            compile_with(&changed_numeric_abi).render_id()
        );
    }
}
