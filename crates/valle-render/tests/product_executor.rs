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

#[test]
fn shader_language_executes_bounded_math_and_sanitizes_invalid_values() {
    use skia_safe::{Data, Paint, RuntimeEffect, shaders};
    use valle_motion::shader::{
        BudgetClass, OutputContract, ShaderManifest, ShaderPackage, ShaderUniform, UniformBindings,
        UniformType, UniformValue,
    };
    let descriptor = ShaderManifest {
        name: "math-check".into(),
        entry: "source.vsksl".into(),
        inputs: vec![],
        uniforms: vec![ShaderUniform {
            name: "tint".into(),
            uniform_type: UniformType::Color,
            required: false,
            default: Some(UniformValue::Color([1.0, 0.0, 0.0, 0.0])),
            min: None,
            max: None,
        }],
        output: OutputContract::default(),
        budget: BudgetClass::Local,
    };
    // Grayscale avoids any dependence on the target's color primaries. RGB=2 checks that
    // the shader boundary does not clip to the display range; alpha remains coverage.
    let source = br#"
        float4 valle_main(float2 uv) {
            float sum = 0.0;
            const int n = 3;
            for (int i = 0; i < n; ++i) { sum += later(float(i)); }
            float2x2 identity = float2x2(1.0);
            float2 p = identity * uv;
            float invalid = sqrt(-p.x) + pow(-p.x, 0.5) + 1.0 / (p.y-p.y);
            float2 zero = normalize(float2(0.0));
            if (p.x < 0.34) { return float4(float3(sum / 1.5 + invalid + zero.x), 0.5); }
            if (p.x < 0.67) { return float4(tint.rgb, 1.0); }
            return float4(exp(1000.0), exp(1000.0), exp(1000.0), 1.0);
        }
        float later(float n) { if (n > 0.0) return n; else return 0.0; }
    "#;
    let package = ShaderPackage::compile(descriptor, source).unwrap();
    let effect = RuntimeEffect::make_for_shader(&package.generated_sksl, None)
        .unwrap_or_else(|error| panic!("{error}\n{}", package.generated_sksl));
    let uniforms = package
        .pack_uniforms(
            [3.0, 1.0],
            &std::collections::BTreeMap::new(),
            &UniformBindings::empty(),
        )
        .unwrap();
    let shader = effect
        .make_shader(Data::new_copy(&uniforms), &[shaders::empty().into()], None)
        .unwrap();
    let info = ImageInfo::new((3, 1), ColorType::RGBAF32, AlphaType::Premul, None);
    let render = || {
        let mut surface = surfaces::raster(&info, None, None).unwrap();
        let mut paint = Paint::default();
        paint.set_shader(shader.clone());
        surface.canvas().draw_paint(&paint);
        let mut bytes = [0u8; 48];
        assert!(surface.read_pixels(&info, &mut bytes, 48, (0, 0)));
        bytes
    };
    let first = render();
    assert_eq!(first, render());
    let values = first
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
        .collect::<Vec<_>>();
    assert!(values.iter().all(|v| v.is_finite()), "{values:?}");
    for channel in &values[..3] {
        assert!((*channel - 1.0).abs() < 0.002, "{values:?}");
    }
    assert_eq!(values[3], 0.5);
    for (value, expected) in values[4..8]
        .iter()
        .zip([0.6274039, 0.06909729, 0.01639144, 1.0])
    {
        assert!((value - expected).abs() < 0.00001, "{values:?}");
    }
    assert_eq!(&values[8..], &[0.0, 0.0, 0.0, 1.0]);
}
