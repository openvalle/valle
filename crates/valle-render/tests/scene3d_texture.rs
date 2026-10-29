//! Local UI material textures must match the existing effect-input path, including seeks.
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

fn verify(backend: SkiaBackendKind) {
    let composition = "export const composition={width:240,height:160,fps:30,duration:1};";
    let base_ui = r##"<View style={{width:240,height:160,backgroundColor:'#3366aa'}}><View style={{position:'absolute',left:20,top:25,width:60,height:50,backgroundColor:'#ffbb44',translate:point(time.seconds*100,0)}}/><View style={{position:'absolute',right:12,bottom:12,width:40,height:40,backgroundColor:'#11cc66'}}/></View>"##;
    let filtered_ui = base_ui.replace(
        "width:240,height:160",
        "width:240,height:160,filter:'blur(0.6px)'",
    );
    let nested_ui = base_ui.replace("</View>", r##"<Scene3D key="inner" style={{position:'absolute',left:150,top:90,width:64,height:48}} camera={{position:[0,0,5],target:[0,0,0]}}><Mesh key="badge" geometry={extrude(path("M -1 -1 L 1 -1 L 0 1 Z"),{depth:0.1})} rotateY={time.seconds*30} material={{type:'unlit',color:'#ffffff'}}/></Scene3D></View>"##);
    for ui in [base_ui, filtered_ui.as_str(), nested_ui.as_str()] {
        let source = format!(
            "{composition} export default function Main(){{const time=useCurrentTime();return <Scene>{ui}</Scene>;}}"
        );
        let effect = include_str!(
            "../../valle-compiler/tests/fixtures/motion/composition/ui-material.motion.tsx"
        );
        let spec = RenderSpec::new(
            240,
            160,
            RenderQuality::Final,
            OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
        )
        .unwrap();
        for transform in ["", ",translate:point(-65,-12),rotate:'17deg',scale:0.85"] {
            let effect = effect.replace(
                "style={{width:240,height:160}} camera",
                &format!("style={{{{width:240,height:160{transform}}}}} camera"),
            );
            let local = effect
                .replace("export const role = effect();", "")
                .replace(
                    "const square =",
                    "const time=useCurrentTime(); const square =",
                )
                .replace(
                    "</Scene3D>",
                    &format!("<Texture3D>{ui}</Texture3D></Scene3D>"),
                );
            let local = NativeProject::from_render(
                support::opened_motion_render(&local, 240, 160),
                Arc::new(NativeResourceCatalog::new()),
            );
            let reference = NativeProject::from_render(
                support::opened_motion_effect_render(&source, Some(&effect), 240, 160),
                Arc::new(NativeResourceCatalog::new()),
            );
            let mut runner = local.frame_runner(backend).unwrap();
            let mut reference = reference.frame_runner(backend).unwrap();
            let mut baseline = None;
            for frame in [0, 15, 29, 0] {
                let (actual, evidence) = runner.render_rgba8(FrameKey::new(frame), spec).unwrap();
                let expected = reference
                    .render_rgba8(FrameKey::new(frame), spec)
                    .unwrap()
                    .0;
                assert_eq!(
                    evidence.execution.scene3d_gpu_rasters,
                    u64::from(backend.supports_scene3d_gpu())
                        * if ui.contains("<Scene3D") { 2 } else { 1 }
                );
                assert_eq!(
                    evidence.execution.scene3d_cpu_fallbacks,
                    u64::from(!backend.supports_scene3d_gpu())
                        * if ui.contains("<Scene3D") { 2 } else { 1 }
                );
                let mean = actual
                    .data
                    .iter()
                    .zip(&expected.data)
                    .map(|(a, b)| a.abs_diff(*b) as f64)
                    .sum::<f64>()
                    / actual.data.len() as f64;
                assert!(
                    mean < 1.2,
                    "local vs effect mean {mean}, frame {frame}, transform {transform}"
                );
                if let Some(first) = &baseline {
                    if frame == 0 {
                        assert_eq!(&actual.data, first, "seek must be deterministic");
                    } else {
                        assert_ne!(&actual.data, first, "current UI must animate");
                    }
                } else {
                    baseline = Some(actual.data);
                }
            }
        }
    }
}

#[test]
fn ui_texture_matches_host_material_on_cpu() {
    verify(SkiaBackendKind::Raster);
}
#[test]
#[cfg(target_os = "macos")]
#[ignore = "requires a Metal GPU"]
fn ui_texture_matches_host_material_on_metal() {
    verify(SkiaBackendKind::Metal);
}
