use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
    sync::{Arc, OnceLock},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use valle_draw::{Rect, program::BlendMode, requirements::DrawRequirements};
use valle_timeline::internal::RenderId;

use crate::{
    canonical,
    compositor::graph::{GraphCapability, GraphOrigin, GraphRoi, PassId, ResourceId},
    frame::RenderSpec,
    prepare::{
        BoundsReason, DynamicBindingId, DynamicBindingKind, ExternalPlacement,
        PreparedDestinationUse, PreparedEffectKernel, PreparedEffectSpace,
        PreparedExternalBackdrop, PreparedMask, PreparedProgramKind, PreparedTransitionKernel,
        ProgramId,
    },
    resource::{ContentDigest, LogicalTextureDesc, OutputSpec, TextureFormat},
};

use super::{
    BindingContractError, ExecutionPassId, ExternalSlotId, PlanBindingLayout, PlanIdError,
    PlanResourceId, ProgramPlan, ProgramSchedule, RenderBindings, SurfaceSlotId, program_patch,
};

pub const RENDER_PLAN_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanTextureBinding {
    pub key: String,
    pub slot: ExternalSlotId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanFontBinding {
    pub face_hash: ContentDigest,
    pub face_index: u32,
    pub slot: ExternalSlotId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanStructureBinding {
    pub key: String,
    pub slot: ExternalSlotId,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanProgramResources {
    pub textures: Vec<PlanTextureBinding>,
    pub fonts: Vec<PlanFontBinding>,
    pub runtime_shaders: Vec<PlanStructureBinding>,
    pub scenes: Vec<PlanStructureBinding>,
}

/// One frame's minimal DrawProgram payload.
///
/// Static requirements, resource slots and local execution topology live exactly once in
/// [`PlanProgramLayout`]. The skipped layout attachment lets trusted in-process lowering avoid a
/// redundant decode; packets decoded at an executor boundary are admitted against the template
/// before any field on the layout is used.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "PlanProgramWire", into = "PlanProgramWire")]
pub struct PlanProgram {
    id: ProgramId,
    content_hash: ContentDigest,
    frame_hash: ContentDigest,
    patch: Vec<u8>,
    frame_patch: Vec<u8>,
    #[serde(skip)]
    frame: OnceLock<PlanProgramFrame>,
    #[serde(skip)]
    packed: OnceLock<Vec<u8>>,
    #[serde(skip)]
    structure_hash: OnceLock<ContentDigest>,
    #[serde(skip)]
    admitted_baseline: OnceLock<PlanProgramAdmissionKey>,
    #[serde(skip)]
    initial_layout: Option<PlanProgramLayout>,
    /// Trusted in-process programs carry their exact decoded arena directly. They deliberately
    /// omit random-access wire patches until a Web/wire path explicitly asks for them.
    #[serde(skip)]
    constructed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanProgramWire {
    id: ProgramId,
    content_hash: ContentDigest,
    frame_hash: ContentDigest,
    #[serde(with = "packed_program_bytes")]
    patch: Vec<u8>,
    #[serde(with = "packed_program_bytes")]
    frame_patch: Vec<u8>,
}

/// Exact admitted program contract carried by one frame binding packet. It is exposed only
/// through shared dereferencing from `PlanProgram`, so trusted construction state cannot be
/// mutated after its validation proof is attached.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanProgramFrame {
    pub id: ProgramId,
    pub kind: PreparedProgramKind,
    pub semantic_path: String,
    pub viewport: Rect,
    pub requirements: DrawRequirements,
    pub resources: PlanProgramResources,
    pub destination_uses: Vec<PreparedDestinationUse>,
    pub local_plan: ProgramPlan,
    pub local_schedule: ProgramSchedule,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanProgramFramePayload {
    viewport: Rect,
    requirements: DrawRequirements,
    local_plan: ProgramPlan,
    local_schedule: ProgramSchedule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanProgramAdmissionKey {
    content: ContentDigest,
    frame: ContentDigest,
}

pub(super) mod packed_program_bytes {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use serde::{Deserialize as _, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        STANDARD.decode(encoded).map_err(|error| {
            D::Error::custom(format!("invalid packed DrawProgram base64: {error}"))
        })
    }
}

/// Static physical identity of one program slot in a reusable RenderPlan template.
///
/// The content hash, packed DrawProgram, animated geometry/paint, local bounds and current
/// resource identities live in `RenderBindings.programs`. Only data that can change pass/storage
/// topology is retained here. A frame program must reproduce `structure_hash` before execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanProgramLayout {
    pub id: ProgramId,
    pub kind: PreparedProgramKind,
    pub semantic_path: String,
    pub structure_hash: ContentDigest,
    pub baseline_content_hash: ContentDigest,
    #[serde(with = "packed_program_bytes")]
    pub baseline_packed: Vec<u8>,
    pub baseline_frame_hash: ContentDigest,
    #[serde(with = "packed_program_bytes")]
    pub baseline_frame_packed: Vec<u8>,
    pub resources: PlanProgramResources,
    pub destination_uses: Vec<PreparedDestinationUse>,
    /// Cached structural projection for the trusted in-process path. Wire-decoded templates
    /// reconstruct it during their one-time baseline admission.
    #[serde(skip)]
    attached_structure: OnceLock<ProgramStructureProjection>,
}

impl PlanProgram {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_lowered(
        id: ProgramId,
        kind: PreparedProgramKind,
        semantic_path: String,
        content_hash: ContentDigest,
        viewport: Rect,
        packed: Vec<u8>,
        requirements: DrawRequirements,
        resources: PlanProgramResources,
        destination_uses: Vec<PreparedDestinationUse>,
        local_plan: ProgramPlan,
        local_schedule: ProgramSchedule,
    ) -> Result<Self, PlanValidationError> {
        let actual_content_hash = ContentDigest::of_bytes(&packed);
        if content_hash != actual_content_hash {
            return Err(invalid(
                "program.contentHash",
                "content hash does not name the packed DrawProgram",
            ));
        }
        let frame = PlanProgramFrame {
            id,
            kind,
            semantic_path,
            viewport,
            requirements,
            resources,
            destination_uses,
            local_plan,
            local_schedule,
        };
        let structure_hash = program_structure_hash(&frame)?;
        let baseline_content_hash = content_hash;
        let patch = program_patch::encode(&packed, &packed)
            .map_err(|error| invalid("program.patch", error))?;
        let baseline_frame_packed = program_frame_payload_bytes(&frame)?;
        let baseline_frame_hash = ContentDigest::of_bytes(&baseline_frame_packed);
        let frame_patch = program_patch::encode(&baseline_frame_packed, &baseline_frame_packed)
            .map_err(|error| invalid("program.framePatch", error))?;
        let layout = PlanProgramLayout {
            id: frame.id,
            kind: frame.kind,
            semantic_path: frame.semantic_path.clone(),
            structure_hash: structure_hash.clone(),
            baseline_content_hash: baseline_content_hash.clone(),
            baseline_packed: packed.clone(),
            baseline_frame_hash: baseline_frame_hash.clone(),
            baseline_frame_packed,
            resources: frame.resources.clone(),
            destination_uses: frame.destination_uses.clone(),
            attached_structure: OnceLock::from(ProgramStructureProjection::from_program(&frame)),
        };
        let program = Self {
            id,
            content_hash,
            frame_hash: baseline_frame_hash.clone(),
            patch,
            frame_patch,
            frame: OnceLock::from(frame),
            packed: OnceLock::from(packed),
            structure_hash: OnceLock::from(structure_hash),
            admitted_baseline: OnceLock::from(PlanProgramAdmissionKey {
                content: baseline_content_hash,
                frame: baseline_frame_hash,
            }),
            initial_layout: Some(layout),
            constructed: false,
        };
        program.validate_payload()?;
        Ok(program)
    }

    /// Builds the first frame of a trusted Native template without manufacturing an identity
    /// patch from a DrawProgram back to itself. The packed baseline and its metadata remain on the
    /// layout, so an independently lowered wire template keeps the complete random-access ABI;
    /// only the same-process frame packet stays attached.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_lowered_constructed(
        id: ProgramId,
        kind: PreparedProgramKind,
        semantic_path: String,
        content_hash: ContentDigest,
        viewport: Rect,
        packed: Vec<u8>,
        requirements: DrawRequirements,
        resources: PlanProgramResources,
        destination_uses: Vec<PreparedDestinationUse>,
        local_plan: ProgramPlan,
        local_schedule: ProgramSchedule,
    ) -> Result<Self, PlanValidationError> {
        // `build_constructed_render_graph` carries the exact admitted digest and arena from the
        // same PreparedFrame. Rehashing that arena here would be a second admission pass; public
        // lowering and every decoded wire packet still verify the digest independently.
        let frame = PlanProgramFrame {
            id,
            kind,
            semantic_path,
            viewport,
            requirements,
            resources,
            destination_uses,
            local_plan,
            local_schedule,
        };
        let structure_hash = program_structure_hash(&frame)?;
        let baseline_content_hash = content_hash;
        let baseline_frame_packed = program_frame_payload_bytes(&frame)?;
        let baseline_frame_hash = ContentDigest::of_bytes(&baseline_frame_packed);
        let layout = PlanProgramLayout {
            id: frame.id,
            kind: frame.kind,
            semantic_path: frame.semantic_path.clone(),
            structure_hash: structure_hash.clone(),
            baseline_content_hash: baseline_content_hash.clone(),
            baseline_packed: packed.clone(),
            baseline_frame_hash: baseline_frame_hash.clone(),
            baseline_frame_packed,
            resources: frame.resources.clone(),
            destination_uses: frame.destination_uses.clone(),
            attached_structure: OnceLock::from(ProgramStructureProjection::from_program(&frame)),
        };
        let program = Self {
            id,
            content_hash,
            frame_hash: baseline_content_hash.clone(),
            patch: Vec::new(),
            frame_patch: Vec::new(),
            frame: OnceLock::from(frame),
            packed: OnceLock::from(packed),
            structure_hash: OnceLock::from(structure_hash),
            admitted_baseline: OnceLock::from(PlanProgramAdmissionKey {
                content: baseline_content_hash,
                frame: baseline_frame_hash,
            }),
            initial_layout: Some(layout),
            constructed: true,
        };
        program.validate_payload()?;
        Ok(program)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_lowered_for_layout(
        id: ProgramId,
        kind: PreparedProgramKind,
        semantic_path: String,
        content_hash: ContentDigest,
        viewport: Rect,
        packed: Vec<u8>,
        requirements: DrawRequirements,
        resources: PlanProgramResources,
        destination_uses: Vec<PreparedDestinationUse>,
        local_plan: ProgramPlan,
        local_schedule: ProgramSchedule,
        layout: &PlanProgramLayout,
    ) -> Result<Self, PlanValidationError> {
        if content_hash != ContentDigest::of_bytes(&packed) {
            return Err(invalid(
                "program.contentHash",
                "content hash does not name the packed DrawProgram",
            ));
        }
        let frame = PlanProgramFrame {
            id,
            kind,
            semantic_path,
            viewport,
            requirements,
            resources,
            destination_uses,
            local_plan,
            local_schedule,
        };
        let structure_hash = program_structure_hash(&frame)?;
        if id != layout.id
            || frame.kind != layout.kind
            || frame.semantic_path != layout.semantic_path
            || structure_hash != layout.structure_hash
            || frame.resources != layout.resources
            || frame.destination_uses != layout.destination_uses
        {
            return Err(invalid(
                "program.layout",
                "lowered frame program does not match the cached template",
            ));
        }
        let frame_packed = program_frame_payload_bytes(&frame)?;
        let frame_hash = ContentDigest::of_bytes(&frame_packed);
        let patch = program_patch::encode(&layout.baseline_packed, &packed)
            .map_err(|error| invalid("program.patch", error))?;
        let frame_patch = program_patch::encode(&layout.baseline_frame_packed, &frame_packed)
            .map_err(|error| invalid("program.framePatch", error))?;
        let program = Self {
            id,
            content_hash,
            frame_hash,
            patch,
            frame_patch,
            frame: OnceLock::from(frame),
            packed: OnceLock::from(packed),
            structure_hash: OnceLock::from(structure_hash),
            admitted_baseline: OnceLock::from(PlanProgramAdmissionKey {
                content: layout.baseline_content_hash.clone(),
                frame: layout.baseline_frame_hash.clone(),
            }),
            initial_layout: None,
            constructed: false,
        };
        program.validate_payload()?;
        Ok(program)
    }

    /// Trusted in-process lowering keeps the exact DrawProgram and derived frame contract
    /// attached instead of serializing both into random-access patches that Native immediately
    /// reconstructs. The reusable layout still proves topology with its admitted structural
    /// projection; public/wire lowering continues through `from_lowered_for_layout` above.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_lowered_constructed_for_layout(
        id: ProgramId,
        kind: PreparedProgramKind,
        semantic_path: String,
        content_hash: ContentDigest,
        viewport: Rect,
        packed: Vec<u8>,
        requirements: DrawRequirements,
        resources: PlanProgramResources,
        destination_uses: Vec<PreparedDestinationUse>,
        local_plan: ProgramPlan,
        local_schedule: ProgramSchedule,
        layout: &PlanProgramLayout,
    ) -> Result<Self, PlanValidationError> {
        let frame = PlanProgramFrame {
            id,
            kind,
            semantic_path,
            viewport,
            requirements,
            resources,
            destination_uses,
            local_plan,
            local_schedule,
        };
        Self::from_constructed_frame(content_hash, packed, frame, layout)
    }

    fn from_constructed_frame(
        content_hash: ContentDigest,
        packed: Vec<u8>,
        frame: PlanProgramFrame,
        layout: &PlanProgramLayout,
    ) -> Result<Self, PlanValidationError> {
        let content = content_hash;
        let structure = ProgramStructureProjection::from_program(&frame);
        let structure_matches = match layout.attached_structure.get() {
            Some(expected) => expected == &structure,
            None => program_structure_hash(&frame)? == layout.structure_hash,
        };
        if frame.id != layout.id
            || frame.kind != layout.kind
            || frame.semantic_path != layout.semantic_path
            || !structure_matches
            || frame.resources != layout.resources
            || frame.destination_uses != layout.destination_uses
        {
            return Err(invalid(
                "program.layout",
                "constructed frame program does not match the cached template",
            ));
        }
        let program = Self {
            id: frame.id,
            content_hash,
            // For an attached frame the exact DrawProgram digest also identifies every derived
            // viewport/requirement/local-plan value. Wire frames retain their payload hash.
            frame_hash: content,
            patch: Vec::new(),
            frame_patch: Vec::new(),
            frame: OnceLock::from(frame),
            packed: OnceLock::from(packed),
            structure_hash: OnceLock::from(layout.structure_hash.clone()),
            admitted_baseline: OnceLock::from(PlanProgramAdmissionKey {
                content: layout.baseline_content_hash.clone(),
                frame: layout.baseline_frame_hash.clone(),
            }),
            initial_layout: None,
            constructed: true,
        };
        program.validate_payload()?;
        Ok(program)
    }

    /// Cheap validation for the minimal binding payload. Full DrawProgram admission needs the
    /// corresponding template layout and is performed by [`Self::admit_layout`].
    pub fn validate_payload(&self) -> Result<(), PlanValidationError> {
        if self.constructed {
            if !self.patch.is_empty()
                || !self.frame_patch.is_empty()
                || self.frame.get().is_none()
                || self.packed.get().is_none()
                || self.structure_hash.get().is_none()
                || self.admitted_baseline.get().is_none()
                || self.frame_hash != self.content_hash
            {
                return Err(invalid(
                    "program.constructed",
                    "attached program proof is incomplete",
                ));
            }
            return Ok(());
        }
        program_patch::validate(&self.patch).map_err(|error| invalid("program.patch", error))?;
        program_patch::validate(&self.frame_patch)
            .map_err(|error| invalid("program.framePatch", error))?;
        Ok(())
    }

    pub(crate) const fn is_constructed(&self) -> bool {
        self.constructed
    }

    pub(crate) fn admit_layout(
        &self,
        layout: &PlanProgramLayout,
    ) -> Result<(), PlanValidationError> {
        self.validate_payload()?;
        if self.id != layout.id {
            return Err(invalid(
                "program.id",
                "frame program id does not match its layout",
            ));
        }
        let admission = PlanProgramAdmissionKey {
            content: layout.baseline_content_hash.clone(),
            frame: layout.baseline_frame_hash.clone(),
        };
        if let Some(attached) = self.admitted_baseline.get() {
            if attached != &admission
                || self.structure_hash.get() != Some(&layout.structure_hash)
                || self.frame.get().is_some_and(|frame| {
                    frame.kind != layout.kind
                        || frame.semantic_path != layout.semantic_path
                        || frame.resources != layout.resources
                        || frame.destination_uses != layout.destination_uses
                })
            {
                return Err(invalid(
                    "program.layout",
                    "attached frame program layout does not match the template",
                ));
            }
            if self.packed.get().is_some() && self.frame.get().is_some() {
                return Ok(());
            }
        }
        let frame_packed = program_patch::apply(&layout.baseline_frame_packed, &self.frame_patch)
            .map_err(|error| invalid("program.framePatch", error))?;
        if self.frame_hash != ContentDigest::of_bytes(&frame_packed) {
            return Err(invalid(
                "program.frameHash",
                "frame hash does not name the reconstructed program metadata",
            ));
        }
        let payload: PlanProgramFramePayload = serde_json::from_slice(&frame_packed)
            .map_err(|error| invalid("program.framePatch", error))?;
        let frame = PlanProgramFrame {
            id: layout.id,
            kind: layout.kind,
            semantic_path: layout.semantic_path.clone(),
            viewport: payload.viewport,
            requirements: payload.requirements,
            resources: layout.resources.clone(),
            destination_uses: layout.destination_uses.clone(),
            local_plan: payload.local_plan,
            local_schedule: payload.local_schedule,
        };
        let packed = program_patch::apply(&layout.baseline_packed, &self.patch)
            .map_err(|error| invalid("program.patch", error))?;
        if self.content_hash != ContentDigest::of_bytes(&packed) {
            return Err(invalid(
                "program.contentHash",
                "content hash does not name the reconstructed DrawProgram",
            ));
        }
        let decoded = valle_draw::program::DrawProgram::from_packed(&packed)
            .map_err(|error| invalid("program.packed", error))?;
        let expected_local_plan =
            ProgramPlan::derive(&decoded).map_err(|error| invalid("program.localPlan", error))?;
        let expected_local_schedule = ProgramSchedule::derive(&expected_local_plan)
            .map_err(|error| invalid("program.localSchedule", error))?;
        validate_program_contract(
            &frame,
            layout,
            &decoded,
            &expected_local_plan,
            &expected_local_schedule,
        )?;
        self.structure_hash
            .set(layout.structure_hash.clone())
            .map_err(|_| invalid("program.layout", "program structure admission raced"))?;
        self.admitted_baseline
            .set(admission)
            .map_err(|_| invalid("program.layout", "program baseline admission raced"))?;
        self.packed
            .set(packed)
            .map_err(|_| invalid("program.packed", "frame program admission raced"))?;
        self.frame
            .set(frame)
            .map_err(|_| invalid("program.frame", "frame program admission raced"))?;
        Ok(())
    }

    /// Re-encodes this exact frame against the baseline owned by a compatible cached template.
    /// The result remains random-access: it never depends on another frame's binding packet.
    pub(crate) fn rebase(&self, layout: &PlanProgramLayout) -> Result<Self, PlanValidationError> {
        self.validate_payload()?;
        let packed = self
            .packed
            .get()
            .ok_or_else(|| invalid("program.packed", "frame program has not been admitted"))?
            .clone();
        let frame = self
            .frame
            .get()
            .ok_or_else(|| invalid("program.frame", "frame program has not been admitted"))?
            .clone();
        if self.structure_hash.get() != Some(&layout.structure_hash)
            || frame.id != layout.id
            || frame.kind != layout.kind
            || frame.semantic_path != layout.semantic_path
            || frame.resources != layout.resources
            || frame.destination_uses != layout.destination_uses
        {
            return Err(invalid(
                "program.layout",
                "frame program structure does not match the cached template",
            ));
        }
        let patch = program_patch::encode(&layout.baseline_packed, &packed)
            .map_err(|error| invalid("program.patch", error))?;
        let frame_packed = program_frame_payload_bytes(&frame)?;
        let frame_hash = ContentDigest::of_bytes(&frame_packed);
        let frame_patch = program_patch::encode(&layout.baseline_frame_packed, &frame_packed)
            .map_err(|error| invalid("program.framePatch", error))?;
        Ok(Self {
            id: self.id,
            content_hash: self.content_hash.clone(),
            frame_hash,
            patch,
            frame_patch,
            frame: OnceLock::from(frame),
            packed: OnceLock::from(packed),
            structure_hash: OnceLock::from(layout.structure_hash.clone()),
            admitted_baseline: OnceLock::from(PlanProgramAdmissionKey {
                content: layout.baseline_content_hash.clone(),
                frame: layout.baseline_frame_hash.clone(),
            }),
            initial_layout: None,
            constructed: false,
        })
    }

    pub(crate) fn into_constructed(
        self,
        layout: &PlanProgramLayout,
    ) -> Result<Self, PlanValidationError> {
        let packed = self
            .packed
            .into_inner()
            .ok_or_else(|| invalid("program.packed", "frame program has not been admitted"))?;
        let frame = self
            .frame
            .into_inner()
            .ok_or_else(|| invalid("program.frame", "frame program has not been admitted"))?;
        Self::from_constructed_frame(self.content_hash, packed, frame, layout)
    }

    pub const fn frame_id(&self) -> ProgramId {
        self.id
    }

    pub const fn content_hash(&self) -> &ContentDigest {
        &self.content_hash
    }

    pub fn packed(&self) -> &[u8] {
        self.packed
            .get()
            .expect("template admission reconstructs every frame DrawProgram")
    }

    pub fn local_plan(&self) -> &ProgramPlan {
        &self.deref().local_plan
    }

    pub fn local_schedule(&self) -> &ProgramSchedule {
        &self.deref().local_schedule
    }

    fn take_initial_layout(&mut self) -> Result<PlanProgramLayout, PlanValidationError> {
        self.initial_layout
            .take()
            .ok_or_else(|| invalid("program.layout", "lowered program layout is absent"))
    }
}

impl Deref for PlanProgram {
    type Target = PlanProgramFrame;

    fn deref(&self) -> &Self::Target {
        self.frame
            .get()
            .expect("template admission reconstructs every frame program contract")
    }
}

impl PartialEq for PlanProgram {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.content_hash == other.content_hash
            && self.frame_hash == other.frame_hash
            && self.patch == other.patch
            && self.frame_patch == other.frame_patch
    }
}

impl From<PlanProgram> for PlanProgramWire {
    fn from(value: PlanProgram) -> Self {
        Self {
            id: value.id,
            content_hash: value.content_hash,
            frame_hash: value.frame_hash,
            patch: value.patch,
            frame_patch: value.frame_patch,
        }
    }
}

impl TryFrom<PlanProgramWire> for PlanProgram {
    type Error = PlanValidationError;

    fn try_from(value: PlanProgramWire) -> Result<Self, Self::Error> {
        let program = Self {
            id: value.id,
            content_hash: value.content_hash,
            frame_hash: value.frame_hash,
            patch: value.patch,
            frame_patch: value.frame_patch,
            frame: OnceLock::new(),
            packed: OnceLock::new(),
            structure_hash: OnceLock::new(),
            admitted_baseline: OnceLock::new(),
            initial_layout: None,
            constructed: false,
        };
        program.validate_payload()?;
        Ok(program)
    }
}

impl PlanProgramLayout {
    fn validate_baseline(&self) -> Result<(), PlanValidationError> {
        if self.baseline_content_hash != ContentDigest::of_bytes(&self.baseline_packed) {
            return Err(invalid(
                "program.baselineContentHash",
                "baseline content hash does not name the baseline DrawProgram",
            ));
        }
        if self.baseline_frame_hash != ContentDigest::of_bytes(&self.baseline_frame_packed) {
            return Err(invalid(
                "program.baselineFrameHash",
                "baseline frame hash does not name the baseline metadata",
            ));
        }
        let payload: PlanProgramFramePayload = serde_json::from_slice(&self.baseline_frame_packed)
            .map_err(|error| invalid("program.baselineFramePacked", error))?;
        let decoded = valle_draw::program::DrawProgram::from_packed(&self.baseline_packed)
            .map_err(|error| invalid("program.baselinePacked", error))?;
        let local_plan =
            ProgramPlan::derive(&decoded).map_err(|error| invalid("program.localPlan", error))?;
        let local_schedule = ProgramSchedule::derive(&local_plan)
            .map_err(|error| invalid("program.localSchedule", error))?;
        let frame = PlanProgramFrame {
            id: self.id,
            kind: self.kind,
            semantic_path: self.semantic_path.clone(),
            viewport: payload.viewport,
            requirements: payload.requirements,
            resources: self.resources.clone(),
            destination_uses: self.destination_uses.clone(),
            local_plan: payload.local_plan,
            local_schedule: payload.local_schedule,
        };
        validate_program_contract(&frame, self, &decoded, &local_plan, &local_schedule)?;
        let projection = ProgramStructureProjection::from_program(&frame);
        if let Some(attached) = self.attached_structure.get() {
            if attached != &projection {
                return Err(invalid(
                    "program.structureHash",
                    "attached structure projection disagrees with the baseline",
                ));
            }
        } else {
            let _ = self.attached_structure.set(projection);
        }
        Ok(())
    }
}

fn program_frame_payload_bytes(frame: &PlanProgramFrame) -> Result<Vec<u8>, PlanValidationError> {
    canonical::bytes(&PlanProgramFramePayload {
        viewport: frame.viewport,
        requirements: frame.requirements.clone(),
        local_plan: frame.local_plan.clone(),
        local_schedule: frame.local_schedule.clone(),
    })
    .map_err(|error| PlanValidationError::Canonical(error.to_string()))
}

fn validate_program_contract(
    frame: &PlanProgramFrame,
    layout: &PlanProgramLayout,
    decoded: &valle_draw::program::DrawProgram,
    expected_local_plan: &ProgramPlan,
    expected_local_schedule: &ProgramSchedule,
) -> Result<(), PlanValidationError> {
    if program_structure_hash(frame)? != layout.structure_hash {
        return Err(invalid(
            "program.structureHash",
            "frame program structure does not match its template layout",
        ));
    }
    frame
        .local_plan
        .validate_shape(layout.destination_uses.len())
        .map_err(|error| invalid("program.localPlan", error))?;
    if decoded.viewport() != frame.viewport
        || decoded.requirements() != &frame.requirements
        || &frame.local_plan != expected_local_plan
        || &frame.local_schedule != expected_local_schedule
    {
        return Err(invalid(
            "program",
            "packed program, viewport and derived requirements disagree",
        ));
    }
    if frame.resources.textures.len() != frame.requirements.external_textures.len()
        || frame.resources.fonts.len() != frame.requirements.fonts.len()
        || frame.resources.runtime_shaders.len() != frame.requirements.runtime_shaders.len()
        || frame.resources.scenes.len() != frame.requirements.scene3d.len()
        || frame.destination_uses.len() != frame.requirements.destination_uses.len()
    {
        return Err(invalid(
            "program.resources",
            "resource and destination layouts do not match program requirements",
        ));
    }
    for (index, (binding, requirement)) in frame
        .resources
        .textures
        .iter()
        .zip(&frame.requirements.external_textures)
        .enumerate()
    {
        if binding.key != requirement.key {
            return Err(invalid(
                format!("program.resources.textures[{index}]"),
                "texture binding order/key does not match the packed program requirement",
            ));
        }
    }
    for (index, (prepared, required)) in frame
        .destination_uses
        .iter()
        .zip(&frame.requirements.destination_uses)
        .enumerate()
    {
        if prepared.node != required.node
            || prepared.scope != required.scope
            || prepared.operation != required.operation
            || prepared.sample_bounds == prepared.output_bounds
        {
            return Err(invalid(
                format!("program.destinationUses[{index}]"),
                "destination slot does not match the packed program",
            ));
        }
    }
    Ok(())
}

fn program_structure_hash(
    program: &PlanProgramFrame,
) -> Result<ContentDigest, PlanValidationError> {
    let projection = ProgramStructureProjection::from_program(program);
    let bytes = canonical::bytes(&projection)
        .map_err(|error| PlanValidationError::Canonical(error.to_string()))?;
    Ok(ContentDigest::of_bytes(&bytes))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgramStructureProjection {
    kind: PreparedProgramKind,
    viewport_empty: bool,
    texture_kinds: Vec<valle_draw::requirements::TextureKind>,
    font_count: usize,
    runtime_shader_count: usize,
    scene_count: usize,
    capabilities: Vec<valle_draw::requirements::DrawCapability>,
    destinations: Vec<DestinationStructure>,
    resources: Vec<ProgramResourceStructure>,
    passes: Vec<ProgramPassStructure>,
    output: u32,
    storage: Vec<ProgramStorageStructure>,
    surface_slots: Vec<ProgramSurfaceSlotStructure>,
}

impl ProgramStructureProjection {
    fn from_program(program: &PlanProgramFrame) -> Self {
        Self {
            kind: program.kind,
            viewport_empty: program.viewport.is_empty(),
            texture_kinds: program
                .requirements
                .external_textures
                .iter()
                .map(|texture| texture.kind)
                .collect(),
            font_count: program.requirements.fonts.len(),
            runtime_shader_count: program.requirements.runtime_shaders.len(),
            scene_count: program.requirements.scene3d.len(),
            capabilities: program.requirements.capabilities.clone(),
            destinations: program
                .destination_uses
                .iter()
                .map(DestinationStructure::from)
                .collect(),
            resources: program
                .local_plan
                .resources()
                .iter()
                .map(ProgramResourceStructure::from)
                .collect(),
            passes: program
                .local_plan
                .passes()
                .iter()
                .map(ProgramPassStructure::from)
                .collect(),
            output: program.local_plan.output().get(),
            storage: program
                .local_schedule
                .resources()
                .iter()
                .map(ProgramStorageStructure::from)
                .collect(),
            surface_slots: program
                .local_schedule
                .surface_slots()
                .iter()
                .map(ProgramSurfaceSlotStructure::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct DestinationStructure {
    node: u32,
    scope: valle_draw::program::BackdropScope,
    operation: DestinationOperationStructure,
    bounds_reason: BoundsReason,
}

impl From<&PreparedDestinationUse> for DestinationStructure {
    fn from(value: &PreparedDestinationUse) -> Self {
        let operation = match value.operation {
            valle_draw::requirements::DestinationOperation::Backdrop { sampling, .. } => {
                DestinationOperationStructure::Backdrop { sampling }
            }
            valle_draw::requirements::DestinationOperation::Blend { mode } => {
                DestinationOperationStructure::Blend { mode }
            }
        };
        Self {
            node: value.node.raw(),
            scope: value.scope.clone(),
            operation,
            bounds_reason: value.bounds_reason,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum DestinationOperationStructure {
    Backdrop {
        sampling: valle_draw::requirements::SamplingMode,
    },
    Blend {
        mode: BlendMode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgramResourceStructure {
    id: u32,
    empty: bool,
}

impl From<&super::ProgramResource> for ProgramResourceStructure {
    fn from(value: &super::ProgramResource) -> Self {
        Self {
            id: value.id.get(),
            empty: value.bounds.rect().is_none(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum ProgramPassStructure {
    Clear {
        output: u32,
    },
    RasterNode {
        node: u32,
        output: u32,
    },
    RasterTree {
        roots: Vec<u32>,
        output: u32,
    },
    ReadDestination {
        node: u32,
        operation: super::ProgramDestinationKind,
        scope: valle_draw::program::BackdropScope,
        external: Option<u32>,
        local_inputs: Vec<u32>,
        output: u32,
    },
    Backdrop {
        node: u32,
        input: u32,
        output: u32,
    },
    MotionGlass {
        node: u32,
        input: u32,
        output: u32,
    },
    ApplyMotionGlassForeground {
        node: u32,
        input: u32,
        output: u32,
    },
    SourceOver {
        source: u32,
        destination: u32,
        output: u32,
    },
    ApplyClip {
        node: u32,
        input: u32,
        output: u32,
    },
    ApplyFilter {
        node: u32,
        filter_index: u32,
        input: u32,
        output: u32,
    },
    ApplyMask {
        node: u32,
        input: u32,
        mask: u32,
        output: u32,
        mode: valle_draw::program::MaskMode,
    },
    ApplyOpacity {
        node: u32,
        input: u32,
        output: u32,
    },
    ApplyShader {
        node: u32,
        input: u32,
        output: u32,
    },
    ApplyTransform {
        node: u32,
        input: u32,
        output: u32,
    },
    Blend {
        node: u32,
        source: u32,
        destination: u32,
        output: u32,
    },
}

impl From<&super::ProgramPass> for ProgramPassStructure {
    fn from(value: &super::ProgramPass) -> Self {
        use super::ProgramPassKind as Kind;
        match &value.kind {
            Kind::Clear { output } => Self::Clear {
                output: output.get(),
            },
            Kind::RasterNode { node, output } => Self::RasterNode {
                node: node.raw(),
                output: output.get(),
            },
            Kind::RasterTree { roots, output } => Self::RasterTree {
                roots: roots.iter().map(|node| node.raw()).collect(),
                output: output.get(),
            },
            Kind::ReadDestination {
                node,
                operation,
                scope,
                external,
                local_inputs,
                output,
            } => Self::ReadDestination {
                node: node.raw(),
                operation: *operation,
                scope: scope.clone(),
                external: external.map(|value| value.get()),
                local_inputs: local_inputs.iter().map(|value| value.get()).collect(),
                output: output.get(),
            },
            Kind::Backdrop {
                node,
                input,
                output,
                ..
            } => Self::Backdrop {
                node: node.raw(),
                input: input.get(),
                output: output.get(),
            },
            Kind::MotionGlass {
                node,
                input,
                output,
                ..
            } => Self::MotionGlass {
                node: node.raw(),
                input: input.get(),
                output: output.get(),
            },
            Kind::ApplyMotionGlassForeground {
                node,
                input,
                output,
                ..
            } => Self::ApplyMotionGlassForeground {
                node: node.raw(),
                input: input.get(),
                output: output.get(),
            },
            Kind::SourceOver {
                source,
                destination,
                output,
            } => Self::SourceOver {
                source: source.get(),
                destination: destination.get(),
                output: output.get(),
            },
            Kind::ApplyClip {
                node,
                input,
                output,
                ..
            } => Self::ApplyClip {
                node: node.raw(),
                input: input.get(),
                output: output.get(),
            },
            Kind::ApplyFilter {
                node,
                filter_index,
                input,
                output,
                ..
            } => Self::ApplyFilter {
                node: node.raw(),
                filter_index: *filter_index,
                input: input.get(),
                output: output.get(),
            },
            Kind::ApplyMask {
                node,
                input,
                mask,
                output,
                mode,
            } => Self::ApplyMask {
                node: node.raw(),
                input: input.get(),
                mask: mask.get(),
                output: output.get(),
                mode: *mode,
            },
            Kind::ApplyOpacity {
                node,
                input,
                output,
                ..
            } => Self::ApplyOpacity {
                node: node.raw(),
                input: input.get(),
                output: output.get(),
            },
            Kind::ApplyShader {
                node,
                input,
                output,
                ..
            } => Self::ApplyShader {
                node: node.raw(),
                input: input.get(),
                output: output.get(),
            },
            Kind::ApplyTransform {
                node,
                input,
                output,
                ..
            } => Self::ApplyTransform {
                node: node.raw(),
                input: input.get(),
                output: output.get(),
            },
            Kind::Blend {
                node,
                source,
                destination,
                output,
                ..
            } => Self::Blend {
                node: node.raw(),
                source: source.get(),
                destination: destination.get(),
                output: output.get(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum ProgramStorageStructure {
    Transparent { resource: u32 },
    Destination { resource: u32, destination: u32 },
    Alias { resource: u32, source: u32 },
    Output { resource: u32 },
    Surface { resource: u32, slot: u32 },
}

impl From<&super::ProgramResourceStorage> for ProgramStorageStructure {
    fn from(value: &super::ProgramResourceStorage) -> Self {
        use super::ProgramStorageKind as Kind;
        let resource = value.resource.get();
        match value.storage {
            Kind::Transparent {} => Self::Transparent { resource },
            Kind::Destination { destination } => Self::Destination {
                resource,
                destination: destination.get(),
            },
            Kind::Alias { source } => Self::Alias {
                resource,
                source: source.get(),
            },
            Kind::Output {} => Self::Output { resource },
            Kind::Surface { slot } => Self::Surface {
                resource,
                slot: slot.get(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgramSurfaceSlotStructure {
    id: u32,
    allocations: Vec<ProgramSurfaceAllocationStructure>,
}

impl From<&super::ProgramSurfaceSlot> for ProgramSurfaceSlotStructure {
    fn from(value: &super::ProgramSurfaceSlot) -> Self {
        Self {
            id: value.id.get(),
            allocations: value
                .allocations
                .iter()
                .map(ProgramSurfaceAllocationStructure::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgramSurfaceAllocationStructure {
    resource: u32,
    first: u32,
    last: u32,
    reason: super::ProgramAllocationReason,
}

impl From<&super::ProgramSurfaceAllocation> for ProgramSurfaceAllocationStructure {
    fn from(value: &super::ProgramSurfaceAllocation) -> Self {
        Self {
            resource: value.resource.get(),
            first: value.interval.first.get(),
            last: value.interval.last.get(),
            reason: value.reason,
        }
    }
}

#[cfg(test)]
mod packed_program_bytes_tests {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Probe {
        #[serde(with = "super::packed_program_bytes")]
        packed: Vec<u8>,
    }

    #[test]
    fn nested_program_bytes_are_one_string_not_a_generic_value_per_byte() {
        let probe = Probe {
            packed: vec![0x5a; 1_000_001],
        };
        let value = serde_json::to_value(&probe).unwrap();
        assert!(value["packed"].is_string());
        assert_eq!(serde_json::from_value::<Probe>(value).unwrap(), probe);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanEffect {
    pub semantic_path: String,
    pub space: PreparedEffectSpace,
    pub kernel: PreparedEffectKernel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanSourcePipeline {
    pub chroma_key: Option<PlanEffect>,
}

impl PlanSourcePipeline {
    pub fn is_empty(&self) -> bool {
        self.chroma_key.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceAliasReason {
    DirectSampleableImmutableInput,
    ReusedBackdropResolve,
    NoOpGroup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OptimizationRewriteKind {
    BackdropResolveReuse,
    NoOpGroupElimination,
}

/// One deterministic physical rewrite. Logical pass/resource tables remain intact; these ids make
/// the exact storage owner and the mechanical pass replacement inspectable on every backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizationRewrite {
    pub kind: OptimizationRewriteKind,
    pub pass: ExecutionPassId,
    pub output: PlanResourceId,
    pub source: PlanResourceId,
}

/// Proof carried by the product plan that physical rewrites were deterministic. Hashes normalize
/// surface-slot numbers because liveness coloring happens after rewriting; the final hash can
/// therefore be recomputed from the admitted resource/pass tables on Native and Web.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizationReport {
    pub input_physical_hash: ContentDigest,
    pub output_physical_hash: ContentDigest,
    pub rewrites: Vec<OptimizationRewrite>,
}

impl OptimizationReport {
    /// Construct the proof for a plan whose canonical builder needed no physical rewrite.
    pub fn identity(
        resources: &[PlanResource],
        passes: &[ExecutionPass],
    ) -> Result<Self, PlanValidationError> {
        let hash = super::optimizer::state_hash(resources, passes)
            .map_err(|error| PlanValidationError::Canonical(error.to_string()))?;
        Ok(Self {
            input_physical_hash: hash.clone(),
            output_physical_hash: hash,
            rewrites: Vec::new(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResolveReason {
    NonSampleableRenderTarget,
    ReadWriteHazard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SurfaceAllocationReason {
    WorkingComposite,
    LayerIntermediate,
    BackdropResolve,
    ProgramIntermediate,
    KernelIntermediate,
    OutputStaging,
    LifetimeAlias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CopyReason {
    ReadWriteHazard,
    OutputStaging,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FormatConversionReason {
    OutputContract,
    KernelContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PlanResourceKind {
    External {
        slot: ExternalSlotId,
    },
    Surface {
        slot: SurfaceSlotId,
    },
    Alias {
        source: PlanResourceId,
        reason: ResourceAliasReason,
    },
    OutputTarget {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanResource {
    pub id: PlanResourceId,
    pub semantic_path: String,
    /// One or more immutable RenderGraph resource IDs explained by this physical resource.
    pub logical_resources: Vec<ResourceId>,
    pub kind: PlanResourceKind,
    pub roi: GraphRoi,
    pub origin: GraphOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PassInterval {
    pub first: ExecutionPassId,
    pub last: ExecutionPassId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SurfaceAllocation {
    pub resource: PlanResourceId,
    pub interval: PassInterval,
    pub reason: SurfaceAllocationReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SurfaceSlot {
    pub id: SurfaceSlotId,
    pub texture: LogicalTextureDesc,
    pub allocations: Vec<SurfaceAllocation>,
    pub estimated_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum KernelInvocation {
    Group {
        input: PlanResourceId,
        output: PlanResourceId,
    },
    Filter {
        input: PlanResourceId,
        output: PlanResourceId,
        effect: PlanEffect,
    },
    Mask {
        input: PlanResourceId,
        output: PlanResourceId,
        mask: PreparedMask,
    },
    Transition {
        backdrop: PlanResourceId,
        from: PlanResourceId,
        to: PlanResourceId,
        output: PlanResourceId,
        kernel: PreparedTransitionKernel,
        progress: DynamicBindingId,
        from_opacity: DynamicBindingId,
        to_opacity: DynamicBindingId,
    },
    AdjustmentEffect {
        input: PlanResourceId,
        output: PlanResourceId,
        effect: PlanEffect,
    },
}

impl KernelInvocation {
    fn reads(&self) -> Vec<PlanResourceId> {
        let mut reads = match self {
            Self::Group { input, .. } | Self::Mask { input, .. } => vec![*input],
            Self::Filter { input, .. } | Self::AdjustmentEffect { input, .. } => vec![*input],
            Self::Transition {
                backdrop, from, to, ..
            } => vec![*backdrop, *from, *to],
        };
        reads.sort_unstable();
        reads.dedup();
        reads
    }

    const fn output(&self) -> PlanResourceId {
        match self {
            Self::Group { output, .. }
            | Self::Filter { output, .. }
            | Self::Mask { output, .. }
            | Self::Transition { output, .. }
            | Self::AdjustmentEffect { output, .. } => *output,
        }
    }

    const fn capability(&self) -> GraphCapability {
        match self {
            Self::Group { .. } => GraphCapability::Group,
            Self::Filter { .. } => GraphCapability::Filter,
            Self::Mask { .. } => GraphCapability::Mask,
            Self::Transition { .. } => GraphCapability::Transition,
            Self::AdjustmentEffect { .. } => GraphCapability::AdjustmentEffect,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CompositeMode {
    SourceOver {},
    Blend { mode: BlendMode },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CopyOperation {
    Copy {
        reason: CopyReason,
    },
    FormatConvert {
        source: TextureFormat,
        destination: TextureFormat,
        reason: FormatConversionReason,
    },
    OutputTransform {
        spec: OutputSpec,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ExecutionPassKind {
    ClearRegion {
        output: PlanResourceId,
        working_linear_rec2020_premul: [f32; 4],
    },
    ImportRegion {
        external: PlanResourceId,
        source_pipeline: PlanSourcePipeline,
        output: PlanResourceId,
        placement: ExternalPlacement,
        transform: DynamicBindingId,
        bounds: DynamicBindingId,
    },
    RasterProgram {
        program: ProgramId,
        external_inputs: Vec<PlanResourceId>,
        destination_inputs: Vec<PlanResourceId>,
        output: PlanResourceId,
        transform: DynamicBindingId,
        bounds: DynamicBindingId,
        bounds_reason: BoundsReason,
    },
    RasterCaption {
        program: ProgramId,
        external_inputs: Vec<PlanResourceId>,
        destination: PlanResourceId,
        output: PlanResourceId,
        transform: DynamicBindingId,
        bounds: DynamicBindingId,
        bounds_reason: BoundsReason,
        opacity: DynamicBindingId,
    },
    BindBackdropView {
        input: PlanResourceId,
        output: PlanResourceId,
        reason: ResourceAliasReason,
    },
    AliasResource {
        input: PlanResourceId,
        output: PlanResourceId,
        reason: ResourceAliasReason,
    },
    ResolveRegion {
        input: PlanResourceId,
        output: PlanResourceId,
        sample_bounds: DynamicBindingId,
        output_bounds: DynamicBindingId,
        reason: ResolveReason,
    },
    DispatchKernel {
        invocation: KernelInvocation,
    },
    CompositeRegion {
        backdrop: PlanResourceId,
        layer: PlanResourceId,
        output: PlanResourceId,
        opacity: DynamicBindingId,
        mode: CompositeMode,
    },
    CopyConvert {
        input: PlanResourceId,
        output: PlanResourceId,
        operation: CopyOperation,
    },
}

impl ExecutionPassKind {
    pub fn reads(&self) -> Vec<PlanResourceId> {
        let mut reads = match self {
            Self::ClearRegion { .. } => Vec::new(),
            Self::ImportRegion { external, .. } => vec![*external],
            Self::RasterProgram {
                external_inputs,
                destination_inputs,
                ..
            } => external_inputs
                .iter()
                .chain(destination_inputs)
                .copied()
                .collect(),
            Self::RasterCaption {
                external_inputs,
                destination,
                ..
            } => external_inputs
                .iter()
                .copied()
                .chain(std::iter::once(*destination))
                .collect(),
            Self::BindBackdropView { input, .. }
            | Self::AliasResource { input, .. }
            | Self::ResolveRegion { input, .. }
            | Self::CopyConvert { input, .. } => vec![*input],
            Self::DispatchKernel { invocation } => invocation.reads(),
            Self::CompositeRegion {
                backdrop, layer, ..
            } => vec![*backdrop, *layer],
        };
        reads.sort_unstable();
        reads.dedup();
        reads
    }

    pub const fn output(&self) -> PlanResourceId {
        match self {
            Self::ClearRegion { output, .. }
            | Self::ImportRegion { output, .. }
            | Self::RasterProgram { output, .. }
            | Self::RasterCaption { output, .. }
            | Self::BindBackdropView { output, .. }
            | Self::AliasResource { output, .. }
            | Self::ResolveRegion { output, .. }
            | Self::CompositeRegion { output, .. }
            | Self::CopyConvert { output, .. } => *output,
            Self::DispatchKernel { invocation } => invocation.output(),
        }
    }

    fn capabilities(&self) -> Vec<GraphCapability> {
        match self {
            Self::ClearRegion { .. } => vec![GraphCapability::Clear],
            Self::ImportRegion {
                source_pipeline, ..
            } => {
                let mut capabilities = vec![GraphCapability::ExternalImport];
                if !source_pipeline.is_empty() {
                    capabilities.push(GraphCapability::SourcePipeline);
                }
                capabilities
            }
            Self::RasterProgram { .. } => vec![GraphCapability::DrawProgram],
            Self::RasterCaption { .. } => vec![GraphCapability::Caption],
            Self::BindBackdropView { .. } | Self::ResolveRegion { .. } => {
                vec![GraphCapability::BackdropRead]
            }
            Self::AliasResource { .. } => Vec::new(),
            Self::DispatchKernel { invocation } => vec![invocation.capability()],
            Self::CompositeRegion { .. } => vec![GraphCapability::Blend],
            Self::CopyConvert {
                operation: CopyOperation::OutputTransform { .. },
                ..
            } => vec![GraphCapability::OutputTransform],
            Self::CopyConvert { .. } => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionPass {
    pub id: ExecutionPassId,
    pub semantic_path: String,
    /// Canonical sorted RenderGraph pass IDs explained by this execution step.
    pub logical_passes: Vec<PassId>,
    pub kind: ExecutionPassKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RenderPlanTemplateWire", rename_all = "camelCase")]
pub struct RenderPlanTemplate {
    render_id: RenderId,
    structure_hash: ContentDigest,
    capability_fingerprint: ContentDigest,
    render_spec: RenderSpec,
    logical_pass_count: u32,
    logical_resource_count: u32,
    required_capabilities: Vec<GraphCapability>,
    programs: Vec<PlanProgramLayout>,
    binding_layout: PlanBindingLayout,
    resources: Vec<PlanResource>,
    surface_slots: Vec<SurfaceSlot>,
    passes: Vec<ExecutionPass>,
    optimization: OptimizationReport,
    output: PlanResourceId,
    estimated_peak_surface_bytes: u64,
    /// Construction-time identity of the validated, versioned packed form. The value is derived
    /// rather than serialized, so it cannot make the wire self-referential. Every public
    /// constructor and decoder calls `finalize`; immutable templates never need to repack and
    /// revalidate their full DrawPrograms on the per-frame binding path.
    #[serde(skip)]
    cached_template_hash: ContentDigest,
    #[serde(skip)]
    cached_packed: Arc<[u8]>,
    /// Trusted Native templates use a compact semantic execution identity and deliberately have
    /// no transport packet. Wire templates keep the SHA-256 identity of their exact packed bytes.
    #[serde(skip)]
    constructed: bool,
    /// Exact frame programs produced alongside this template candidate. They never enter the
    /// template packet/hash and are moved into RenderBindings before execution.
    #[serde(skip)]
    frame_programs: Vec<PlanProgram>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenderPlanTemplateWire {
    render_id: RenderId,
    structure_hash: ContentDigest,
    capability_fingerprint: ContentDigest,
    render_spec: RenderSpec,
    logical_pass_count: u32,
    logical_resource_count: u32,
    required_capabilities: Vec<GraphCapability>,
    programs: Vec<PlanProgramLayout>,
    binding_layout: PlanBindingLayout,
    resources: Vec<PlanResource>,
    surface_slots: Vec<SurfaceSlot>,
    passes: Vec<ExecutionPass>,
    optimization: OptimizationReport,
    output: PlanResourceId,
    estimated_peak_surface_bytes: u64,
}

/// Canonical identity of the reusable physical template. Per-frame DrawProgram payloads,
/// dynamic values and executor handles are deliberately absent.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderPlanStructureSeed<'a> {
    render_id: &'a RenderId,
    capability_fingerprint: &'a ContentDigest,
    render_spec: RenderSpec,
    logical_pass_count: u32,
    logical_resource_count: u32,
    required_capabilities: &'a [GraphCapability],
    programs: Vec<PlanProgramStructureSeed<'a>>,
    binding_layout: &'a PlanBindingLayout,
    resources: &'a [PlanResource],
    surface_slots: &'a [SurfaceSlot],
    passes: &'a [ExecutionPass],
    optimization: &'a OptimizationReport,
    output: PlanResourceId,
    estimated_peak_surface_bytes: u64,
}

/// Baseline bytes are physical template payload, but not topology. Excluding them from this seed
/// lets one cached baseline admit every structurally compatible frame while `templateHash` still
/// identifies the exact packet executors cache.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanProgramStructureSeed<'a> {
    id: ProgramId,
    kind: PreparedProgramKind,
    semantic_path: &'a str,
    structure_hash: &'a ContentDigest,
    resources: &'a PlanProgramResources,
    destination_uses: &'a [PreparedDestinationUse],
}

/// Exact same-process identity of a trusted template. `structure_hash` covers every static
/// execution field except the per-program baselines; their two content digests close that gap
/// without serializing the complete transport packet merely to hash it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConstructedTemplateIdentity<'a> {
    structure_hash: &'a ContentDigest,
    programs: Vec<ConstructedTemplateProgramIdentity<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConstructedTemplateProgramIdentity<'a> {
    id: ProgramId,
    baseline_content_hash: &'a ContentDigest,
    baseline_frame_hash: &'a ContentDigest,
}

impl<'a> From<&'a PlanProgramLayout> for PlanProgramStructureSeed<'a> {
    fn from(value: &'a PlanProgramLayout) -> Self {
        Self {
            id: value.id,
            kind: value.kind,
            semantic_path: &value.semantic_path,
            structure_hash: &value.structure_hash,
            resources: &value.resources,
            destination_uses: &value.destination_uses,
        }
    }
}

impl RenderPlanTemplate {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        render_id: RenderId,
        capability_fingerprint: ContentDigest,
        render_spec: RenderSpec,
        logical_pass_count: u32,
        logical_resource_count: u32,
        required_capabilities: Vec<GraphCapability>,
        frame_programs: Vec<PlanProgram>,
        binding_layout: PlanBindingLayout,
        resources: Vec<PlanResource>,
        surface_slots: Vec<SurfaceSlot>,
        passes: Vec<ExecutionPass>,
        optimization: OptimizationReport,
        output: PlanResourceId,
        estimated_peak_surface_bytes: u64,
    ) -> Result<Self, PlanValidationError> {
        Self::new_impl(
            render_id,
            capability_fingerprint,
            render_spec,
            logical_pass_count,
            logical_resource_count,
            required_capabilities,
            frame_programs,
            binding_layout,
            resources,
            surface_slots,
            passes,
            optimization,
            output,
            estimated_peak_surface_bytes,
            true,
        )
    }

    /// Internal lowering has already closed every graph, resource and program invariant while
    /// constructing these values. Keep the same packed template identity, but do not decode and
    /// re-derive its baseline DrawPrograms merely to prove the just-constructed values a second
    /// time. Public constructors and every wire decoder retain full independent validation.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_constructed(
        render_id: RenderId,
        capability_fingerprint: ContentDigest,
        render_spec: RenderSpec,
        logical_pass_count: u32,
        logical_resource_count: u32,
        required_capabilities: Vec<GraphCapability>,
        frame_programs: Vec<PlanProgram>,
        binding_layout: PlanBindingLayout,
        resources: Vec<PlanResource>,
        surface_slots: Vec<SurfaceSlot>,
        passes: Vec<ExecutionPass>,
        optimization: OptimizationReport,
        output: PlanResourceId,
        estimated_peak_surface_bytes: u64,
    ) -> Result<Self, PlanValidationError> {
        Self::new_impl(
            render_id,
            capability_fingerprint,
            render_spec,
            logical_pass_count,
            logical_resource_count,
            required_capabilities,
            frame_programs,
            binding_layout,
            resources,
            surface_slots,
            passes,
            optimization,
            output,
            estimated_peak_surface_bytes,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_impl(
        render_id: RenderId,
        capability_fingerprint: ContentDigest,
        render_spec: RenderSpec,
        logical_pass_count: u32,
        logical_resource_count: u32,
        required_capabilities: Vec<GraphCapability>,
        mut frame_programs: Vec<PlanProgram>,
        binding_layout: PlanBindingLayout,
        resources: Vec<PlanResource>,
        surface_slots: Vec<SurfaceSlot>,
        passes: Vec<ExecutionPass>,
        optimization: OptimizationReport,
        output: PlanResourceId,
        estimated_peak_surface_bytes: u64,
        verify_template: bool,
    ) -> Result<Self, PlanValidationError> {
        let programs = frame_programs
            .iter_mut()
            .map(PlanProgram::take_initial_layout)
            .collect::<Result<Vec<_>, _>>()?;
        let placeholder = ContentDigest::from_bytes([0; 32]);
        let cached_template_hash = placeholder.clone();
        let template = Self {
            render_id,
            structure_hash: placeholder,
            capability_fingerprint,
            render_spec,
            logical_pass_count,
            logical_resource_count,
            required_capabilities,
            programs,
            binding_layout,
            resources,
            surface_slots,
            passes,
            optimization,
            output,
            estimated_peak_surface_bytes,
            cached_template_hash,
            cached_packed: Arc::from([]),
            constructed: false,
            frame_programs,
        };
        template.finalize(verify_template)
    }

    fn finalize(mut self, verify_template: bool) -> Result<Self, PlanValidationError> {
        self.structure_hash = self.compute_structure_hash()?;
        if verify_template {
            self.validate()?;
            let packed = super::packed::encode(
                &self,
                super::packed::Contract::plan(RENDER_PLAN_FORMAT_VERSION),
            )
            .map_err(|error| PlanValidationError::Canonical(error.to_string()))?;
            self.cached_template_hash = ContentDigest::of_bytes(&packed);
            self.cached_packed = packed.into();
        } else {
            self.constructed = true;
            let identity = ConstructedTemplateIdentity {
                structure_hash: &self.structure_hash,
                programs: self
                    .programs
                    .iter()
                    .map(|program| ConstructedTemplateProgramIdentity {
                        id: program.id,
                        baseline_content_hash: &program.baseline_content_hash,
                        baseline_frame_hash: &program.baseline_frame_hash,
                    })
                    .collect(),
            };
            let bytes = canonical::bytes(&identity)
                .map_err(|error| PlanValidationError::Canonical(error.to_string()))?;
            self.cached_template_hash = ContentDigest::of_bytes(&bytes);
        }
        Ok(self)
    }

    fn compute_structure_hash(&self) -> Result<ContentDigest, PlanValidationError> {
        let seed = RenderPlanStructureSeed {
            render_id: &self.render_id,
            capability_fingerprint: &self.capability_fingerprint,
            render_spec: self.render_spec,
            logical_pass_count: self.logical_pass_count,
            logical_resource_count: self.logical_resource_count,
            required_capabilities: &self.required_capabilities,
            programs: self
                .programs
                .iter()
                .map(PlanProgramStructureSeed::from)
                .collect(),
            binding_layout: &self.binding_layout,
            resources: &self.resources,
            surface_slots: &self.surface_slots,
            passes: &self.passes,
            optimization: &self.optimization,
            output: self.output,
            estimated_peak_surface_bytes: self.estimated_peak_surface_bytes,
        };
        let bytes = canonical::bytes(&seed)
            .map_err(|error| PlanValidationError::Canonical(error.to_string()))?;
        Ok(ContentDigest::of_bytes(&bytes))
    }

    pub fn validate(&self) -> Result<(), PlanValidationError> {
        PlanValidator::new(self).validate()
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlanValidationError> {
        self.validate()?;
        canonical::bytes(self).map_err(|error| PlanValidationError::Canonical(error.to_string()))
    }

    /// Versioned binary product ABI. JSON canonical bytes remain an inspector-only form.
    pub fn packed_bytes(&self) -> Result<Vec<u8>, super::PackedPlanError> {
        // Every construction path runs `finalize`, and the template is immutable afterwards.
        // Re-validating and re-serializing here made every Web frame pay for the entire nested
        // DrawProgram packet a second time even when lowering had already produced the exact bytes.
        if self.constructed {
            return Err(super::PackedPlanError::InvalidValue {
                kind: "RenderPlanTemplate",
                reason: "trusted in-process template has no wire packet".into(),
            });
        }
        Ok(self.cached_packed.to_vec())
    }

    pub fn from_packed(bytes: &[u8]) -> Result<Self, super::PackedPlanError> {
        super::packed::decode(
            bytes,
            super::packed::Contract::plan(RENDER_PLAN_FORMAT_VERSION),
        )
    }

    pub fn template_hash(&self) -> Result<ContentDigest, PlanValidationError> {
        Ok(self.cached_template_hash.clone())
    }

    pub fn validate_bindings(&self, bindings: &RenderBindings) -> Result<(), PlanValidationError> {
        if bindings.render_id() != &self.render_id {
            return Err(invalid(
                "bindings.renderId",
                "binding renderId does not match the render plan renderId",
            ));
        }
        self.binding_layout
            .validate_bindings(&self.cached_template_hash, bindings)?;
        if bindings.programs().len() != self.programs.len() {
            return Err(invalid(
                "bindings.programs",
                "program binding count does not match the template layout",
            ));
        }
        for (index, (program, layout)) in bindings.programs().iter().zip(&self.programs).enumerate()
        {
            if program.frame_id() != layout.id || program.admit_layout(layout).is_err() {
                return Err(invalid(
                    format!("bindings.programs[{index}]"),
                    "frame program does not match the template structure",
                ));
            }
        }
        Ok(())
    }

    pub fn packed_size_bytes(&self) -> u64 {
        u64::try_from(self.cached_packed.len()).expect("packed plan is capped below u64::MAX")
    }

    /// Conservative resident-size charge for the bounded plan cache. Trusted templates do not
    /// materialize a packed packet, so charge at least 2 MiB plus twice their exact baseline
    /// payloads; wire templates use their actual packet size.
    pub(crate) fn cache_size_bytes(&self) -> u64 {
        if !self.constructed {
            return self.packed_size_bytes();
        }
        let baselines = self.programs.iter().fold(0_u64, |bytes, program| {
            bytes
                .saturating_add(u64::try_from(program.baseline_packed.len()).unwrap_or(u64::MAX))
                .saturating_add(
                    u64::try_from(program.baseline_frame_packed.len()).unwrap_or(u64::MAX),
                )
        });
        baselines.saturating_mul(2).saturating_add(2 * 1024 * 1024)
    }

    pub const fn render_id(&self) -> &RenderId {
        &self.render_id
    }

    pub const fn structure_hash(&self) -> &ContentDigest {
        &self.structure_hash
    }

    pub const fn capability_fingerprint(&self) -> &ContentDigest {
        &self.capability_fingerprint
    }

    pub const fn render_spec(&self) -> RenderSpec {
        self.render_spec
    }

    pub fn required_capabilities(&self) -> &[GraphCapability] {
        &self.required_capabilities
    }

    pub fn program_layouts(&self) -> &[PlanProgramLayout] {
        &self.programs
    }

    pub fn frame_programs(&self) -> &[PlanProgram] {
        &self.frame_programs
    }

    /// Exact programs attached to this lower result. Product execution moves these into the
    /// per-frame binding packet; decoded template packets intentionally return an empty slice.
    pub fn programs(&self) -> &[PlanProgram] {
        self.frame_programs()
    }

    pub(crate) fn rebase_frame_programs(
        &self,
        programs: Vec<PlanProgram>,
    ) -> Result<Vec<PlanProgram>, PlanValidationError> {
        if programs.len() != self.programs.len() {
            return Err(invalid(
                "programs",
                "frame programs do not match the cached template structure",
            ));
        }
        programs
            .iter()
            .zip(&self.programs)
            .map(|(program, layout)| program.rebase(layout))
            .collect::<Result<Vec<_>, _>>()
    }

    pub(crate) fn attach_frame_programs(
        &self,
        programs: Vec<PlanProgram>,
    ) -> Result<Vec<PlanProgram>, PlanValidationError> {
        if programs.len() != self.programs.len() {
            return Err(invalid(
                "programs",
                "frame programs do not match the cached template structure",
            ));
        }
        programs
            .into_iter()
            .zip(&self.programs)
            .map(|(program, layout)| program.into_constructed(layout))
            .collect::<Result<Vec<_>, _>>()
    }

    /// Product lowering consumes a candidate and physically separates reusable template state
    /// from exact frame programs without cloning either large packet.
    pub(crate) fn detach_frame_programs(mut self) -> (Self, Vec<PlanProgram>) {
        let programs = std::mem::take(&mut self.frame_programs);
        (self, programs)
    }

    pub const fn binding_layout(&self) -> &PlanBindingLayout {
        &self.binding_layout
    }

    pub fn resources(&self) -> &[PlanResource] {
        &self.resources
    }

    pub fn surface_slots(&self) -> &[SurfaceSlot] {
        &self.surface_slots
    }

    pub fn passes(&self) -> &[ExecutionPass] {
        &self.passes
    }

    pub const fn optimization(&self) -> &OptimizationReport {
        &self.optimization
    }

    pub const fn output(&self) -> PlanResourceId {
        self.output
    }

    pub const fn estimated_peak_surface_bytes(&self) -> u64 {
        self.estimated_peak_surface_bytes
    }
}

impl TryFrom<RenderPlanTemplateWire> for RenderPlanTemplate {
    type Error = PlanValidationError;

    fn try_from(value: RenderPlanTemplateWire) -> Result<Self, Self::Error> {
        let declared_structure_hash = value.structure_hash.clone();
        let cached_template_hash = declared_structure_hash.clone();
        let template = Self {
            render_id: value.render_id,
            structure_hash: value.structure_hash,
            capability_fingerprint: value.capability_fingerprint,
            render_spec: value.render_spec,
            logical_pass_count: value.logical_pass_count,
            logical_resource_count: value.logical_resource_count,
            required_capabilities: value.required_capabilities,
            programs: value.programs,
            binding_layout: value.binding_layout,
            resources: value.resources,
            surface_slots: value.surface_slots,
            passes: value.passes,
            optimization: value.optimization,
            output: value.output,
            estimated_peak_surface_bytes: value.estimated_peak_surface_bytes,
            cached_template_hash,
            cached_packed: Arc::from([]),
            constructed: false,
            frame_programs: Vec::new(),
        };
        if template.compute_structure_hash()? != declared_structure_hash {
            return Err(invalid(
                "structureHash",
                "template structure hash does not describe the decoded static contract",
            ));
        }
        template.finalize(true)
    }
}

struct PlanValidator<'a> {
    template: &'a RenderPlanTemplate,
    external_resources: BTreeMap<ExternalSlotId, PlanResourceId>,
    writers: BTreeMap<PlanResourceId, ExecutionPassId>,
}

impl<'a> PlanValidator<'a> {
    fn new(template: &'a RenderPlanTemplate) -> Self {
        Self {
            template,
            external_resources: BTreeMap::new(),
            writers: BTreeMap::new(),
        }
    }

    fn validate(mut self) -> Result<(), PlanValidationError> {
        if self.template.logical_pass_count == 0 || self.template.logical_resource_count == 0 {
            return Err(invalid(
                "logicalCounts",
                "render plan must explain at least one logical pass and resource",
            ));
        }
        self.template.binding_layout.validate()?;
        self.validate_resources()?;
        self.validate_programs()?;
        self.validate_passes()?;
        self.validate_optimization()?;
        self.validate_surface_slots()?;
        self.validate_reachability()?;
        Ok(())
    }

    fn validate_optimization(&self) -> Result<(), PlanValidationError> {
        let report = &self.template.optimization;
        let actual = super::optimizer::state_hash(&self.template.resources, &self.template.passes)
            .map_err(|error| invalid("optimization.outputPhysicalHash", error))?;
        if actual != report.output_physical_hash {
            return Err(invalid(
                "optimization.outputPhysicalHash",
                "hash does not describe the final normalized physical state",
            ));
        }
        if report.rewrites.is_empty() {
            if report.input_physical_hash != report.output_physical_hash {
                return Err(invalid(
                    "optimization.inputPhysicalHash",
                    "an identity optimization must preserve its physical hash",
                ));
            }
        } else if report.input_physical_hash == report.output_physical_hash {
            return Err(invalid(
                "optimization.inputPhysicalHash",
                "a non-empty rewrite set must change the normalized physical state",
            ));
        }

        let mut previous_pass = None;
        let mut outputs = BTreeSet::new();
        let mut rewrite_passes = BTreeSet::new();
        for (index, rewrite) in report.rewrites.iter().enumerate() {
            let path = format!("optimization.rewrites[{index}]");
            if previous_pass.is_some_and(|previous| rewrite.pass <= previous) {
                return Err(invalid(
                    &path,
                    "rewrites must be ordered by unique execution pass",
                ));
            }
            previous_pass = Some(rewrite.pass);
            rewrite_passes.insert(rewrite.pass);
            if !outputs.insert(rewrite.output) || rewrite.source.index() >= rewrite.output.index() {
                return Err(invalid(
                    &path,
                    "rewrite source/output identity is not canonical",
                ));
            }
            let resource = self
                .template
                .resources
                .get(rewrite.output.index())
                .filter(|resource| resource.id == rewrite.output)
                .ok_or_else(|| invalid(&path, "rewrite output resource is undefined"))?;
            let pass = self
                .template
                .passes
                .get(rewrite.pass.index())
                .filter(|pass| pass.id == rewrite.pass)
                .ok_or_else(|| invalid(&path, "rewrite execution pass is undefined"))?;
            let valid = match rewrite.kind {
                OptimizationRewriteKind::BackdropResolveReuse => {
                    matches!(
                        resource.kind,
                        PlanResourceKind::Alias {
                            source,
                            reason: ResourceAliasReason::ReusedBackdropResolve,
                        } if source == rewrite.source
                    ) && matches!(
                        pass.kind,
                        ExecutionPassKind::BindBackdropView {
                            input,
                            output,
                            reason: ResourceAliasReason::ReusedBackdropResolve,
                        } if input == rewrite.source && output == rewrite.output
                    )
                }
                OptimizationRewriteKind::NoOpGroupElimination => {
                    matches!(
                        resource.kind,
                        PlanResourceKind::Alias {
                            source,
                            reason: ResourceAliasReason::NoOpGroup,
                        } if source == rewrite.source
                    ) && matches!(
                        pass.kind,
                        ExecutionPassKind::AliasResource {
                            input,
                            output,
                            reason: ResourceAliasReason::NoOpGroup,
                        } if input == rewrite.source && output == rewrite.output
                    )
                }
            };
            if !valid {
                return Err(invalid(
                    &path,
                    "rewrite proof does not match the final plan",
                ));
            }
        }

        for resource in &self.template.resources {
            if matches!(
                resource.kind,
                PlanResourceKind::Alias {
                    reason: ResourceAliasReason::ReusedBackdropResolve
                        | ResourceAliasReason::NoOpGroup,
                    ..
                }
            ) && !outputs.contains(&resource.id)
            {
                return Err(invalid(
                    "optimization.rewrites",
                    "an optimizer-owned resource alias is missing from the rewrite proof",
                ));
            }
        }
        for pass in &self.template.passes {
            if matches!(
                pass.kind,
                ExecutionPassKind::BindBackdropView {
                    reason: ResourceAliasReason::ReusedBackdropResolve,
                    ..
                } | ExecutionPassKind::AliasResource {
                    reason: ResourceAliasReason::NoOpGroup,
                    ..
                }
            ) && !rewrite_passes.contains(&pass.id)
            {
                return Err(invalid(
                    "optimization.rewrites",
                    "an optimizer-owned execution alias is missing from the rewrite proof",
                ));
            }
        }
        Ok(())
    }

    fn validate_resources(&mut self) -> Result<(), PlanValidationError> {
        let mut logical = BTreeSet::new();
        let mut output_count = 0_usize;
        for (index, resource) in self.template.resources.iter().enumerate() {
            if resource.id != PlanResourceId::from_index(index)? {
                return Err(invalid(
                    "resources",
                    "resource ids must be contiguous and ordered",
                ));
            }
            if resource.semantic_path.is_empty() {
                return Err(invalid(
                    format!("resources[{index}].semanticPath"),
                    "semantic path must not be empty",
                ));
            }
            validate_logical_ids(
                &resource.logical_resources,
                self.template.logical_resource_count,
                &format!("resources[{index}].logicalResources"),
            )?;
            logical.extend(resource.logical_resources.iter().map(|id| id.get()));
            self.validate_resource_dynamic(resource, index)?;
            match resource.kind {
                PlanResourceKind::External { slot } => {
                    if self
                        .template
                        .binding_layout
                        .external_slots()
                        .get(slot.index())
                        .is_none_or(|value| value.id != slot)
                    {
                        return Err(invalid(
                            format!("resources[{index}].kind.slot"),
                            "external slot is undefined",
                        ));
                    }
                    if self.external_resources.insert(slot, resource.id).is_some() {
                        return Err(invalid(
                            format!("resources[{index}].kind.slot"),
                            "external slot has more than one plan resource",
                        ));
                    }
                }
                PlanResourceKind::Surface { slot } => {
                    if self
                        .template
                        .surface_slots
                        .get(slot.index())
                        .is_none_or(|value| value.id != slot)
                    {
                        return Err(invalid(
                            format!("resources[{index}].kind.slot"),
                            "surface slot is undefined",
                        ));
                    }
                }
                PlanResourceKind::Alias { source, .. } => {
                    if source.index() >= index
                        || !matches!(
                            self.template.resources[source.index()].kind,
                            PlanResourceKind::Surface { .. }
                        )
                    {
                        return Err(invalid(
                            format!("resources[{index}].kind.source"),
                            "resource alias must reference an earlier physical surface",
                        ));
                    }
                }
                PlanResourceKind::OutputTarget {} => output_count += 1,
            }
        }
        if !covers_one_based(&logical, self.template.logical_resource_count) {
            return Err(invalid(
                "resources.logicalResources",
                "plan resources do not cover every RenderGraph resource",
            ));
        }
        if output_count != 1
            || self
                .template
                .resources
                .get(self.template.output.index())
                .is_none_or(|resource| !matches!(resource.kind, PlanResourceKind::OutputTarget {}))
        {
            return Err(invalid(
                "output",
                "plan must identify exactly one output target",
            ));
        }
        let expected_external: BTreeSet<_> = self
            .template
            .binding_layout
            .external_slots()
            .iter()
            .map(|slot| slot.id)
            .collect();
        if self
            .external_resources
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
            != expected_external
        {
            return Err(invalid(
                "resources",
                "external resources are not the exact binding-layout projection",
            ));
        }
        Ok(())
    }

    fn validate_resource_dynamic(
        &self,
        resource: &PlanResource,
        index: usize,
    ) -> Result<(), PlanValidationError> {
        if let GraphRoi::Dynamic { binding } = resource.roi {
            self.require_dynamic_any(
                binding,
                &[
                    DynamicBindingKind::Bounds,
                    DynamicBindingKind::BackdropSampleBounds,
                    DynamicBindingKind::BackdropOutputBounds,
                ],
                &format!("resources[{index}].roi"),
            )?;
        }
        if let GraphOrigin::Dynamic { bounds } = resource.origin {
            self.require_dynamic_any(
                bounds,
                &[
                    DynamicBindingKind::Bounds,
                    DynamicBindingKind::BackdropSampleBounds,
                    DynamicBindingKind::BackdropOutputBounds,
                ],
                &format!("resources[{index}].origin"),
            )?;
        }
        Ok(())
    }

    fn validate_programs(&self) -> Result<(), PlanValidationError> {
        for (index, program) in self.template.programs.iter().enumerate() {
            let expected = u32::try_from(index + 1)
                .map_err(|_| invalid("programs", "program id budget exceeded"))?;
            if program.id.get() != expected || program.semantic_path.is_empty() {
                return Err(invalid(
                    format!("programs[{index}]"),
                    "program ids must be contiguous and paths non-empty",
                ));
            }
            program
                .validate_baseline()
                .map_err(|error| invalid(format!("programs[{index}]"), error))?;
        }
        Ok(())
    }

    fn validate_passes(&mut self) -> Result<(), PlanValidationError> {
        let mut produced: BTreeSet<_> = self.external_resources.values().copied().collect();
        let mut logical = BTreeSet::new();
        let mut referenced_programs = BTreeSet::new();
        let mut capabilities = BTreeSet::new();
        for (index, pass) in self.template.passes.iter().enumerate() {
            if pass.id != ExecutionPassId::from_index(index)? || pass.semantic_path.is_empty() {
                return Err(invalid(
                    format!("passes[{index}]"),
                    "pass ids must be contiguous and paths non-empty",
                ));
            }
            validate_logical_ids(
                &pass.logical_passes,
                self.template.logical_pass_count,
                &format!("passes[{index}].logicalPasses"),
            )?;
            logical.extend(pass.logical_passes.iter().map(|id| id.get()));
            let output = pass.kind.output();
            self.resource(output, &format!("passes[{index}].output"))?;
            if matches!(
                self.template.resources[output.index()].kind,
                PlanResourceKind::External { .. }
            ) || self.writers.insert(output, pass.id).is_some()
            {
                return Err(invalid(
                    format!("passes[{index}].output"),
                    "pass output must be a uniquely written non-external resource",
                ));
            }
            for input in pass.kind.reads() {
                self.resource(input, &format!("passes[{index}].inputs"))?;
                if input == output || !produced.contains(&input) {
                    return Err(invalid(
                        format!("passes[{index}].inputs"),
                        "pass reads an unavailable or simultaneously written resource",
                    ));
                }
            }
            produced.insert(output);
            capabilities.extend(pass.kind.capabilities());
            self.validate_pass_payload(pass, index, &mut referenced_programs)?;
        }
        if !covers_one_based(&logical, self.template.logical_pass_count) {
            return Err(invalid(
                "passes.logicalPasses",
                "execution passes do not cover every RenderGraph pass",
            ));
        }
        if referenced_programs
            != self
                .template
                .programs
                .iter()
                .map(|program| program.id.get())
                .collect()
        {
            return Err(invalid(
                "programs",
                "program table contains an unreferenced entry",
            ));
        }
        let required: BTreeSet<_> = self
            .template
            .required_capabilities
            .iter()
            .copied()
            .collect();
        if self.template.required_capabilities.is_empty()
            || !strictly_sorted(&self.template.required_capabilities)
            || !capabilities.is_subset(&required)
        {
            return Err(invalid(
                "requiredCapabilities",
                "capabilities must be canonical and cover every physical pass requirement",
            ));
        }
        if self
            .template
            .passes
            .last()
            .is_none_or(|pass| pass.kind.output() != self.template.output)
            || self.writers.len() + self.external_resources.len() != self.template.resources.len()
        {
            return Err(invalid(
                "passes",
                "every non-external resource must have one writer and output must be terminal",
            ));
        }
        Ok(())
    }

    fn validate_pass_payload(
        &self,
        pass: &ExecutionPass,
        index: usize,
        referenced_programs: &mut BTreeSet<u32>,
    ) -> Result<(), PlanValidationError> {
        let path = format!("passes[{index}]");
        match &pass.kind {
            ExecutionPassKind::ClearRegion {
                working_linear_rec2020_premul,
                ..
            } => {
                let clear = crate::compositor::reference::PremulRgba32::from_premultiplied(
                    *working_linear_rec2020_premul,
                )
                .map_err(|error| invalid(&path, error))?;
                let expected = crate::compositor::reference::output_root_pixel(
                    self.template.render_spec.output(),
                )
                .map_err(|error| invalid(&path, error))?;
                if clear != expected {
                    return Err(invalid(
                        path,
                        "clear color must exactly mirror the RenderSpec background",
                    ));
                }
            }
            ExecutionPassKind::ImportRegion {
                external,
                source_pipeline,
                placement,
                transform,
                bounds,
                ..
            } => {
                if !matches!(
                    self.template.resources[external.index()].kind,
                    PlanResourceKind::External { .. }
                ) {
                    return Err(invalid(path, "import input must be an external resource"));
                }
                placement
                    .validate()
                    .map_err(|error| invalid(&path, error))?;
                let owner = pass
                    .semantic_path
                    .strip_suffix(".import")
                    .ok_or_else(|| invalid(&path, "Import path must end in .import"))?;
                if let Some(PreparedExternalBackdrop::Blur {
                    sigma_device_px, ..
                }) = placement.backdrop
                {
                    let slot = self.require_dynamic(
                        sigma_device_px,
                        DynamicBindingKind::DeviceLength,
                        &path,
                    )?;
                    if slot.semantic_path
                        != format!("{owner}.sourcePlacement.backdrop.sigmaDevicePx")
                    {
                        return Err(invalid(
                            &path,
                            "external blur backdrop dynamic slot has the wrong semantic owner",
                        ));
                    }
                }
                let expected_space = PreparedEffectSpace::Layer {
                    transform: *transform,
                    bounds: *bounds,
                };
                if let Some(effect) = &source_pipeline.chroma_key {
                    let semantic_path = format!("{owner}.effects[0]");
                    self.validate_effect(effect, false, true, &path, &semantic_path)?;
                    if effect.space != expected_space {
                        return Err(invalid(
                            &path,
                            "source effect coordinate space does not match its Import",
                        ));
                    }
                }
                self.require_dynamic(*transform, DynamicBindingKind::DeviceTransform, &path)?;
                self.require_dynamic(*bounds, DynamicBindingKind::Bounds, &path)?;
            }
            ExecutionPassKind::RasterProgram {
                program,
                external_inputs,
                destination_inputs,
                output,
                transform,
                bounds,
                bounds_reason,
                ..
            } => {
                let prepared = self.program(*program, &path)?;
                let owner = pass
                    .semantic_path
                    .strip_suffix(".draw")
                    .ok_or_else(|| invalid(&path, "RasterProgram path must end in .draw"))?;
                let program_suffix = match prepared.kind {
                    PreparedProgramKind::Motion => "motion",
                    PreparedProgramKind::Solid => "solid",
                    PreparedProgramKind::Caption => "invalid",
                };
                if !matches!(
                    prepared.kind,
                    PreparedProgramKind::Motion | PreparedProgramKind::Solid
                ) || prepared.semantic_path != format!("{owner}.{program_suffix}")
                    || self.resource(*output, &path)?.semantic_path != format!("{owner}.program")
                    || *external_inputs != self.program_external_inputs(prepared, &path)?
                    || destination_inputs.len() != prepared.destination_uses.len()
                    || prepared
                        .destination_uses
                        .iter()
                        .any(|destination| destination.bounds_reason != *bounds_reason)
                {
                    return Err(invalid(
                        path,
                        "raster program binding layout is inconsistent",
                    ));
                }
                self.validate_program_destinations(prepared, destination_inputs, owner, &path)?;
                referenced_programs.insert(program.get());
                self.require_dynamic(*transform, DynamicBindingKind::DeviceTransform, &path)?;
                self.require_dynamic(*bounds, DynamicBindingKind::Bounds, &path)?;
            }
            ExecutionPassKind::RasterCaption {
                program,
                external_inputs,
                destination,
                transform,
                bounds,
                opacity,
                ..
            } => {
                let prepared = self.program(*program, &path)?;
                if prepared.kind != PreparedProgramKind::Caption
                    || !prepared.destination_uses.is_empty()
                    || !matches!(
                        self.template.resources[destination.index()].kind,
                        PlanResourceKind::Surface { .. }
                    )
                    || *external_inputs != self.program_external_inputs(prepared, &path)?
                {
                    return Err(invalid(
                        path,
                        "caption raster binding layout is inconsistent",
                    ));
                }
                referenced_programs.insert(program.get());
                self.require_dynamic(*transform, DynamicBindingKind::DeviceTransform, &path)?;
                self.require_dynamic(*bounds, DynamicBindingKind::Bounds, &path)?;
                self.require_dynamic(*opacity, DynamicBindingKind::Opacity, &path)?;
            }
            ExecutionPassKind::BindBackdropView {
                input,
                output,
                reason,
            } => {
                let alias_matches = matches!(
                    self.template.resources[output.index()].kind,
                    PlanResourceKind::Alias {
                        source,
                        reason: actual,
                    } if source == *input && actual == *reason
                );
                let reason_matches = match reason {
                    ResourceAliasReason::DirectSampleableImmutableInput => true,
                    ResourceAliasReason::ReusedBackdropResolve => {
                        let source = self.resource(*input, &path)?;
                        let output = self.resource(*output, &path)?;
                        source.roi == output.roi
                            && source.origin == output.origin
                            && self
                                .writers
                                .get(input)
                                .and_then(|id| self.template.passes.get(id.index()))
                                .is_some_and(|writer| {
                                    matches!(
                                        writer.kind,
                                        ExecutionPassKind::ResolveRegion {
                                            output: resolved,
                                            ..
                                        } if resolved == *input
                                    )
                                })
                    }
                    ResourceAliasReason::NoOpGroup => false,
                };
                if !alias_matches || !reason_matches {
                    return Err(invalid(path, "backdrop view alias is inconsistent"));
                }
            }
            ExecutionPassKind::AliasResource {
                input,
                output,
                reason,
            } => {
                if *reason != ResourceAliasReason::NoOpGroup
                    || !matches!(
                        self.template.resources[output.index()].kind,
                        PlanResourceKind::Alias {
                            source,
                            reason: actual,
                        } if source == *input && actual == *reason
                    )
                {
                    return Err(invalid(path, "optimized resource alias is inconsistent"));
                }
            }
            ExecutionPassKind::ResolveRegion {
                output,
                sample_bounds,
                output_bounds,
                ..
            } => {
                if !matches!(
                    self.template.resources[output.index()].kind,
                    PlanResourceKind::Surface { .. }
                ) {
                    return Err(invalid(path, "resolved backdrop must write a surface"));
                }
                self.require_dynamic(
                    *sample_bounds,
                    DynamicBindingKind::BackdropSampleBounds,
                    &path,
                )?;
                self.require_dynamic(
                    *output_bounds,
                    DynamicBindingKind::BackdropOutputBounds,
                    &path,
                )?;
            }
            ExecutionPassKind::DispatchKernel { invocation } => {
                self.validate_kernel(invocation, &path, &pass.semantic_path)?;
            }
            ExecutionPassKind::CompositeRegion { opacity, .. } => {
                self.require_dynamic(*opacity, DynamicBindingKind::Opacity, &path)?;
            }
            ExecutionPassKind::CopyConvert { operation, .. } => {
                if let CopyOperation::FormatConvert {
                    source,
                    destination,
                    ..
                } = operation
                    && source == destination
                {
                    return Err(invalid(path, "format conversion must change format"));
                }
                if let CopyOperation::OutputTransform { spec } = operation
                    && *spec != self.template.render_spec.output()
                {
                    return Err(invalid(path, "output operation must mirror RenderSpec"));
                }
            }
        }
        Ok(())
    }

    fn validate_kernel(
        &self,
        invocation: &KernelInvocation,
        path: &str,
        semantic_path: &str,
    ) -> Result<(), PlanValidationError> {
        match invocation {
            KernelInvocation::Filter { effect, .. } => {
                self.validate_effect(effect, false, false, path, semantic_path)?
            }
            KernelInvocation::AdjustmentEffect { effect, .. } => {
                self.validate_effect(effect, true, false, path, semantic_path)?
            }
            KernelInvocation::Mask { mask, .. } => {
                mask.validate_wire().map_err(|error| invalid(path, error))?;
            }
            KernelInvocation::Transition {
                kernel,
                progress,
                from_opacity,
                to_opacity,
                ..
            } => {
                kernel
                    .validate_wire()
                    .map_err(|error| invalid(path, error))?;
                self.require_dynamic(*progress, DynamicBindingKind::TransitionProgress, path)?;
                self.require_dynamic(*from_opacity, DynamicBindingKind::Opacity, path)?;
                self.require_dynamic(*to_opacity, DynamicBindingKind::Opacity, path)?;
            }
            KernelInvocation::Group { .. } => {}
        }
        Ok(())
    }

    fn validate_effect(
        &self,
        effect: &PlanEffect,
        root_space: bool,
        source_pipeline: bool,
        path: &str,
        semantic_path: &str,
    ) -> Result<(), PlanValidationError> {
        effect
            .kernel
            .validate_wire()
            .map_err(|error| invalid(path, error))?;
        if source_pipeline && !matches!(effect.kernel, PreparedEffectKernel::ChromaKey { .. }) {
            return Err(invalid(
                path,
                "source pipeline admits only ChromaKey effect kernels",
            ));
        }
        if !source_pipeline && effect.kernel.is_source_operator() {
            return Err(invalid(
                path,
                "source-alpha operators cannot execute as Filter or AdjustmentEffect kernels",
            ));
        }
        if effect.semantic_path != semantic_path {
            return Err(invalid(
                path,
                "effect semantic identity does not match its execution pass",
            ));
        }
        match (root_space, effect.space) {
            (true, PreparedEffectSpace::Root) => {}
            (false, PreparedEffectSpace::Layer { transform, bounds }) => {
                let marker = effect.semantic_path.rfind(".effects[").ok_or_else(|| {
                    invalid(path, "layer effect semantic path has no canonical owner")
                })?;
                let owner = &effect.semantic_path[..marker];
                let transform_slot =
                    self.require_dynamic(transform, DynamicBindingKind::DeviceTransform, path)?;
                let bounds_slot = self.require_dynamic(bounds, DynamicBindingKind::Bounds, path)?;
                if transform_slot.semantic_path != format!("{owner}.transform")
                    || bounds_slot.semantic_path != format!("{owner}.bounds")
                {
                    return Err(invalid(
                        path,
                        "layer effect coordinate slots do not belong to its semantic owner",
                    ));
                }
            }
            _ => {
                return Err(invalid(
                    path,
                    "effect coordinate space does not match its execution band",
                ));
            }
        }
        for (id, suffix) in effect.kernel.device_lengths() {
            let slot = self.require_dynamic(id, DynamicBindingKind::DeviceLength, path)?;
            if slot.semantic_path != format!("{}.{}", effect.semantic_path, suffix) {
                return Err(invalid(
                    path,
                    "effect device-length slot does not belong to its typed parameter",
                ));
            }
        }
        Ok(())
    }

    fn validate_surface_slots(&self) -> Result<(), PlanValidationError> {
        let mut allocations = BTreeMap::new();
        for (index, slot) in self.template.surface_slots.iter().enumerate() {
            if slot.id != SurfaceSlotId::from_index(index)? {
                return Err(invalid(
                    "surfaceSlots",
                    "surface slot ids must be contiguous and ordered",
                ));
            }
            let expected_bytes = texture_bytes(&slot.texture)?;
            if slot.estimated_bytes != expected_bytes || slot.allocations.is_empty() {
                return Err(invalid(
                    format!("surfaceSlots[{index}]"),
                    "surface byte estimate or allocation table is invalid",
                ));
            }
            let mut previous = None;
            for allocation in &slot.allocations {
                let resource = self.resource(allocation.resource, "surfaceSlots.allocations")?;
                if !matches!(resource.kind, PlanResourceKind::Surface { slot: actual } if actual == slot.id)
                    || allocation.interval.first.get() > allocation.interval.last.get()
                    || allocation.interval.last.index() >= self.template.passes.len()
                    || previous.is_some_and(|last: ExecutionPassId| {
                        last.get() >= allocation.interval.first.get()
                    })
                    || allocations
                        .insert(allocation.resource, allocation.interval)
                        .is_some()
                {
                    return Err(invalid(
                        format!("surfaceSlots[{index}].allocations"),
                        "surface allocations must be exact, ordered and non-overlapping",
                    ));
                }
                previous = Some(allocation.interval.last);
            }
        }
        let surface_resources: BTreeSet<_> = self
            .template
            .resources
            .iter()
            .filter_map(|resource| {
                matches!(resource.kind, PlanResourceKind::Surface { .. }).then_some(resource.id)
            })
            .collect();
        if allocations.keys().copied().collect::<BTreeSet<_>>() != surface_resources {
            return Err(invalid(
                "surfaceSlots.allocations",
                "surface allocations are not the exact surface-resource projection",
            ));
        }
        for resource in surface_resources {
            let actual =
                resource_interval(resource, &self.template.resources, &self.template.passes)?;
            if allocations.get(&resource) != Some(&actual) {
                return Err(invalid(
                    "surfaceSlots.allocations.interval",
                    "declared liveness does not equal actual pass use",
                ));
            }
        }
        let peak = peak_surface_bytes(&self.template.surface_slots, self.template.passes.len())?;
        if peak != self.template.estimated_peak_surface_bytes {
            return Err(invalid(
                "estimatedPeakSurfaceBytes",
                "peak estimate does not equal declared liveness",
            ));
        }
        Ok(())
    }

    fn validate_reachability(&self) -> Result<(), PlanValidationError> {
        let pass_by_id: BTreeMap<_, _> = self
            .template
            .passes
            .iter()
            .map(|pass| (pass.id, pass))
            .collect();
        let mut resources = BTreeSet::new();
        let mut passes = BTreeSet::new();
        let mut stack = vec![self.template.output];
        while let Some(resource) = stack.pop() {
            if !resources.insert(resource) {
                continue;
            }
            if let Some(writer) = self.writers.get(&resource)
                && passes.insert(*writer)
            {
                stack.extend(pass_by_id[writer].kind.reads());
            }
        }
        if resources.len() != self.template.resources.len()
            || passes.len() != self.template.passes.len()
        {
            return Err(invalid(
                "output",
                "all plan passes and resources must reach the terminal output",
            ));
        }
        Ok(())
    }

    fn program(
        &self,
        id: ProgramId,
        path: &str,
    ) -> Result<&PlanProgramLayout, PlanValidationError> {
        self.template
            .programs
            .get(id.get() as usize - 1)
            .filter(|program| program.id == id)
            .ok_or_else(|| invalid(path, format!("undefined program {}", id.get())))
    }

    fn resource(
        &self,
        id: PlanResourceId,
        path: &str,
    ) -> Result<&PlanResource, PlanValidationError> {
        self.template
            .resources
            .get(id.index())
            .filter(|resource| resource.id == id)
            .ok_or_else(|| invalid(path, format!("undefined plan resource {}", id.get())))
    }

    fn external_slot(
        &self,
        id: ExternalSlotId,
        path: &str,
    ) -> Result<&super::ExternalSlot, PlanValidationError> {
        self.template
            .binding_layout
            .external_slots()
            .get(id.index())
            .filter(|slot| slot.id == id)
            .ok_or_else(|| invalid(path, format!("undefined external slot {}", id.get())))
    }

    fn program_external_inputs(
        &self,
        program: &PlanProgramLayout,
        path: &str,
    ) -> Result<Vec<PlanResourceId>, PlanValidationError> {
        let slots = program
            .resources
            .textures
            .iter()
            .map(|value| value.slot)
            .chain(program.resources.fonts.iter().map(|value| value.slot))
            .chain(
                program
                    .resources
                    .runtime_shaders
                    .iter()
                    .map(|value| value.slot),
            )
            .chain(program.resources.scenes.iter().map(|value| value.slot));
        self.external_inputs_for_slots(slots, path)
    }

    fn validate_program_destinations(
        &self,
        program: &PlanProgramLayout,
        inputs: &[PlanResourceId],
        owner: &str,
        path: &str,
    ) -> Result<(), PlanValidationError> {
        for (index, (input, binding)) in inputs.iter().zip(&program.destination_uses).enumerate() {
            let expected_path = format!("{owner}.destination[{index}]");
            let resource = self.resource(*input, path)?;
            if resource.semantic_path != expected_path
                || resource.roi
                    != (GraphRoi::Dynamic {
                        binding: binding.sample_bounds,
                    })
                || resource.origin
                    != (GraphOrigin::Dynamic {
                        bounds: binding.sample_bounds,
                    })
            {
                return Err(invalid(
                    path,
                    "raster destination resource does not match its program slot",
                ));
            }
            let writer = self
                .writers
                .get(input)
                .and_then(|id| self.template.passes.get(id.index()))
                .ok_or_else(|| invalid(path, "raster destination has no execution writer"))?;
            let writer_matches = match &writer.kind {
                ExecutionPassKind::BindBackdropView {
                    input: source,
                    output,
                    reason,
                } => {
                    *output == *input
                        && match reason {
                            ResourceAliasReason::DirectSampleableImmutableInput => true,
                            ResourceAliasReason::ReusedBackdropResolve => self
                                .writers
                                .get(source)
                                .and_then(|id| self.template.passes.get(id.index()))
                                .is_some_and(|source_writer| {
                                    matches!(
                                        source_writer.kind,
                                        ExecutionPassKind::ResolveRegion {
                                            output: resolved,
                                            sample_bounds,
                                            output_bounds,
                                            ..
                                        } if resolved == *source
                                            && sample_bounds == binding.sample_bounds
                                            && output_bounds == binding.output_bounds
                                    )
                                }),
                            ResourceAliasReason::NoOpGroup => false,
                        }
                }
                ExecutionPassKind::ResolveRegion {
                    output,
                    sample_bounds,
                    output_bounds,
                    ..
                } => {
                    *output == *input
                        && *sample_bounds == binding.sample_bounds
                        && *output_bounds == binding.output_bounds
                }
                _ => false,
            };
            if writer.semantic_path != expected_path || !writer_matches {
                return Err(invalid(
                    path,
                    "raster destination writer does not match its program slot",
                ));
            }
        }
        Ok(())
    }

    fn external_inputs_for_slots(
        &self,
        slots: impl IntoIterator<Item = ExternalSlotId>,
        path: &str,
    ) -> Result<Vec<PlanResourceId>, PlanValidationError> {
        let mut result = Vec::new();
        for slot in slots {
            self.external_slot(slot, path)?;
            result.push(
                self.external_resources
                    .get(&slot)
                    .copied()
                    .ok_or_else(|| invalid(path, "external slot has no plan resource"))?,
            );
        }
        result.sort_unstable();
        result.dedup();
        Ok(result)
    }

    fn require_dynamic(
        &self,
        id: DynamicBindingId,
        expected: DynamicBindingKind,
        path: &str,
    ) -> Result<&super::DynamicSlot, PlanValidationError> {
        self.require_dynamic_any(id, &[expected], path)
    }

    fn require_dynamic_any(
        &self,
        id: DynamicBindingId,
        expected: &[DynamicBindingKind],
        path: &str,
    ) -> Result<&super::DynamicSlot, PlanValidationError> {
        let slot = self
            .template
            .binding_layout
            .dynamic_slots()
            .get(id.get() as usize - 1)
            .filter(|slot| slot.id == id)
            .ok_or_else(|| invalid(path, format!("undefined dynamic slot {}", id.get())))?;
        if !expected.contains(&slot.binding_kind) {
            return Err(invalid(path, "dynamic slot has the wrong binding kind"));
        }
        Ok(slot)
    }
}

fn validate_logical_ids<T>(values: &[T], upper: u32, path: &str) -> Result<(), PlanValidationError>
where
    T: Copy + Ord + Into<u32>,
{
    if values.is_empty() || !strictly_sorted(values) {
        return Err(invalid(
            path,
            "logical ids must be non-empty, sorted and unique",
        ));
    }
    if values.iter().copied().map(Into::into).any(|id| id > upper) {
        return Err(invalid(path, "logical id exceeds declared graph count"));
    }
    Ok(())
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn covers_one_based(values: &BTreeSet<u32>, count: u32) -> bool {
    usize::try_from(count).ok() == Some(values.len())
        && values
            .iter()
            .copied()
            .zip(1..=count)
            .all(|(actual, expected)| actual == expected)
}

pub(super) fn texture_bytes(texture: &LogicalTextureDesc) -> Result<u64, PlanValidationError> {
    let bytes_per_pixel = match texture.format {
        TextureFormat::Rgba16Float => 8_u64,
        TextureFormat::Rgba32Float => 16_u64,
    };
    u64::from(texture.extent.width())
        .checked_mul(u64::from(texture.extent.height()))
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .and_then(|bytes| bytes.checked_mul(u64::from(texture.sample_count())))
        .ok_or_else(|| invalid("surfaceSlots.texture", "surface byte estimate overflow"))
}

pub(super) fn resource_interval(
    resource: PlanResourceId,
    resources: &[PlanResource],
    passes: &[ExecutionPass],
) -> Result<PassInterval, PlanValidationError> {
    let aliases = resources
        .iter()
        .filter_map(|candidate| {
            matches!(
                candidate.kind,
                PlanResourceKind::Alias { source, .. } if source == resource
            )
            .then_some(candidate.id)
        })
        .collect::<BTreeSet<_>>();
    let uses: Vec<_> = passes
        .iter()
        .filter(|pass| {
            pass.kind.output() == resource
                || pass.kind.reads().contains(&resource)
                || aliases.contains(&pass.kind.output())
                || pass.kind.reads().iter().any(|read| aliases.contains(read))
        })
        .map(|pass| pass.id)
        .collect();
    let Some(first) = uses.first().copied() else {
        return Err(invalid(
            "surfaceSlots.allocations",
            "surface resource has no pass use",
        ));
    };
    Ok(PassInterval {
        first,
        last: *uses.last().expect("non-empty surface use list"),
    })
}

pub(super) fn peak_surface_bytes(
    slots: &[SurfaceSlot],
    pass_count: usize,
) -> Result<u64, PlanValidationError> {
    let mut peak = 0_u64;
    for index in 0..pass_count {
        let pass = ExecutionPassId::from_index(index)?;
        let mut current = 0_u64;
        for slot in slots {
            if slot.allocations.iter().any(|allocation| {
                allocation.interval.first <= pass && pass <= allocation.interval.last
            }) {
                current = current.checked_add(slot.estimated_bytes).ok_or_else(|| {
                    invalid("estimatedPeakSurfaceBytes", "peak byte estimate overflow")
                })?;
            }
        }
        peak = peak.max(current);
    }
    Ok(peak)
}

fn invalid(path: impl Into<String>, reason: impl ToString) -> PlanValidationError {
    PlanValidationError::Invalid {
        path: path.into(),
        reason: reason.to_string(),
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PlanValidationError {
    #[error("invalid render plan at {path}: {reason}")]
    Invalid { path: String, reason: String },
    #[error("RenderPlanTemplate canonicalization failed: {0}")]
    Canonical(String),
    #[error(transparent)]
    Binding(#[from] BindingContractError),
    #[error(transparent)]
    Id(#[from] PlanIdError),
}
