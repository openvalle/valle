//! Immutable logical render graph and its fail-closed validator.

mod build;
mod ids;
mod pass;
mod resource;
mod structure_hash;
mod validate;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use valle_timeline::internal::RenderId;

use crate::{canonical, frame::RenderSpec, prepare::PreparedProgram, resource::ContentDigest};

pub(crate) use build::build_constructed_render_graph;
pub use build::{GraphBuildError, build_render_graph};
pub use ids::{CompositeVersionId, GraphIdError, PassId, ResourceId};
pub use pass::*;
pub use resource::*;
pub use validate::{GraphValidationError, validate_graph};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RenderGraphWire", rename_all = "camelCase")]
pub struct RenderGraph {
    pub render_id: RenderId,
    pub render_spec: RenderSpec,
    pub programs: Vec<PreparedProgram>,
    pub resources: Vec<GraphResource>,
    pub passes: Vec<GraphPass>,
    pub edges: Vec<ResourceEdge>,
    pub order_edges: Vec<OrderEdge>,
    pub capabilities: Vec<GraphCapability>,
    pub output: ResourceId,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenderGraphWire {
    render_id: RenderId,
    render_spec: RenderSpec,
    programs: Vec<PreparedProgram>,
    resources: Vec<GraphResource>,
    passes: Vec<GraphPass>,
    edges: Vec<ResourceEdge>,
    order_edges: Vec<OrderEdge>,
    capabilities: Vec<GraphCapability>,
    output: ResourceId,
}

impl TryFrom<RenderGraphWire> for RenderGraph {
    type Error = GraphValidationError;

    fn try_from(value: RenderGraphWire) -> Result<Self, Self::Error> {
        let graph = Self {
            render_id: value.render_id,
            render_spec: value.render_spec,
            programs: value.programs,
            resources: value.resources,
            passes: value.passes,
            edges: value.edges,
            order_edges: value.order_edges,
            capabilities: value.capabilities,
            output: value.output,
        };
        graph.validate()?;
        Ok(graph)
    }
}

impl RenderGraph {
    pub fn validate(&self) -> Result<(), GraphValidationError> {
        validate_graph(self)
    }

    pub(crate) fn validate_constructed(&self) -> Result<(), GraphValidationError> {
        validate::validate_constructed_graph(self)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, GraphCanonicalError> {
        self.validate().map_err(|error| GraphCanonicalError {
            reason: error.to_string(),
        })?;
        canonical::bytes(self).map_err(|error| GraphCanonicalError {
            reason: error.to_string(),
        })
    }

    pub fn semantic_hash(&self) -> Result<ContentDigest, GraphCanonicalError> {
        Ok(ContentDigest::of_bytes(&self.canonical_bytes()?))
    }

    /// Stable outer-graph identity used to prove that a cached physical template can be reused
    /// before repeating backend lowering. Exact DrawProgram payloads are admitted separately
    /// against the cached program layouts; excluding them here is what makes animated geometry a
    /// binding concern instead of a false outer-topology cache miss.
    pub(crate) fn structure_hash(&self) -> ContentDigest {
        // Product graphs come only from `build_render_graph`, whose closed constructor already
        // validates this exact value. Keep this process-local cache key independent of serde
        // field names and exclude exact DrawProgram bytes, which are admitted separately against
        // the cached program layouts.
        structure_hash::hash(self)
    }

    pub fn topological_order(&self) -> Result<Vec<PassId>, GraphValidationError> {
        let mut writers = BTreeMap::new();
        for edge in self
            .edges
            .iter()
            .filter(|edge| edge.access == ResourceAccess::Write)
        {
            if let Some(first) = writers.insert(edge.resource, edge.pass) {
                return Err(GraphValidationError::MultipleWriters {
                    resource: edge.resource,
                    first,
                    second: edge.pass,
                });
            }
        }
        let mut outgoing: Vec<BTreeSet<PassId>> = vec![BTreeSet::new(); self.passes.len()];
        let mut incoming = vec![0_usize; self.passes.len()];
        let mut insert_dependency = |before: PassId, after: PassId| {
            if outgoing[before.index()].insert(after) {
                incoming[after.index()] += 1;
            }
        };
        for edge in self
            .edges
            .iter()
            .filter(|edge| edge.access == ResourceAccess::Read)
        {
            if let Some(writer) = writers.get(&edge.resource).copied() {
                insert_dependency(writer, edge.pass);
            }
        }
        for edge in &self.order_edges {
            if edge.before.index() >= self.passes.len() {
                return Err(GraphValidationError::UndefinedPass { id: edge.before });
            }
            if edge.after.index() >= self.passes.len() {
                return Err(GraphValidationError::UndefinedPass { id: edge.after });
            }
            insert_dependency(edge.before, edge.after);
        }
        let mut ready: BTreeSet<_> = self
            .passes
            .iter()
            .filter(|pass| incoming[pass.id.index()] == 0)
            .map(|pass| pass.id)
            .collect();
        let mut result = Vec::with_capacity(self.passes.len());
        while let Some(next) = ready.pop_first() {
            result.push(next);
            for after in outgoing[next.index()].iter().copied() {
                incoming[after.index()] -= 1;
                if incoming[after.index()] == 0 {
                    ready.insert(after);
                }
            }
        }
        if result.len() != self.passes.len() {
            return Err(GraphValidationError::Cycle);
        }
        Ok(result)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("RenderGraph canonicalization failed: {reason}")]
pub struct GraphCanonicalError {
    reason: String,
}

impl From<GraphCanonicalError> for GraphBuildError {
    fn from(error: GraphCanonicalError) -> Self {
        GraphBuildError::at("graph.canonical", error)
    }
}
