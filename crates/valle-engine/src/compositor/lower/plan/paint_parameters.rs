//! Paint-only frames reuse an admitted immutable local plan. No full encoding, structural
//! projection, range traversal or binary patch is needed to change a solid color.
use super::*;
use valle_draw::program::{PaintFrame, PaintTemplate};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProgramPaintParameters {
    #[serde(with = "packed_program_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(super) struct AdmittedPaintLayout {
    template: Arc<PaintTemplate>,
    frame: Arc<PlanProgramFrame>,
}

impl PartialEq for AdmittedPaintLayout {
    fn eq(&self, other: &Self) -> bool {
        self.template.digest().ok() == other.template.digest().ok() && self.frame == other.frame
    }
}

impl PlanProgram {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_paint_frame(
        id: ProgramId,
        kind: PreparedProgramKind,
        semantic_path: String,
        paint: PaintFrame,
        resources: PlanProgramResources,
        destination_uses: Vec<PreparedDestinationUse>,
        layout: Option<&PlanProgramLayout>,
    ) -> Result<Self, PlanValidationError> {
        let digest = ContentDigest::from_bytes(
            paint
                .template()
                .digest()
                .map_err(|error| invalid("program.parameters", error))?,
        );
        let initial_layout;
        let layout = if let Some(layout) = layout {
            if !layout.paint_parameters
                || layout.baseline_content_hash != digest
                || layout.id != id
                || layout.kind != kind
                || layout.semantic_path != semantic_path
                || layout.resources != resources
                || layout.destination_uses != destination_uses
            {
                return Err(invalid(
                    "program.layout",
                    "paint template or resource layout changed",
                ));
            }
            if layout.attached_paint.get().is_none() {
                layout.validate_paint_baseline()?;
            }
            initial_layout = None;
            layout
        } else {
            let draw = paint.template().program();
            let local_plan =
                ProgramPlan::derive(draw).map_err(|error| invalid("program.localPlan", error))?;
            let local_schedule = ProgramSchedule::derive(&local_plan)
                .map_err(|error| invalid("program.localSchedule", error))?;
            let frame = Arc::new(PlanProgramFrame {
                id,
                kind,
                semantic_path,
                viewport: draw.viewport(),
                requirements: draw.requirements().clone(),
                resources,
                destination_uses,
                local_plan,
                local_schedule,
            });
            let baseline_frame_packed = program_frame_payload_bytes(&frame)?;
            initial_layout = Some(PlanProgramLayout {
                id,
                kind,
                semantic_path: frame.semantic_path.clone(),
                structure_hash: program_structure_hash(&frame)?,
                paint_parameters: true,
                baseline_content_hash: digest,
                baseline_packed: paint
                    .template()
                    .packed()
                    .map_err(|error| invalid("program.parameters", error))?
                    .to_vec(),
                baseline_frame_hash: ContentDigest::of_bytes(&baseline_frame_packed),
                baseline_frame_packed,
                resources: frame.resources.clone(),
                destination_uses: frame.destination_uses.clone(),
                attached_structure: OnceLock::from(ProgramStructureProjection::from_program(
                    &frame,
                )),
                attached_paint: OnceLock::from(Arc::new(AdmittedPaintLayout {
                    template: paint.template().clone(),
                    frame,
                })),
            });
            initial_layout.as_ref().unwrap()
        };
        let frame = layout
            .attached_paint
            .get()
            .expect("paint layout admitted")
            .frame
            .clone();
        let result = Self {
            id,
            content_hash: ContentDigest::from_bytes(
                paint
                    .digest()
                    .map_err(|error| invalid("program.parameters", error))?,
            ),
            frame_hash: layout.baseline_frame_hash,
            patch: Vec::new(),
            frame_patch: Vec::new(),
            parameters: Some(ProgramPaintParameters {
                bytes: paint.parameter_bytes(),
            }),
            frame: OnceLock::from(frame),
            packed: OnceLock::new(),
            runtime_program: None,
            paint_frame: OnceLock::from(paint),
            structure_hash: OnceLock::from(layout.structure_hash),
            admitted_baseline: OnceLock::from(PlanProgramAdmissionKey {
                content: digest,
                frame: layout.baseline_frame_hash,
            }),
            initial_layout,
            constructed: false,
        };
        result.validate_payload()?;
        Ok(result)
    }

    pub(super) fn admit_paint_layout(
        &self,
        layout: &PlanProgramLayout,
    ) -> Result<(), PlanValidationError> {
        let parameters = self
            .parameters
            .as_ref()
            .ok_or_else(|| invalid("program.parameters", "parameter payload required"))?;
        if !layout.paint_parameters || self.frame_hash != layout.baseline_frame_hash {
            return Err(invalid(
                "program.parameters",
                "parameter frame metadata or representation mismatch",
            ));
        }
        let admission = PlanProgramAdmissionKey {
            content: layout.baseline_content_hash,
            frame: layout.baseline_frame_hash,
        };
        if let Some(attached) = self.admitted_baseline.get() {
            if attached != &admission
                || self.structure_hash.get() != Some(&layout.structure_hash)
                || self.frame.get().is_none_or(|frame| {
                    frame.id != layout.id
                        || frame.kind != layout.kind
                        || frame.semantic_path != layout.semantic_path
                        || frame.resources != layout.resources
                        || frame.destination_uses != layout.destination_uses
                })
            {
                return Err(invalid(
                    "program.layout",
                    "parameter frame attachment disagrees with template",
                ));
            }
            return Ok(());
        }
        if layout.attached_paint.get().is_none() {
            layout.validate_paint_baseline()?;
        }
        let admitted = layout.attached_paint.get().expect("paint layout admitted");
        let paint = PaintFrame::from_parameters(admitted.template.clone(), &parameters.bytes)
            .map_err(|error| invalid("program.parameters", error))?;
        if self.content_hash
            != ContentDigest::from_bytes(
                paint
                    .digest()
                    .map_err(|error| invalid("program.parameters", error))?,
            )
        {
            return Err(invalid(
                "program.contentHash",
                "hash does not name template and exact parameters",
            ));
        }
        self.paint_frame
            .set(paint)
            .map_err(|_| invalid("program.parameters", "frame admission raced"))?;
        self.frame
            .set(admitted.frame.clone())
            .map_err(|_| invalid("program.frame", "frame admission raced"))?;
        self.structure_hash
            .set(layout.structure_hash)
            .map_err(|_| invalid("program.layout", "frame admission raced"))?;
        self.admitted_baseline
            .set(admission)
            .map_err(|_| invalid("program.layout", "frame admission raced"))?;
        Ok(())
    }
}

impl PlanProgramLayout {
    pub(super) fn validate_paint_baseline(&self) -> Result<(), PlanValidationError> {
        if self.baseline_content_hash != ContentDigest::of_bytes(&self.baseline_packed)
            || self.baseline_frame_hash != ContentDigest::of_bytes(&self.baseline_frame_packed)
        {
            return Err(invalid(
                "program.baseline",
                "paint template or metadata hash mismatch",
            ));
        }
        let template = Arc::new(
            PaintTemplate::from_packed(&self.baseline_packed)
                .map_err(|error| invalid("program.baselinePacked", error))?,
        );
        let payload: PlanProgramFramePayload = serde_json::from_slice(&self.baseline_frame_packed)
            .map_err(|error| invalid("program.baselineFramePacked", error))?;
        let local_plan = ProgramPlan::derive(template.program())
            .map_err(|error| invalid("program.localPlan", error))?;
        let local_schedule = ProgramSchedule::derive(&local_plan)
            .map_err(|error| invalid("program.localSchedule", error))?;
        let frame = Arc::new(PlanProgramFrame {
            id: self.id,
            kind: self.kind,
            semantic_path: self.semantic_path.clone(),
            viewport: payload.viewport,
            requirements: payload.requirements,
            resources: self.resources.clone(),
            destination_uses: self.destination_uses.clone(),
            local_plan: payload.local_plan,
            local_schedule: payload.local_schedule,
        });
        validate_program_contract(
            &frame,
            self,
            template.program(),
            &local_plan,
            &local_schedule,
        )?;
        let admitted = Arc::new(AdmittedPaintLayout { template, frame });
        if self
            .attached_paint
            .get()
            .is_some_and(|existing| existing != &admitted)
        {
            return Err(invalid(
                "program.baseline",
                "attached paint layout disagrees with baseline",
            ));
        }
        let _ = self
            .attached_structure
            .set(ProgramStructureProjection::from_program(&admitted.frame));
        let _ = self.attached_paint.set(admitted);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use valle_draw::program::*;

    fn template(count: usize, width: f64) -> Arc<PaintTemplate> {
        let mut builder = DrawProgramBuilder::new(Rect::new(0., 0., 64., 64.));
        let paint = builder.push_paint(Paint::Solid(LinearColor::new(1., 0., 0., 1.)));
        let path = builder.push_path(PathData {
            verbs: vec![
                PathVerb::MoveTo,
                PathVerb::LineTo,
                PathVerb::LineTo,
                PathVerb::Close,
            ],
            points: vec![[0., 0.], [width, 0.], [width, 64.]],
        });
        for _ in 0..count {
            let node = builder.push_node(Node::Path(PathNode {
                path,
                fill_rule: FillRule::NonZero,
                fill: Some(paint),
                stroke: None,
            }));
            builder.add_root(node);
        }
        Arc::new(
            PaintTemplate::new(
                builder.finish().unwrap(),
                vec![PaintParameter { paint, slot: 0 }],
                1,
            )
            .unwrap(),
        )
    }

    fn lower(
        template: &Arc<PaintTemplate>,
        value: f32,
        layout: Option<&PlanProgramLayout>,
    ) -> PlanProgram {
        PlanProgram::from_paint_frame(
            ProgramId::try_from(1).unwrap(),
            PreparedProgramKind::Motion,
            "visual[0].program".into(),
            template
                .bind_frame(vec![LinearColor::new(value, -0., 0.25, 1.)])
                .unwrap(),
            PlanProgramResources::default(),
            vec![],
            layout,
        )
        .unwrap()
    }

    #[test]
    fn random_access_and_eviction_preserve_exact_draw_bytes_and_share_the_local_plan() {
        let template = template(200, 64.);
        let mut first = lower(&template, 0.2, None);
        let layout = first.take_initial_layout().unwrap();
        for value in [0.8, 0.1, 4., -0.2, 0.8] {
            let hot = lower(&template, value, Some(&layout));
            let materialized_plan = ProgramPlan::derive(hot.runtime_program().unwrap()).unwrap();
            assert_eq!(hot.local_plan(), &materialized_plan);
            assert_eq!(
                hot.local_schedule(),
                &ProgramSchedule::derive(&materialized_plan).unwrap()
            );
            assert!(Arc::ptr_eq(
                first.frame.get().unwrap(),
                hot.frame.get().unwrap()
            ));
            assert!(hot.packed.get().is_none());
            let _cost = hot.encoding_cost_bytes();
            assert!(
                hot.packed.get().is_none(),
                "cache accounting must not pack a full frame"
            );
            let wire = serde_json::to_vec(&hot).unwrap();
            assert!(wire.len() < 400);
            let admitted: PlanProgram = serde_json::from_slice(&wire).unwrap();
            assert!(!admitted.has_attached_program());
            // A fresh deserialized layout models eviction and an arbitrary cold seek.
            let cold: PlanProgramLayout =
                serde_json::from_slice(&serde_json::to_vec(&layout).unwrap()).unwrap();
            cold.validate_baseline().unwrap();
            admitted.admit_layout(&cold).unwrap();
            assert!(admitted.has_attached_program());
            assert!(admitted.packed.get().is_none());
            assert!(admitted.runtime_program().is_some());
            assert_eq!(admitted.packed(), hot.packed());
            assert_eq!(admitted.packed(), lower(&template, value, None).packed());
            assert_eq!(hot.local_plan(), admitted.local_plan());
        }
    }

    #[test]
    fn parameters_and_template_contracts_fail_closed() {
        let template = template(2, 64.);
        let mut first = lower(&template, 0.5, None);
        let layout = first.take_initial_layout().unwrap();
        let wire = serde_json::to_value(&first).unwrap();
        let decode = |value| serde_json::from_value::<PlanProgram>(value).unwrap();
        let mut corrupt = wire.clone();
        corrupt["contentHash"] = serde_json::to_value(ContentDigest::from_bytes([0; 32])).unwrap();
        assert!(decode(corrupt).admit_layout(&layout).is_err());
        let mut corrupt = wire.clone();
        corrupt["frameHash"] = serde_json::to_value(ContentDigest::from_bytes([0; 32])).unwrap();
        assert!(decode(corrupt).admit_layout(&layout).is_err());
        for bytes in [vec![0; 32], vec![255; 16]] {
            let mut frame = decode(wire.clone());
            frame.parameters.as_mut().unwrap().bytes = bytes;
            assert!(frame.admit_layout(&layout).is_err());
        }
        let mut frame = decode(wire.clone());
        frame.patch.push(1);
        assert!(frame.validate_payload().is_err());
        let mut wrong = layout.clone();
        wrong.semantic_path.push('x');
        assert!(first.admit_layout(&wrong).is_err());
        let mut changed = lower(&self::template(2, 32.), 0.5, None);
        let changed_layout = changed.take_initial_layout().unwrap();
        assert!(first.rebase(&changed_layout).is_err());
        assert_ne!(
            canonical::bytes(&PlanProgramStructureSeed::from(&layout)).unwrap(),
            canonical::bytes(&PlanProgramStructureSeed::from(&changed_layout)).unwrap()
        );
        let mut bad_baseline = layout.clone();
        bad_baseline.baseline_packed[16] = 255;
        assert!(bad_baseline.validate_baseline().is_err());
        let mut bad_plan: PlanProgramLayout =
            serde_json::from_value(serde_json::to_value(&layout).unwrap()).unwrap();
        let mut payload: serde_json::Value =
            serde_json::from_slice(&bad_plan.baseline_frame_packed).unwrap();
        payload["localPlan"]["passes"] = serde_json::json!([]);
        bad_plan.baseline_frame_packed = serde_json::to_vec(&payload).unwrap();
        bad_plan.baseline_frame_hash = ContentDigest::of_bytes(&bad_plan.baseline_frame_packed);
        assert!(bad_plan.validate_baseline().is_err());
    }

    #[test]
    #[ignore = "manual warm paint lowering benchmark; run without other builds"]
    fn benchmark_paint_plan_lowering() {
        use std::{hint::black_box, time::Instant};
        for nodes in [200, 2000] {
            let template = template(nodes, 64.);
            let mut first = lower(&template, 0.5, None);
            let layout = first.take_initial_layout().unwrap();
            let literal = |value, layout: Option<&PlanProgramLayout>| {
                let paint = template
                    .bind_frame(vec![LinearColor::new(value, -0., 0.25, 1.)])
                    .unwrap();
                let packed = paint.program().packed_bytes().unwrap();
                let hash = ContentDigest::of_bytes(&packed);
                let plan = ProgramPlan::derive(paint.program()).unwrap();
                let schedule = ProgramSchedule::derive(&plan).unwrap();
                match layout {
                    Some(layout) => PlanProgram::from_lowered_for_layout(
                        ProgramId::try_from(1).unwrap(),
                        PreparedProgramKind::Motion,
                        "visual[0].program".into(),
                        hash,
                        paint.program().viewport(),
                        packed,
                        Some(paint.program().clone()),
                        paint.program().requirements().clone(),
                        PlanProgramResources::default(),
                        vec![],
                        plan,
                        schedule,
                        layout,
                    ),
                    None => PlanProgram::from_lowered(
                        ProgramId::try_from(1).unwrap(),
                        PreparedProgramKind::Motion,
                        "visual[0].program".into(),
                        hash,
                        paint.program().viewport(),
                        packed,
                        Some(paint.program().clone()),
                        paint.program().requirements().clone(),
                        PlanProgramResources::default(),
                        vec![],
                        plan,
                        schedule,
                    ),
                }
                .unwrap()
            };
            let literal_layout = literal(0.5, None).take_initial_layout().unwrap();
            for _ in 0..10 {
                black_box(lower(&template, 0.3, Some(&layout)));
                black_box(literal(0.3, Some(&literal_layout)));
            }
            for round in 0..5 {
                for parameter in if round % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                } {
                    let start = Instant::now();
                    for index in 0..120 {
                        let value = index as f32 / 120.;
                        black_box(if parameter {
                            lower(&template, value, Some(&layout))
                        } else {
                            literal(value, Some(&literal_layout))
                        });
                    }
                    println!(
                        "nodes={nodes} round={round} parameter={parameter} ms={:.6}",
                        start.elapsed().as_secs_f64() * 1000. / 120.
                    );
                }
            }
        }
    }
}
