use serde::{Deserialize, Serialize};
use valle_draw::program::BackdropScope;

use crate::{
    prepare::{DeviceRect, DynamicBindingId},
    resource::{ExternalHandleId, ExternalResourceDesc, LogicalTextureDesc, OutputSpec},
};

use super::{CompositeVersionId, ResourceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayerRole {
    ImportedSource,
    ProgramSource,
    Filtered,
    Masked,
    Grouped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum GraphRoi {
    FullFrame,
    Static { rect: DeviceRect },
    Dynamic { binding: DynamicBindingId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum GraphOrigin {
    Static { x: i32, y: i32 },
    Dynamic { bounds: DynamicBindingId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum GraphResourceKind {
    ExternalResource {
        handle: ExternalHandleId,
        key: crate::resource::ResourceKey,
        expected: ExternalResourceDesc,
    },
    Layer {
        role: LayerRole,
    },
    Composite {
        version: CompositeVersionId,
    },
    BackdropView {
        source_version: CompositeVersionId,
        scope: BackdropScope,
    },
    Mask,
    Auxiliary,
    HistorySlot {
        slot: u32,
    },
    Output {
        spec: OutputSpec,
    },
}

impl GraphResourceKind {
    pub const fn is_external(&self) -> bool {
        matches!(
            self,
            Self::ExternalResource { .. } | Self::HistorySlot { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphResource {
    pub id: ResourceId,
    pub semantic_path: String,
    pub kind: GraphResourceKind,
    pub texture: Option<LogicalTextureDesc>,
    pub roi: GraphRoi,
    pub origin: GraphOrigin,
}
