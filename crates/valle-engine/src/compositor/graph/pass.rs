use serde::{Deserialize, Serialize};
use valle_draw::{
    program::{BackdropScope, BlendMode},
    requirements::HostFrame,
};

use crate::{
    prepare::{
        BoundsReason, DynamicBindingId, ExternalPlacement, PreparedEffect, PreparedMask,
        PreparedTransitionKernel, ProgramId,
    },
    resource::OutputSpec,
};

use super::{CompositeVersionId, PassId, ResourceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GraphCapability {
    Clear,
    ExternalImport,
    SourcePipeline,
    DrawProgram,
    BackdropRead,
    Group,
    Filter,
    Mask,
    Blend,
    Transition,
    AdjustmentEffect,
    Caption,
    OutputTransform,
    HostFrame,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphSourcePipeline {
    pub chroma_key: Option<PreparedEffect>,
}

impl GraphSourcePipeline {
    pub fn is_empty(&self) -> bool {
        self.chroma_key.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PassStage {
    Visual,
    Caption,
    Output,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackdropToken {
    pub scope: BackdropScope,
    pub composite: ResourceId,
    pub version: CompositeVersionId,
}

/// A host frame a Draw program reads, named as the program names it: a `Frame` pass's picture,
/// or for an adjustment's template, the composite under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameInput {
    pub frame: HostFrame,
    pub input: ResourceId,
}

/// The clip a host frame pictures, at the clip's opacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameLayer {
    pub layer: ResourceId,
    pub opacity: DynamicBindingId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum LogicalPassKind {
    ClearComposite {
        output: ResourceId,
        working_linear_rec2020_premul: [f32; 4],
    },
    Import {
        external: ResourceId,
        source_pipeline: GraphSourcePipeline,
        output: ResourceId,
        placement: ExternalPlacement,
        transform: DynamicBindingId,
    },
    Draw {
        program: ProgramId,
        external_inputs: Vec<ResourceId>,
        destination_inputs: Vec<ResourceId>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        frame_inputs: Vec<FrameInput>,
        output: ResourceId,
        transform: DynamicBindingId,
        bounds_reason: BoundsReason,
    },
    /// A full-frame picture of one clip over transparent pixels, for the program that reads it.
    /// Without a clip, as outside a Motion transition, the frame stays transparent.
    Frame {
        layer: Option<FrameLayer>,
        output: ResourceId,
    },
    Group {
        input: ResourceId,
        output: ResourceId,
    },
    BackdropRead {
        token: BackdropToken,
        output: ResourceId,
        sample_bounds: DynamicBindingId,
        output_bounds: DynamicBindingId,
    },
    Filter {
        input: ResourceId,
        output: ResourceId,
        effect: PreparedEffect,
    },
    Mask {
        input: ResourceId,
        output: ResourceId,
        mask: PreparedMask,
    },
    CompositeLayer {
        backdrop: ResourceId,
        layer: ResourceId,
        output: ResourceId,
        opacity: DynamicBindingId,
    },
    Blend {
        backdrop: ResourceId,
        layer: ResourceId,
        output: ResourceId,
        mode: BlendMode,
        opacity: DynamicBindingId,
    },
    Transition {
        backdrop: ResourceId,
        from: ResourceId,
        to: ResourceId,
        output: ResourceId,
        kernel: PreparedTransitionKernel,
        progress: DynamicBindingId,
        from_opacity: DynamicBindingId,
        to_opacity: DynamicBindingId,
    },
    AdjustmentEffect {
        input: ResourceId,
        output: ResourceId,
        effect: PreparedEffect,
    },
    /// An adjustment's Motion template replaces the composite it read as its source: the
    /// template's layer, at its opacity, over transparent pixels.
    ReplaceComposite {
        backdrop: ResourceId,
        layer: ResourceId,
        output: ResourceId,
        opacity: DynamicBindingId,
    },
    Caption {
        backdrop: ResourceId,
        program: ProgramId,
        external_inputs: Vec<ResourceId>,
        output: ResourceId,
        transform: DynamicBindingId,
        bounds: DynamicBindingId,
        bounds_reason: BoundsReason,
        opacity: DynamicBindingId,
    },
    OutputTransform {
        input: ResourceId,
        output: ResourceId,
        spec: OutputSpec,
    },
}

impl LogicalPassKind {
    pub fn capabilities(&self) -> Vec<GraphCapability> {
        match self {
            Self::ClearComposite { .. } => vec![GraphCapability::Clear],
            Self::Import {
                source_pipeline, ..
            } => {
                let mut capabilities = vec![GraphCapability::ExternalImport];
                if !source_pipeline.is_empty() {
                    capabilities.push(GraphCapability::SourcePipeline);
                }
                capabilities
            }
            Self::Draw { .. } => vec![GraphCapability::DrawProgram],
            Self::Frame { .. } | Self::ReplaceComposite { .. } => vec![GraphCapability::HostFrame],
            Self::Group { .. } => vec![GraphCapability::Group],
            Self::BackdropRead { .. } => vec![GraphCapability::BackdropRead],
            Self::Filter { .. } => vec![GraphCapability::Filter],
            Self::Mask { .. } => vec![GraphCapability::Mask],
            Self::CompositeLayer { .. } | Self::Blend { .. } => vec![GraphCapability::Blend],
            Self::Transition { .. } => vec![GraphCapability::Transition],
            Self::AdjustmentEffect { .. } => vec![GraphCapability::AdjustmentEffect],
            Self::Caption { .. } => vec![GraphCapability::Caption],
            Self::OutputTransform { .. } => vec![GraphCapability::OutputTransform],
        }
    }

    pub fn reads(&self) -> Vec<ResourceId> {
        let mut result = match self {
            Self::ClearComposite { .. } => Vec::new(),
            Self::Import { external, .. } => vec![*external],
            Self::Draw {
                external_inputs,
                destination_inputs,
                frame_inputs,
                ..
            } => external_inputs
                .iter()
                .chain(destination_inputs)
                .copied()
                .chain(frame_inputs.iter().map(|frame| frame.input))
                .collect(),
            Self::Frame { layer, .. } => layer.iter().map(|layer| layer.layer).collect(),
            Self::Group { input, .. }
            | Self::Mask { input, .. }
            | Self::OutputTransform { input, .. } => vec![*input],
            Self::BackdropRead { token, .. } => vec![token.composite],
            Self::Filter { input, .. } | Self::AdjustmentEffect { input, .. } => vec![*input],
            Self::CompositeLayer {
                backdrop, layer, ..
            }
            | Self::Blend {
                backdrop, layer, ..
            }
            | Self::ReplaceComposite {
                backdrop, layer, ..
            } => vec![*backdrop, *layer],
            Self::Transition {
                backdrop, from, to, ..
            } => vec![*backdrop, *from, *to],
            Self::Caption {
                backdrop,
                external_inputs,
                ..
            } => std::iter::once(*backdrop)
                .chain(external_inputs.iter().copied())
                .collect(),
        };
        result.sort_unstable();
        result.dedup();
        result
    }

    pub const fn output(&self) -> ResourceId {
        match self {
            Self::ClearComposite { output, .. }
            | Self::Import { output, .. }
            | Self::Draw { output, .. }
            | Self::Frame { output, .. }
            | Self::Group { output, .. }
            | Self::BackdropRead { output, .. }
            | Self::Filter { output, .. }
            | Self::Mask { output, .. }
            | Self::CompositeLayer { output, .. }
            | Self::Blend { output, .. }
            | Self::Transition { output, .. }
            | Self::AdjustmentEffect { output, .. }
            | Self::ReplaceComposite { output, .. }
            | Self::Caption { output, .. }
            | Self::OutputTransform { output, .. } => *output,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphPass {
    pub id: PassId,
    pub semantic_path: String,
    pub stage: PassStage,
    pub kind: LogicalPassKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceEdge {
    pub pass: PassId,
    pub resource: ResourceId,
    pub access: ResourceAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrderReason {
    VisualSpine,
    CaptionTerminal,
    OutputTerminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrderEdge {
    pub before: PassId,
    pub after: PassId,
    pub reason: OrderReason,
}
