mod support;

use std::sync::Arc;
use valle_engine::{
    frame::{RenderQuality, RenderSpec},
    render::FrameKey,
    resource::{OutputBackground, OutputSpec},
};
use valle_render::{
    executor::skia::SkiaBackendKind,
    host::{NativeProject, NativeResourceCatalog},
};

fn render(child: &str, backend: SkiaBackendKind) -> Vec<u8> {
    let source = format!(
        r##"
export const composition = {{width:160,height:120,duration:1}};
export default function Scene() {{
 return <Scene style={{{{width:160,height:120,backgroundColor:"#224488"}}}}>
   <View style={{{{position:"absolute",left:0,top:60,width:160,height:60,backgroundColor:"#ee9922"}}}} />
   <View style={{{{position:"absolute",left:70,top:0,width:8,height:120,backgroundColor:"#ffffff"}}}} />
   {child}
 </Scene>;
}}
"##
    );
    let project = NativeProject::from_render(
        support::opened_motion_render(&source, 160, 120),
        Arc::new(NativeResourceCatalog::new()),
    );
    let spec = RenderSpec::new(
        160,
        120,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap();
    let mut runner = project.frame_runner(backend).unwrap();
    let first = runner.render_rgba8(FrameKey::new(0), spec).unwrap().0.data;
    for index in [29, 7, 0] {
        let again = runner
            .render_rgba8(FrameKey::new(index), spec)
            .unwrap()
            .0
            .data;
        assert_eq!(first, again, "static optics must be seek independent");
    }
    first
}

fn verify(backend: SkiaBackendKind) {
    let baseline = render("", backend);
    for child in [
        r#"<Glass material={{frost:3,refraction:5}} style={{width:100,height:70,borderRadius:12}} />"#,
        r#"<GlassField material={{frost:3,refraction:5}}><Glass style={{width:70,height:70,borderRadius:12}}/><Glass style={{position:"absolute",left:50,top:10,width:50,height:50,borderRadius:12}}/></GlassField>"#,
    ] {
        let nested = format!(
            r#"<View style={{{{position:"absolute",left:30,top:25,width:100,height:70,transform:"perspective(300px) rotateY(25deg)"}}}}>{child}</View>"#
        );
        let actual = render(&nested, backend);
        assert!(actual.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        assert_eq!(
            &actual[..160 * 4],
            &baseline[..160 * 4],
            "optics must not alter distant backdrop"
        );
        let changes = actual
            .as_chunks::<4>()
            .0
            .iter()
            .zip(baseline.as_chunks::<4>().0)
            .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(a, b)| a.abs_diff(*b) > 8))
            .count();
        assert!(
            changes > 200,
            "visible frost/refraction required: {changes}"
        );
        if backend == SkiaBackendKind::Raster
            && let Ok(directory) = std::env::var("VALLE_MOTION_PACKAGE_TEST_OUTPUT")
        {
            std::fs::write(
                std::path::Path::new(&directory).join("glass-projection-reference.rgba"),
                &actual,
            )
            .unwrap();
        }
    }
}

#[test]
fn projected_glass_and_field_render_on_cpu() {
    verify(SkiaBackendKind::Raster);
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "requires a hardware Metal GPU"]
fn projected_glass_and_field_render_on_metal() {
    verify(SkiaBackendKind::Metal);
}

fn verify_dielectric(backend: SkiaBackendKind) {
    let source = include_str!(
        "../../valle-compiler/tests/fixtures/motion/composition/transmission.motion.tsx"
    );
    let project = NativeProject::from_render(
        support::opened_motion_render(source, 192, 128),
        Arc::new(NativeResourceCatalog::new()),
    );
    let spec = RenderSpec::new(
        192,
        128,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap();
    let mut runner = project.frame_runner(backend).unwrap();
    let first = runner.render_rgba8(FrameKey::new(0), spec).unwrap().0.data;
    let middle = runner.render_rgba8(FrameKey::new(15), spec).unwrap().0.data;
    assert!(
        first
            .iter()
            .zip(&middle)
            .filter(|(a, b)| a.abs_diff(**b) > 10)
            .count()
            > 100
    );
    runner.render_rgba8(FrameKey::new(29), spec).unwrap();
    assert_eq!(
        first,
        runner.render_rgba8(FrameKey::new(0), spec).unwrap().0.data
    );
    if backend == SkiaBackendKind::Raster
        && let Ok(directory) = std::env::var("VALLE_MOTION_PACKAGE_TEST_OUTPUT")
    {
        std::fs::write(
            std::path::Path::new(&directory).join("transmission-product-reference.rgba"),
            middle,
        )
        .unwrap();
    }
}

#[test]
fn dielectric_motion_renders_on_cpu() {
    verify_dielectric(SkiaBackendKind::Raster);
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "requires a hardware Metal GPU"]
fn dielectric_motion_renders_on_metal() {
    verify_dielectric(SkiaBackendKind::Metal);
}

fn verify_ui_material(backend: SkiaBackendKind) {
    let source = r##"
export const composition={width:240,height:160,duration:1};
export default function UI() { const t=useCurrentTime();return <Scene style={{width:240,height:160,backgroundColor:"#163a66"}}>
  <View style={{position:"absolute",left:10,top:10,width:220,height:140,borderRadius:14,backgroundColor:"#327799"}} />
  <View style={{position:"absolute",left:22,top:25,width:75,height:8,backgroundColor:"#ffffff"}} />
  <View style={{position:"absolute",left:22,top:46,width:120,height:5,backgroundColor:"#aaccee"}} />
  <View style={{position:"absolute",left:22,top:80,width:35,height:35,borderRadius:10,backgroundColor:"#ee5588",translate:point(t.seconds*145,0)}} />
</Scene>}
"##;
    let effect = include_str!(
        "../../valle-compiler/tests/fixtures/motion/composition/ui-material.motion.tsx"
    );
    let project = NativeProject::from_render(
        support::opened_motion_effect_render(source, Some(effect), 240, 160),
        Arc::new(NativeResourceCatalog::new()),
    );
    let spec = RenderSpec::new(
        240,
        160,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap();
    let mut runner = project.frame_runner(backend).unwrap();
    let first = runner.render_rgba8(FrameKey::new(0), spec).unwrap().0.data;
    let (output, evidence) = runner.render_rgba8(FrameKey::new(15), spec).unwrap();
    if backend.supports_scene3d_gpu() {
        assert_eq!(evidence.execution.scene3d_gpu_rasters, 1);
        assert_eq!(evidence.execution.scene3d_cpu_fallbacks, 0);
    }
    let middle = output.data;
    assert!(
        first
            .iter()
            .zip(&middle)
            .filter(|(a, b)| a.abs_diff(**b) > 20)
            .count()
            > 300,
        "current UI must animate even when Scene3D request is constant"
    );
    for frame in [29, 3, 0] {
        let actual = runner
            .render_rgba8(FrameKey::new(frame), spec)
            .unwrap()
            .0
            .data;
        if frame == 0 {
            assert_eq!(
                first, actual,
                "current materials must not retain the preceding host picture"
            );
        }
    }
    let white_rows = |range: std::ops::Range<usize>| {
        range
            .flat_map(|y| (20..210).map(move |x| (y * 240 + x) * 4))
            .filter(|&i| middle[i..i + 3].iter().all(|&v| v > 230))
            .count()
    };
    assert!(
        white_rows(10..80) > 20 && white_rows(80..150) == 0,
        "UI cap texture must stay upright"
    );
    let center = &middle[(80 * 240 + 120) * 4..][..3];
    assert!(
        center[0] > 240 && center[1] > 150 && center[2] < 70,
        "foreground mesh occludes UI: {center:?}"
    );
    let pixel = &middle[(80 * 240 + 115) * 4..][..3];
    assert!(
        pixel[1] > 50 && pixel[2] > 80,
        "UI occludes the rear red mesh: {pixel:?}"
    );
    if backend == SkiaBackendKind::Raster
        && let Ok(directory) = std::env::var("VALLE_MOTION_PACKAGE_TEST_OUTPUT")
    {
        std::fs::write(
            std::path::Path::new(&directory).join("ui-material-reference.rgba"),
            middle,
        )
        .unwrap();
    }
}

#[test]
fn current_ui_material_shares_depth_on_cpu() {
    verify_ui_material(SkiaBackendKind::Raster);
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "requires a hardware Metal GPU"]
fn current_ui_material_shares_depth_on_metal() {
    verify_ui_material(SkiaBackendKind::Metal);
}
