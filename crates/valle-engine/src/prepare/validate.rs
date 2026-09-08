use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;
use valle_draw::{program::DrawProgram, requirements::TextureKind};
use valle_timeline::RationalTime;

use crate::{
    compositor::reference::{PremulRgba32, decode_author_srgb},
    resource::{
        ContentDigest, ExternalHandleId, ExternalResourceDesc, ResourceInterpretation,
        ResourcePayload, ResourceRequest, ResourceSample,
    },
};

use super::{
    BoundsReason, Diagnostic, DiagnosticCode, DiagnosticSeverity, DynamicBindingId,
    DynamicBindingKind, DynamicBindings, DynamicValue, FrameGeometry, PrepareOutput,
    PreparedCaption, PreparedDestinationUse, PreparedEffect, PreparedEffectSpace,
    PreparedExternalBackdrop, PreparedFrame, PreparedLayer, PreparedProgram, PreparedProgramKind,
    PreparedResource, PreparedSource, PreparedSourceKind, PreparedTransition, PreparedVisualItem,
    ProgramId, bounds::MAX_DEVICE_INTERMEDIATE_PIXELS,
};

pub fn validate_prepare_output(output: &PrepareOutput) -> Result<(), PreparedFrameValidationError> {
    output.frame.validate()?;
    validate_prepare_output_bindings(output)
}

pub(crate) fn validate_prepare_output_bindings(
    output: &PrepareOutput,
) -> Result<(), PreparedFrameValidationError> {
    output
        .dynamic
        .validate()
        .map_err(|error| invalid("dynamic", error))?;
    let expected_requests: Vec<_> = output
        .frame
        .resources
        .resources()
        .iter()
        .map(|resource| resource.request.clone())
        .collect();
    if output.resource_requests.requests() != expected_requests {
        return Err(invalid(
            "resourceRequests",
            "request set is not the exact projection of PreparedFrame.resources",
        ));
    }

    let mut bindings = BindingAdmission {
        frame: &output.frame,
        dynamic: &output.dynamic,
        next: 1,
        diagnostics: Vec::new(),
    };
    for item in &output.frame.visual {
        match item {
            PreparedVisualItem::Layer(layer) => bindings.layer(layer)?,
            PreparedVisualItem::Transition(transition) => {
                bindings.binding(
                    transition.progress,
                    &format!("{}.progress", transition.semantic_path),
                    DynamicBindingKind::TransitionProgress,
                    None,
                )?;
                bindings.layer(&transition.from)?;
                bindings.layer(&transition.to)?;
            }
        }
    }
    for effect in &output.frame.adjustments {
        bindings.effect(effect, PreparedEffectSpace::Root)?;
    }
    for caption in &output.frame.captions {
        bindings.caption(caption)?;
    }
    bindings.finish(&output.diagnostics)
}

struct BindingAdmission<'a> {
    frame: &'a PreparedFrame,
    dynamic: &'a DynamicBindings,
    next: u32,
    diagnostics: Vec<Diagnostic>,
}

impl BindingAdmission<'_> {
    fn layer(&mut self, layer: &PreparedLayer) -> Result<(), PreparedFrameValidationError> {
        let path = &layer.semantic_path;
        if let PreparedSource::External { placement, .. } = &layer.source
            && let Some(PreparedExternalBackdrop::Blur {
                sigma_device_px, ..
            }) = placement.backdrop
        {
            self.binding(
                sigma_device_px,
                &format!("{path}.sourcePlacement.backdrop.sigmaDevicePx"),
                DynamicBindingKind::DeviceLength,
                None,
            )?;
        }
        self.binding(
            layer.dynamic.transform,
            &format!("{path}.transform"),
            DynamicBindingKind::DeviceTransform,
            Some(DynamicValue::DeviceTransform(layer.device_transform)),
        )?;
        self.binding(
            layer.dynamic.bounds,
            &format!("{path}.bounds"),
            DynamicBindingKind::Bounds,
            Some(DynamicValue::Bounds(layer.bounds.output)),
        )?;
        self.binding(
            layer.dynamic.opacity,
            &format!("{path}.opacity"),
            DynamicBindingKind::Opacity,
            None,
        )?;
        if let PreparedSource::Program { program, .. } = layer.source {
            let program = &self.frame.programs[program.get() as usize - 1];
            let root = super::DeviceRect::full(
                self.frame.render_spec.width(),
                self.frame.render_spec.height(),
            );
            for (index, (prepared, required)) in program
                .destination_uses
                .iter()
                .zip(&program.requirements.destination_uses)
                .enumerate()
            {
                let (sample, output) =
                    if prepared.bounds_reason == BoundsReason::ConservativeCameraTarget {
                        (root, root)
                    } else {
                        super::bounds::program_destination_to_device(
                            required.sample_bounds,
                            required.output_bounds,
                            program.viewport,
                            layer.device_transform,
                            root,
                        )
                        .map_err(|error| invalid(format!("{path}.destination[{index}]"), error))?
                    };
                let destination_path = format!("{path}.destination[{index}]");
                self.binding(
                    prepared.sample_bounds,
                    &format!("{destination_path}.sampleBounds"),
                    DynamicBindingKind::BackdropSampleBounds,
                    Some(DynamicValue::Bounds(sample)),
                )?;
                self.binding(
                    prepared.output_bounds,
                    &format!("{destination_path}.outputBounds"),
                    DynamicBindingKind::BackdropOutputBounds,
                    Some(DynamicValue::Bounds(output)),
                )?;
            }
        }
        for effect in &layer.effects {
            self.effect(
                effect,
                PreparedEffectSpace::Layer {
                    transform: layer.dynamic.transform,
                    bounds: layer.dynamic.bounds,
                },
            )?;
        }
        if layer.bounds.reason == BoundsReason::ConservativeCameraTarget {
            self.diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                code: DiagnosticCode::ConservativeBounds,
                semantic_path: path.clone(),
                message: "camera target resolution requires prepared scene geometry; full-frame bounds are explicit"
                    .to_owned(),
            });
        }
        Ok(())
    }

    fn effect(
        &mut self,
        effect: &PreparedEffect,
        expected_space: PreparedEffectSpace,
    ) -> Result<(), PreparedFrameValidationError> {
        if effect.space != expected_space {
            return Err(invalid(
                &effect.semantic_path,
                "effect space does not match its owning band",
            ));
        }
        for (id, suffix) in effect.kernel.device_lengths() {
            self.binding(
                id,
                &format!("{}.{}", effect.semantic_path, suffix),
                DynamicBindingKind::DeviceLength,
                None,
            )?;
        }
        Ok(())
    }

    fn caption(&mut self, caption: &PreparedCaption) -> Result<(), PreparedFrameValidationError> {
        let path = &caption.semantic_path;
        self.binding(
            caption.dynamic.transform,
            &format!("{path}.transform"),
            DynamicBindingKind::DeviceTransform,
            Some(DynamicValue::DeviceTransform(caption.device_transform)),
        )?;
        self.binding(
            caption.dynamic.bounds,
            &format!("{path}.bounds"),
            DynamicBindingKind::Bounds,
            Some(DynamicValue::Bounds(caption.bounds.output)),
        )?;
        self.binding(
            caption.dynamic.opacity,
            &format!("{path}.opacity"),
            DynamicBindingKind::Opacity,
            None,
        )
    }

    fn binding(
        &mut self,
        id: DynamicBindingId,
        path: &str,
        kind: DynamicBindingKind,
        expected: Option<DynamicValue>,
    ) -> Result<(), PreparedFrameValidationError> {
        if id.get() != self.next {
            return Err(invalid(path, "dynamic reference order is not canonical"));
        }
        self.next += 1;
        let binding = self
            .dynamic
            .get(id)
            .ok_or_else(|| invalid(path, format!("missing dynamic binding {}", id.get())))?;
        if binding.id != id
            || binding.semantic_path != path
            || binding.binding_kind != kind
            || expected.is_some_and(|expected| binding.value != expected)
        {
            return Err(invalid(
                path,
                "dynamic binding identity, kind or mirrored value is inconsistent",
            ));
        }
        Ok(())
    }

    fn finish(self, diagnostics: &[Diagnostic]) -> Result<(), PreparedFrameValidationError> {
        if self.next as usize != self.dynamic.values().len() + 1 {
            return Err(invalid(
                "dynamic",
                "dynamic table contains an unreferenced binding",
            ));
        }
        if diagnostics != self.diagnostics {
            return Err(invalid(
                "diagnostics",
                "diagnostics are not the exact projection of prepared conservative bounds",
            ));
        }
        Ok(())
    }
}

pub fn validate_prepared_frame(frame: &PreparedFrame) -> Result<(), PreparedFrameValidationError> {
    validate_prepared_frame_impl(frame, true)
}

pub(crate) fn validate_constructed_prepared_frame(
    frame: &PreparedFrame,
) -> Result<(), PreparedFrameValidationError> {
    validate_prepared_frame_impl(frame, false)
}

fn validate_prepared_frame_impl(
    frame: &PreparedFrame,
    verify_packed_programs: bool,
) -> Result<(), PreparedFrameValidationError> {
    let root_pixels = u64::from(frame.render_spec.width()) * u64::from(frame.render_spec.height());
    if root_pixels > MAX_DEVICE_INTERMEDIATE_PIXELS {
        return Err(invalid(
            "renderSpec",
            format!(
                "output pixel budget exceeded: {root_pixels} > {MAX_DEVICE_INTERMEDIATE_PIXELS}"
            ),
        ));
    }
    let expected_background =
        decode_author_srgb(frame.render_spec.output().background().author_color())
            .map_err(|error| invalid("background", error))?
            .channels();
    PremulRgba32::from_premultiplied(frame.background.working_linear_rec2020_premul)
        .map_err(|error| invalid("background", error))?;
    if frame.background.working_linear_rec2020_premul != expected_background {
        return Err(invalid(
            "background",
            "prepared working color does not match RenderSpec background",
        ));
    }

    validate_manifest_shape(frame)?;
    validate_prepared_program_table(&frame.programs, verify_packed_programs)?;

    let mut admission = Admission {
        frame,
        next_dynamic: 1,
        next_program: 1,
        next_handle: 1,
        expected_resource_paths: BTreeMap::new(),
        geometry: Vec::new(),
    };
    for (index, item) in frame.visual.iter().enumerate() {
        let path = format!("visual[{index}]");
        match item {
            PreparedVisualItem::Layer(layer) => admission.layer(layer, &path)?,
            PreparedVisualItem::Transition(transition) => {
                admission.transition(transition, &path)?
            }
        }
    }
    validate_effect_order(&frame.adjustments, "adjustments")?;
    for (index, effect) in frame.adjustments.iter().enumerate() {
        admission.effect(
            effect,
            &format!("adjustments[{index}]"),
            PreparedEffectSpace::Root,
        )?;
    }
    for (index, caption) in frame.captions.iter().enumerate() {
        admission.caption(caption, &format!("captions[{index}]"))?;
    }
    admission.finish()
}

struct Admission<'a> {
    frame: &'a PreparedFrame,
    next_dynamic: u32,
    next_program: u32,
    next_handle: u32,
    expected_resource_paths: BTreeMap<ExternalHandleId, Vec<String>>,
    geometry: Vec<FrameGeometry>,
}

impl Admission<'_> {
    fn transition(
        &mut self,
        transition: &PreparedTransition,
        path: &str,
    ) -> Result<(), PreparedFrameValidationError> {
        expect_path(&transition.semantic_path, path)?;
        self.dynamic(transition.progress, &format!("{path}.progress"))?;
        self.layer(&transition.from, &format!("{path}.from"))?;
        self.layer(&transition.to, &format!("{path}.to"))
    }

    fn layer(
        &mut self,
        layer: &PreparedLayer,
        path: &str,
    ) -> Result<(), PreparedFrameValidationError> {
        expect_path(&layer.semantic_path, path)?;
        if layer.clip_id.is_empty() || layer.track_id.is_empty() {
            return Err(invalid(path, "clip and track ids must not be empty"));
        }
        layer
            .device_transform
            .validate_wire()
            .map_err(|error| invalid(format!("{path}.deviceTransform"), error))?;
        validate_bounds_in_root(
            layer.bounds,
            self.frame.render_spec.width(),
            self.frame.render_spec.height(),
            &format!("{path}.bounds"),
        )?;
        validate_mask(layer.mask.as_ref(), &format!("{path}.mask"))?;

        let program = match &layer.source {
            PreparedSource::External {
                source_kind,
                handle,
                placement,
            } => {
                if matches!(
                    source_kind,
                    PreparedSourceKind::Motion | PreparedSourceKind::Solid
                ) {
                    return Err(invalid(path, "program source kind used an external handle"));
                }
                placement
                    .validate()
                    .map_err(|error| invalid(format!("{path}.source.placement"), error))?;
                if let Some(PreparedExternalBackdrop::Blur {
                    sigma_device_px, ..
                }) = placement.backdrop
                {
                    self.dynamic(
                        sigma_device_px,
                        &format!("{path}.sourcePlacement.backdrop.sigmaDevicePx"),
                    )?;
                }
                let request = self.use_handle(*handle, format!("{path}.source"))?;
                expect_visual_request(&request, &format!("{path}.source"))?;
                match source_kind {
                    PreparedSourceKind::Image if request.sample() != ResourceSample::Static => {
                        return Err(invalid(
                            path,
                            "image source must use a static resource sample",
                        ));
                    }
                    PreparedSourceKind::Video | PreparedSourceKind::Lottie
                        if !matches!(request.sample(), ResourceSample::SourceTime(_)) =>
                    {
                        return Err(invalid(
                            path,
                            "video and Lottie sources require an explicit source-time sample",
                        ));
                    }
                    _ => {}
                }
                None
            }
            PreparedSource::Program {
                source_kind,
                program,
            } => {
                let (kind, program_path) = match source_kind {
                    PreparedSourceKind::Motion => {
                        (PreparedProgramKind::Motion, format!("{path}.motion"))
                    }
                    PreparedSourceKind::Solid => {
                        (PreparedProgramKind::Solid, format!("{path}.solid"))
                    }
                    PreparedSourceKind::Video
                    | PreparedSourceKind::Image
                    | PreparedSourceKind::Lottie => {
                        return Err(invalid(path, "external source kind used a Draw program"));
                    }
                };
                Some(self.program(*program, kind, &program_path)?)
            }
        };

        validate_effect_order(&layer.effects, path)?;
        let has_source_operators = layer
            .effects
            .iter()
            .any(|effect| effect.kernel.is_source_operator());
        if has_source_operators && !matches!(layer.source, PreparedSource::External { .. }) {
            return Err(invalid(
                path,
                "source-alpha operators require an external layer",
            ));
        }
        self.dynamic(layer.dynamic.transform, &format!("{path}.transform"))?;
        self.dynamic(layer.dynamic.bounds, &format!("{path}.bounds"))?;
        self.dynamic(layer.dynamic.opacity, &format!("{path}.opacity"))?;
        if let Some(program) = program {
            self.program_destinations(&program, layer.bounds.reason, path)?;
        }
        for (index, effect) in layer.effects.iter().enumerate() {
            self.effect(
                effect,
                &format!("{path}.effects[{index}]"),
                PreparedEffectSpace::Layer {
                    transform: layer.dynamic.transform,
                    bounds: layer.dynamic.bounds,
                },
            )?;
        }
        self.geometry.push(FrameGeometry {
            semantic_path: path.to_owned(),
            transform: layer.device_transform,
            bounds: layer.bounds,
        });
        Ok(())
    }

    fn caption(
        &mut self,
        caption: &PreparedCaption,
        path: &str,
    ) -> Result<(), PreparedFrameValidationError> {
        expect_path(&caption.semantic_path, path)?;
        if caption.clip_id.is_empty() || caption.track_id.is_empty() {
            return Err(invalid(path, "clip and track ids must not be empty"));
        }
        caption
            .device_transform
            .validate_wire()
            .map_err(|error| invalid(format!("{path}.deviceTransform"), error))?;
        validate_bounds_in_root(
            caption.bounds,
            self.frame.render_spec.width(),
            self.frame.render_spec.height(),
            &format!("{path}.bounds"),
        )?;
        let program = self.program(
            caption.program,
            PreparedProgramKind::Caption,
            &format!("{path}.program"),
        )?;
        if !program.destination_uses.is_empty() || !program.requirements.destination_uses.is_empty()
        {
            return Err(invalid(
                path,
                "caption program cannot read the visual destination",
            ));
        }
        self.dynamic(caption.dynamic.transform, &format!("{path}.transform"))?;
        self.dynamic(caption.dynamic.bounds, &format!("{path}.bounds"))?;
        self.dynamic(caption.dynamic.opacity, &format!("{path}.opacity"))?;
        self.geometry.push(FrameGeometry {
            semantic_path: path.to_owned(),
            transform: caption.device_transform,
            bounds: caption.bounds,
        });
        Ok(())
    }

    fn program(
        &mut self,
        id: ProgramId,
        kind: PreparedProgramKind,
        path: &str,
    ) -> Result<PreparedProgram, PreparedFrameValidationError> {
        if id.get() != self.next_program {
            return Err(invalid(
                path,
                format!(
                    "program id {} is not the next canonical id {}",
                    id.get(),
                    self.next_program
                ),
            ));
        }
        self.next_program += 1;
        let program = self
            .frame
            .programs
            .get(id.get() as usize - 1)
            .filter(|program| program.id == id)
            .ok_or_else(|| invalid(path, format!("undefined program {}", id.get())))?
            .clone();
        if program.kind != kind {
            return Err(invalid(path, "program kind does not match its owning band"));
        }
        expect_path(&program.semantic_path, path)?;
        self.program_resources(&program)?;
        Ok(program)
    }

    fn program_resources(
        &mut self,
        program: &PreparedProgram,
    ) -> Result<(), PreparedFrameValidationError> {
        let path = &program.semantic_path;
        let requirements = &program.requirements;
        if program.resources.textures.len() != requirements.external_textures.len()
            || program.resources.fonts.len() != requirements.fonts.len()
            || program.resources.runtime_shaders.len() != requirements.runtime_shaders.len()
            || program.resources.scenes.len() != requirements.scene3d.len()
        {
            return Err(invalid(
                path,
                "program resource bindings do not match requirements",
            ));
        }
        for (binding, requirement) in program
            .resources
            .textures
            .iter()
            .zip(&requirements.external_textures)
        {
            if binding.key != requirement.key {
                return Err(invalid(path, "texture binding order/key is not canonical"));
            }
            let request =
                self.use_handle(binding.handle, format!("{path}.texture[{}]", binding.key))?;
            expect_visual_request(&request, path)?;
            let expected_sample = match requirement.kind {
                TextureKind::Video => {
                    let micros = requirement.sample_time_micros.ok_or_else(|| {
                        invalid(path, "video texture requirement lacks its source timestamp")
                    })?;
                    ResourceSample::SourceTime(RationalTime::new(micros, 1_000_000).map_err(
                        |error| invalid(path, format!("invalid video texture timestamp: {error}")),
                    )?)
                }
                TextureKind::Image | TextureKind::Generated => ResourceSample::Static,
            };
            if request.sample() != expected_sample {
                return Err(invalid(
                    path,
                    "texture binding sample does not match its exact producer requirement",
                ));
            }
        }
        for (binding, requirement) in program.resources.fonts.iter().zip(&requirements.fonts) {
            let required_face = ContentDigest::from_bytes(requirement.face_hash.into_bytes());
            if binding.face_hash != required_face || binding.face_index != requirement.face_index {
                return Err(invalid(
                    path,
                    "font binding order/identity is not canonical",
                ));
            }
            let request = self.use_handle(
                binding.handle,
                format!("{path}.font[{}:{}]", binding.face_hash, binding.face_index),
            )?;
            if request.sample() != ResourceSample::Static
                || !matches!(request.expected(), ExternalResourceDesc::FontBytes)
                || request.key().content != required_face
                || !matches!(
                    request.key().interpretation,
                    ResourceInterpretation::FontFace { face_index }
                        if face_index == requirement.face_index
                )
            {
                return Err(invalid(
                    path,
                    "font binding does not match its resource request",
                ));
            }
        }
        for (binding, requirement) in program
            .resources
            .runtime_shaders
            .iter()
            .zip(&requirements.runtime_shaders)
        {
            if binding.key != requirement.uri {
                return Err(invalid(path, "runtime shader binding key is not canonical"));
            }
            let request =
                self.use_handle(binding.handle, format!("{path}.shader[{}]", binding.key))?;
            let ResourceInterpretation::RuntimeShader { abi_digest, .. } =
                &request.key().interpretation
            else {
                return Err(invalid(
                    path,
                    "runtime shader binding has the wrong interpretation",
                ));
            };
            let required_content = ContentDigest::from_bytes(requirement.content_hash.into_bytes());
            let required_abi = ContentDigest::from_bytes(requirement.abi_hash.into_bytes());
            if request.sample() != ResourceSample::Static
                || !matches!(request.expected(), ExternalResourceDesc::RuntimeShader)
                || request.key().content != required_content
                || *abi_digest != required_abi
            {
                return Err(invalid(
                    path,
                    "runtime shader binding identity is inconsistent",
                ));
            }
        }
        for (binding, requirement) in program.resources.scenes.iter().zip(&requirements.scene3d) {
            if binding.key.is_empty() {
                return Err(invalid(path, "Scene3D binding key must not be empty"));
            }
            let request =
                self.use_handle(binding.handle, format!("{path}.scene3d[{}]", binding.key))?;
            let required_content = ContentDigest::from_bytes(requirement.content_hash.into_bytes());
            let required_topology =
                ContentDigest::from_bytes(requirement.topology_hash.into_bytes());
            if request.sample() != ResourceSample::Static
                || !matches!(request.expected(), ExternalResourceDesc::Scene3d)
                || request.key().content != required_content
                || !matches!(
                    &request.key().interpretation,
                    ResourceInterpretation::Scene3d { topology_digest }
                        if *topology_digest == required_topology
                )
                || !matches!(
                    request.payload(),
                    Some(ResourcePayload::Scene3dFrame { canonical_request })
                        if !canonical_request.is_empty()
                )
            {
                return Err(invalid(path, "Scene3D binding identity is inconsistent"));
            }
        }
        Ok(())
    }

    fn program_destinations(
        &mut self,
        program: &PreparedProgram,
        reason: BoundsReason,
        owner_path: &str,
    ) -> Result<(), PreparedFrameValidationError> {
        if program.destination_uses.len() != program.requirements.destination_uses.len() {
            return Err(invalid(
                &program.semantic_path,
                "prepared destination bindings do not match program requirements",
            ));
        }
        for (index, (prepared, required)) in program
            .destination_uses
            .iter()
            .zip(&program.requirements.destination_uses)
            .enumerate()
        {
            if prepared.node != required.node
                || prepared.scope != required.scope
                || prepared.operation != required.operation
                || prepared.bounds_reason != reason
            {
                return Err(invalid(
                    format!("{owner_path}.destination[{index}]"),
                    "destination binding does not match the DrawProgram contract",
                ));
            }
            self.destination_dynamic(prepared, owner_path, index)?;
        }
        Ok(())
    }

    fn destination_dynamic(
        &mut self,
        destination: &PreparedDestinationUse,
        owner_path: &str,
        index: usize,
    ) -> Result<(), PreparedFrameValidationError> {
        let path = format!("{owner_path}.destination[{index}]");
        self.dynamic(destination.sample_bounds, &format!("{path}.sampleBounds"))?;
        self.dynamic(destination.output_bounds, &format!("{path}.outputBounds"))
    }

    fn effect(
        &mut self,
        effect: &PreparedEffect,
        path: &str,
        expected_space: PreparedEffectSpace,
    ) -> Result<(), PreparedFrameValidationError> {
        expect_path(&effect.semantic_path, path)?;
        effect
            .validate_wire()
            .map_err(|error| invalid(path, error))?;
        if effect.space != expected_space {
            return Err(invalid(path, "effect space does not match its owning band"));
        }
        for (id, suffix) in effect.kernel.device_lengths() {
            self.dynamic(id, &format!("{path}.{suffix}"))?;
        }
        Ok(())
    }

    fn dynamic(
        &mut self,
        id: DynamicBindingId,
        path: &str,
    ) -> Result<(), PreparedFrameValidationError> {
        if id.get() != self.next_dynamic {
            return Err(invalid(
                path,
                format!(
                    "dynamic id {} is not the next canonical id {}",
                    id.get(),
                    self.next_dynamic
                ),
            ));
        }
        self.next_dynamic += 1;
        Ok(())
    }

    fn use_handle(
        &mut self,
        handle: ExternalHandleId,
        path: String,
    ) -> Result<ResourceRequest, PreparedFrameValidationError> {
        let resource = self.resource(handle)?.clone();
        let paths = self.expected_resource_paths.entry(handle).or_default();
        if paths.is_empty() {
            if handle.get() != self.next_handle {
                return Err(invalid(
                    path,
                    format!(
                        "first-use handle {} is not the next canonical handle {}",
                        handle.get(),
                        self.next_handle
                    ),
                ));
            }
            self.next_handle += 1;
        }
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
        Ok(resource.request)
    }

    fn resource(
        &self,
        handle: ExternalHandleId,
    ) -> Result<&PreparedResource, PreparedFrameValidationError> {
        self.frame
            .resources
            .resources()
            .get(handle.get() as usize - 1)
            .filter(|resource| resource.request.handle() == handle)
            .ok_or_else(|| invalid("resources", format!("undefined handle {}", handle.get())))
    }

    fn finish(self) -> Result<(), PreparedFrameValidationError> {
        if self.next_program as usize != self.frame.programs.len() + 1 {
            return Err(invalid(
                "programs",
                "program table contains an unreferenced entry",
            ));
        }
        if self.next_handle as usize != self.frame.resources.resources().len() + 1 {
            return Err(invalid(
                "resources",
                "resource manifest contains an unreferenced handle",
            ));
        }
        for resource in self.frame.resources.resources() {
            let expected = self
                .expected_resource_paths
                .get(&resource.request.handle())
                .ok_or_else(|| invalid("resources", "resource has no semantic use"))?;
            if &resource.semantic_paths != expected {
                return Err(invalid(
                    format!("resources[{}]", resource.request.handle().get() - 1),
                    "resource semantic paths do not equal canonical first-use traversal",
                ));
            }
        }
        if self.frame.metadata.geometry != self.geometry {
            return Err(invalid(
                "metadata.geometry",
                "geometry metadata is not the exact visual/caption projection",
            ));
        }
        Ok(())
    }
}

fn validate_effect_order(
    effects: &[PreparedEffect],
    path: &str,
) -> Result<(), PreparedFrameValidationError> {
    if effects
        .windows(2)
        .any(|pair| pair[0].kernel.canonical_rank() >= pair[1].kernel.canonical_rank())
    {
        Err(invalid(
            path,
            "effects are not in strict canonical compositor pipeline order",
        ))
    } else {
        Ok(())
    }
}

fn validate_manifest_shape(frame: &PreparedFrame) -> Result<(), PreparedFrameValidationError> {
    for (index, resource) in frame.resources.resources().iter().enumerate() {
        let expected = u32::try_from(index + 1)
            .map_err(|_| invalid("resources", "resource count exceeds u32 handle space"))?;
        if resource.request.handle().get() != expected {
            return Err(invalid(
                format!("resources[{index}]"),
                "resource handles must be contiguous and index ordered",
            ));
        }
        if resource.semantic_paths.is_empty()
            || resource.semantic_paths.iter().any(|path| path.is_empty())
        {
            return Err(invalid(
                format!("resources[{index}].semanticPaths"),
                "resource must carry at least one non-empty semantic path",
            ));
        }
        let unique: BTreeSet<_> = resource.semantic_paths.iter().collect();
        if unique.len() != resource.semantic_paths.len() {
            return Err(invalid(
                format!("resources[{index}].semanticPaths"),
                "resource semantic paths must be unique",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_prepared_program_table(
    programs: &[PreparedProgram],
    verify_packed: bool,
) -> Result<(), PreparedFrameValidationError> {
    for (index, program) in programs.iter().enumerate() {
        let expected = u32::try_from(index + 1)
            .map_err(|_| invalid("programs", "program count exceeds u32 id space"))?;
        if program.id.get() != expected {
            return Err(invalid(
                format!("programs[{index}]"),
                "program ids must be contiguous and index ordered",
            ));
        }
        if verify_packed {
            let decoded = DrawProgram::from_packed(&program.packed)
                .map_err(|error| invalid(format!("programs[{index}].packed"), error))?;
            if decoded.viewport() != program.viewport
                || decoded.requirements() != &program.requirements
            {
                return Err(invalid(
                    format!("programs[{index}]"),
                    "packed DrawProgram, viewport and derived requirements disagree",
                ));
            }
        }
        if program.content_hash != ContentDigest::of_bytes(&program.packed) {
            return Err(invalid(
                format!("programs[{index}].contentHash"),
                "program content hash does not match canonical packed bytes",
            ));
        }
        if program.semantic_path.is_empty() {
            return Err(invalid(
                format!("programs[{index}].semanticPath"),
                "program semantic path must not be empty",
            ));
        }
        if program.resources.textures.len() != program.requirements.external_textures.len()
            || program.resources.fonts.len() != program.requirements.fonts.len()
            || program.resources.runtime_shaders.len() != program.requirements.runtime_shaders.len()
            || program.resources.scenes.len() != program.requirements.scene3d.len()
        {
            return Err(invalid(
                format!("programs[{index}].resources"),
                "program resource bindings do not match derived requirements",
            ));
        }
        if program
            .resources
            .textures
            .iter()
            .zip(&program.requirements.external_textures)
            .any(|(binding, requirement)| binding.key != requirement.key)
            || program
                .resources
                .fonts
                .iter()
                .zip(&program.requirements.fonts)
                .any(|(binding, requirement)| {
                    ContentDigest::from_bytes(requirement.face_hash.into_bytes())
                        != binding.face_hash
                        || binding.face_index != requirement.face_index
                })
            || program
                .resources
                .runtime_shaders
                .iter()
                .zip(&program.requirements.runtime_shaders)
                .any(|(binding, requirement)| binding.key != requirement.uri)
            || program
                .resources
                .scenes
                .iter()
                .any(|binding| binding.key.is_empty())
        {
            return Err(invalid(
                format!("programs[{index}].resources"),
                "program resource binding order or identity is not canonical",
            ));
        }
        if program.destination_uses.len() != program.requirements.destination_uses.len()
            || program
                .destination_uses
                .iter()
                .zip(&program.requirements.destination_uses)
                .any(|(prepared, required)| {
                    prepared.node != required.node
                        || prepared.scope != required.scope
                        || prepared.operation != required.operation
                        || prepared.sample_bounds == prepared.output_bounds
                })
        {
            return Err(invalid(
                format!("programs[{index}].destinationUses"),
                "prepared destination bindings do not match derived requirements",
            ));
        }
    }
    Ok(())
}

fn validate_bounds_in_root(
    bounds: super::PreparedBounds,
    width: u32,
    height: u32,
    path: &str,
) -> Result<(), PreparedFrameValidationError> {
    bounds
        .validate_wire()
        .map_err(|error| invalid(path, error))?;
    for rect in [bounds.content, bounds.output, bounds.sample] {
        let right = i64::from(rect.x) + i64::from(rect.width);
        let bottom = i64::from(rect.y) + i64::from(rect.height);
        if rect.x < 0 || rect.y < 0 || right > i64::from(width) || bottom > i64::from(height) {
            return Err(invalid(
                path,
                "prepared bounds escape the RenderSpec output",
            ));
        }
    }
    if bounds.reason == BoundsReason::ConservativeCameraTarget {
        let root = super::DeviceRect::full(width, height);
        if bounds.content != root || bounds.output != root || bounds.sample != root {
            return Err(invalid(
                path,
                "conservative camera-target bounds must be exactly full-frame",
            ));
        }
    }
    Ok(())
}

fn validate_mask(
    mask: Option<&super::PreparedMask>,
    path: &str,
) -> Result<(), PreparedFrameValidationError> {
    let Some(mask) = mask else {
        return Ok(());
    };
    mask.validate_wire().map_err(|error| invalid(path, error))
}

fn expect_visual_request(
    request: &ResourceRequest,
    path: &str,
) -> Result<(), PreparedFrameValidationError> {
    if !matches!(
        request.key().interpretation,
        ResourceInterpretation::Visual { .. }
    ) || !matches!(request.expected(), ExternalResourceDesc::VisualFrame { .. })
    {
        Err(invalid(path, "resource is not a visual frame request"))
    } else {
        Ok(())
    }
}

fn expect_path(actual: &str, expected: &str) -> Result<(), PreparedFrameValidationError> {
    if actual == expected {
        Ok(())
    } else {
        Err(invalid(
            expected,
            format!("semantic path {actual:?} is not canonical"),
        ))
    }
}

fn invalid(
    path: impl Into<String>,
    reason: impl std::fmt::Display,
) -> PreparedFrameValidationError {
    PreparedFrameValidationError {
        path: path.into(),
        reason: reason.to_string(),
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{path}: {reason}")]
pub struct PreparedFrameValidationError {
    pub path: String,
    pub reason: String,
}
