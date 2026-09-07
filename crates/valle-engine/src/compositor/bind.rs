use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::resource::{
    ContentDigest, ExternalGeneration, ExternalHandleId, ExternalResourceDesc, ResourceKey,
};

use super::lower::{ExternalSlotId, PlanValidationError, RenderBindings, RenderPlanTemplate};

/// Metadata view implemented by an executor-owned external object.
///
/// The object itself can be a CPU buffer, decoder frame, GPU texture, font, shader or scene. Engine
/// sees only the frozen identity and descriptor required to prove that the object satisfies a plan
/// slot. This trait deliberately has no serialization or cloning requirement.
pub trait ExternalObject {
    fn key(&self) -> &ResourceKey;

    fn descriptor(&self) -> &ExternalResourceDesc;
}

/// One immutable fulfillment batch. Handle identity is scoped by `generation`.
///
/// The table is intentionally neither `Clone` nor `Serialize`: backend objects never become plan
/// data or cache identity. A host creates a fresh generation whenever handle values may be reused.
#[derive(Debug)]
pub struct ExternalObjectTable<T> {
    generation: ExternalGeneration,
    objects: BTreeMap<ExternalHandleId, T>,
}

impl<T> ExternalObjectTable<T> {
    pub fn try_from_entries(
        generation: ExternalGeneration,
        entries: impl IntoIterator<Item = (ExternalHandleId, T)>,
    ) -> Result<Self, ExternalBindError> {
        let mut objects = BTreeMap::new();
        for (handle, object) in entries {
            if objects.insert(handle, object).is_some() {
                return Err(ExternalBindError::DuplicateTableHandle { handle });
            }
        }
        Ok(Self {
            generation,
            objects,
        })
    }

    pub const fn generation(&self) -> ExternalGeneration {
        self.generation
    }

    pub fn get(&self, handle: ExternalHandleId) -> Option<&T> {
        self.objects.get(&handle)
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

/// One slot resolved to a borrowed backend object after complete preflight.
#[derive(Debug)]
pub struct BoundExternalObject<'a, T> {
    slot: ExternalSlotId,
    handle: ExternalHandleId,
    object: &'a T,
}

impl<'a, T> BoundExternalObject<'a, T> {
    pub const fn slot(&self) -> ExternalSlotId {
        self.slot
    }

    pub const fn handle(&self) -> ExternalHandleId {
        self.handle
    }

    pub const fn object(&self) -> &'a T {
        self.object
    }
}

/// Exact, template-slot-ordered object projection safe for executor consumption.
///
/// Its lifetime immutably borrows the table, so Rust prevents handle-table mutation between bind
/// and execute. Construction is only possible through [`bind_external_objects`].
#[derive(Debug)]
pub struct BoundExternalObjects<'a, T> {
    template_hash: ContentDigest,
    generation: ExternalGeneration,
    objects: Vec<BoundExternalObject<'a, T>>,
}

impl<'a, T> BoundExternalObjects<'a, T> {
    pub const fn template_hash(&self) -> &ContentDigest {
        &self.template_hash
    }

    pub const fn generation(&self) -> ExternalGeneration {
        self.generation
    }

    pub fn objects(&self) -> &[BoundExternalObject<'a, T>] {
        &self.objects
    }

    pub fn get(&self, slot: ExternalSlotId) -> Option<&'a T> {
        self.objects
            .get(slot.get() as usize - 1)
            .filter(|binding| binding.slot == slot)
            .map(|binding| binding.object)
    }
}

/// Validates a complete plan/binding/object triple before an executor is allowed to touch target.
pub fn bind_external_objects<'a, T: ExternalObject>(
    template: &RenderPlanTemplate,
    bindings: &RenderBindings,
    table: &'a ExternalObjectTable<T>,
) -> Result<BoundExternalObjects<'a, T>, ExternalBindError> {
    template.validate_bindings(bindings)?;
    if bindings.external_generation() != table.generation {
        return Err(ExternalBindError::GenerationMismatch {
            bindings: bindings.external_generation(),
            table: table.generation,
        });
    }

    let mut seen = BTreeSet::new();
    let mut objects = Vec::with_capacity(template.binding_layout().external_slots().len());
    for (slot, handle) in template
        .binding_layout()
        .external_slots()
        .iter()
        .zip(bindings.external_ids())
    {
        if !seen.insert(*handle) {
            return Err(ExternalBindError::DuplicateBindingHandle { handle: *handle });
        }
        let object = table.get(*handle).ok_or(ExternalBindError::MissingHandle {
            slot: slot.id,
            handle: *handle,
        })?;
        if object.key() != &slot.key {
            return Err(ExternalBindError::IdentityMismatch {
                slot: slot.id,
                handle: *handle,
            });
        }
        if object.descriptor() != &slot.expected {
            return Err(ExternalBindError::DescriptorMismatch {
                slot: slot.id,
                handle: *handle,
            });
        }
        objects.push(BoundExternalObject {
            slot: slot.id,
            handle: *handle,
            object,
        });
    }

    Ok(BoundExternalObjects {
        template_hash: bindings.template_hash().clone(),
        generation: table.generation,
        objects,
    })
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExternalBindError {
    #[error(transparent)]
    Plan(#[from] PlanValidationError),
    #[error("external object table contains duplicate handle {handle:?}")]
    DuplicateTableHandle { handle: ExternalHandleId },
    #[error("RenderBindings reuse external handle {handle:?} for more than one template slot")]
    DuplicateBindingHandle { handle: ExternalHandleId },
    #[error("RenderBindings generation {bindings:?} does not match object table {table:?}")]
    GenerationMismatch {
        bindings: ExternalGeneration,
        table: ExternalGeneration,
    },
    #[error("external slot {slot:?} is missing handle {handle:?}")]
    MissingHandle {
        slot: ExternalSlotId,
        handle: ExternalHandleId,
    },
    #[error("external handle {handle:?} does not match identity for slot {slot:?}")]
    IdentityMismatch {
        slot: ExternalSlotId,
        handle: ExternalHandleId,
    },
    #[error("external handle {handle:?} does not match descriptor for slot {slot:?}")]
    DescriptorMismatch {
        slot: ExternalSlotId,
        handle: ExternalHandleId,
    },
}
