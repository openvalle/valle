//! Product projection from an admitted Timeline render into the mechanical compositor IR.
//!
//! The projection consumes only immutable compiled/evaluated state. It never reopens a Timeline,
//! manifest, mutable registry, or host probe result.

use std::f64::consts::PI;

use valle_draw::{
    Rect, Rgba,
    program::{
        BatchGeometry, BatchInstance, DrawProgramBuilder, GeometryBatchNode, LinearColor, Node,
    },
};

use valle_timeline::internal::wire::resource::{
    ColorMatrixWire, ColorPrimariesWire, ColorTransferWire, MediaColorDescriptorWire,
    MediaOrientationWire,
};

use crate::{
    frame::{
        Anchor, BackgroundBand, CanvasMapping, CompositeBlendMode, EvaluatedVisualClip, LayerInset,
        MaskShape, RasterFit, ResolvedCamera, ResolvedClipAnimation, ResolvedMask,
        ResolvedTransform, SourceBoundary, SourceCrop, VisualSource,
    },
    render::{
        CompiledAdjustmentEffect, CompiledBlendMode, CompiledKernelCall, CompiledRasterFit,
        CompiledRender, CompiledSourceKind, CompiledTransitionKernel, EvaluatedRenderFrame,
        EvaluatedResourceRef, EvaluatedSourceRef, EvaluatedVisualLayer as EvaluatedRenderLayer,
        EvaluatedVisualMask, EvaluatedVisualOperation,
        EvaluatedVisualTransition as EvaluatedRenderTransition, VerifiedResourceFacts,
    },
    resource::{
        AuthorSrgbStraight, ColorDescription, ColorPrimaries, ColorRange, ContentDigest,
        InputAlphaMode, MatrixCoefficients, MediaDescriptor, OperatorAlphaBehavior,
        OperatorColorDomain, PixelOrientation, SemanticAsset, SemanticAssetKind, SemanticFont,
        SemanticStructure, SignalLuminance, StructureDescriptor, StructureFootprint,
        TransferFunction, VisualInterpretation,
    },
};

use super::{
    DynamicBindingKind, DynamicValue, FrameInspection, FrameMetadata, PrepareError, PrepareOutput,
    PrepareState, PreparedEffect, PreparedEffectKernel, PreparedEffectSpace, PreparedFrame,
    PreparedLayer, PreparedProgramKind, PreparedSource, PreparedSourceKind, PreparedTransition,
    PreparedTransitionKernel, PreparedVisualItem, ProductPrepareCaches, motion, prepare_background,
    validate,
};

#[cfg(feature = "text")]
use super::PreparedCaption;

/// Prepares one frame from the exact immutable render that produced `evaluated`.
pub fn prepare_compiled_render_frame_cached(
    render: &CompiledRender,
    evaluated: &EvaluatedRenderFrame,
    render_spec: &crate::frame::RenderSpec,
    caches: &mut ProductPrepareCaches,
) -> Result<PrepareOutput, PrepareError> {
    if evaluated.render_id() != render.render_id() {
        return Err(PrepareError::at(
            "renderId",
            format!(
                "evaluated frame render {} does not match compiled render {}",
                evaluated.render_id(),
                render.render_id()
            ),
        ));
    }
    let canvas = render.canvas();
    let canvas_size = [canvas.width(), canvas.height()];
    let mapping = CanvasMapping::contain(canvas_size, *render_spec);
    let camera = evaluated.camera().map(|camera| ResolvedCamera {
        center_x: camera.center_x(),
        center_y: camera.center_y(),
        zoom: camera.zoom(),
        // Authored camera rotation is clockwise; the visual band receives the inverse view
        // transform in composition space.
        rotation: -camera.rotation() * 180.0 / PI,
        target: None,
    });
    let background = prepare_background(BackgroundBand {
        author_srgb_straight: AuthorSrgbStraight(canvas.background_rgba()),
    })?;
    let mut state = PrepareState::new_with_context(
        super::PrepareFrameContext {
            render_spec: *render_spec,
            canvas_mapping: mapping,
            camera,
        },
        caches,
    );
    let mut visual = Vec::with_capacity(evaluated.visual().len());
    for (index, operation) in evaluated.visual().iter().enumerate() {
        let path = format!("visual[{index}]");
        match operation {
            EvaluatedVisualOperation::Clip(clip) => {
                let layer = prepare_endpoint(
                    render,
                    evaluated,
                    clip.clip_id(),
                    clip.track_id(),
                    clip.source(),
                    clip.layer(),
                    &path,
                    &mut state,
                )?;
                visual.push(PreparedVisualItem::Layer(Box::new(layer)));
            }
            EvaluatedVisualOperation::Transition(transition) => {
                visual.push(PreparedVisualItem::Transition(Box::new(
                    prepare_transition(render, evaluated, transition, &path, &mut state)?,
                )));
            }
        }
    }

    let mut adjustments = Vec::with_capacity(evaluated.adjustments().len());
    for (index, effect) in evaluated.adjustments().iter().enumerate() {
        let path = format!("adjustments[{index}]");
        adjustments.push(match effect.effect() {
            CompiledAdjustmentEffect::ColorGrade { temperature } => PreparedEffect {
                semantic_path: path,
                space: PreparedEffectSpace::Root,
                kernel: PreparedEffectKernel::ColorGrade {
                    brightness: 0.0,
                    contrast: 0.0,
                    saturation: 0.0,
                    temperature: *temperature as f32,
                    vignette: 0.0,
                },
            },
            CompiledAdjustmentEffect::Extension { call } => {
                prepared_extension_effect(render, call, &path, PreparedEffectSpace::Root)?
            }
        });
    }

    #[cfg(feature = "text")]
    let captions = {
        let mut prepared = Vec::with_capacity(evaluated.captions().len());
        for (index, caption) in evaluated.captions().iter().enumerate() {
            prepared.push(prepare_compiled_caption(
                render,
                caption,
                &format!("captions[{index}]"),
                &mut state,
            )?);
        }
        prepared
    };
    #[cfg(not(feature = "text"))]
    let captions = {
        if !evaluated.captions().is_empty() {
            return Err(PrepareError::at(
                "captions",
                "render with captions bypassed text-backend admission",
            ));
        }
        Vec::new()
    };

    let (resources, resource_requests) = state
        .requests
        .finish()
        .map_err(|error| PrepareError::at("resources", error))?;
    let dynamic = state.dynamic.finish();
    let frame = PreparedFrame {
        render_id: render.render_id(),
        key: evaluated.frame(),
        sample_time: evaluated.sample_time(),
        render_spec: *render_spec,
        background,
        visual,
        adjustments,
        captions,
        programs: state.programs,
        resources,
        metadata: FrameMetadata {
            geometry: state.geometry,
        },
    };
    validate::validate_constructed_prepared_frame(&frame)
        .map_err(|error| PrepareError::at("frame", error))?;
    let output = PrepareOutput {
        frame,
        resource_requests,
        dynamic,
        diagnostics: state.diagnostics,
        inspection: FrameInspection {
            motion: state.motion_inspection,
        },
    };
    validate::validate_prepare_output_bindings(&output)
        .map_err(|error| PrepareError::at("output", error))?;
    Ok(output)
}

fn prepare_transition(
    render: &CompiledRender,
    evaluated: &EvaluatedRenderFrame,
    transition: &EvaluatedRenderTransition,
    path: &str,
    state: &mut PrepareState<'_>,
) -> Result<PreparedTransition, PrepareError> {
    let kernel = match transition.kernel() {
        CompiledTransitionKernel::CrossFade => PreparedTransitionKernel::Fade,
        CompiledTransitionKernel::Extension { call } => {
            let descriptor = render.admitted_kernel(call.kernel_index()).ok_or_else(|| {
                PrepareError::at(format!("{path}.kernel"), "missing admitted kernel")
            })?;
            if descriptor.abi() != crate::render::EXTENSION_CROSS_FADE_ABI {
                return Err(PrepareError::at(
                    format!("{path}.kernel"),
                    "compiled transition ABI is not executable",
                ));
            }
            let expected = crate::render::engine_owned_kernel_implementation_digest(
                crate::render::EXTENSION_CROSS_FADE_ABI,
            )
            .expect("cross-fade ABI has an engine-owned implementation");
            if descriptor.implementation_digest() != &expected {
                return Err(PrepareError::at(
                    format!("{path}.kernel"),
                    "compiled transition implementation is not executable",
                ));
            }
            PreparedTransitionKernel::ExtensionCrossFade {
                implementation_sha256: digest_bytes(descriptor.implementation_digest())?,
                past_frames: descriptor.visual_footprint().past_frames(),
                future_frames: descriptor.visual_footprint().future_frames(),
            }
        }
    };
    let progress = state
        .dynamic
        .push(
            format!("{path}.progress"),
            DynamicBindingKind::TransitionProgress,
            DynamicValue::Scalar(transition.progress().as_f64()),
        )
        .map_err(|error| PrepareError::at(path, error))?;
    let from = prepare_endpoint(
        render,
        evaluated,
        transition.from_clip_id(),
        transition.track_id(),
        transition.from(),
        transition.from_layer(),
        &format!("{path}.from"),
        state,
    )?;
    let to = prepare_endpoint(
        render,
        evaluated,
        transition.to_clip_id(),
        transition.track_id(),
        transition.to(),
        transition.to_layer(),
        &format!("{path}.to"),
        state,
    )?;
    Ok(PreparedTransition {
        semantic_path: path.to_owned(),
        kernel,
        progress,
        from,
        to,
    })
}

fn prepare_endpoint(
    render: &CompiledRender,
    evaluated: &EvaluatedRenderFrame,
    clip_id: &str,
    track_id: &str,
    source: &EvaluatedSourceRef,
    layer: &EvaluatedRenderLayer,
    path: &str,
    state: &mut PrepareState<'_>,
) -> Result<PreparedLayer, PrepareError> {
    let mut prepared = match source.kind() {
        CompiledSourceKind::Video | CompiledSourceKind::Image | CompiledSourceKind::Lottie => {
            let clip = adapt_endpoint(render, clip_id, track_id, source, layer, path)?;
            state.layer(&clip, path, source.sample_time())?
        }
        CompiledSourceKind::Solid => {
            let clip = adapt_program_clip(render, clip_id, track_id, source, layer, path)?;
            let program_id = state.next_program_id(path)?;
            let program_path = format!("{path}.solid");
            let viewport = state.local_program_extent(&clip.transform, path)?;
            let color = render
                .sources()
                .source(source.source_index())
                .and_then(|source| source.solid_color())
                .ok_or_else(|| {
                    PrepareError::at(format!("{path}.source"), "solid payload missing")
                })?;
            let rgba = valle_timeline::Color(color.to_owned())
                .parse_rgba()
                .ok_or_else(|| {
                    PrepareError::at(format!("{path}.source.color"), "invalid admitted color")
                })?;
            let mut builder = DrawProgramBuilder::new(Rect::new(
                0.0,
                0.0,
                f64::from(viewport.width()),
                f64::from(viewport.height()),
            ));
            let root = builder.push_node(Node::GeometryBatch(GeometryBatchNode {
                geometry: BatchGeometry::Rect,
                instances: vec![BatchInstance {
                    position: [0.0, 0.0],
                    size: [f64::from(viewport.width()), f64::from(viewport.height())],
                    color: LinearColor::from_srgb8(Rgba::new(rgba[0], rgba[1], rgba[2], rgba[3])),
                }],
            }));
            builder.add_root(root);
            let fixture = motion::FixtureProgram::new(
                builder
                    .finish()
                    .map_err(|error| PrepareError::at(&program_path, error))?,
            );
            let program = motion::prepare_program(
                program_id,
                motion::ProgramPrepareContext {
                    kind: PreparedProgramKind::Solid,
                    semantic_path: &program_path,
                    fixture: &fixture,
                    caption_fonts: &[],
                },
                &mut state.requests,
            )
            .map_err(|error| PrepareError::at(&program_path, error))?;
            state.programs.push(program);
            state.finish_compiled_program_layer(
                &clip,
                path,
                PreparedSource::Program {
                    source_kind: PreparedSourceKind::Solid,
                    program: program_id,
                },
            )?
        }
        CompiledSourceKind::Motion => {
            let clip = adapt_program_clip(render, clip_id, track_id, source, layer, path)?;
            let compiled_source =
                render
                    .sources()
                    .source(source.source_index())
                    .ok_or_else(|| {
                        PrepareError::at(format!("{path}.source"), "compiled source missing")
                    })?;
            let instance = compiled_source.motion().ok_or_else(|| {
                PrepareError::at(format!("{path}.source"), "compiled Motion instance missing")
            })?;
            let prepared_scene = render.motion_scene(instance).ok_or_else(|| {
                PrepareError::at(
                    format!("{path}.source.component"),
                    "admitted Motion executable payload missing",
                )
            })?;
            let frame_address = render
                .motion_frame_address(render.render_id(), evaluated.frame(), source.source_index())
                .map_err(|error| PrepareError::at(format!("{path}.source.frame"), error))?;
            let viewport = state.local_program_extent(&clip.transform, path)?;
            let program_to_device = super::bounds::layer_device_transform(
                &clip.transform,
                &clip.resolved_animation,
                state.frame.camera.as_ref(),
                state.frame.canvas_mapping,
                [
                    state.frame.render_spec.width(),
                    state.frame.render_spec.height(),
                ],
            )
            .map_err(|error| PrepareError::at(path, error))?;
            let assets = compiled_motion_assets(source, path)?;
            let font_dependencies: Vec<_> = source
                .motion_artifact_dependencies()
                .iter()
                .filter_map(|resource| match resource.facts() {
                    VerifiedResourceFacts::Font { bytes, .. } => {
                        Some((*resource.digest(), bytes.as_ref()))
                    }
                    _ => None,
                })
                .collect();
            let fonts = state
                .motion_fonts
                .get(&font_dependencies)
                .map_err(|error| PrepareError::at(format!("{path}.motion"), error))?;
            let built =
                motion::build_compiled_motion_program(motion::CompiledMotionProgramContext {
                    prepared: prepared_scene,
                    instance,
                    evaluated: source,
                    source_frame: frame_address.source_frame(),
                    viewport,
                    fps: render.canvas().frame_rate(),
                    styles: state.motion_styles,
                    faces: state.motion_faces,
                    fonts,
                    program_to_device,
                    clip_id: &clip.clip_id,
                    render_seed: render_seed(render),
                    assets: &assets,
                })
                .map_err(|error| PrepareError::at(format!("{path}.motion"), error))?;
            if !built.scene3d_frames.is_empty() {
                return Err(PrepareError::at(
                    format!("{path}.motion.scene3d"),
                    "Scene3D entered a render that did not admit Model3d resources",
                ));
            }
            let mut fixture = motion::FixtureProgram::new(built.program);
            for (name, asset) in &assets {
                fixture = fixture
                    .with_asset(name.clone(), asset.clone())
                    .with_asset(format!("asset://{name}"), asset.clone());
            }
            fixture = install_compiled_motion_fonts(fixture, source, path)?;
            fixture = install_compiled_motion_shaders(render, fixture, source, path)?;
            let program_id = state.next_program_id(path)?;
            let program_path = format!("{path}.motion");
            let program = motion::prepare_program(
                program_id,
                motion::ProgramPrepareContext {
                    kind: PreparedProgramKind::Motion,
                    semantic_path: &program_path,
                    fixture: &fixture,
                    caption_fonts: &[],
                },
                &mut state.requests,
            )
            .map_err(|error| PrepareError::at(&program_path, error))?;
            state
                .motion_inspection
                .push(crate::prepare::MotionFrameInspection {
                    clip_id: clip.clip_id.clone(),
                    composition_frame: frame_address.composition_frame().index(),
                    source_index: frame_address.source_index(),
                    source_frame: frame_address.source_frame(),
                    boxes: built.layout_boxes,
                });
            state.programs.push(program);
            state.finish_compiled_program_layer(
                &clip,
                path,
                PreparedSource::Program {
                    source_kind: PreparedSourceKind::Motion,
                    program: program_id,
                },
            )?
        }
        CompiledSourceKind::Audio => {
            return Err(PrepareError::at(
                format!("{path}.source"),
                "audio source entered the visual program",
            ));
        }
    };
    for (index, call) in layer.filters().iter().enumerate() {
        prepared.effects.push(prepared_extension_effect(
            render,
            call,
            &format!("{path}.effects[{index}]"),
            PreparedEffectSpace::Layer {
                transform: prepared.dynamic.transform,
                bounds: prepared.dynamic.bounds,
            },
        )?);
    }
    Ok(prepared)
}

fn compiled_motion_assets(
    source: &EvaluatedSourceRef,
    path: &str,
) -> Result<std::collections::BTreeMap<String, SemanticAsset>, PrepareError> {
    let mut assets = std::collections::BTreeMap::new();
    for (control, resource) in source.motion_resources() {
        if matches!(
            resource.facts(),
            VerifiedResourceFacts::Video { .. }
                | VerifiedResourceFacts::Image { .. }
                | VerifiedResourceFacts::Lottie { .. }
        ) {
            assets.insert(control.clone(), semantic_asset(resource, path)?);
        }
    }
    for resource in source.motion_artifact_dependencies() {
        if matches!(
            resource.facts(),
            VerifiedResourceFacts::Video { .. }
                | VerifiedResourceFacts::Image { .. }
                | VerifiedResourceFacts::Lottie { .. }
        ) {
            let asset = semantic_asset(resource, path)?;
            assets
                .entry(resource.role().to_owned())
                .or_insert_with(|| asset.clone());
            assets
                .entry(resource.resource_id().to_owned())
                .or_insert(asset);
        }
    }
    Ok(assets)
}

fn install_compiled_motion_fonts(
    mut fixture: motion::FixtureProgram,
    source: &EvaluatedSourceRef,
    path: &str,
) -> Result<motion::FixtureProgram, PrepareError> {
    // Font identities are shared metadata; Wasm obtains actual bytes from render resources.
    static DEFAULT_FONTS: std::sync::OnceLock<Vec<SemanticFont>> = std::sync::OnceLock::new();
    for font in DEFAULT_FONTS.get_or_init(|| {
        valle_motion::DEFAULT_MOTION_FONT_FILES
            .iter()
            .map(|name| {
                let spec = valle_motion::runtime_fonts::specs()
                    .iter()
                    .find(|spec| spec.name == *name)
                    .expect("default font identity");
                SemanticFont::new(
                    *name,
                    ContentDigest::from_hex(&spec.sha256).expect("default font digest"),
                    0,
                )
            })
            .collect()
    }) {
        fixture = fixture.with_font(font.clone());
    }
    for resource in source.motion_artifact_dependencies() {
        let VerifiedResourceFacts::Font { descriptor, .. } = resource.facts() else {
            continue;
        };
        fixture = fixture.with_font(SemanticFont::new(
            resource.resource_id(),
            resource_content_digest(resource, path)?,
            descriptor.face_index,
        ));
    }
    Ok(fixture)
}

fn install_compiled_motion_shaders(
    render: &CompiledRender,
    mut fixture: motion::FixtureProgram,
    source: &EvaluatedSourceRef,
    path: &str,
) -> Result<motion::FixtureProgram, PrepareError> {
    for resource in source.motion_artifact_dependencies() {
        if !matches!(resource.facts(), VerifiedResourceFacts::Shader { .. }) {
            continue;
        }
        let package = render.shader_package(resource).ok_or_else(|| {
            PrepareError::at(
                format!("{path}.motion.shader"),
                "admitted shader executable payload missing",
            )
        })?;
        let digest = package.content_hash;
        let abi_digest = package.manifest.abi_digest;
        fixture = fixture.with_structure(SemanticStructure::new(
            package.uri().to_string(),
            digest,
            StructureDescriptor::RuntimeShader {
                abi_digest,
                color_domain: OperatorColorDomain::PerceptualSrgb,
                alpha_behavior: OperatorAlphaBehavior::RewritesCoverage,
                footprint: StructureFootprint::Local,
            },
        ));
    }
    Ok(fixture)
}

fn resource_content_digest(
    resource: &EvaluatedResourceRef,
    _path: &str,
) -> Result<ContentDigest, PrepareError> {
    Ok(*resource.digest())
}

fn render_seed(render: &CompiledRender) -> u32 {
    const DOMAIN: &[u8] = b"valle.render-seed/1\0";
    let mut preimage = [0_u8; DOMAIN.len() + 32];
    preimage[..DOMAIN.len()].copy_from_slice(DOMAIN);
    preimage[DOMAIN.len()..].copy_from_slice(render.render_id().as_bytes());
    let digest = valle_timeline::internal::hash::sha256(&preimage);
    u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]])
}

#[cfg(feature = "text")]
fn prepare_compiled_caption(
    render: &CompiledRender,
    caption: &crate::render::EvaluatedCaption,
    path: &str,
    state: &mut PrepareState<'_>,
) -> Result<PreparedCaption, PrepareError> {
    let VerifiedResourceFacts::Font { descriptor, bytes } = caption.font().facts() else {
        return Err(PrepareError::at(
            format!("{path}.font"),
            "compiled caption font facts changed after admission",
        ));
    };
    let digest = resource_content_digest(caption.font(), path)?;
    let family = caption.font().resource_id();
    let semantics = state
        .text_semantics
        .as_deref_mut()
        .ok_or_else(|| PrepareError::at(path, "compiled caption shaper is unavailable"))?;
    semantics
        .register_program_font(
            family,
            &digest.as_hex(),
            descriptor.face_index,
            std::sync::Arc::new(bytes.to_vec()),
        )
        .map_err(|error| PrepareError::at(format!("{path}.font"), error))?;

    // Presentation and typed caption behaviors share the exact same admitted
    // caption-local frame-left clock. Do not reconstruct it from composition
    // time or the authored (possibly non-frame-aligned) sequence start here.
    let elapsed = caption.local_time();
    let behavior = match caption.behavior() {
        None => crate::text_semantic::CaptionBehavior::Static,
        Some(crate::render::CompiledCaptionBehavior::Scroll { horizontal, speed }) => {
            crate::text_semantic::CaptionBehavior::Scroll {
                horizontal: *horizontal,
                speed: *speed,
                elapsed_seconds: elapsed.as_f64(),
            }
        }
        Some(crate::render::CompiledCaptionBehavior::Karaoke { line_mode }) => {
            crate::text_semantic::CaptionBehavior::Karaoke {
                reveal_bytes: karaoke_reveal_bytes(
                    caption.runs(),
                    caption.run_timings(),
                    elapsed,
                    *line_mode,
                )
                .ok_or_else(|| {
                    PrepareError::at(
                        format!("{path}.runs"),
                        "compiled karaoke run timing is incomplete",
                    )
                })?,
            }
        }
    };
    let align = compiled_caption_alignment(caption.align());
    let run_styles = caption
        .run_styles()
        .iter()
        .enumerate()
        .map(|(index, style)| {
            let color = valle_timeline::Color(style.color().to_owned())
                .parse_rgba()
                .ok_or_else(|| {
                    PrepareError::at(
                        format!("{path}.runs.{index}.style.color"),
                        "invalid admitted color",
                    )
                })?;
            Ok(crate::text_semantic::CaptionRunStyle {
                font_size: style.font_size(),
                color,
                font_weight: style.font_weight(),
            })
        })
        .collect::<Result<Vec<_>, PrepareError>>()?;
    let shadow = caption
        .shadow()
        .map(|shadow| -> Result<_, PrepareError> {
            let color = valle_timeline::Color(shadow.color().to_owned())
                .parse_rgba()
                .ok_or_else(|| {
                    PrepareError::at(
                        format!("{path}.style.shadow.color"),
                        "invalid admitted shadow color",
                    )
                })?;
            Ok(crate::text_semantic::CaptionShadow {
                color,
                offset: shadow.offset(),
                blur_sigma: shadow.blur_sigma(),
            })
        })
        .transpose()?;
    let presentation = caption.presentation();
    let draw = semantics
        .build_caption_draw_program(
            &crate::text_semantic::CaptionDrawInput {
                runs: caption.runs(),
                run_styles: &run_styles,
                font_family: family,
                shadow,
                region: caption.region(),
                align,
                presentation: crate::text_semantic::CaptionPresentation {
                    opacity: presentation.opacity(),
                    translation: presentation.translation(),
                    scale: presentation.scale(),
                    rotation: presentation.rotation(),
                    clip_inset: presentation.clip_inset(),
                    blur_sigma: presentation.blur_sigma(),
                },
                behavior,
            },
            render.canvas().width(),
            render.canvas().height(),
        )
        .map_err(|error| PrepareError::at(format!("{path}.program"), error))?;
    if !draw.requirements().destination_uses.is_empty() {
        return Err(PrepareError::at(
            format!("{path}.program"),
            "compiled caption unexpectedly reads the visual destination",
        ));
    }
    let program_id = state.next_program_id(path)?;
    let fixture = motion::FixtureProgram::new(draw).with_font(SemanticFont::new(
        family,
        digest,
        descriptor.face_index,
    ));
    let program_path = format!("{path}.program");
    let program = motion::prepare_program(
        program_id,
        motion::ProgramPrepareContext {
            kind: PreparedProgramKind::Caption,
            semantic_path: &program_path,
            fixture: &fixture,
            caption_fonts: &[],
        },
        &mut state.requests,
    )
    .map_err(|error| PrepareError::at(&program_path, error))?;
    state.programs.push(program);
    state.finish_compiled_caption(
        program_id,
        format!(
            "caption/{}/{}",
            caption.track_order(),
            caption.range().start()
        ),
        format!("caption-track/{}", caption.track_order()),
        path,
    )
}

#[cfg(feature = "text")]
fn compiled_caption_alignment(align: crate::render::CompiledCaptionAlign) -> [f64; 2] {
    use crate::render::CompiledCaptionAlign;
    match align {
        CompiledCaptionAlign::TopLeft => [0.0, 0.0],
        CompiledCaptionAlign::TopCenter => [0.5, 0.0],
        CompiledCaptionAlign::TopRight => [1.0, 0.0],
        CompiledCaptionAlign::CenterLeft => [0.0, 0.5],
        CompiledCaptionAlign::Center => [0.5, 0.5],
        CompiledCaptionAlign::CenterRight => [1.0, 0.5],
        CompiledCaptionAlign::BottomLeft => [0.0, 1.0],
        CompiledCaptionAlign::BottomCenter => [0.5, 1.0],
        CompiledCaptionAlign::BottomRight => [1.0, 1.0],
    }
}

#[cfg(feature = "text")]
fn karaoke_reveal_bytes(
    runs: &[String],
    timings: &[Option<crate::render::CompiledCaptionRunTiming>],
    elapsed: valle_timeline::RationalTime,
    line_mode: bool,
) -> Option<usize> {
    if runs.len() != timings.len() {
        return None;
    }
    let text = runs.concat();
    let mut reveal = 0_usize;
    for (run, timing) in runs.iter().zip(timings) {
        let timing = timing.as_ref()?;
        if elapsed < timing.start() {
            break;
        }
        reveal = reveal.checked_add(run.len())?;
        if elapsed < timing.end() {
            break;
        }
    }
    if line_mode && reveal > 0 {
        reveal = text[reveal..]
            .find('\n')
            .map_or(text.len(), |offset| reveal + offset);
    }
    Some(reveal)
}

fn prepared_extension_effect(
    render: &CompiledRender,
    call: &CompiledKernelCall,
    path: &str,
    space: PreparedEffectSpace,
) -> Result<PreparedEffect, PrepareError> {
    let descriptor = render
        .admitted_kernel(call.kernel_index())
        .ok_or_else(|| PrepareError::at(path, "missing admitted extension kernel"))?;
    if descriptor.abi() != crate::render::EXTENSION_COLOR_GAIN_ABI {
        return Err(PrepareError::at(
            path,
            "compiled extension ABI is not executable",
        ));
    }
    let expected = crate::render::engine_owned_kernel_implementation_digest(
        crate::render::EXTENSION_COLOR_GAIN_ABI,
    )
    .expect("color-gain ABI has an engine-owned implementation");
    if descriptor.implementation_digest() != &expected {
        return Err(PrepareError::at(
            path,
            "compiled extension implementation is not executable",
        ));
    }
    let gain = call
        .parameters()
        .get("gain")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| PrepareError::at(path, "admitted color-gain packet is missing gain"))?;
    Ok(PreparedEffect {
        semantic_path: path.to_owned(),
        space,
        kernel: PreparedEffectKernel::ExtensionColorGain {
            implementation_sha256: digest_bytes(descriptor.implementation_digest())?,
            gain: gain as f32,
            past_frames: descriptor.visual_footprint().past_frames(),
            future_frames: descriptor.visual_footprint().future_frames(),
        },
    })
}

fn digest_bytes(digest: &ContentDigest) -> Result<[u8; 32], PrepareError> {
    Ok(*digest.as_bytes())
}

fn adapt_endpoint(
    render: &CompiledRender,
    clip_id: &str,
    track_id: &str,
    source: &EvaluatedSourceRef,
    layer: &EvaluatedRenderLayer,
    path: &str,
) -> Result<EvaluatedVisualClip, PrepareError> {
    let resource = source.resource().ok_or_else(|| {
        PrepareError::at(
            format!("{path}.source"),
            "solid and Motion sources require a compiled program producer",
        )
    })?;
    let asset = semantic_asset(resource, path)?;
    let extent = asset
        .descriptor
        .extent()
        .ok_or_else(|| PrepareError::at(path, "visual source has no extent"))?;
    let transform = adapt_transform(
        render,
        layer,
        source.source_index(),
        extent.width(),
        extent.height(),
        path,
    )?;
    let source_boundary = match source.kind() {
        CompiledSourceKind::Image => SourceBoundary::Static,
        CompiledSourceKind::Video | CompiledSourceKind::Lottie => SourceBoundary::ClampToDescriptor,
        _ => {
            return Err(PrepareError::at(
                format!("{path}.source"),
                "source kind has no external visual projection",
            ));
        }
    };
    Ok(EvaluatedVisualClip {
        clip_id: clip_id.to_owned(),
        track_id: track_id.to_owned(),
        source: VisualSource::Asset {
            id: asset.id.clone(),
            asset_kind: asset.kind,
            digest: asset.digest.clone(),
            descriptor: asset.descriptor.clone(),
        },
        transform,
        source_crop: SourceCrop::default(),
        source_time_s: source.sample_time().as_f64(),
        source_boundary,
        mask: layer.mask().map(adapt_mask),
        blend: adapt_blend(layer.blend()),
        resolved_animation: ResolvedClipAnimation::RASTER_IDENTITY,
    })
}

fn adapt_program_clip(
    render: &CompiledRender,
    clip_id: &str,
    track_id: &str,
    source: &EvaluatedSourceRef,
    layer: &EvaluatedRenderLayer,
    path: &str,
) -> Result<EvaluatedVisualClip, PrepareError> {
    let canvas = render.canvas();
    let transform =
        adapt_transform_with_fit(render, layer, canvas.width(), canvas.height(), None, path)?;
    let digest = ContentDigest::from_bytes([0; 32]);
    let descriptor = MediaDescriptor::visual(
        crate::resource::Extent2d::new(canvas.width(), canvas.height())
            .expect("compiled canvas is non-empty"),
        None,
        VisualInterpretation::new(
            ColorDescription::SRGB,
            SignalLuminance::SDR_100,
            InputAlphaMode::StraightCoverage,
        ),
    )
    .map_err(|error| PrepareError::at(path, error))?;
    Ok(EvaluatedVisualClip {
        clip_id: clip_id.to_owned(),
        track_id: track_id.to_owned(),
        source: VisualSource::Asset {
            id: format!("compiled-program/{}", source.source_index()),
            asset_kind: SemanticAssetKind::Image,
            digest,
            descriptor,
        },
        transform,
        source_crop: SourceCrop::default(),
        source_time_s: source.sample_time().as_f64(),
        source_boundary: SourceBoundary::ExtendedSemantic,
        mask: layer.mask().map(adapt_mask),
        blend: adapt_blend(layer.blend()),
        resolved_animation: ResolvedClipAnimation::RASTER_IDENTITY,
    })
}

fn adapt_mask(mask: &EvaluatedVisualMask) -> ResolvedMask {
    let (kind, rect, feather, invert) = match mask {
        EvaluatedVisualMask::Rect {
            rect,
            feather,
            invert,
        } => (MaskShape::Rect, *rect, *feather, *invert),
        EvaluatedVisualMask::Ellipse {
            rect,
            feather,
            invert,
        } => (MaskShape::Ellipse, *rect, *feather, *invert),
    };
    ResolvedMask {
        kind,
        x: rect[0] + rect[2] * 0.5,
        y: rect[1] + rect[3] * 0.5,
        width: rect[2],
        height: rect[3],
        feather,
        rotation: 0.0,
        invert,
    }
}

fn adapt_transform(
    render: &CompiledRender,
    layer: &EvaluatedRenderLayer,
    source_index: u32,
    source_width: u32,
    source_height: u32,
    path: &str,
) -> Result<ResolvedTransform, PrepareError> {
    let raster_fit = render
        .sources()
        .source(source_index)
        .and_then(|source| source.raster_fit())
        .ok_or_else(|| PrepareError::at(path, "source has no compiled raster fit"))?;
    let fit = match raster_fit {
        CompiledRasterFit::Contain => RasterFit::Contain,
        CompiledRasterFit::Cover => RasterFit::Cover,
        CompiledRasterFit::Fill => RasterFit::Fill,
        // The outer transform already has the admitted intrinsic extent, so
        // Fill performs no second aspect-ratio policy for `fit: none`.
        CompiledRasterFit::None => RasterFit::Fill,
    };
    adapt_transform_with_fit(render, layer, source_width, source_height, Some(fit), path)
}

fn adapt_transform_with_fit(
    render: &CompiledRender,
    layer: &EvaluatedRenderLayer,
    source_width: u32,
    source_height: u32,
    fit: Option<RasterFit>,
    path: &str,
) -> Result<ResolvedTransform, PrepareError> {
    let canvas = render.canvas();
    let [position_x, position_y] = layer.position();
    let [scale_x, scale_y] = layer.scale();
    let [anchor_x, anchor_y] = layer.anchor();
    if ![
        position_x,
        position_y,
        scale_x,
        scale_y,
        anchor_x,
        anchor_y,
        layer.rotation(),
        layer.opacity(),
    ]
    .iter()
    .all(|value| value.is_finite())
        || scale_x == 0.0
        || scale_y == 0.0
    {
        return Err(PrepareError::at(
            path,
            "compiled layer transform is invalid",
        ));
    }
    let [source_width, source_height] = layer
        .size()
        .unwrap_or([f64::from(source_width), f64::from(source_height)]);
    let rotation = layer.rotation();
    let (sin, cos) = rotation.sin_cos();
    let local_x = (0.5 - anchor_x) * f64::from(source_width) * scale_x;
    let local_y = (0.5 - anchor_y) * f64::from(source_height) * scale_y;
    let adjusted_x = position_x + (cos * local_x - sin * local_y) / f64::from(canvas.width());
    let adjusted_y = position_y + (sin * local_x + cos * local_y) / f64::from(canvas.height());
    Ok(ResolvedTransform {
        x: adjusted_x,
        y: adjusted_y,
        width: f64::from(source_width) * scale_x.abs() / f64::from(canvas.width()),
        height: f64::from(source_height) * scale_y.abs() / f64::from(canvas.height()),
        anchor: Anchor::Center,
        scale: 1.0,
        rotation: rotation * 180.0 / PI,
        opacity: layer.opacity(),
        fit,
        crop: None,
        inset: LayerInset::default(),
        flip_x: scale_x.is_sign_negative(),
        flip_y: scale_y.is_sign_negative(),
        backdrop: None,
    })
}

fn semantic_asset(
    resource: &EvaluatedResourceRef,
    path: &str,
) -> Result<SemanticAsset, PrepareError> {
    let digest = *resource.digest();
    let (kind, descriptor) = match resource.facts() {
        VerifiedResourceFacts::Video { descriptor, .. } => (
            SemanticAssetKind::Video,
            MediaDescriptor::video(
                descriptor.width,
                descriptor.height,
                descriptor.duration,
                visual_interpretation(&descriptor.color, descriptor.orientation, true),
            ),
        ),
        VerifiedResourceFacts::Image { descriptor, .. } => (
            SemanticAssetKind::Image,
            MediaDescriptor::visual(
                crate::resource::Extent2d::new(descriptor.width, descriptor.height)
                    .map_err(|error| PrepareError::at(path, error))?,
                None,
                visual_interpretation(&descriptor.color, descriptor.orientation, false),
            ),
        ),
        VerifiedResourceFacts::Lottie { descriptor, .. } => (
            SemanticAssetKind::Lottie,
            MediaDescriptor::video(
                descriptor.width,
                descriptor.height,
                descriptor.duration,
                VisualInterpretation::new(
                    ColorDescription::SRGB,
                    SignalLuminance::SDR_100,
                    InputAlphaMode::StraightCoverage,
                ),
            ),
        ),
        _ => {
            return Err(PrepareError::at(
                format!("{path}.source"),
                "verified resource is not an external visual",
            ));
        }
    };
    let descriptor = descriptor.map_err(|error| PrepareError::at(path, error))?;
    Ok(SemanticAsset::new(
        resource.role().to_owned(),
        kind,
        digest,
        descriptor,
    ))
}

fn visual_interpretation(
    color: &MediaColorDescriptorWire,
    orientation: MediaOrientationWire,
    opaque: bool,
) -> VisualInterpretation {
    let color = ColorDescription {
        primaries: match color.primaries {
            ColorPrimariesWire::Srgb | ColorPrimariesWire::Bt709 => ColorPrimaries::Rec709,
            ColorPrimariesWire::DisplayP3 => ColorPrimaries::DisplayP3,
            ColorPrimariesWire::Bt2020 => ColorPrimaries::Rec2020,
        },
        transfer: match color.transfer {
            ColorTransferWire::Srgb => TransferFunction::Srgb,
            ColorTransferWire::Bt709 => TransferFunction::Rec709,
            ColorTransferWire::Pq => TransferFunction::Pq,
            ColorTransferWire::Hlg => TransferFunction::Hlg,
        },
        matrix: match color.matrix {
            ColorMatrixWire::Identity => MatrixCoefficients::Identity,
            ColorMatrixWire::Bt709 => MatrixCoefficients::Bt709,
            ColorMatrixWire::Bt2020Ncl => MatrixCoefficients::Bt2020Ncl,
        },
        range: if color.full_range {
            ColorRange::Full
        } else {
            ColorRange::Limited
        },
    };
    VisualInterpretation::new(
        color,
        SignalLuminance::SDR_100,
        if opaque {
            InputAlphaMode::Opaque
        } else {
            InputAlphaMode::StraightCoverage
        },
    )
    .with_orientation(match orientation {
        MediaOrientationWire::Identity => PixelOrientation::Identity,
        MediaOrientationWire::Rotate90 => PixelOrientation::Rotate90,
        MediaOrientationWire::Rotate180 => PixelOrientation::Rotate180,
        MediaOrientationWire::Rotate270 => PixelOrientation::Rotate270,
        MediaOrientationWire::FlipHorizontal => PixelOrientation::MirrorHorizontal,
        MediaOrientationWire::FlipVertical => PixelOrientation::MirrorVertical,
    })
}

fn adapt_blend(blend: CompiledBlendMode) -> CompositeBlendMode {
    match blend {
        CompiledBlendMode::Normal => CompositeBlendMode::Normal,
        CompiledBlendMode::Screen => CompositeBlendMode::Screen,
        CompiledBlendMode::Lighten => CompositeBlendMode::Lighten,
        CompiledBlendMode::ColorDodge => CompositeBlendMode::ColorDodge,
        CompiledBlendMode::Multiply => CompositeBlendMode::Multiply,
        CompiledBlendMode::Darken => CompositeBlendMode::Darken,
        CompiledBlendMode::ColorBurn => CompositeBlendMode::ColorBurn,
        CompiledBlendMode::LinearBurn => CompositeBlendMode::LinearBurn,
        CompiledBlendMode::Overlay => CompositeBlendMode::Overlay,
        CompiledBlendMode::SoftLight => CompositeBlendMode::SoftLight,
        CompiledBlendMode::HardLight => CompositeBlendMode::HardLight,
        CompiledBlendMode::Difference => CompositeBlendMode::Difference,
        CompiledBlendMode::Exclusion => CompositeBlendMode::Exclusion,
        CompiledBlendMode::Hue => CompositeBlendMode::Hue,
        CompiledBlendMode::Saturation => CompositeBlendMode::Saturation,
        CompiledBlendMode::Color => CompositeBlendMode::Color,
        CompiledBlendMode::Luminosity => CompositeBlendMode::Luminosity,
    }
}
