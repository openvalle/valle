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
