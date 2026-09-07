use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::{
    prepare::{
        PreparedEffect, PreparedEffectKernel, PreparedFrame, PreparedLayer, PreparedProgram,
        PreparedSource, PreparedTransition, PreparedVisualItem, ProgramId,
    },
    resource::{
        Extent2d, ExternalHandleId, LogicalTextureDesc, ResourceContractError, TextureUsage,
    },
};

use super::{
    BackdropToken, CompositeVersionId, GraphCapability, GraphPass, GraphResource,
    GraphResourceKind, GraphRoi, GraphSourcePipeline, LayerRole, LogicalPassKind, OrderEdge,
    OrderReason, PassId, PassStage, RenderGraph, ResourceAccess, ResourceEdge, ResourceId,
};

pub fn build_render_graph(frame: &PreparedFrame) -> Result<RenderGraph, GraphBuildError> {
    frame
        .validate()
        .map_err(|error| GraphBuildError::at("frame", error))?;
    Builder::new(frame)?.build()
}

/// Build from a frame produced by this Engine instance.
///
/// The public builder above is an admission boundary and must decode every packed DrawProgram.
/// Product compilation has already constructed and admitted the same frame through closed Rust
/// types, so repeating external-wire admission here only re-parses large frame payloads. Keep the
/// cheaper constructed-state cross checks, then use the exact same graph builder and validator.
pub(crate) fn build_constructed_render_graph(
    frame: &PreparedFrame,
) -> Result<RenderGraph, GraphBuildError> {
    // PreparedTicket is created only after the closed producer path has already run constructed
    // frame validation, and it exposes no mutable frame access. Repeating that same walk here is
    // not an admission boundary; the builder and constructed graph validator still prove the
    // graph projection below.
    Builder::new(frame)?.build()
}

struct Builder<'a> {
    frame: &'a PreparedFrame,
    resources: Vec<GraphResource>,
    passes: Vec<GraphPass>,
    edges: Vec<ResourceEdge>,
    order_edges: Vec<OrderEdge>,
    capabilities: BTreeSet<GraphCapability>,
    external_by_handle: BTreeMap<ExternalHandleId, ResourceId>,
    next_composite_version: usize,
    last_spine_pass: Option<PassId>,
    working_texture: LogicalTextureDesc,
}

impl<'a> Builder<'a> {
    fn new(frame: &'a PreparedFrame) -> Result<Self, GraphBuildError> {
        let extent = Extent2d::new(frame.render_spec.width(), frame.render_spec.height())
            .map_err(|error| GraphBuildError::at("renderSpec", error))?;
        let working_texture = LogicalTextureDesc::production(
            extent,
            [
                TextureUsage::Sampled,
                TextureUsage::StorageRead,
                TextureUsage::StorageWrite,
                TextureUsage::ColorAttachment,
            ],
        )?;
        let mut builder = Self {
            frame,
            resources: Vec::new(),
            passes: Vec::new(),
            edges: Vec::new(),
            order_edges: Vec::new(),
            capabilities: BTreeSet::new(),
            external_by_handle: BTreeMap::new(),
            next_composite_version: 0,
            last_spine_pass: None,
            working_texture,
        };
        for resource in frame.resources.resources() {
            let handle = resource.request.handle();
            if builder.external_by_handle.contains_key(&handle) {
                return Err(GraphBuildError::at(
                    "resources",
                    format!("duplicate external handle {}", handle.get()),
                ));
            }
            let id = builder.push_resource(
                resource
                    .semantic_paths
                    .first()
                    .cloned()
                    .unwrap_or_else(|| format!("resource[{}]", handle.get())),
                GraphResourceKind::ExternalResource {
                    handle,
                    key: resource.request.key().clone(),
                    expected: resource.request.expected().clone(),
                },
                None,
                GraphRoi::FullFrame,
            )?;
            builder.external_by_handle.insert(handle, id);
        }
        Ok(builder)
    }

    fn build(mut self) -> Result<RenderGraph, GraphBuildError> {
        let clear_output = self.composite_resource("background.composite")?;
        let clear = self.add_pass(
            "background.clear",
            PassStage::Visual,
            LogicalPassKind::ClearComposite {
                output: clear_output,
                working_linear_rec2020_premul: self.frame.background.working_linear_rec2020_premul,
            },
        )?;
        self.last_spine_pass = Some(clear);
        let mut current = clear_output;

        for item in &self.frame.visual {
            current = match item {
                PreparedVisualItem::Layer(layer) => self.composite_layer(layer, current)?,
                PreparedVisualItem::Transition(transition) => {
                    self.transition(transition, current)?
                }
            };
        }
        for effect in &self.frame.adjustments {
            current = self.adjustment_effect(effect, current)?;
        }
        for caption in &self.frame.captions {
            current = self.caption(caption, current)?;
        }

        let output = self.push_resource(
            "output",
            GraphResourceKind::Output {
                spec: self.frame.render_spec.output(),
            },
            None,
            GraphRoi::FullFrame,
        )?;
        let output_pass = self.add_pass(
            "output.transform",
            PassStage::Output,
            LogicalPassKind::OutputTransform {
                input: current,
                output,
                spec: self.frame.render_spec.output(),
            },
        )?;
        self.link_spine(output_pass, OrderReason::OutputTerminal);

        let graph = RenderGraph {
            render_id: self.frame.render_id,
            render_spec: self.frame.render_spec,
            programs: self.frame.programs.clone(),
            resources: self.resources,
            passes: self.passes,
            edges: self.edges,
            order_edges: self.order_edges,
            capabilities: self.capabilities.into_iter().collect(),
            output,
        };
        // Closed constructors already produce canonical ordering. Validate the graph without
        // serializing the complete frame-owned DrawProgram payload; wire decoding still runs the
        // same validator independently.
        graph.validate_constructed()?;
        Ok(graph)
    }

    fn composite_layer(
        &mut self,
        layer: &PreparedLayer,
        backdrop: ResourceId,
    ) -> Result<ResourceId, GraphBuildError> {
        let source = self.layer_source(layer, backdrop)?;
        let output = self.composite_resource(format!("{}.composite", layer.semantic_path))?;
        let kind = if layer.blend.is_normal() {
            LogicalPassKind::CompositeLayer {
                backdrop,
                layer: source,
                output,
                opacity: layer.dynamic.opacity,
            }
        } else {
            LogicalPassKind::Blend {
                backdrop,
                layer: source,
                output,
                mode: timeline_blend(layer.blend),
                opacity: layer.dynamic.opacity,
            }
        };
        let pass = self.add_pass(
            format!("{}.composite", layer.semantic_path),
            PassStage::Visual,
            kind,
        )?;
        self.link_spine(pass, OrderReason::VisualSpine);
        Ok(output)
    }

    fn layer_source(
        &mut self,
        layer: &PreparedLayer,
        entry_composite: ResourceId,
    ) -> Result<ResourceId, GraphBuildError> {
        let mut current = match &layer.source {
            PreparedSource::External {
                handle, placement, ..
            } => {
                let external =
                    self.external(*handle, &format!("{}.source", layer.semantic_path))?;
                let chroma_key = layer
                    .effects
                    .iter()
                    .find(|effect| matches!(effect.kernel, PreparedEffectKernel::ChromaKey { .. }))
                    .cloned();
                let output = self.layer_resource(
                    format!("{}.imported", layer.semantic_path),
                    LayerRole::ImportedSource,
                    layer.dynamic.bounds,
                )?;
                self.add_pass(
                    format!("{}.import", layer.semantic_path),
                    PassStage::Visual,
                    LogicalPassKind::Import {
                        external,
                        source_pipeline: GraphSourcePipeline { chroma_key },
                        output,
                        placement: **placement,
                        transform: layer.dynamic.transform,
                    },
                )?;
                output
            }
            PreparedSource::Program { program, .. } => {
                self.program_layer(layer, *program, entry_composite)?
            }
        };

        for (index, effect) in layer.effects.iter().enumerate() {
            if effect.kernel.is_source_operator() {
                continue;
            }
            let output = self.layer_resource(
                format!("{}.effects[{index}]", layer.semantic_path),
                LayerRole::Filtered,
                layer.dynamic.bounds,
            )?;
            self.add_pass(
                format!("{}.effects[{index}]", layer.semantic_path),
                PassStage::Visual,
                LogicalPassKind::Filter {
                    input: current,
                    output,
                    effect: effect.clone(),
                },
            )?;
            current = output;
        }
        if let Some(mask) = layer.mask {
            let output = self.layer_resource(
                format!("{}.mask", layer.semantic_path),
                LayerRole::Masked,
                layer.dynamic.bounds,
            )?;
            self.add_pass(
                format!("{}.mask", layer.semantic_path),
                PassStage::Visual,
                LogicalPassKind::Mask {
                    input: current,
                    output,
                    mask,
                },
            )?;
            current = output;
        }
        Ok(current)
    }

    fn program_layer(
        &mut self,
        layer: &PreparedLayer,
        program_id: ProgramId,
        entry_composite: ResourceId,
    ) -> Result<ResourceId, GraphBuildError> {
        let program = self.program(program_id, &layer.semantic_path)?.clone();
        if program.destination_uses.len() != program.requirements.destination_uses.len() {
            return Err(GraphBuildError::at(
                &layer.semantic_path,
                "prepared destination bindings do not match DrawRequirements",
            ));
        }
        let version = self.composite_version(entry_composite, &layer.semantic_path)?;
        let mut destination_inputs = Vec::with_capacity(program.destination_uses.len());
        for (index, destination) in program.destination_uses.iter().enumerate() {
            let output = self.push_resource(
                format!("{}.destination[{index}]", layer.semantic_path),
                GraphResourceKind::BackdropView {
                    source_version: version,
                    scope: destination.scope.clone(),
                },
                Some(self.working_texture.clone()),
                GraphRoi::Dynamic {
                    binding: destination.sample_bounds,
                },
            )?;
            self.add_pass(
                format!("{}.destination[{index}]", layer.semantic_path),
                PassStage::Visual,
                LogicalPassKind::BackdropRead {
                    token: BackdropToken {
                        scope: destination.scope.clone(),
                        composite: entry_composite,
                        version,
                    },
                    output,
                    sample_bounds: destination.sample_bounds,
                    output_bounds: destination.output_bounds,
                },
            )?;
            destination_inputs.push(output);
        }
        let external_inputs = self.program_inputs(&program)?;
        let output = self.layer_resource(
            format!("{}.program", layer.semantic_path),
            LayerRole::ProgramSource,
            layer.dynamic.bounds,
        )?;
        self.add_pass(
            format!("{}.draw", layer.semantic_path),
            PassStage::Visual,
            LogicalPassKind::Draw {
                program: program_id,
                external_inputs,
                destination_inputs,
                output,
                transform: layer.dynamic.transform,
                bounds_reason: layer.bounds.reason,
            },
        )?;
        Ok(output)
    }

    fn transition(
        &mut self,
        transition: &PreparedTransition,
        backdrop: ResourceId,
    ) -> Result<ResourceId, GraphBuildError> {
        let from = self.layer_source(&transition.from, backdrop)?;
        let to = self.layer_source(&transition.to, backdrop)?;
        let output = self.composite_resource(format!("{}.composite", transition.semantic_path))?;
        let pass = self.add_pass(
            format!("{}.transition", transition.semantic_path),
            PassStage::Visual,
            LogicalPassKind::Transition {
                backdrop,
                from,
                to,
                output,
                kernel: transition.kernel,
                progress: transition.progress,
                from_opacity: transition.from.dynamic.opacity,
                to_opacity: transition.to.dynamic.opacity,
            },
        )?;
        self.link_spine(pass, OrderReason::VisualSpine);
        Ok(output)
    }

    fn adjustment_effect(
        &mut self,
        effect: &PreparedEffect,
        input: ResourceId,
    ) -> Result<ResourceId, GraphBuildError> {
        let output = self.composite_resource(format!("{}.composite", effect.semantic_path))?;
        let pass = self.add_pass(
            &effect.semantic_path,
            PassStage::Visual,
            LogicalPassKind::AdjustmentEffect {
                input,
                output,
                effect: effect.clone(),
            },
        )?;
        self.link_spine(pass, OrderReason::VisualSpine);
        Ok(output)
    }

    fn caption(
        &mut self,
        caption: &crate::prepare::PreparedCaption,
        backdrop: ResourceId,
    ) -> Result<ResourceId, GraphBuildError> {
        let program = self
            .program(caption.program, &caption.semantic_path)?
            .clone();
        if !program.destination_uses.is_empty() || !program.requirements.destination_uses.is_empty()
        {
            return Err(GraphBuildError::at(
                &caption.semantic_path,
                "caption program contains a destination dependency",
            ));
        }
        let external_inputs = self.program_inputs(&program)?;
        let output = self.composite_resource(format!("{}.composite", caption.semantic_path))?;
        let pass = self.add_pass(
            &caption.semantic_path,
            PassStage::Caption,
            LogicalPassKind::Caption {
                backdrop,
                program: caption.program,
                external_inputs,
                output,
                transform: caption.dynamic.transform,
                bounds: caption.dynamic.bounds,
                bounds_reason: caption.bounds.reason,
                opacity: caption.dynamic.opacity,
            },
        )?;
        self.link_spine(pass, OrderReason::CaptionTerminal);
        Ok(output)
    }

    fn program_inputs(
        &self,
        program: &PreparedProgram,
    ) -> Result<Vec<ResourceId>, GraphBuildError> {
        let handles = program
            .resources
            .textures
            .iter()
            .map(|binding| binding.handle)
            .chain(program.resources.fonts.iter().map(|binding| binding.handle))
            .chain(
                program
                    .resources
                    .runtime_shaders
                    .iter()
                    .map(|binding| binding.handle),
            )
            .chain(
                program
                    .resources
                    .scenes
                    .iter()
                    .map(|binding| binding.handle),
            );
        let mut result: Vec<_> = handles
            .map(|handle| self.external(handle, &program.semantic_path))
            .collect::<Result<_, _>>()?;
        result.sort_unstable();
        result.dedup();
        Ok(result)
    }

    fn program(&self, id: ProgramId, path: &str) -> Result<&PreparedProgram, GraphBuildError> {
        self.frame
            .programs
            .get(id.get() as usize - 1)
            .filter(|program| program.id == id)
            .ok_or_else(|| GraphBuildError::at(path, format!("undefined program {}", id.get())))
    }

    fn external(
        &self,
        handle: ExternalHandleId,
        path: &str,
    ) -> Result<ResourceId, GraphBuildError> {
        self.external_by_handle
            .get(&handle)
            .copied()
            .ok_or_else(|| {
                GraphBuildError::at(path, format!("undefined external handle {}", handle.get()))
            })
    }

    fn composite_version(
        &self,
        resource: ResourceId,
        path: &str,
    ) -> Result<CompositeVersionId, GraphBuildError> {
        match &self.resources[resource.index()].kind {
            GraphResourceKind::Composite { version } => Ok(*version),
            _ => Err(GraphBuildError::at(
                path,
                "backdrop token is not a composite",
            )),
        }
    }

    fn composite_resource(
        &mut self,
        path: impl Into<String>,
    ) -> Result<ResourceId, GraphBuildError> {
        let version = CompositeVersionId::from_index(self.next_composite_version)?;
        self.next_composite_version += 1;
        self.push_resource(
            path,
            GraphResourceKind::Composite { version },
            Some(self.working_texture.clone()),
            GraphRoi::FullFrame,
        )
    }

    fn layer_resource(
        &mut self,
        path: impl Into<String>,
        role: LayerRole,
        bounds: crate::prepare::DynamicBindingId,
    ) -> Result<ResourceId, GraphBuildError> {
        self.push_resource(
            path,
            GraphResourceKind::Layer { role },
            Some(self.working_texture.clone()),
            GraphRoi::Dynamic { binding: bounds },
        )
    }

    fn push_resource(
        &mut self,
        semantic_path: impl Into<String>,
        kind: GraphResourceKind,
        texture: Option<LogicalTextureDesc>,
        roi: GraphRoi,
    ) -> Result<ResourceId, GraphBuildError> {
        let id = ResourceId::from_index(self.resources.len())?;
        let origin = match &roi {
            GraphRoi::FullFrame => super::GraphOrigin::Static { x: 0, y: 0 },
            GraphRoi::Static { rect } => super::GraphOrigin::Static {
                x: rect.x,
                y: rect.y,
            },
            GraphRoi::Dynamic { binding } => super::GraphOrigin::Dynamic { bounds: *binding },
        };
        self.resources.push(GraphResource {
            id,
            semantic_path: semantic_path.into(),
            kind,
            texture,
            roi,
            origin,
        });
        Ok(id)
    }

    fn add_pass(
        &mut self,
        semantic_path: impl Into<String>,
        stage: PassStage,
        kind: LogicalPassKind,
    ) -> Result<PassId, GraphBuildError> {
        let id = PassId::from_index(self.passes.len())?;
        for resource in kind.reads() {
            self.edges.push(ResourceEdge {
                pass: id,
                resource,
                access: ResourceAccess::Read,
            });
        }
        self.edges.push(ResourceEdge {
            pass: id,
            resource: kind.output(),
            access: ResourceAccess::Write,
        });
        self.capabilities.extend(kind.capabilities());
        self.passes.push(GraphPass {
            id,
            semantic_path: semantic_path.into(),
            stage,
            kind,
        });
        Ok(id)
    }

    fn link_spine(&mut self, pass: PassId, reason: OrderReason) {
        if let Some(before) = self.last_spine_pass {
            self.order_edges.push(OrderEdge {
                before,
                after: pass,
                reason,
            });
        }
        self.last_spine_pass = Some(pass);
    }
}

const fn timeline_blend(value: crate::frame::CompositeBlendMode) -> valle_draw::program::BlendMode {
    use crate::frame::CompositeBlendMode as Timeline;
    use valle_draw::program::BlendMode as Draw;
    match value {
        Timeline::Normal => Draw::Normal,
        Timeline::Multiply => Draw::Multiply,
        Timeline::Screen => Draw::Screen,
        Timeline::Overlay => Draw::Overlay,
        Timeline::Darken => Draw::Darken,
        Timeline::Lighten => Draw::Lighten,
        Timeline::ColorDodge => Draw::ColorDodge,
        Timeline::ColorBurn => Draw::ColorBurn,
        Timeline::LinearBurn => Draw::LinearBurn,
        Timeline::HardLight => Draw::HardLight,
        Timeline::SoftLight => Draw::SoftLight,
        Timeline::Difference => Draw::Difference,
        Timeline::Exclusion => Draw::Exclusion,
        Timeline::Hue => Draw::Hue,
        Timeline::Saturation => Draw::Saturation,
        Timeline::Color => Draw::Color,
        Timeline::Luminosity => Draw::Luminosity,
    }
}

#[derive(Debug, Error)]
#[error("{path}: {reason}")]
pub struct GraphBuildError {
    pub path: String,
    pub reason: String,
}

impl GraphBuildError {
    pub(crate) fn at(path: impl Into<String>, reason: impl std::fmt::Display) -> Self {
        Self {
            path: path.into(),
            reason: reason.to_string(),
        }
    }
}

impl From<super::GraphIdError> for GraphBuildError {
    fn from(error: super::GraphIdError) -> Self {
        Self::at("graph", error)
    }
}

impl From<ResourceContractError> for GraphBuildError {
    fn from(error: ResourceContractError) -> Self {
        Self::at("graph.resource", error)
    }
}

impl From<super::GraphValidationError> for GraphBuildError {
    fn from(error: super::GraphValidationError) -> Self {
        Self::at("graph.validate", error)
    }
}
