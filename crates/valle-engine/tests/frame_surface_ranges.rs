#[path = "test_support/executor.rs"]
mod support;

use valle_engine::{
    compositor::{
        graph::build_render_graph,
        lower::{
            BackendCapabilities, ProgramBindingError, RenderBindings, RenderPlanTemplate,
            lower_render_graph,
        },
    },
    prepare::{DeviceRect, DeviceTransform, DynamicBindings, DynamicValue},
    resource::ExternalGeneration,
};

fn dynamic(scale: f64) -> DynamicBindings {
    let (_, mut values) = support::fixture(support::solid_program(vec![]));
    values[0].value = DynamicValue::DeviceTransform(DeviceTransform::from_affine([
        32.0 * scale,
        0.0,
        0.0,
        0.0,
        32.0 * scale,
        0.0,
    ]));
    values[1].value = DynamicValue::Bounds(DeviceRect::new(
        (4.0 * scale) as i32,
        (4.0 * scale) as i32,
        (24.0 * scale) as u32,
        (24.0 * scale) as u32,
    ));
    serde_json::from_value(serde_json::to_value(values).unwrap()).unwrap()
}

fn lower(capabilities: &BackendCapabilities, scale: f64) -> RenderPlanTemplate {
    let (frame, _) = support::fixture(support::solid_program(vec![]));
    lower_render_graph(
        &build_render_graph(&frame).unwrap(),
        &dynamic(scale),
        capabilities,
    )
    .unwrap()
}

fn bindings(template: &RenderPlanTemplate, scale: f64) -> RenderBindings {
    RenderBindings::new(
        *template.render_id(),
        template.template_hash().unwrap(),
        dynamic(scale),
        ExternalGeneration::new(1).unwrap(),
        vec![],
        template.programs().to_vec(),
    )
    .unwrap()
}

fn budget(surface: u64, frame: u64) -> BackendCapabilities {
    let mut wire = serde_json::to_value(support::capabilities()).unwrap();
    wire["maxSurfaceBytes"] = surface.into();
    wire["maxFrameBytes"] = frame.into();
    serde_json::from_value(wire).unwrap()
}

#[test]
fn template_is_extent_free_and_ranges_are_independent_of_seek_order() {
    let capabilities = support::capabilities();
    let template = lower(&capabilities, 0.25);
    let wire = serde_json::to_value(&template).unwrap();
    assert!(wire.get("estimatedPeakSurfaceBytes").is_none());
    for slot in wire["surfaceSlots"].as_array().unwrap() {
        assert!(slot.get("texture").is_none());
        assert!(slot.get("estimatedBytes").is_none());
        assert!(slot["surfaceClass"].get("extent").is_none());
    }
    for scale in [0.25, 1.0, 0.5, 0.0, 1.0, 0.25] {
        let cold = lower(&capabilities, scale);
        assert_eq!(
            template.packed_bytes().unwrap(),
            cold.packed_bytes().unwrap()
        );
        let bound = template
            .bind_program_schedules(&bindings(&template, scale), &capabilities)
            .unwrap();
        let independent = cold
            .bind_program_schedules(&bindings(&cold, scale), &capabilities)
            .unwrap();
        assert_eq!(bound, independent);
        assert!(bound.surface_slots().iter().any(|slot| {
            slot.extent().is_some_and(|extent| {
                extent.width() == (24.0 * scale) as u32 && extent.height() == (24.0 * scale) as u32
            }) || (scale == 0.0 && slot.extent().is_none())
        }));
    }
}

#[test]
fn frame_budget_uses_current_ranges_and_rejects_growth_on_a_cached_template() {
    let capabilities = support::capabilities();
    let template = lower(&capabilities, 0.25);
    let bytes = |scale| {
        template
            .bind_program_schedules(&bindings(&template, scale), &capabilities)
            .unwrap()
            .estimated_peak_surface_bytes()
    };
    let small = bytes(0.25);
    let large = bytes(1.0);
    // Two root composites overlap the regional program surface at the composition pass.
    assert_eq!(small, 2 * 32 * 32 * 8 + 6 * 6 * 8);
    assert_eq!(large, 2 * 32 * 32 * 8 + 24 * 24 * 8);
    let capabilities = budget(32 * 32 * 8, small);
    // Planning does not reject the root-sized logical declarations. Only the current bound
    // range schedule decides whether physical surfaces fit, even after a successful cache hit.
    let template = lower(&capabilities, 0.25);
    for scale in [0.25, 1.0, 0.25, 1.0] {
        let result = template.bind_program_schedules(&bindings(&template, scale), &capabilities);
        if scale == 0.25 {
            assert_eq!(result.unwrap().estimated_peak_surface_bytes(), small);
        } else {
            assert!(matches!(result,
                Err(ProgramBindingError::FrameBudgetExceeded { required_bytes, max_bytes })
                    if required_bytes == large && max_bytes == small
            ));
        }
    }
}

#[test]
fn root_surface_still_requires_its_actual_bytes_at_binding() {
    let capabilities = budget(32 * 32 * 8 - 1, 64 * 1024);
    let template = lower(&capabilities, 0.25);
    assert!(matches!(
        template.bind_program_schedules(&bindings(&template, 0.25), &capabilities),
        Err(ProgramBindingError::PlanSurfaceBudgetExceeded { required_bytes, .. })
            if required_bytes == 32 * 32 * 8
    ));
}
