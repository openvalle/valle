//! Structured, immutable drawing IR shared by native and Web executors.
//!
//! `DrawProgramBuilder` is the only public construction path. Finishing a builder validates the
//! forest, canonicalizes arena IDs, rejects unreachable side-table entries, and derives
//! [`DrawRequirements`]. The resulting value has no backend state and is safe to hash/cache.

mod arena;
mod compile;
pub mod glass;
mod group;
mod node;
mod packed;
mod paint;
mod path;
#[doc(hidden)]
pub mod recording;
mod transform;
mod validate;

pub use arena::{NodeId, PaintId, PathId};
pub use compile::{
    ProgramRecordingError, ProgramResourceCatalog, ProgramTextureExtent, compile_recording,
    compile_recording_with_catalog,
};
pub use glass::{
    MOTION_GLASS_KERNEL_DIGEST_HEX, MOTION_GLASS_KERNEL_ID, MOTION_GLASS_KERNEL_MANIFEST,
    MOTION_GLASS_SCHEMA_DIGEST_HEX, MOTION_GLASS_SCHEMA_ID, MOTION_GLASS_SCHEMA_MANIFEST,
    MotionGlassError, MotionGlassForegroundProgram, MotionGlassProgram, PackedGlassForegroundTone,
    PackedGlassLight, PackedGlassShapeKind, schema_digest,
};
pub use group::{
    BackdropRead, BackdropScope, BlendMode, Clip, FILTER_GAUSSIAN_SUPPORT_SIGMAS, FillRule, Filter,
    Group, Mask, MaskMode, RoundRect, ShaderLayer, ShaderTextureBinding, ShaderUniformBinding,
    ShaderUniformValue,
};
pub use node::{
    BatchGeometry, BatchInstance, GeometryBatchNode, Glyph, GlyphRun, ImageNode, Node, PathNode,
    PathStroke, RuntimeShaderNode, Scene3dNode, ShadowNode, StrokeCap, StrokeJoin,
};
pub use packed::{DRAW_PROGRAM_FORMAT_VERSION, PackedDrawError};
pub use paint::{GradientStop, LinearColor, Paint, SpreadMode};
pub use path::{PathData, PathVerb};
pub use transform::{Affine2d, Transform2d};
pub use validate::{
    DrawProgramError, MAX_EDGES, MAX_FILTER_SIGMA, MAX_FILTERS, MAX_GLYPHS, MAX_GROUP_DEPTH,
    MAX_LOCAL_INTERMEDIATE_PIXELS, MAX_NODES, MAX_PACKED_BYTES, MAX_PAINTS, MAX_PATH_POINTS,
    MAX_PATH_VERBS, MAX_PATHS, MAX_ROOTS,
};

use crate::{
    Rect,
    requirements::{DrawRequirements, LocalBounds},
};

/// Validator-derived geometry for one canonical node.
///
/// This table is runtime state, not an independently serialized authority: builder finish and
/// packed decode both derive it from the canonical forest. A lowerer can therefore assign exact
/// local subpass ROIs without recursively reinterpreting group semantics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeGeometry {
    pub content_bounds: LocalBounds,
    pub output_bounds: LocalBounds,
    /// Largest single local intermediate needed anywhere below this node.
    pub max_intermediate_pixels: u64,
}

impl NodeGeometry {
    pub const EMPTY: Self = Self {
        content_bounds: LocalBounds::Empty,
        output_bounds: LocalBounds::Empty,
        max_intermediate_pixels: 0,
    };
}

/// Validated immutable arena. Fields stay private so requirements cannot be forged.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawProgram {
    pub(crate) viewport: Rect,
    pub(crate) roots: Vec<NodeId>,
    pub(crate) nodes: Vec<Node>,
    pub(crate) paths: Vec<PathData>,
    pub(crate) paints: Vec<Paint>,
    pub(crate) node_geometry: Vec<NodeGeometry>,
    pub(crate) requirements: DrawRequirements,
}

impl DrawProgram {
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    /// Finite, positive local coordinate viewport. Engine maps it to the owning layer box.
    pub const fn viewport(&self) -> Rect {
        self.viewport
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn paths(&self) -> &[PathData] {
        &self.paths
    }

    pub fn paints(&self) -> &[Paint] {
        &self.paints
    }

    /// Validator-derived geometry parallel to [`Self::nodes`]. It is not a packed wire section.
    pub fn geometries(&self) -> &[NodeGeometry] {
        &self.node_geometry
    }

    pub fn geometry(&self, node: NodeId) -> Option<NodeGeometry> {
        self.node_geometry.get(node.index()).copied()
    }

    pub fn requirements(&self) -> &DrawRequirements {
        &self.requirements
    }

    /// Revalidates IDs, budgets, values, canonical order, and derived requirements.
    pub fn validate(&self) -> Result<(), DrawProgramError> {
        validate::validate_canonical(self)
    }

    pub fn packed_bytes(&self) -> Result<Vec<u8>, PackedDrawError> {
        packed::encode(self)
    }

    pub fn from_packed(bytes: &[u8]) -> Result<Self, PackedDrawError> {
        packed::decode(bytes)
    }

    /// SHA-256 of the complete canonical packed representation.
    pub fn content_hash(&self) -> Result<[u8; 32], PackedDrawError> {
        packed::content_hash(self)
    }
}

#[derive(Debug)]
pub struct DrawProgramBuilder {
    viewport: Rect,
    roots: Vec<NodeId>,
    nodes: Vec<Option<Node>>,
    paths: Vec<PathData>,
    paints: Vec<Paint>,
}

impl DrawProgramBuilder {
    pub fn new(viewport: Rect) -> Self {
        Self {
            viewport,
            roots: Vec::new(),
            nodes: Vec::new(),
            paths: Vec::new(),
            paints: Vec::new(),
        }
    }

    pub fn push_path(&mut self, path: PathData) -> PathId {
        let id = PathId::from_index(self.paths.len());
        self.paths.push(path);
        id
    }

    pub fn push_paint(&mut self, paint: Paint) -> PaintId {
        let id = PaintId::from_index(self.paints.len());
        self.paints.push(paint);
        id
    }

    pub fn push_node(&mut self, node: Node) -> NodeId {
        let id = self.reserve_node();
        // The freshly reserved slot is necessarily in range and empty.
        self.nodes[id.index()] = Some(node);
        id
    }

    /// Reserves an ID for forward references. Every reservation must be defined before finish.
    pub fn reserve_node(&mut self) -> NodeId {
        let id = NodeId::from_index(self.nodes.len());
        self.nodes.push(None);
        id
    }

    pub fn define_node(&mut self, id: NodeId, node: Node) -> Result<(), DrawProgramError> {
        let Some(slot) = self.nodes.get_mut(id.index()) else {
            return Err(DrawProgramError::InvalidNodeId {
                owner: None,
                referenced: id,
            });
        };
        if slot.is_some() {
            return Err(DrawProgramError::NodeAlreadyDefined { id });
        }
        *slot = Some(node);
        Ok(())
    }

    pub fn add_root(&mut self, root: NodeId) {
        self.roots.push(root);
    }

    pub fn finish(self) -> Result<DrawProgram, DrawProgramError> {
        validate::canonicalize(
            self.viewport,
            self.roots,
            self.nodes,
            self.paths,
            self.paints,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revalidation_rejects_forged_derived_geometry() {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 1.0, 1.0));
        let root = builder.push_node(Node::Group(Group::plain(Vec::new())));
        builder.add_root(root);
        let mut program = builder.finish().expect("empty group");
        program.node_geometry[0].max_intermediate_pixels = 1;
        assert_eq!(program.validate(), Err(DrawProgramError::GeometryMismatch));
    }

    fn drop_shadow_program(sigma: f32) -> Result<DrawProgram, DrawProgramError> {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 1.0, 1.0));
        let mut group = Group::plain(Vec::new());
        group.filters.push(Filter::DropShadow {
            offset: [0.0, 0.0],
            sigma_x: sigma,
            sigma_y: sigma,
            color: LinearColor {
                red: 0.0,
                green: 0.0,
                blue: 0.0,
                alpha: 1.0,
            },
        });
        let root = builder.push_node(Node::Group(group));
        builder.add_root(root);
        builder.finish()
    }

    #[test]
    fn drop_shadow_sigma_has_a_closed_public_bound() {
        assert!(drop_shadow_program(MAX_FILTER_SIGMA).is_ok());
        assert!(matches!(
            drop_shadow_program(MAX_FILTER_SIGMA + 1.0),
            Err(DrawProgramError::InvalidValue { reason, .. })
                if reason.contains("drop-shadow sigma exceeds")
        ));
    }
}
