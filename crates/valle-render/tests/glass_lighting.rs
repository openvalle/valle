#![cfg(feature = "native")]

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

fn render(source: &str, width: u32, height: u32, backend: SkiaBackendKind) -> Vec<u8> {
    let project = NativeProject::from_render(
        support::opened_motion_render(source, width, height),
        Arc::new(NativeResourceCatalog::new()),
    );
    let spec = RenderSpec::new(
        width,
        height,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap();
    project
        .frame_runner(backend)
        .unwrap()
        .render_rgba8(FrameKey::new(0), spec)
        .unwrap()
        .0
        .data
}

fn save(name: &str, pixels: &[u8]) {
    if let Ok(directory) = std::env::var("VALLE_GLASS_LIGHTING_OUTPUT") {
        std::fs::write(std::path::Path::new(&directory).join(name), pixels).unwrap();
    }
}

fn board() -> String {
    let mut shapes = String::new();
    for (y, background) in [(0, "#26384c"), (240, "#d7e1e9")] {
        shapes += &format!(
            r##"<View style={{{{position:"absolute",left:0,top:{y},width:720,height:240,backgroundColor:"{background}"}}}}>
  <Glass material={{{{refraction:0}}}} style={{{{position:"absolute",left:38,top:44,width:96,height:96,borderRadius:48}}}} />
  <Glass material={{{{refraction:0}}}} style={{{{position:"absolute",left:180,top:44,width:140,height:96,borderRadius:22}}}} />
  <Glass material={{{{refraction:0}}}} style={{{{position:"absolute",left:366,top:74,width:24,height:24,borderRadius:12}}}} />
  <Glass material={{{{refraction:0}}}} style={{{{position:"absolute",left:444,top:54,width:100,height:72,borderRadius:18,transform:"rotate(30deg)"}}}} />
  <View style={{{{position:"absolute",left:584,top:44,width:100,height:96,transform:"perspective(250px) rotateY(32deg)"}}}}>
    <Glass material={{{{refraction:0}}}} style={{{{width:100,height:96,borderRadius:22}}}} />
  </View>
  <View style={{{{position:"absolute",left:38,top:178,width:644,height:18,backgroundImage:"linear-gradient(90deg, #ef7a68, #eec46a, #65c9bb, #6689dc)"}}}} />
  <GlassField material={{{{frost:2,refraction:8}}}} merge={{{{distance:20}}}}>
    <Glass style={{{{position:"absolute",left:72,top:158,width:110,height:64,borderRadius:20}}}} />
    <Glass style={{{{position:"absolute",left:190,top:158,width:100,height:64,borderRadius:20}}}} />
  </GlassField>
  <Glass material={{{{frost:2,refraction:8}}}} style={{{{position:"absolute",left:400,top:158,width:210,height:64,borderRadius:24}}}} />
</View>"##
        );
    }
    format!(
        "export const composition={{width:720,height:480,duration:1}}; export default function Test(){{return <Scene style={{{{width:720,height:480}}}}>{shapes}</Scene>;}}"
    )
}

#[test]
fn glass_lighting_board_on_cpu() {
    let source = board();
    let pixels = render(&source, 720, 480, SkiaBackendKind::Raster);
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 255)
    );
    save("board-cpu.rgba", &pixels);
    if let Ok(directory) = std::env::var("VALLE_GLASS_LIGHTING_OUTPUT") {
        std::fs::write(
            std::path::Path::new(&directory).join("board.motion.tsx"),
            source,
        )
        .unwrap();
    }
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "requires a hardware Metal GPU"]
fn glass_lighting_board_on_metal() {
    let pixels = render(&board(), 720, 480, SkiaBackendKind::Metal);
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 255)
    );
    save("board-metal.rgba", &pixels);
}

fn verify_lighting(backend: SkiaBackendKind) {
    let source = |direction: i32, intensity: f64| {
        format!(
            r##"
export const composition={{width:96,height:96,duration:1}};
export default function Test() {{ return <Scene style={{{{width:96,height:96,backgroundColor:"#b8ccd8"}}}}>
  <Glass material={{{{bevel:12,refraction:0}}}} light={{{{direction:point(0,{direction}),intensity:{intensity}}}}}
    style={{{{position:"absolute",left:16,top:16,width:64,height:64,borderRadius:16}}}} />
</Scene>; }}"##
        )
    };
    let unlit = render(&source(-1, 0.0), 96, 96, backend);
    let up = render(&source(-1, 0.65), 96, 96, backend);
    let down = render(&source(1, 0.65), 96, 96, backend);
    assert_eq!(
        &up[(48 * 96 + 48) * 4..][..4],
        &unlit[(48 * 96 + 48) * 4..][..4],
        "the flat clear interior must not acquire a painted highlight"
    );
    let clipped = up
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[..3].iter().all(|&v| v >= 254))
        .count();
    assert_eq!(
        clipped, 0,
        "default light must not clip the light backdrop into a white band"
    );
    let strip = |pixels: &[u8], y: usize| -> i32 {
        (32..64)
            .flat_map(|x| (0..3).map(move |c| i32::from(pixels[(y * 96 + x) * 4 + c])))
            .sum::<i32>()
            / 96
    };
    assert!(
        strip(&up, 16) > strip(&up, 79) + 3,
        "upper light must favour the upper rim"
    );
    assert!(
        strip(&down, 79) > strip(&down, 16) + 3,
        "reversing light must move the reflection"
    );
    let mut total = 0_u64;
    for y in 0..96 {
        for x in 0..96 {
            for c in 0..3 {
                total +=
                    u64::from(up[(y * 96 + x) * 4 + c].abs_diff(down[((95 - y) * 96 + x) * 4 + c]));
            }
        }
    }
    assert!(
        total as f64 / (96.0 * 96.0 * 3.0) < 0.5,
        "opposite lights must mirror consistently"
    );
    let ellipse = render(
        r##"export const composition={width:128,height:128,duration:1};
export default function Test() {return <Scene style={{width:128,height:128,backgroundColor:"#202020"}}>
 <Glass material={{bevel:8,refraction:0}} light={{direction:point(1,-1)}}
  style={{position:"absolute",left:44,top:44,width:40,height:40,borderRadius:20,transform:"scale(2, 1)"}} />
</Scene>}"##,
        128,
        128,
        backend,
    );
    let peak = |cx: usize, cy: usize| {
        (cy - 3..=cy + 3)
            .flat_map(|y| (cx - 3..=cx + 3).map(move |x| (y * 128 + x) * 4))
            .map(|i| u32::from(ellipse[i]) + u32::from(ellipse[i + 1]) + u32::from(ellipse[i + 2]))
            .max()
            .unwrap()
    };
    // For an ellipse (a=40,b=20), n=(x/a²,y/b²). Its upper-right 45° screen normal
    // is at (100,55). Inverse-transforming only the light incorrectly peaks near (82,46).
    assert!(
        peak(99, 55) > peak(82, 46) + 12,
        "scaled glass must peak towards the screen light: {} vs {}",
        peak(99, 55),
        peak(82, 46)
    );
}

#[test]
fn bounded_directional_glass_light_on_cpu() {
    verify_lighting(SkiaBackendKind::Raster);
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "requires a hardware Metal GPU"]
fn bounded_directional_glass_light_on_metal() {
    verify_lighting(SkiaBackendKind::Metal);
}

fn verify_liquid(backend: SkiaBackendKind) {
    let source = include_str!(
        "../../valle-compiler/tests/fixtures/motion/composition/glass-liquid.motion.tsx"
    );
    let project = NativeProject::from_render(
        support::opened_motion_render(source, 960, 540),
        Arc::new(NativeResourceCatalog::new()),
    );
    let spec = RenderSpec::new(
        960,
        540,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap();
    let mut runner = project.frame_runner(backend).unwrap();
    let mut frames = Vec::new();
    for index in [0, 15, 29, 0] {
        frames.push(
            runner
                .render_rgba8(FrameKey::new(index), spec)
                .unwrap()
                .0
                .data,
        );
    }
    assert_eq!(
        frames[0], frames[3],
        "moving glass must be seek independent"
    );
    assert_ne!(frames[0], frames[1], "the lens and backdrop must animate");
    let backend_name = if backend == SkiaBackendKind::Raster {
        "cpu"
    } else {
        "metal"
    };
    for (index, frame) in [0, 15, 29].iter().zip(&frames) {
        save(&format!("liquid-{backend_name}-{index}.rgba"), frame);
    }
    if std::env::var_os("VALLE_GLASS_ANIMATION_OUTPUT").is_some()
        && backend != SkiaBackendKind::Raster
    {
        for frame in 0..30 {
            let pixels = runner
                .render_rgba8(FrameKey::new(frame), spec)
                .unwrap()
                .0
                .data;
            save(&format!("liquid-metal-{frame:02}.frame"), &pixels);
        }
    }
}

#[test]
fn liquid_glass_motion_on_cpu() {
    verify_liquid(SkiaBackendKind::Raster);
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "requires a hardware Metal GPU"]
fn liquid_glass_motion_on_metal() {
    verify_liquid(SkiaBackendKind::Metal);
}
