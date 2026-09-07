mod support;

use skia_safe::{AlphaType, ColorSpace, ColorType, ImageInfo, surfaces};
use valle_engine::render::FrameKey;
use valle_engine::{
    compositor::{ExternalObjectTable, lower::RenderBindings},
    frame::{RenderQuality, RenderSpec},
    product::EngineRender,
    resource::{
        Extent2d, ExternalGeneration, ExternalPixelLayout, ExternalResourceDesc, OutputBackground,
        OutputSpec,
    },
};
use valle_render::executor::skia::{
    SkiaExternalObject, SkiaTarget, cpu_capabilities, execute_skia,
};

#[test]
fn fixed_render_image_plan_executes_with_matching_bindings() {
    let render = support::opened_image_render(support::DEFAULT_DIGEST);
    let rgba = execute_image_render(
        render,
        [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
    );
    assert!(rgba.iter().any(|channel| *channel != 0));
}

#[test]
fn admitted_extension_color_gain_executes_on_native_skia() {
    let pixels = [64, 32, 16, 255].repeat(4).try_into().unwrap();
    let baseline = execute_image_render(
        support::opened_image_render(support::DEFAULT_DIGEST),
        pixels,
    );
    let gained = execute_image_render(
        support::opened_color_gain_render(support::DEFAULT_DIGEST, 2.0),
        pixels,
    );
    assert!(
        gained[0] > baseline[0] && gained[1] > baseline[1] && gained[2] > baseline[2],
        "color gain did not brighten the native Skia result: baseline={baseline:?}, gained={gained:?}"
    );
    assert_eq!(gained[3], baseline[3]);
}

fn execute_image_render(render: EngineRender, pixels: [u8; 16]) -> [u8; 16] {
    let render_id = render.render_id();
    let output = OutputSpec::srgb_preview(OutputBackground::opaque_srgb([12, 34, 56])).unwrap();
    let spec = RenderSpec::new(2, 2, RenderQuality::Preview, output).unwrap();
    let capabilities = cpu_capabilities(
        Extent2d::new(4096, 4096).unwrap(),
        32 * 1024 * 1024,
        64 * 1024 * 1024,
    )
    .unwrap();
    let mut compiler = render.frame_compiler();
    let prepared = compiler
        .evaluate_prepare(render_id, FrameKey::new(0), spec)
        .unwrap();
    let request = prepared.prepared().resource_requests.requests()[0].clone();
    let bound = prepared
        .lower_constructed(&capabilities)
        .unwrap()
        .bind_constructed(ExternalGeneration::new(1).unwrap())
        .unwrap();
    assert_eq!(bound.evaluated().render_id(), render_id);
    assert_eq!(*bound.template().render_id(), render_id);
    assert_eq!(*bound.bindings().render_id(), render_id);
    let (template, bindings): (_, RenderBindings) = bound.into_plan();

    let ExternalResourceDesc::VisualFrame {
        extent,
        pixel_layout,
    } = request.expected()
    else {
        panic!("image request must be visual")
    };
    assert_eq!(*pixel_layout, ExternalPixelLayout::Rgba8);
    let object =
        SkiaExternalObject::visual_rgba8_as(request.key().clone(), *pixel_layout, *extent, &pixels)
            .unwrap();
    let objects = ExternalObjectTable::try_from_entries(
        ExternalGeneration::new(1).unwrap(),
        [(request.handle(), object)],
    )
    .unwrap();

    let info = ImageInfo::new(
        (2, 2),
        ColorType::RGBA8888,
        AlphaType::Premul,
        Some(ColorSpace::new_srgb()),
    );
    let mut surface = surfaces::raster(&info, None, None).unwrap();
    let mut target = SkiaTarget::new(&mut surface, output).unwrap();
    execute_skia(&template, &bindings, &capabilities, &objects, &mut target).unwrap();
    let mut rgba = [0_u8; 16];
    assert!(surface.read_pixels(&info, &mut rgba, 8, (0, 0)));
    rgba
}
