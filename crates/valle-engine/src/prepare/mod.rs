//! Pure per-frame semantic preparation.
//!
//! C1 intentionally accepts fixture `DrawProgram`s. Real Motion layout/emit and caption shaping
//! replace that producer seam in C3/C2 respectively; every resource slot, bounds value and dynamic
//! binding emitted here already uses the final platform-free contract.

pub mod bounds;
mod compiled_render;
mod effect;
mod external;
mod image;
mod mask;
mod motion;
mod request;
mod transition;
mod validate;
mod video;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use valle_timeline::RationalTime;
use valle_timeline::internal::{FrameKey, RenderId, SampleTime};

use crate::{
    compositor::reference::{ColorMathError, decode_author_srgb},
    frame::{BackgroundBand, CompositeBlendMode, EvaluatedVisualClip, RenderSpec, VisualSource},
    resource::{
        Extent2d, ExternalHandleId, ResourceRequestSet, ResourceSample, SemanticAsset,
        SemanticAssetKind,
    },
};

pub use bounds::{BoundsError, BoundsReason, DeviceRect, DeviceTransform, PreparedBounds};
pub use compiled_render::prepare_compiled_render_frame_cached;
pub use effect::{
    EffectPrepareError, PreparedBlurAxis, PreparedEffect, PreparedEffectKernel,
    PreparedEffectSpace, PreparedEffectValidationError, PreparedUnitRect,
};
pub use external::{
    ExternalPlacement, ExternalPlacementError, ExternalSample, ExternalSampling,
    PreparedExternalBackdrop,
};
pub(crate) use mask::PreparedMaskGeometry;
pub use mask::{
    MASK_FEATHER_SIGMA_PER_RADIUS, MASK_GAUSSIAN_SUPPORT_SIGMAS, PreparedMask, PreparedMaskShape,
};
pub use motion::{
    FixtureProgram, PreparedDestinationUse, PreparedProgram, PreparedProgramKind,
    ProgramFontBinding, ProgramId, ProgramPrepareError, ProgramResourceBindings,
    ProgramStructureBinding, ProgramTextureBinding,
};
pub use request::{
    DynamicBinding, DynamicBindingId, DynamicBindingKind, DynamicBindings, DynamicValue,
    PreparedResource, RequestError, ResourceManifest,
};
pub use transition::PreparedTransitionKernel;
pub use validate::PreparedFrameValidationError;
pub(crate) use validate::validate_prepared_program_table;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedBackground {
    pub working_linear_rec2020_premul: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreparedSourceKind {
    Video,
    Image,
    Lottie,
    Motion,
    Solid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum PreparedSource {
    External {
        source_kind: PreparedSourceKind,
        handle: ExternalHandleId,
        placement: Box<ExternalPlacement>,
    },
    Program {
        source_kind: PreparedSourceKind,
        program: ProgramId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayerDynamicSlots {
    pub transform: DynamicBindingId,
    pub bounds: DynamicBindingId,
    pub opacity: DynamicBindingId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedLayer {
    pub semantic_path: String,
    pub clip_id: String,
    pub track_id: String,
    pub source: PreparedSource,
    pub device_transform: DeviceTransform,
    pub bounds: PreparedBounds,
    pub dynamic: LayerDynamicSlots,
    pub effects: Vec<PreparedEffect>,
    pub mask: Option<PreparedMask>,
    pub blend: CompositeBlendMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedTransition {
    pub semantic_path: String,
    pub kernel: PreparedTransitionKernel,
    pub progress: DynamicBindingId,
    pub from: PreparedLayer,
    pub to: PreparedLayer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum PreparedVisualItem {
    Layer(Box<PreparedLayer>),
    Transition(Box<PreparedTransition>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedCaption {
    pub semantic_path: String,
    pub clip_id: String,
    pub track_id: String,
    pub program: ProgramId,
    pub device_transform: DeviceTransform,
    pub bounds: PreparedBounds,
    pub dynamic: LayerDynamicSlots,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameGeometry {
    pub semantic_path: String,
    pub transform: DeviceTransform,
    pub bounds: PreparedBounds,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameMetadata {
    pub geometry: Vec<FrameGeometry>,
}

/// Authoring-only projection produced by the exact same prepare work as the executable frame.
/// It never participates in graph/lower hashes and therefore cannot become a second render IR.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameInspection {
    pub motion: Vec<MotionFrameInspection>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionFrameInspection {
    pub clip_id: String,
    pub composition_frame: i64,
    pub source_index: u32,
    pub source_frame: u32,
    pub boxes: std::collections::BTreeMap<String, [f32; 4]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PreparedFrameWire", rename_all = "camelCase")]
pub struct PreparedFrame {
    pub render_id: RenderId,
    pub key: FrameKey,
    pub sample_time: SampleTime,
    pub render_spec: RenderSpec,
    pub background: PreparedBackground,
    pub visual: Vec<PreparedVisualItem>,
    pub adjustments: Vec<PreparedEffect>,
    pub captions: Vec<PreparedCaption>,
    pub programs: Vec<PreparedProgram>,
    pub resources: ResourceManifest,
    pub metadata: FrameMetadata,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedFrameWire {
    render_id: RenderId,
    key: FrameKey,
    sample_time: SampleTime,
    render_spec: RenderSpec,
    background: PreparedBackground,
    visual: Vec<PreparedVisualItem>,
    adjustments: Vec<PreparedEffect>,
    captions: Vec<PreparedCaption>,
    programs: Vec<PreparedProgram>,
    resources: ResourceManifest,
    metadata: FrameMetadata,
}

impl TryFrom<PreparedFrameWire> for PreparedFrame {
    type Error = PreparedFrameValidationError;

    fn try_from(value: PreparedFrameWire) -> Result<Self, Self::Error> {
        let frame = Self {
            render_id: value.render_id,
            key: value.key,
            sample_time: value.sample_time,
            render_spec: value.render_spec,
            background: value.background,
            visual: value.visual,
            adjustments: value.adjustments,
            captions: value.captions,
            programs: value.programs,
            resources: value.resources,
            metadata: value.metadata,
        };
        frame.validate()?;
        Ok(frame)
    }
}

impl PreparedFrame {
    pub fn validate(&self) -> Result<(), PreparedFrameValidationError> {
        validate::validate_prepared_frame(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticSeverity {
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticCode {
    ConservativeBounds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: DiagnosticCode,
    pub semantic_path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PrepareOutputWire", rename_all = "camelCase")]
pub struct PrepareOutput {
    pub frame: PreparedFrame,
    pub resource_requests: ResourceRequestSet,
    pub dynamic: DynamicBindings,
    pub diagnostics: Vec<Diagnostic>,
    pub inspection: FrameInspection,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PrepareOutputWire {
    frame: PreparedFrame,
    resource_requests: ResourceRequestSet,
    dynamic: DynamicBindings,
    diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    inspection: FrameInspection,
}

impl TryFrom<PrepareOutputWire> for PrepareOutput {
    type Error = PreparedFrameValidationError;

    fn try_from(value: PrepareOutputWire) -> Result<Self, Self::Error> {
        let output = Self {
            frame: value.frame,
            resource_requests: value.resource_requests,
            dynamic: value.dynamic,
            diagnostics: value.diagnostics,
            inspection: value.inspection,
        };
        // Nested values already completed their own Deserialize admission; only their projection
        // against each other remains here.
        validate::validate_prepare_output_bindings(&output)?;
        Ok(output)
    }
}

impl PrepareOutput {
    /// Rewrite VisualFrame expected layouts the backend cannot import.
    ///
    /// Must run before `resource_requests()` is observed: bind pairs request.expected with the
    /// lowered slot, and Web fulfillment starts as soon as the request packet is returned.
    pub fn adapt_external_pixel_layouts(
        &mut self,
        supported: &[crate::resource::ExternalPixelLayout],
    ) -> Result<(), crate::resource::ResourceContractError> {
        self.frame.resources.adapt_visual_pixel_layouts(supported)?;
        self.resource_requests = ResourceRequestSet::try_from_requests(
            self.frame
                .resources
                .resources()
                .iter()
                .map(|resource| resource.request.clone()),
        )?;
        Ok(())
    }
}

impl PrepareOutput {
    pub fn validate(&self) -> Result<(), PreparedFrameValidationError> {
        validate::validate_prepare_output(self)
    }
}

/// Pure-result memoization owned by one Native worker or one Web engine instance. Dropping or
/// recreating this value can only reduce cache hits; it cannot alter a prepared frame.
pub struct ProductPrepareCaches {
    motion_styles: valle_motion::StyleCache,
    motion_faces: valle_motion::FaceCache,
    #[cfg(feature = "text")]
    text_semantics: crate::text_semantic::TextSemantics,
}

impl ProductPrepareCaches {
    pub fn new() -> Self {
        Self {
            motion_styles: valle_motion::StyleCache::new(),
            motion_faces: valle_motion::FaceCache::new(),
            #[cfg(feature = "text")]
            text_semantics: crate::text_semantic::TextSemantics::product(),
        }
    }
}

impl Default for ProductPrepareCaches {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn prepare_background(
    background: BackgroundBand,
) -> Result<PreparedBackground, PrepareError> {
    let pixel = decode_author_srgb(background.author_srgb_straight)
        .map_err(|error| PrepareError::at("background", error))?;
    Ok(PreparedBackground {
        working_linear_rec2020_premul: pixel.channels(),
    })
}

struct PrepareState<'a> {
    frame: PrepareFrameContext,
    motion_styles: &'a valle_motion::StyleCache,
    motion_faces: &'a valle_motion::FaceCache,
    #[cfg(feature = "text")]
    text_semantics: Option<&'a mut crate::text_semantic::TextSemantics>,
    requests: request::RequestAllocator,
    dynamic: request::DynamicAllocator,
    programs: Vec<PreparedProgram>,
    geometry: Vec<FrameGeometry>,
    diagnostics: Vec<Diagnostic>,
    motion_inspection: Vec<MotionFrameInspection>,
}

#[derive(Debug, Clone)]
pub(super) struct PrepareFrameContext {
    pub(super) render_spec: RenderSpec,
    pub(super) canvas_mapping: crate::frame::CanvasMapping,
    pub(super) camera: Option<crate::frame::ResolvedCamera>,
}

impl<'a> PrepareState<'a> {
    pub(super) fn new_with_context(
        frame: PrepareFrameContext,
        caches: &'a mut ProductPrepareCaches,
    ) -> Self {
        Self {
            frame,
            motion_styles: &caches.motion_styles,
            motion_faces: &caches.motion_faces,
            #[cfg(feature = "text")]
            text_semantics: Some(&mut caches.text_semantics),
            requests: request::RequestAllocator::default(),
            dynamic: request::DynamicAllocator::default(),
            programs: Vec::new(),
            geometry: Vec::new(),
            diagnostics: Vec::new(),
            motion_inspection: Vec::new(),
        }
    }

    fn layer(
        &mut self,
        clip: &EvaluatedVisualClip,
        path: &str,
        source_time: RationalTime,
    ) -> Result<PreparedLayer, PrepareError> {
        let effect_scale = self.frame.canvas_mapping.scale
            * self.frame.camera.as_ref().map_or(1.0, |camera| camera.zoom);
        let VisualSource::Asset {
            id,
            asset_kind,
            digest,
            descriptor,
        } = &clip.source;
        let asset = SemanticAsset::new(id.clone(), *asset_kind, digest.clone(), descriptor.clone());
        let sample = match asset_kind {
            SemanticAssetKind::Image => ResourceSample::Static,
            SemanticAssetKind::Video | SemanticAssetKind::Lottie => {
                ResourceSample::SourceTime(source_time)
            }
            SemanticAssetKind::Audio => {
                return Err(PrepareError::at(
                    path,
                    "non-visual asset entered the visual band",
                ));
            }
        };
        let handle = self
            .requests
            .asset(&asset, sample, &format!("{path}.source"))
            .map_err(|error| PrepareError::at(path, error))?;
        let placement = external::prepare_external_placement(
            &asset,
            clip.source_crop,
            &clip.transform,
            self.frame.canvas_mapping,
            effect_scale,
            &format!("{path}.sourcePlacement"),
            &mut self.dynamic,
        )
        .map_err(|error| PrepareError::at(format!("{path}.sourcePlacement"), error))?;
        let source = PreparedSource::External {
            source_kind: match asset_kind {
                SemanticAssetKind::Video => PreparedSourceKind::Video,
                SemanticAssetKind::Image => PreparedSourceKind::Image,
                SemanticAssetKind::Lottie => PreparedSourceKind::Lottie,
                SemanticAssetKind::Audio => unreachable!(),
            },
            handle,
            placement: Box::new(placement),
        };

        let geometry = match &source {
            PreparedSource::Program { program, .. } => self
                .programs
                .iter()
                .find(|value| value.id == *program)
                .map(prepared_program_geometry),
            PreparedSource::External { placement, .. } => Some(placement.local_geometry()),
        };
        let prepared_mask = clip
            .mask
            .as_ref()
            .map(|mask| {
                bounds::prepare_mask(mask, self.frame.camera.as_ref(), self.frame.canvas_mapping)
            })
            .transpose()
            .map_err(|error| PrepareError::at(format!("{path}.mask"), error))?;
        let (device_transform, prepared_bounds) =
            bounds::layer_geometry(bounds::LayerGeometryInput {
                transform: &clip.transform,
                animation: &clip.resolved_animation,
                camera: self.frame.camera.as_ref(),
                mask: prepared_mask.as_ref(),
                mapping: self.frame.canvas_mapping,
                output: [
                    self.frame.render_spec.width(),
                    self.frame.render_spec.height(),
                ],
                geometry,
            })
            .map_err(|error| PrepareError::at(path, error))?;
        if prepared_bounds.reason == BoundsReason::ConservativeCameraTarget {
            self.diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                code: DiagnosticCode::ConservativeBounds,
                semantic_path: path.to_owned(),
                message: "camera target resolution requires prepared scene geometry; full-frame bounds are explicit"
                    .to_owned(),
            });
        }
        let dynamic = self.layer_bindings(
            path,
            device_transform,
            prepared_bounds.output,
            clip.transform.opacity * f64::from(clip.resolved_animation.opacity),
        )?;
        if let PreparedSource::Program { program, .. } = source {
            self.resolve_program_destinations(
                program,
                device_transform,
                prepared_bounds.reason,
                path,
            )?;
        }
        let mut effects = Vec::new();
        let animation_blur_path = format!("{path}.animationBlur");
        if let Some(effect) = effect::prepare_animation_blur(
            &animation_blur_path,
            PreparedEffectSpace::Layer {
                transform: dynamic.transform,
                bounds: dynamic.bounds,
            },
            f64::from(clip.resolved_animation.blur_sigma_px) * self.frame.canvas_mapping.scale,
            &mut self.dynamic,
        )
        .map_err(|error| PrepareError::at(&animation_blur_path, error))?
        {
            effects.push(effect);
        }
        self.geometry.push(FrameGeometry {
            semantic_path: path.to_owned(),
            transform: device_transform,
            bounds: prepared_bounds,
        });
        Ok(PreparedLayer {
            semantic_path: path.to_owned(),
            clip_id: clip.clip_id.clone(),
            track_id: clip.track_id.clone(),
            source,
            device_transform,
            bounds: prepared_bounds,
            dynamic,
            effects,
            mask: prepared_mask,
            blend: clip.blend,
        })
    }

    /// Assemble a layer whose program was produced directly from an admitted
    /// compiled render. This shares the exact geometry/binding/backdrop
    /// machinery with the authoring-frame path without requiring a synthetic
    /// `VisualSource::Motion` payload.
    pub(super) fn finish_compiled_program_layer(
        &mut self,
        clip: &EvaluatedVisualClip,
        path: &str,
        source: PreparedSource,
    ) -> Result<PreparedLayer, PrepareError> {
        debug_assert!(matches!(source, PreparedSource::Program { .. }));
        let geometry = match &source {
            PreparedSource::Program { program, .. } => self
                .programs
                .iter()
                .find(|value| value.id == *program)
                .map(prepared_program_geometry),
            PreparedSource::External { .. } => unreachable!(),
        };
        let prepared_mask = clip
            .mask
            .as_ref()
            .map(|mask| {
                bounds::prepare_mask(mask, self.frame.camera.as_ref(), self.frame.canvas_mapping)
            })
            .transpose()
            .map_err(|error| PrepareError::at(format!("{path}.mask"), error))?;
        let (device_transform, prepared_bounds) =
            bounds::layer_geometry(bounds::LayerGeometryInput {
                transform: &clip.transform,
                animation: &clip.resolved_animation,
                camera: self.frame.camera.as_ref(),
                mask: prepared_mask.as_ref(),
                mapping: self.frame.canvas_mapping,
                output: [
                    self.frame.render_spec.width(),
                    self.frame.render_spec.height(),
                ],
                geometry,
            })
            .map_err(|error| PrepareError::at(path, error))?;
        let dynamic = self.layer_bindings(
            path,
            device_transform,
            prepared_bounds.output,
            clip.transform.opacity * f64::from(clip.resolved_animation.opacity),
        )?;
        let PreparedSource::Program { program, .. } = source else {
            unreachable!()
        };
        self.resolve_program_destinations(program, device_transform, prepared_bounds.reason, path)?;
        self.geometry.push(FrameGeometry {
            semantic_path: path.to_owned(),
            transform: device_transform,
            bounds: prepared_bounds,
        });
        Ok(PreparedLayer {
            semantic_path: path.to_owned(),
            clip_id: clip.clip_id.clone(),
            track_id: clip.track_id.clone(),
            source: PreparedSource::Program {
                source_kind: match self
                    .programs
                    .iter()
                    .find(|value| value.id == program)
                    .expect("compiled program was just inserted")
                    .kind
                {
                    PreparedProgramKind::Motion => PreparedSourceKind::Motion,
                    PreparedProgramKind::Solid => PreparedSourceKind::Solid,
                    PreparedProgramKind::Caption => unreachable!(),
                },
                program,
            },
            device_transform,
            bounds: prepared_bounds,
            dynamic,
            effects: Vec::new(),
            mask: prepared_mask,
            blend: clip.blend,
        })
    }

    /// Assemble a terminal screen-space caption whose GlyphRun program was
    /// shaped directly from a compiled Timeline packet.
    #[cfg(feature = "text")]
    pub(super) fn finish_compiled_caption(
        &mut self,
        program: ProgramId,
        clip_id: String,
        track_id: String,
        path: &str,
    ) -> Result<PreparedCaption, PrepareError> {
        let transform = crate::frame::ResolvedTransform {
            x: 0.5,
            y: 0.5,
            width: 1.0,
            height: 1.0,
            anchor: crate::frame::Anchor::Center,
            scale: 1.0,
            rotation: 0.0,
            opacity: 1.0,
            fit: None,
            crop: None,
            inset: crate::frame::LayerInset::default(),
            flip_x: false,
            flip_y: false,
            backdrop: None,
        };
        let animation = crate::frame::ResolvedClipAnimation::SEMANTIC_IDENTITY;
        let geometry = self
            .programs
            .iter()
            .find(|value| value.id == program)
            .map(prepared_program_geometry);
        let (device_transform, prepared_bounds) =
            bounds::layer_geometry(bounds::LayerGeometryInput {
                transform: &transform,
                animation: &animation,
                camera: None,
                mask: None,
                mapping: self.frame.canvas_mapping,
                output: [
                    self.frame.render_spec.width(),
                    self.frame.render_spec.height(),
                ],
                geometry,
            })
            .map_err(|error| PrepareError::at(path, error))?;
        let dynamic = self.layer_bindings(path, device_transform, prepared_bounds.output, 1.0)?;
        self.geometry.push(FrameGeometry {
            semantic_path: path.to_owned(),
            transform: device_transform,
            bounds: prepared_bounds,
        });
        Ok(PreparedCaption {
            semantic_path: path.to_owned(),
            clip_id,
            track_id,
            program,
            device_transform,
            bounds: prepared_bounds,
            dynamic,
        })
    }

    fn layer_bindings(
        &mut self,
        path: &str,
        transform: DeviceTransform,
        bounds: DeviceRect,
        opacity: f64,
    ) -> Result<LayerDynamicSlots, PrepareError> {
        let transform_id = self
            .dynamic
            .push(
                format!("{path}.transform"),
                DynamicBindingKind::DeviceTransform,
                DynamicValue::DeviceTransform(transform),
            )
            .map_err(|error| PrepareError::at(path, error))?;
        let bounds_id = self
            .dynamic
            .push(
                format!("{path}.bounds"),
                DynamicBindingKind::Bounds,
                DynamicValue::Bounds(bounds),
            )
            .map_err(|error| PrepareError::at(path, error))?;
        let opacity_id = self
            .dynamic
            .push(
                format!("{path}.opacity"),
                DynamicBindingKind::Opacity,
                DynamicValue::Scalar(opacity),
            )
            .map_err(|error| PrepareError::at(path, error))?;
        Ok(LayerDynamicSlots {
            transform: transform_id,
            bounds: bounds_id,
            opacity: opacity_id,
        })
    }

    fn resolve_program_destinations(
        &mut self,
        program_id: ProgramId,
        transform: DeviceTransform,
        reason: BoundsReason,
        path: &str,
    ) -> Result<(), PrepareError> {
        let program_index = self
            .programs
            .iter()
            .position(|program| program.id == program_id)
            .ok_or_else(|| PrepareError::at(path, "prepared program id is undefined"))?;
        let uses = self.programs[program_index]
            .requirements
            .destination_uses
            .clone();
        if uses.is_empty() {
            return Ok(());
        }
        let viewport = self.programs[program_index].viewport;
        let root = DeviceRect::full(
            self.frame.render_spec.width(),
            self.frame.render_spec.height(),
        );
        let mut prepared = Vec::with_capacity(uses.len());
        for (index, usage) in uses.into_iter().enumerate() {
            let use_path = format!("{path}.destination[{index}]");
            let (sample, output) = if reason == BoundsReason::ConservativeCameraTarget {
                (root, root)
            } else {
                let sample = bounds::program_bounds_to_device(
                    valle_draw::requirements::LocalBounds::from_rect(usage.sample_bounds),
                    viewport,
                    transform,
                    root,
                )
                .map_err(|error| PrepareError::at(&use_path, error))?;
                let output = bounds::program_bounds_to_device(
                    valle_draw::requirements::LocalBounds::from_rect(usage.output_bounds),
                    viewport,
                    transform,
                    root,
                )
                .map_err(|error| PrepareError::at(&use_path, error))?;
                (sample, output)
            };
            let sample_bounds = self
                .dynamic
                .push(
                    format!("{use_path}.sampleBounds"),
                    DynamicBindingKind::BackdropSampleBounds,
                    DynamicValue::Bounds(sample),
                )
                .map_err(|error| PrepareError::at(&use_path, error))?;
            let output_bounds = self
                .dynamic
                .push(
                    format!("{use_path}.outputBounds"),
                    DynamicBindingKind::BackdropOutputBounds,
                    DynamicValue::Bounds(output),
                )
                .map_err(|error| PrepareError::at(&use_path, error))?;
            prepared.push(PreparedDestinationUse {
                node: usage.node,
                scope: usage.scope,
                sample_bounds,
                output_bounds,
                operation: usage.operation,
                bounds_reason: reason,
            });
        }
        self.programs[program_index].destination_uses = prepared;
        Ok(())
    }

    fn next_program_id(&self, path: &str) -> Result<ProgramId, PrepareError> {
        let value = self
            .programs
            .len()
            .checked_add(1)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| PrepareError::at(path, "prepared program budget exceeded"))?;
        ProgramId::new(value).map_err(|error| PrepareError::at(path, error))
    }

    fn local_program_extent(
        &self,
        transform: &crate::frame::ResolvedTransform,
        path: &str,
    ) -> Result<Extent2d, PrepareError> {
        let mapping = self.frame.canvas_mapping;
        let canvas_width = mapping.viewport_width_px / mapping.scale;
        let canvas_height = mapping.viewport_height_px / mapping.scale;
        let width = (transform.width * canvas_width).round().max(1.0);
        let height = (transform.height * canvas_height).round().max(1.0);
        if !width.is_finite()
            || !height.is_finite()
            || width > f64::from(u32::MAX)
            || height > f64::from(u32::MAX)
        {
            return Err(PrepareError::at(path, "local program viewport is invalid"));
        }
        Extent2d::new(width as u32, height as u32).map_err(|error| PrepareError::at(path, error))
    }
}

fn prepared_program_geometry(program: &PreparedProgram) -> bounds::LocalGeometry {
    let mut domain = valle_draw::requirements::LocalBounds::from_rect(program.viewport);
    domain = union_local_bounds(domain, program.requirements.content_bounds);
    domain = union_local_bounds(domain, program.requirements.output_bounds);
    for destination in &program.requirements.destination_uses {
        domain = union_local_bounds(
            domain,
            valle_draw::requirements::LocalBounds::from_rect(destination.sample_bounds),
        );
    }
    bounds::LocalGeometry {
        viewport: program.viewport,
        content: program.requirements.content_bounds,
        output: program.requirements.output_bounds,
        scale_domain: domain,
        max_intermediate_pixels: program.requirements.max_intermediate_pixels,
    }
}

fn union_local_bounds(
    left: valle_draw::requirements::LocalBounds,
    right: valle_draw::requirements::LocalBounds,
) -> valle_draw::requirements::LocalBounds {
    match (left.rect(), right.rect()) {
        (None, None) => valle_draw::requirements::LocalBounds::Empty,
        (Some(rect), None) | (None, Some(rect)) => {
            valle_draw::requirements::LocalBounds::Finite { rect }
        }
        (Some(left), Some(right)) => {
            valle_draw::requirements::LocalBounds::from_rect(valle_draw::Rect::from_edges(
                left.left().min(right.left()),
                left.top().min(right.top()),
                left.right().max(right.right()),
                left.bottom().max(right.bottom()),
            ))
        }
    }
}

#[derive(Debug, Error)]
#[error("{path}: {reason}")]
pub struct PrepareError {
    pub path: String,
    pub reason: String,
}

impl PrepareError {
    fn at(path: impl Into<String>, reason: impl std::fmt::Display) -> Self {
        Self {
            path: path.into(),
            reason: reason.to_string(),
        }
    }
}

impl From<ColorMathError> for PrepareError {
    fn from(error: ColorMathError) -> Self {
        Self::at("background", error)
    }
}
