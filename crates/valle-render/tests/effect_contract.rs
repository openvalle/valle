#![cfg(feature = "motion")]
#[path = "../../valle-engine/tests/test_support/executor.rs"]
mod support;

use skia_safe::{AlphaType, ColorSpace, ColorType, ImageInfo, surfaces};
use valle_draw::program::Filter;
use valle_engine::{
    compositor::{
        ExternalObjectTable,
        graph::build_render_graph,
        lower::{RenderBindings, lower_render_graph},
    },
    prepare::{DynamicBinding, DynamicBindings, PreparedFrame},
    resource::{Extent2d, ExternalGeneration},
};
use valle_render::executor::skia::{SkiaTarget, cpu_capabilities, execute_skia};

fn render(frame: &PreparedFrame, values: Vec<DynamicBinding>) -> Vec<u8> {
    frame.validate().unwrap();
    let caps = cpu_capabilities(
        Extent2d::new(4096, 4096).unwrap(),
        32 * 1024 * 1024,
        64 * 1024 * 1024,
    )
    .unwrap();
    let dynamic: DynamicBindings =
        serde_json::from_value(serde_json::to_value(values).unwrap()).unwrap();
    let graph = build_render_graph(frame).unwrap();
    let plan = lower_render_graph(&graph, &dynamic, &caps).unwrap();
    let generation = ExternalGeneration::new(1).unwrap();
    let bindings = RenderBindings::new(
        frame.render_id,
        plan.template_hash().unwrap(),
        dynamic,
        generation,
        vec![],
        plan.programs().to_vec(),
    )
    .unwrap();
    let objects = ExternalObjectTable::try_from_entries(generation, []).unwrap();
    let info = ImageInfo::new(
        (32, 32),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        Some(ColorSpace::new_srgb()),
    );
    let mut surface = surfaces::raster(&info, None, None).unwrap();
    let mut target = SkiaTarget::new(&mut surface, frame.render_spec.output()).unwrap();
    execute_skia(&plan, &bindings, &caps, &objects, &mut target).unwrap();
    let mut pixels = vec![0; 32 * 32 * 4];
    assert!(surface.read_pixels(&info, &mut pixels, 32 * 4, (0, 0)));
    pixels
}

#[test]
fn native_effects_execute_in_layer_and_root_spaces_with_numeric_oracles() {
    let (frame, values) = support::fixture(support::solid_program(vec![]));
    let baseline = render(&frame, values);
    for root in [false, true] {
        for kind in 0..7 {
            for region in [false, true] {
                let (frame, values) = support::effect_fixture(kind, root, region);
                let pixels = render(&frame, values.clone());
                assert_eq!(pixels, render(&frame, values));
                assert_eq!(pixels[3], 0);
                if kind == 6 {
                    let center = (16 * 32 + 16) * 4;
                    assert!(pixels[center] > baseline[center]);
                    assert!(pixels[center + 1] > baseline[center + 1]);
                    assert_eq!(pixels[center + 3], baseline[center + 3]);
                }
            }
        }
    }
}

#[test]
fn native_geometric_mask_cuts_and_inverts_alpha() {
    for ellipse in [false, true] {
        for feather in [0.0, 1.0] {
            for invert in [false, true] {
                let (frame, values) = support::mask_fixture(ellipse, feather, invert);
                let pixels = render(&frame, values);
                let center = pixels[(16 * 32 + 16) * 4 + 3];
                let outside = pixels[(6 * 32 + 6) * 4 + 3];
                if invert {
                    assert_eq!(center, 0);
                    assert!(
                        outside > 220,
                        "ellipse={ellipse}, feather={feather}, invert={invert}: center={center}, outside={outside}"
                    );
                } else {
                    assert!(center > 240);
                    assert!(
                        outside < 35,
                        "ellipse={ellipse}, feather={feather}, invert={invert}: center={center}, outside={outside}"
                    );
                }
                if ellipse && feather == 0.0 && !invert {
                    assert!(pixels[(9 * 32 + 9) * 4 + 3] < 5);
                }
            }
        }
    }
}

#[test]
fn native_css_filters_obey_opacity_and_color_oracles() {
    let (frame, values) = support::fixture(support::solid_program(vec![]));
    let baseline = render(&frame, values);
    let filters = [
        Filter::Brightness { amount: 0.5 },
        Filter::Contrast { amount: 0.5 },
        Filter::Grayscale { amount: 1.0 },
        Filter::HueRotate { degrees: 45.0 },
        Filter::Invert { amount: 0.5 },
        Filter::Opacity { amount: 0.5 },
        Filter::Saturate { amount: 0.5 },
        Filter::Sepia { amount: 0.5 },
        Filter::NoiseDisplacement {
            frequency: [0.1, 0.1],
            octaves: 2,
            seed: 19,
            scale: 1.0,
            turbulence: true,
        },
        Filter::VelocityBlur {
            velocity: [2.0, 0.0],
            shutter_angle_degrees: 180.0,
        },
    ];
    for filter in filters {
        let (frame, values) = support::fixture(support::solid_program(vec![filter.clone()]));
        let pixels = render(&frame, values.clone());
        assert_eq!(pixels, render(&frame, values));
        let center = (16 * 32 + 16) * 4;
        match filter {
            Filter::Opacity { .. } => assert!((i16::from(pixels[center + 3]) - 128).abs() <= 1),
            Filter::Grayscale { .. } => {
                assert_eq!(pixels[center], pixels[center + 1]);
                assert_eq!(pixels[center + 1], pixels[center + 2]);
            }
            Filter::Brightness { .. } => assert!(pixels[center] < baseline[center]),
            _ => assert_eq!(pixels[center + 3], 255),
        }
    }
}
