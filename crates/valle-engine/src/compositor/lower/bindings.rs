use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use valle_timeline::internal::RenderId;

use crate::{
    prepare::{DynamicBindingId, DynamicBindingKind, DynamicBindings, ProgramId, RequestError},
    resource::{
        ContentDigest, ExternalGeneration, ExternalHandleId, ExternalResourceDesc,
        ResourceInterpretation, ResourceKey,
    },
};

use super::PlanProgram;

pub const RENDER_BINDINGS_FORMAT_VERSION: u32 = 1;

/// Stable, one-based slot in a plan's external binding layout.
///
/// The slot belongs to the reusable template. The [`ExternalHandleId`] bound to it belongs only to
/// one executor call and is deliberately absent from the template identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct ExternalSlotId(u32);

impl ExternalSlotId {
    fn from_index(index: usize) -> Result<Self, BindingContractError> {
        let value = index
            .checked_add(1)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(BindingContractError::SlotBudgetExceeded)?;
        Ok(Self(value))
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub(crate) fn index(self) -> usize {
        (self.0 - 1) as usize
    }
}

impl TryFrom<u32> for ExternalSlotId {
    type Error = BindingContractError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value == 0 {
            Err(BindingContractError::ZeroExternalSlot)
        } else {
            Ok(Self(value))
        }
    }
}

impl From<ExternalSlotId> for u32 {
    fn from(value: ExternalSlotId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DynamicSlot {
    pub id: DynamicBindingId,
    pub semantic_path: String,
    pub binding_kind: DynamicBindingKind,
}

impl DynamicSlot {
    pub fn new(
        id: DynamicBindingId,
        semantic_path: impl Into<String>,
        binding_kind: DynamicBindingKind,
    ) -> Result<Self, BindingContractError> {
        let slot = Self {
            id,
            semantic_path: semantic_path.into(),
            binding_kind,
        };
        if slot.semantic_path.is_empty() {
            return Err(BindingContractError::EmptySemanticPath {
                kind: "dynamic slot",
            });
        }
        Ok(slot)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalSlot {
    pub id: ExternalSlotId,
    pub semantic_path: String,
    pub key: ResourceKey,
    pub expected: ExternalResourceDesc,
}

impl ExternalSlot {
    pub fn new(
        id: ExternalSlotId,
        semantic_path: impl Into<String>,
        key: ResourceKey,
        expected: ExternalResourceDesc,
    ) -> Result<Self, BindingContractError> {
        let slot = Self {
            id,
            semantic_path: semantic_path.into(),
            key,
            expected,
        };
        slot.validate()?;
        Ok(slot)
    }

    fn validate(&self) -> Result<(), BindingContractError> {
        if self.semantic_path.is_empty() {
            return Err(BindingContractError::EmptySemanticPath {
                kind: "external slot",
            });
        }
        if !external_shape_matches(&self.key.interpretation, &self.expected) {
            return Err(BindingContractError::ExternalTypeMismatch { slot: self.id });
        }
        Ok(())
    }
}

/// Static slot contract embedded in a `RenderPlanTemplate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PlanBindingLayoutWire", rename_all = "camelCase")]
pub struct PlanBindingLayout {
    dynamic_slots: Vec<DynamicSlot>,
    external_slots: Vec<ExternalSlot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanBindingLayoutWire {
    dynamic_slots: Vec<DynamicSlot>,
    external_slots: Vec<ExternalSlot>,
}

impl PlanBindingLayout {
    pub fn new(
        dynamic_slots: Vec<DynamicSlot>,
        external_slots: Vec<ExternalSlot>,
    ) -> Result<Self, BindingContractError> {
        let layout = Self {
            dynamic_slots,
            external_slots,
        };
        layout.validate()?;
        Ok(layout)
    }

    pub fn validate(&self) -> Result<(), BindingContractError> {
        for (index, slot) in self.dynamic_slots.iter().enumerate() {
            let expected =
                u32::try_from(index + 1).map_err(|_| BindingContractError::SlotBudgetExceeded)?;
            if slot.id.get() != expected {
                return Err(BindingContractError::NonCanonicalDynamicSlots);
            }
            if slot.semantic_path.is_empty() {
                return Err(BindingContractError::EmptySemanticPath {
                    kind: "dynamic slot",
                });
            }
        }
        for (index, slot) in self.external_slots.iter().enumerate() {
            if slot.id != ExternalSlotId::from_index(index)? {
                return Err(BindingContractError::NonCanonicalExternalSlots);
            }
            slot.validate()?;
        }
        Ok(())
    }

    pub fn dynamic_slots(&self) -> &[DynamicSlot] {
        &self.dynamic_slots
    }

    pub fn external_slots(&self) -> &[ExternalSlot] {
        &self.external_slots
    }

    pub(crate) fn admits_dynamic_layout(
        &self,
        dynamic: &DynamicBindings,
    ) -> Result<(), BindingContractError> {
        dynamic.validate()?;
        if dynamic.values().len() != self.dynamic_slots.len() {
            return Err(BindingContractError::DynamicCountMismatch {
                expected: self.dynamic_slots.len(),
                actual: dynamic.values().len(),
            });
        }
        for (layout, value) in self.dynamic_slots.iter().zip(dynamic.values()) {
            if layout.id != value.id
                || layout.semantic_path != value.semantic_path
                || layout.binding_kind != value.binding_kind
            {
                return Err(BindingContractError::DynamicLayoutMismatch { id: layout.id });
            }
        }
        Ok(())
    }

    /// Verifies the complete per-frame binding packet before an executor may touch its target.
    pub fn validate_bindings(
        &self,
        template_hash: &ContentDigest,
        bindings: &RenderBindings,
    ) -> Result<(), BindingContractError> {
        self.validate()?;
        bindings.validate()?;
        if bindings.template_hash() != template_hash {
            return Err(BindingContractError::TemplateHashMismatch);
        }
        self.admits_dynamic_layout(bindings.dynamic())?;
        if bindings.external_ids().len() != self.external_slots.len() {
            return Err(BindingContractError::ExternalCountMismatch {
                expected: self.external_slots.len(),
                actual: bindings.external_ids().len(),
            });
        }
        Ok(())
    }
}

impl TryFrom<PlanBindingLayoutWire> for PlanBindingLayout {
    type Error = BindingContractError;

    fn try_from(value: PlanBindingLayoutWire) -> Result<Self, Self::Error> {
        Self::new(value.dynamic_slots, value.external_slots)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RenderBindingsWire", rename_all = "camelCase")]
pub struct RenderBindings {
    render_id: RenderId,
    template_hash: ContentDigest,
    dynamic: DynamicBindings,
    external_generation: ExternalGeneration,
    external_ids: Vec<ExternalHandleId>,
    programs: Vec<PlanProgram>,
    #[serde(skip)]
    cached_hash: ContentDigest,
    #[serde(skip)]
    cached_packed: Arc<[u8]>,
    #[serde(skip)]
    constructed: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenderBindingsWire {
    render_id: RenderId,
    template_hash: ContentDigest,
    dynamic: DynamicBindings,
    external_generation: ExternalGeneration,
    external_ids: Vec<ExternalHandleId>,
    programs: Vec<PlanProgram>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConstructedBindingsIdentity<'a> {
    render_id: &'a RenderId,
    template_hash: &'a ContentDigest,
    dynamic: &'a DynamicBindings,
    external_generation: ExternalGeneration,
    external_ids: &'a [ExternalHandleId],
    programs: Vec<ConstructedProgramIdentity<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConstructedProgramIdentity<'a> {
    id: ProgramId,
    content_hash: &'a ContentDigest,
}

impl RenderBindings {
    pub fn new(
        render_id: RenderId,
        template_hash: ContentDigest,
        dynamic: DynamicBindings,
        external_generation: ExternalGeneration,
        external_ids: Vec<ExternalHandleId>,
        programs: Vec<PlanProgram>,
    ) -> Result<Self, BindingContractError> {
        let cached_hash = template_hash.clone();
        let bindings = Self {
            render_id,
            template_hash,
            dynamic,
            external_generation,
            external_ids,
            programs,
            cached_hash,
            cached_packed: Arc::from([]),
            constructed: false,
        };
        bindings.finalize()
    }

    pub(crate) fn new_constructed(
        render_id: RenderId,
        template_hash: ContentDigest,
        dynamic: DynamicBindings,
        external_generation: ExternalGeneration,
        external_ids: Vec<ExternalHandleId>,
        programs: Vec<PlanProgram>,
    ) -> Result<Self, BindingContractError> {
        if programs.iter().any(|program| !program.is_constructed()) {
            return Err(BindingContractError::Canonical(
                "constructed bindings require attached frame programs".into(),
            ));
        }
        let cached_hash = template_hash.clone();
        let mut bindings = Self {
            render_id,
            template_hash,
            dynamic,
            external_generation,
            external_ids,
            programs,
            cached_hash,
            cached_packed: Arc::from([]),
            constructed: true,
        };
        bindings.validate()?;
        let identity = ConstructedBindingsIdentity {
            render_id: &bindings.render_id,
            template_hash: &bindings.template_hash,
            dynamic: &bindings.dynamic,
            external_generation: bindings.external_generation,
            external_ids: &bindings.external_ids,
            programs: bindings
                .programs
                .iter()
                .map(|program| ConstructedProgramIdentity {
                    id: program.frame_id(),
                    content_hash: program.content_hash(),
                })
                .collect(),
        };
        let bytes = crate::canonical::bytes(&identity)
            .map_err(|error| BindingContractError::Canonical(error.to_string()))?;
        bindings.cached_hash = ContentDigest::of_bytes(&bytes);
        Ok(bindings)
    }

    pub fn validate(&self) -> Result<(), BindingContractError> {
        self.dynamic.validate()?;
        u32::try_from(self.external_ids.len())
            .map_err(|_| BindingContractError::SlotBudgetExceeded)?;
        for (index, program) in self.programs.iter().enumerate() {
            let expected =
                u32::try_from(index + 1).map_err(|_| BindingContractError::SlotBudgetExceeded)?;
            if program.frame_id().get() != expected {
                return Err(BindingContractError::Program {
                    index,
                    reason: "program ids must be contiguous".into(),
                });
            }
            program
                .validate_payload()
                .map_err(|error| BindingContractError::Program {
                    index,
                    reason: error.to_string(),
                })?;
        }
        Ok(())
    }

    fn finalize(mut self) -> Result<Self, BindingContractError> {
        if self.constructed || self.programs.iter().any(PlanProgram::is_constructed) {
            return Err(BindingContractError::Canonical(
                "attached bindings cannot be serialized as a wire packet".into(),
            ));
        }
        self.validate()?;
        let packed = super::packed::encode(
            &self,
            super::packed::Contract::bindings(RENDER_BINDINGS_FORMAT_VERSION),
        )
        .map_err(|error| BindingContractError::Canonical(error.to_string()))?;
        self.cached_hash = ContentDigest::of_bytes(&packed);
        self.cached_packed = packed.into();
        Ok(self)
    }

    pub const fn render_id(&self) -> &RenderId {
        &self.render_id
    }

    pub const fn template_hash(&self) -> &ContentDigest {
        &self.template_hash
    }

    pub const fn dynamic(&self) -> &DynamicBindings {
        &self.dynamic
    }

    pub const fn external_generation(&self) -> ExternalGeneration {
        self.external_generation
    }

    pub fn external_ids(&self) -> &[ExternalHandleId] {
        &self.external_ids
    }

    pub fn programs(&self) -> &[PlanProgram] {
        &self.programs
    }

    pub const fn binding_hash(&self) -> &ContentDigest {
        &self.cached_hash
    }

    /// Versioned binary product ABI. It contains generation-local integer handles only; platform
    /// objects remain in the executor table.
    pub fn packed_bytes(&self) -> Result<Vec<u8>, super::PackedPlanError> {
        if self.constructed {
            return Err(super::PackedPlanError::InvalidValue {
                kind: "RenderBindings",
                reason: "trusted in-process bindings have no wire packet".into(),
            });
        }
        Ok(self.cached_packed.to_vec())
    }

    pub fn from_packed(bytes: &[u8]) -> Result<Self, super::PackedPlanError> {
        super::packed::decode(
            bytes,
            super::packed::Contract::bindings(RENDER_BINDINGS_FORMAT_VERSION),
        )
    }
}

impl TryFrom<RenderBindingsWire> for RenderBindings {
    type Error = BindingContractError;

    fn try_from(value: RenderBindingsWire) -> Result<Self, Self::Error> {
        let cached_hash = value.template_hash.clone();
        let bindings = Self {
            render_id: value.render_id,
            template_hash: value.template_hash,
            dynamic: value.dynamic,
            external_generation: value.external_generation,
            external_ids: value.external_ids,
            programs: value.programs,
            cached_hash,
            cached_packed: Arc::from([]),
            constructed: false,
        };
        bindings.finalize()
    }
}

fn external_shape_matches(
    interpretation: &ResourceInterpretation,
    expected: &ExternalResourceDesc,
) -> bool {
    matches!(
        (interpretation, expected),
        (
            ResourceInterpretation::Visual { .. },
            ExternalResourceDesc::VisualFrame { .. }
        ) | (
            ResourceInterpretation::FontFace { .. },
            ExternalResourceDesc::FontBytes
        ) | (
            ResourceInterpretation::RuntimeShader { .. },
            ExternalResourceDesc::RuntimeShader
        ) | (
            ResourceInterpretation::Scene3d { .. },
            ExternalResourceDesc::Scene3d
        )
    )
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BindingContractError {
    #[error("external binding slot zero is reserved")]
    ZeroExternalSlot,
    #[error("binding slot budget exceeded")]
    SlotBudgetExceeded,
    #[error("{kind} semantic path must not be empty")]
    EmptySemanticPath { kind: &'static str },
    #[error("dynamic binding slots must be contiguous and one-based")]
    NonCanonicalDynamicSlots,
    #[error("external binding slots must be contiguous and one-based")]
    NonCanonicalExternalSlots,
    #[error("external slot {slot:?} key and expected descriptor have incompatible types")]
    ExternalTypeMismatch { slot: ExternalSlotId },
    #[error("RenderBindings template hash does not match the RenderPlanTemplate")]
    TemplateHashMismatch,
    #[error("dynamic binding count mismatch: expected {expected}, got {actual}")]
    DynamicCountMismatch { expected: usize, actual: usize },
    #[error("dynamic binding {id:?} does not match the template layout")]
    DynamicLayoutMismatch { id: DynamicBindingId },
    #[error("external binding count mismatch: expected {expected}, got {actual}")]
    ExternalCountMismatch { expected: usize, actual: usize },
    #[error("program binding {index} is invalid: {reason}")]
    Program { index: usize, reason: String },
    #[error("RenderBindings canonicalization failed: {0}")]
    Canonical(String),
    #[error(transparent)]
    Dynamic(#[from] RequestError),
}
