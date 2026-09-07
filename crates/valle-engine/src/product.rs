//! Product-facing staged compositor API.
//!
//! This is the only orchestration surface shared by Native and Web hosts. It deliberately keeps
//! resource fulfillment outside Engine while preserving the required order:
//! evaluate/prepare may start fulfillment, graph/lower can run in parallel, and binding closes
//! the exact generation before an executor is admitted.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

use thiserror::Error;
use valle_timeline::internal::{CanonicalTimeline, FrameKey, RenderId, ResourceManifest};

use crate::{
    compositor::{
        graph::{GraphBuildError, RenderGraph, build_constructed_render_graph},
        lower::{
            BackendCapabilities, BindingContractError, LowerError, RenderBindings,
            RenderPlanTemplate, lower_constructed_programs_for_template,
            lower_constructed_render_graph, lower_programs_for_template, lower_render_graph,
        },
    },
    frame::RenderSpec,
    prepare::{
        PrepareError, PrepareOutput, ProductPrepareCaches, prepare_compiled_render_frame_cached,
    },
    render::{
        Capabilities, CompiledRender, EngineOpenReport, EvaluatedRenderFrame, ExecutionProfile,
        ResourceBindings, RuntimeFault, compile_render_input,
    },
    resource::{ContentDigest, ExternalGeneration, ExternalPixelLayout, ResourceContractError},
};

/// One immutable, fully preflighted Engine render.
#[derive(Debug, Clone)]
pub struct EngineRender {
    compiled: Arc<CompiledRender>,
    plan_cache: Arc<Mutex<PlanTemplateCache>>,
}

impl EngineRender {
    /// Admit, compile, and pin one package-bound render input as the immutable
    /// Product handle. The public cross-process boundary is
    /// [`crate::fixed_package::VerifiedFixedPackage`]; this constructor only
    /// joins its already-decoded members inside Engine.
    pub(crate) fn open(
        timeline: &CanonicalTimeline,
        manifest: &ResourceManifest,
        bindings: &ResourceBindings,
        capabilities: &Capabilities,
        profile: &ExecutionProfile,
    ) -> Result<Self, EngineOpenReport> {
        let compiled = compile_render_input(timeline, manifest, bindings, capabilities, profile)?;
        Ok(Self::from_compiled(Arc::new(compiled)))
    }

    /// Pins one already admitted immutable render for Product execution.
    fn from_compiled(compiled: Arc<CompiledRender>) -> Self {
        Self {
            compiled,
            plan_cache: Arc::new(Mutex::new(PlanTemplateCache::default())),
        }
    }

    pub fn compiled(&self) -> &CompiledRender {
        self.compiled.as_ref()
    }

    /// Clone the same immutable compiled graph pinned by this open render.
    pub fn compiled_arc(&self) -> Arc<CompiledRender> {
        Arc::clone(&self.compiled)
    }

    pub fn render_id(&self) -> RenderId {
        self.compiled.render_id()
    }

    /// Creates one worker-local frame compiler. Native creates one per raster worker; Web keeps
    /// one per engine instance. The render remains immutable and can be shared freely.
    pub fn frame_compiler(&self) -> FrameCompiler {
        FrameCompiler {
            render: self.clone(),
            caches: ProductPrepareCaches::new(),
            plan_cache: Arc::clone(&self.plan_cache),
        }
    }
}

impl std::ops::Deref for EngineRender {
    type Target = CompiledRender;

    fn deref(&self) -> &Self::Target {
        self.compiled()
    }
}

/// Stateful only by deterministic memoization; all semantic inputs still come from the immutable
/// render and explicit frame arguments.
pub struct FrameCompiler {
    render: EngineRender,
    caches: ProductPrepareCaches,
    plan_cache: Arc<Mutex<PlanTemplateCache>>,
}

impl FrameCompiler {
    /// Evaluate and prepare one frame without waiting for any platform resource. The returned
    /// request set is complete and may be fulfilled while [`PreparedTicket::lower`] runs.
    pub fn evaluate_prepare(
        &mut self,
        render_id: RenderId,
        key: FrameKey,
        render_spec: RenderSpec,
    ) -> Result<PreparedTicket, ProductEngineError> {
        if render_id != self.render.render_id() {
            return Err(ProductEngineError::RenderMismatch {
                expected: self.render.render_id(),
                actual: render_id,
            });
        }
        let evaluated = self.render.compiled.evaluate(key)?;
        let prepared = prepare_compiled_render_frame_cached(
            self.render.compiled(),
            &evaluated,
            &render_spec,
            &mut self.caches,
        )?;
        Ok(PreparedTicket {
            evaluated,
            prepared,
            plan_cache: Arc::clone(&self.plan_cache),
        })
    }
}

/// Staged frame after deterministic semantic preparation and before backend lowering.
#[derive(Debug)]
pub struct PreparedTicket {
    evaluated: EvaluatedRenderFrame,
    prepared: PrepareOutput,
    plan_cache: Arc<Mutex<PlanTemplateCache>>,
}

impl PreparedTicket {
    pub const fn evaluated(&self) -> &EvaluatedRenderFrame {
        &self.evaluated
    }

    pub const fn prepared(&self) -> &PrepareOutput {
        &self.prepared
    }

    pub fn adapt_external_pixel_layouts(
        &mut self,
        supported: &[ExternalPixelLayout],
    ) -> Result<(), ResourceContractError> {
        self.prepared.adapt_external_pixel_layouts(supported)
    }

    pub fn lower(
        self,
        capabilities: &BackendCapabilities,
    ) -> Result<LoweredTicket, ProductEngineError> {
        self.lower_with_mode(capabilities, FrameProgramMode::Wire)
    }

    /// Trusted in-process path used by Native. Exact arenas remain attached to the frame packet;
    /// random-access wire patches are a transport concern and are not generated merely to be
    /// reconstructed by the executor in the same process.
    pub fn lower_constructed(
        self,
        capabilities: &BackendCapabilities,
    ) -> Result<LoweredTicket, ProductEngineError> {
        self.lower_with_mode(capabilities, FrameProgramMode::Constructed)
    }

    fn lower_with_mode(
        self,
        capabilities: &BackendCapabilities,
        mode: FrameProgramMode,
    ) -> Result<LoweredTicket, ProductEngineError> {
        let graph = build_constructed_render_graph(&self.prepared.frame)?;
        let graph_structure_hash = graph.structure_hash();
        let capability_fingerprint = capabilities.fingerprint().map_err(LowerError::from)?;
        let recent = self
            .plan_cache
            .lock()
            .map_err(|_| ProductEngineError::PlanCachePoisoned)?
            .recent_for(
                &graph_structure_hash,
                &graph.render_id,
                graph.render_spec,
                &capability_fingerprint,
                mode,
            );
        if let Some(template) = recent {
            let frame_programs = match mode {
                FrameProgramMode::Wire => {
                    lower_programs_for_template(&graph, &self.prepared.dynamic, &template)
                }
                FrameProgramMode::Constructed => lower_constructed_programs_for_template(
                    &graph,
                    &self.prepared.dynamic,
                    &template,
                ),
            };
            if let Ok(frame_programs) = frame_programs {
                return Ok(LoweredTicket {
                    evaluated: self.evaluated,
                    prepared: self.prepared,
                    graph,
                    template,
                    frame_programs,
                    capabilities: capabilities.clone(),
                    template_cache_hit: true,
                });
            }
        }
        // Lower a current-frame candidate first. Its baseline DrawPrograms belong to the physical
        // packet identity, while `structure_hash` excludes those baseline bytes and therefore
        // remains the reusable topology key.
        let candidate = match mode {
            FrameProgramMode::Wire => {
                lower_render_graph(&graph, &self.prepared.dynamic, capabilities)?
            }
            FrameProgramMode::Constructed => {
                lower_constructed_render_graph(&graph, &self.prepared.dynamic, capabilities)?
            }
        };
        let key = PlanCacheKey {
            structure_hash: *candidate.structure_hash(),
            mode,
        };
        let cached = self
            .plan_cache
            .lock()
            .map_err(|_| ProductEngineError::PlanCachePoisoned)?
            .get(&key);
        let (candidate, frame_programs) = candidate.detach_frame_programs();
        let (template, frame_programs, template_cache_hit) = match cached {
            Some(template) => {
                let frame_programs = match mode {
                    FrameProgramMode::Wire => template.rebase_frame_programs(frame_programs)?,
                    FrameProgramMode::Constructed => {
                        template.attach_frame_programs(frame_programs)?
                    }
                };
                (template, frame_programs, true)
            }
            None => {
                let template = Arc::new(candidate);
                self.plan_cache
                    .lock()
                    .map_err(|_| ProductEngineError::PlanCachePoisoned)?
                    .insert(key, Arc::clone(&template), graph_structure_hash);
                let frame_programs = match mode {
                    FrameProgramMode::Wire => frame_programs,
                    FrameProgramMode::Constructed => {
                        template.attach_frame_programs(frame_programs)?
                    }
                };
                (template, frame_programs, false)
            }
        };
        Ok(LoweredTicket {
            evaluated: self.evaluated,
            prepared: self.prepared,
            graph,
            template,
            frame_programs,
            capabilities: capabilities.clone(),
            template_cache_hit,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FrameProgramMode {
    Wire,
    Constructed,
}

/// Complete pure plan plus the still-unbound external resource request set.
#[derive(Debug)]
pub struct LoweredTicket {
    evaluated: EvaluatedRenderFrame,
    prepared: PrepareOutput,
    graph: RenderGraph,
    template: Arc<RenderPlanTemplate>,
    frame_programs: Vec<crate::compositor::lower::PlanProgram>,
    capabilities: BackendCapabilities,
    template_cache_hit: bool,
}

impl LoweredTicket {
    pub const fn evaluated(&self) -> &EvaluatedRenderFrame {
        &self.evaluated
    }

    pub const fn prepared(&self) -> &PrepareOutput {
        &self.prepared
    }

    pub const fn graph(&self) -> &RenderGraph {
        &self.graph
    }

    pub fn template(&self) -> &RenderPlanTemplate {
        self.template.as_ref()
    }

    pub const fn capabilities(&self) -> &BackendCapabilities {
        &self.capabilities
    }

    pub const fn template_cache_hit(&self) -> bool {
        self.template_cache_hit
    }

    /// Assemble bindings for one immutable fulfillment generation. Request handles are the
    /// generation-local object-table IDs by contract; slot order is derived by the lowerer and
    /// checked again by `RenderPlanTemplate::validate_bindings`.
    pub fn bind(self, generation: ExternalGeneration) -> Result<BoundTicket, ProductEngineError> {
        self.bind_with_mode(generation, FrameProgramMode::Wire)
    }

    pub fn bind_constructed(
        self,
        generation: ExternalGeneration,
    ) -> Result<BoundTicket, ProductEngineError> {
        self.bind_with_mode(generation, FrameProgramMode::Constructed)
    }

    fn bind_with_mode(
        self,
        generation: ExternalGeneration,
        mode: FrameProgramMode,
    ) -> Result<BoundTicket, ProductEngineError> {
        let external_ids = self
            .template
            .binding_layout()
            .external_slots()
            .iter()
            .map(|slot| {
                self.prepared
                    .resource_requests
                    .requests()
                    .iter()
                    .find(|request| {
                        request.key() == &slot.key && request.expected() == &slot.expected
                    })
                    .map(|request| request.handle())
                    .ok_or(ProductEngineError::MissingPreparedExternalSlot {
                        slot: slot.id.get(),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let bindings = match mode {
            FrameProgramMode::Wire => RenderBindings::new(
                *self.template.render_id(),
                self.template.template_hash()?,
                self.prepared.dynamic.clone(),
                generation,
                external_ids,
                self.frame_programs,
            )?,
            FrameProgramMode::Constructed => RenderBindings::new_constructed(
                *self.template.render_id(),
                self.template.template_hash()?,
                self.prepared.dynamic.clone(),
                generation,
                external_ids,
                self.frame_programs,
            )?,
        };
        self.template.validate_bindings(&bindings)?;
        Ok(BoundTicket {
            evaluated: self.evaluated,
            prepared: self.prepared,
            graph: self.graph,
            template: self.template,
            bindings,
            capabilities: self.capabilities,
            template_cache_hit: self.template_cache_hit,
        })
    }
}

/// A fully assembled plan/binding pair. Executor object admission is deliberately separate and
/// borrows the host-owned object table, preventing platform objects from entering this value.
#[derive(Debug)]
pub struct BoundTicket {
    evaluated: EvaluatedRenderFrame,
    prepared: PrepareOutput,
    graph: RenderGraph,
    template: Arc<RenderPlanTemplate>,
    bindings: RenderBindings,
    capabilities: BackendCapabilities,
    template_cache_hit: bool,
}

impl BoundTicket {
    pub const fn evaluated(&self) -> &EvaluatedRenderFrame {
        &self.evaluated
    }

    pub const fn prepared(&self) -> &PrepareOutput {
        &self.prepared
    }

    pub const fn graph(&self) -> &RenderGraph {
        &self.graph
    }

    pub fn template(&self) -> &RenderPlanTemplate {
        self.template.as_ref()
    }

    pub const fn bindings(&self) -> &RenderBindings {
        &self.bindings
    }

    pub const fn capabilities(&self) -> &BackendCapabilities {
        &self.capabilities
    }

    pub fn bound_program_schedules(
        &self,
    ) -> Result<crate::compositor::lower::BoundProgramSchedules, ProductEngineError> {
        self.template
            .bind_program_schedules(&self.bindings, &self.capabilities)
            .map_err(ProductEngineError::from)
    }

    pub const fn template_cache_hit(&self) -> bool {
        self.template_cache_hit
    }

    pub fn into_plan(self) -> (Arc<RenderPlanTemplate>, RenderBindings) {
        (self.template, self.bindings)
    }
}

#[derive(Debug, Error)]
pub enum ProductEngineError {
    #[error("frame request render {actual} does not match pinned render {expected}")]
    RenderMismatch {
        expected: RenderId,
        actual: RenderId,
    },
    #[error(transparent)]
    Evaluate(#[from] RuntimeFault),
    #[error(transparent)]
    Prepare(#[from] PrepareError),
    #[error(transparent)]
    Graph(#[from] GraphBuildError),
    #[error(transparent)]
    Lower(#[from] LowerError),
    #[error(transparent)]
    Plan(#[from] crate::compositor::lower::PlanValidationError),
    #[error(transparent)]
    Binding(#[from] BindingContractError),
    #[error(transparent)]
    ProgramBinding(#[from] crate::compositor::lower::ProgramBindingError),
    #[error("prepared resource requests do not satisfy external plan slot {slot}")]
    MissingPreparedExternalSlot { slot: u32 },
    #[error("RenderPlan template cache lock was poisoned")]
    PlanCachePoisoned,
}

const MAX_PLAN_TEMPLATE_CACHE: usize = 64;
const MAX_PLAN_TEMPLATE_CACHE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PlanCacheKey {
    structure_hash: ContentDigest,
    mode: FrameProgramMode,
}

#[derive(Debug, Default)]
struct PlanTemplateCache {
    entries: BTreeMap<PlanCacheKey, CachedPlanTemplate>,
    order: VecDeque<PlanCacheKey>,
    resident_bytes: u64,
}

#[derive(Debug)]
struct CachedPlanTemplate {
    template: Arc<RenderPlanTemplate>,
    graph_structure_hash: ContentDigest,
}

impl PlanTemplateCache {
    fn get(&mut self, key: &PlanCacheKey) -> Option<Arc<RenderPlanTemplate>> {
        let value = Arc::clone(&self.entries.get(key)?.template);
        if let Some(index) = self.order.iter().position(|candidate| candidate == key) {
            self.order.remove(index);
        }
        self.order.push_back(key.clone());
        Some(value)
    }

    fn recent_for(
        &mut self,
        graph_structure_hash: &ContentDigest,
        render_id: &RenderId,
        render_spec: RenderSpec,
        capability_fingerprint: &ContentDigest,
        mode: FrameProgramMode,
    ) -> Option<Arc<RenderPlanTemplate>> {
        let key = self
            .order
            .iter()
            .rev()
            .find(|key| {
                self.entries.get(*key).is_some_and(|entry| {
                    key.mode == mode
                        && entry.graph_structure_hash == *graph_structure_hash
                        && entry.template.render_id() == render_id
                        && entry.template.render_spec() == render_spec
                        && entry.template.capability_fingerprint() == capability_fingerprint
                })
            })?
            .clone();
        self.get(&key)
    }

    fn insert(
        &mut self,
        key: PlanCacheKey,
        value: Arc<RenderPlanTemplate>,
        graph_structure_hash: ContentDigest,
    ) {
        let bytes = value.cache_size_bytes();
        if bytes > MAX_PLAN_TEMPLATE_CACHE_BYTES {
            return;
        }

        if let Some(previous) = self.entries.remove(&key) {
            self.resident_bytes = self
                .resident_bytes
                .saturating_sub(previous.template.cache_size_bytes());
        }
        if let Some(index) = self.order.iter().position(|candidate| candidate == &key) {
            self.order.remove(index);
        }

        while self.entries.len() >= MAX_PLAN_TEMPLATE_CACHE
            || self.resident_bytes.saturating_add(bytes) > MAX_PLAN_TEMPLATE_CACHE_BYTES
        {
            let Some(oldest) = self.order.pop_front() else {
                self.entries.clear();
                self.resident_bytes = 0;
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.resident_bytes = self
                    .resident_bytes
                    .saturating_sub(removed.template.cache_size_bytes());
            }
        }
        self.entries.insert(
            key.clone(),
            CachedPlanTemplate {
                template: value,
                graph_structure_hash,
            },
        );
        self.order.push_back(key);
        self.resident_bytes = self.resident_bytes.saturating_add(bytes);
        debug_assert!(self.entries.len() <= MAX_PLAN_TEMPLATE_CACHE);
        debug_assert!(self.resident_bytes <= MAX_PLAN_TEMPLATE_CACHE_BYTES);
    }
}
