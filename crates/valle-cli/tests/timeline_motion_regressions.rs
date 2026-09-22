//! Exercise authored inputs through the actual CLI, package admission and native delivery.
use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Output},
};

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_valle"))
        .current_dir(dir)
        .env("VALLE_HOME", dir.join("home"))
        .args(["--json"])
        .args(args)
        .output()
        .unwrap()
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn put(dir: &Path, name: &str, value: Value) {
    std::fs::write(dir.join(name), value.to_string()).unwrap();
}
fn png(dir: &Path, name: &str, color: [u8; 3]) {
    let file = std::fs::File::create(dir.join(name)).unwrap();
    let mut encoder = png::Encoder::new(file, 8, 8);
    encoder.set_color(png::ColorType::Rgb);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&color.repeat(64)).unwrap();
}
fn pixel(dir: &Path, name: &str, x: usize, y: usize) -> Vec<u8> {
    let frame = valle_media::codec::read_rgba_png(&dir.join(name)).unwrap();
    let offset = (y * frame.width as usize + x) * 4;
    frame.data[offset..offset + 3].to_vec()
}

#[test]
fn environment_assets_are_frozen_and_render_in_arbitrary_frame_order() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../valle-motion/tests/fixtures/scene3d/triangle.glb"),
        dir.join("model.glb"),
    )
    .unwrap();
    let write_environment = |changed: bool| {
        let mut encoder =
            png::Encoder::new(std::fs::File::create(dir.join("sky.png")).unwrap(), 16, 8);
        encoder.set_color(png::ColorType::Rgb);
        let pixels = (0..128)
            .flat_map(|i| {
                if changed {
                    [0, 255, 0]
                } else if i % 16 < 8 {
                    [255, 32, 0]
                } else {
                    [0, 32, 255]
                }
            })
            .collect::<Vec<_>>();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    };
    write_environment(false);
    png(dir, "map.png", [128, 128, 255]);
    let source = r##"export const composition = { width: 64, height: 64, fps: 30, duration: 3 };
export const controls=({assets:{model:asset({kind:"model3d"}),sky:asset({kind:"environment"}),map:asset({kind:"image"})}});
export default function T(ctx){return <Scene style={{width:64,height:64}}><Scene3D key="scene" camera={{position:[ctx.seconds*0.1,0,4],target:[0,ctx.seconds*0.05,0],near:0.1+ctx.seconds*0.01,far:10+ctx.seconds}} pbr={{environment:{src:"asset://sky",intensity:1,rotation:ctx.seconds*90,background:true},toneMapping:"aces",exposure:0.7+ctx.seconds*0.2}} style={{width:64,height:64}}><Mesh key="mesh" src="asset://model" nodes={[{id:0,position:[ctx.seconds*0.01,0,0],rotation:[0,ctx.seconds*5,0],scale:[-1,1,1]}]} material={{type:"pbr",color:interpolate(ctx.seconds,[0,2],["#ffaaaa","#aaaaff"]),metallic:ctx.seconds*0.2,roughness:0.3+ctx.seconds*0.2,emissive:"#ffffff",emissiveIntensity:ctx.seconds*0.1,normalScale:ctx.seconds*0.2,textures:{baseColor:"asset://map",normal:"asset://map",emissive:"asset://map"}}}/><DirectionalLight color={interpolate(ctx.seconds,[0,2],["#ff0000","#0000ff"])} direction={[ctx.seconds*0.1,0,1]} intensity={0.5+ctx.seconds*0.1}/></Scene3D></Scene>;}"##;
    std::fs::write(dir.join("scene.motion.tsx"), source).unwrap();
    put(
        dir,
        "scene.timeline.json",
        json!({"canvas":{"width":64,"height":64,"fps":30},"resources":{"scene":"scene.motion.tsx","model":"model.glb","sky":"sky.png","map":"map.png"},"tracks":{"visual":[{"clips":[{"kind":"motion","component":"scene","start":0,"duration":3,"resources":{"model":"model","sky":"sky","map":"map"}}]}]}}),
    );
    let render = |frame: u32, name: &str| {
        run(
            dir,
            &[
                "motion",
                "render",
                "scene.motion.tsx",
                "--asset",
                "model=model.glb",
                "--asset",
                "sky=sky.png",
                "--asset",
                "map=map.png",
                "--frame",
                &frame.to_string(),
                "--backend",
                "raster",
                "-o",
                name,
            ],
        )
    };
    let mut frames = std::collections::BTreeMap::new();
    for (index, frame) in [60, 0, 30, 60].into_iter().enumerate() {
        let name = format!("frame-{index}.png");
        success(render(frame, &name));
        let pixels = valle_media::codec::read_rgba_png(&dir.join(name))
            .unwrap()
            .data;
        assert!(
            pixels.chunks_exact(4).all(|p| p[3] == 255),
            "environment background must cover the scene"
        );
        if let Some(previous) = frames.insert(frame, pixels.clone()) {
            assert_eq!(previous, pixels);
        }
    }
    assert_ne!(frames[&0], frames[&60]);
    std::thread::scope(|scope| {
        let a = scope.spawn(|| success(render(60, "parallel-a.png")));
        let b = scope.spawn(|| success(render(60, "parallel-b.png")));
        a.join().unwrap();
        b.join().unwrap();
    });
    for name in ["parallel-a.png", "parallel-b.png"] {
        assert_eq!(
            frames[&60],
            valle_media::codec::read_rgba_png(&dir.join(name))
                .unwrap()
                .data
        );
    }
    success(run(
        dir,
        &[
            "timeline",
            "render",
            "scene.timeline.json",
            "--frame",
            "60",
            "-o",
            "timeline.png",
        ],
    ));
    assert_eq!(
        frames[&60],
        valle_media::codec::read_rgba_png(&dir.join("timeline.png"))
            .unwrap()
            .data
    );
    write_environment(true);
    success(render(60, "changed.png"));
    assert_ne!(
        frames[&60],
        valle_media::codec::read_rgba_png(&dir.join("changed.png"))
            .unwrap()
            .data
    );
    write_environment(false);
    png(dir, "map.png", [255, 128, 128]);
    success(render(60, "changed-map.png"));
    assert_ne!(
        frames[&60],
        valle_media::codec::read_rgba_png(&dir.join("changed-map.png"))
            .unwrap()
            .data
    );
}

#[test]
fn model3d_assets_render_from_motion_and_timeline() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../valle-motion/tests/fixtures/scene3d");
    for file in ["triangle.glb", "narrow.glb"] {
        std::fs::copy(fixture.join(file), dir.join(file)).unwrap();
    }
    // The entry owns the delivery contract now: restate the 64x64 canvas, 10 fps and 0.6s the
    // removed flags used to pass so standalone frames still line up with `model.json`.
    let triangle = std::fs::read_to_string(fixture.join("triangle.motion.tsx"))
        .unwrap()
        .lines()
        .map(|line| {
            if line.starts_with("export const composition") {
                "export const composition = { width: 64, height: 64, fps: 10, duration: 0.6 };"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dir.join("triangle.motion.tsx"), triangle).unwrap();
    put(
        dir,
        "model.json",
        json!({"canvas":{"width":64,"height":64,"fps":10},
        "resources":{"motion":"triangle.motion.tsx","model":"triangle.glb"},
        "tracks":{"visual":[{"clips":[{"kind":"motion","component":"motion","start":0,"duration":0.6,"resources":{"model":"model"}}]}]}}),
    );
    let mut frames = std::collections::BTreeMap::new();
    for (request, frame) in [3, 0, 5, 3].into_iter().enumerate() {
        let standalone = format!("motion-{request}.png");
        let timeline = format!("timeline-{request}.png");
        success(run(
            dir,
            &[
                "motion",
                "render",
                "triangle.motion.tsx",
                "--asset",
                "model=triangle.glb",
                "--frame",
                &frame.to_string(),
                "-o",
                &standalone,
            ],
        ));
        success(run(
            dir,
            &[
                "timeline",
                "render",
                "model.json",
                "--frame",
                &frame.to_string(),
                "-o",
                &timeline,
            ],
        ));
        let pixels = valle_media::codec::read_rgba_png(&dir.join(standalone))
            .unwrap()
            .data;
        assert!(
            pixels
                .chunks_exact(4)
                .filter(|pixel| pixel[0] > 150)
                .count()
                > 100,
            "model must produce visible geometry"
        );
        assert_eq!(
            pixels,
            valle_media::codec::read_rgba_png(&dir.join(timeline))
                .unwrap()
                .data
        );
        if let Some(previous) = frames.insert(frame, pixels.clone()) {
            assert_eq!(previous, pixels);
        }
    }
    assert_ne!(frames[&0], frames[&3], "frame time must change model pose");
    assert_ne!(frames[&3], frames[&5]);
    success(run(
        dir,
        &[
            "motion",
            "render",
            "triangle.motion.tsx",
            "--asset",
            "model=narrow.glb",
            "--frame",
            "3",
            "-o",
            "narrow.png",
        ],
    ));
    assert_ne!(
        frames[&3],
        valle_media::codec::read_rgba_png(&dir.join("narrow.png"))
            .unwrap()
            .data,
        "resource replacement must change pixels"
    );
    for (command, input, extra) in [
        (
            "motion",
            "triangle.motion.tsx",
            vec!["--asset", "model=triangle.glb"],
        ),
        ("timeline", "model.json", vec![]),
    ] {
        let output = format!("{command}.mp4");
        let mut args = vec![command, "render", input];
        args.extend(extra);
        args.extend(["-o", &output]);
        let report = success(run(dir, &args));
        assert_eq!(report["delivery"]["frames"], 6);
        let decode = Command::new("ffmpeg")
            .current_dir(dir)
            .args(["-v", "error", "-xerror", "-i", &output, "-f", "null", "-"])
            .output()
            .unwrap();
        assert!(
            decode.status.success(),
            "{}",
            String::from_utf8_lossy(&decode.stderr)
        );
    }
    for (file, bytes, diagnostic) in [
        ("corrupt.glb", vec![0; 32], "magic"),
        (
            "oversize.glb",
            vec![0; 16 * 1024 * 1024 + 1],
            "GLB bytes must be <=",
        ),
    ] {
        std::fs::write(dir.join(file), bytes).unwrap();
        let report = run(
            dir,
            &[
                "motion",
                "check",
                "triangle.motion.tsx",
                "--asset",
                &format!("model={file}"),
            ],
        );
        assert!(!report.status.success());
        assert!(
            format!(
                "{}{}",
                String::from_utf8_lossy(&report.stdout),
                String::from_utf8_lossy(&report.stderr)
            )
            .contains(diagnostic)
        );
    }
}

#[test]
fn multiple_motion_components_share_fonts_and_keep_later_formula_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let mut resources = serde_json::Map::new();
    let mut clips = Vec::new();
    for index in 0..8 {
        let name = format!("card{index}");
        let file = format!("{name}.motion.tsx");
        std::fs::write(dir.join(&file), format!(r##"
export const composition = {{ width: 160, height: 96, fps: 10, duration: 9 }};
export default function Card(ctx) {{return <Scene style={{{{width:160,height:96}}}}>
  <Text style={{{{fontSize:20,color:"#ffffff"}}}}>Card {index}</Text>
  <MathFormula latex="\\frac{{1}}{{2}}" style={{{{position:"absolute",left:8,top:40,fontSize:24,color:"#ffffff",opacity:ctx.localFrame > 0 ? 1 : 0}}}} />
</Scene>;}}
"##)).unwrap();
        resources.insert(name.clone(), json!(file));
        clips.push(json!({"kind":"motion","component":name,"start":index,"duration":1}));
    }
    put(
        dir,
        "cards.json",
        json!({"canvas":{"width":160,"height":96,"fps":10,"background":"#000000"},
        "resources":resources,"tracks":{"visual":[{"clips":clips}]}}),
    );
    success(run(dir, &["timeline", "check", "cards.json"]));
    for (frame, file) in [
        (71, "later.png"),
        (0, "initial.png"),
        (1, "formula.png"),
        (71, "repeat.png"),
    ] {
        success(run(
            dir,
            &[
                "timeline",
                "render",
                "cards.json",
                "--frame",
                &frame.to_string(),
                "-o",
                file,
            ],
        ));
    }
    let load = |name: &str| valle_media::codec::read_rgba_png(&dir.join(name)).unwrap();
    assert_eq!(load("later.png").data, load("repeat.png").data);
    let formula_ink = |name: &str| {
        let frame = load(name);
        frame
            .data
            .chunks_exact(4)
            .enumerate()
            .filter(|(i, p)| i / frame.width as usize >= 40 && p[0] > 128)
            .count()
    };
    assert_eq!(formula_ink("initial.png"), 0);
    assert!(formula_ink("formula.png") > 20);
    assert!(formula_ink("later.png") > 20);
}

#[test]
fn motion_video_offsets_and_rates_select_the_numbered_source_frame() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    // Encode the source frame number as eight black/white bars. This remains readable through
    // H.264 and color conversion, so the assertion checks media sampling rather than color math.
    std::fs::write(dir.join("numbered.motion.tsx"), r##"
export const composition = { width: 80, height: 32, fps: 10, duration: 12 };
const BITS = [0,1,2,3,4,5,6,7];
export default function Numbered(ctx) {return <Scene style={{width:80,height:32,backgroundColor:"#000000"}}>
  {BITS.map(bit => <View key={`bit-${bit}`} style={{position:"absolute",left:bit*10,top:0,width:10,height:32,
    backgroundColor:floor(ctx.localFrame / pow(2,bit)) % 2 === 1 ? "#ffffff" : "#000000"}} />)}
</Scene>;}
"##).unwrap();
    success(run(
        dir,
        &[
            "motion",
            "render",
            "numbered.motion.tsx",
            "--backend",
            "raster",
            "-o",
            "numbered.mp4",
        ],
    ));
    for (case, (start, speed, dynamic)) in [
        (3.0, 0.0, false),
        (6.0, 0.5, false),
        (6.0, 2.0, false),
        (9.0, 1.0, false),
        (6.0, 2.0, true),
    ]
    .into_iter()
    .enumerate()
    {
        let attr = |value: f64| {
            if dynamic {
                format!("ctx.seconds * 0 + {value}")
            } else {
                value.to_string()
            }
        };
        std::fs::write(dir.join("video.motion.tsx"), format!(r#"
export const composition = {{ width: 80, height: 32, fps: 10, duration: 9 }};
export const controls = ({{assets:{{clip:asset({{kind:"video"}})}}}});
export default function Clip(ctx) {{return <Scene style={{{{width:80,height:32}}}}><Video src="asset://clip" sourceStart={{{}}} speed={{{}}} style={{{{width:80,height:32}}}} /></Scene>;}}
"#,attr(start),attr(speed))).unwrap();
        put(
            dir,
            "video.json",
            json!({"canvas":{"width":80,"height":32,"fps":10},
            "resources":{"motion":"video.motion.tsx","v":"numbered.mp4"},
            "tracks":{"visual":[{"clips":[{"kind":"motion","component":"motion","start":0,"duration":3,"resources":{"clip":"v"}}]}]}}),
        );
        let mut first = None;
        for (request, frame) in [10, 0, 20, 10].into_iter().enumerate() {
            let file = format!("sample-{case}-{request}.png");
            success(run(
                dir,
                &[
                    "timeline",
                    "render",
                    "video.json",
                    "--frame",
                    &frame.to_string(),
                    "-o",
                    &file,
                ],
            ));
            let image = valle_media::codec::read_rgba_png(&dir.join(&file)).unwrap();
            let actual = (0..8).fold(0, |number, bit| {
                number | (usize::from(image.data[(16 * 80 + bit * 10 + 5) * 4] > 128) << bit)
            });
            assert_eq!(
                actual,
                (start * 10.0 + f64::from(frame) * speed) as usize,
                "start={start}, speed={speed}, frame={frame}, dynamic={dynamic}"
            );
            if request == 0 {
                first = Some(image.data);
            } else if request == 3 {
                assert_eq!(first.as_ref().unwrap(), &image.data);
            }
        }
    }
}

#[test]
fn motion_check_prepares_glass_and_native_shader_resources() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    png(dir, "noise.png", [128; 3]);
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../valle-compiler/tests/fixtures/motion/shader-packages/local-dissolve");
    let target = dir.join("effects");
    std::fs::create_dir_all(&target).unwrap();
    for file in ["manifest.json", "shader.vsksl"] {
        std::fs::copy(
            package.join(file),
            target.join(if file == "manifest.json" {
                "dissolve.shader.json"
            } else {
                file
            }),
        )
        .unwrap();
    }
    let sources = [
        (
            "glass.motion.tsx",
            r##"export const composition = { width: 64, height: 64, fps: 30, duration: 3 };
export default function T(){return <Scene style={{width:64,height:64,backgroundColor:"#468aff"}}><Glass surfaceId="card" shape={{kind:"continuousRect",radius:4}} material={{clarity:0.8,depth:0.4,tint:"#b9d7ff18"}} style={{position:"absolute",left:8,top:8,width:40,height:30}}><Text style={{fontSize:12}}>Hi</Text></Glass></Scene>; }"##,
        ),
        (
            "shader.motion.tsx",
            r##"export const composition = { width: 64, height: 64, fps: 30, duration: 3 };
export const controls=({assets:{noise:asset({kind:"image"}),effect:asset({kind:"shader"})}}); export default function T(ctx){return <Scene style={{width:64,height:64}}><ShaderLayer source="asset://effect" inputs={{noise:"asset://noise"}} uniforms={{progress:ctx.progress,edgeWidth:0.06,edgeColor:"#38bdf8"}} style={{width:64,height:64}}><View style={{width:64,height:64,background:"#ffffff"}}/></ShaderLayer></Scene>; }"##,
        ),
    ];
    for (name, source) in sources {
        std::fs::write(dir.join(name), source).unwrap();
        let mut args = vec!["motion", "check", name];
        if name.starts_with("shader") {
            args.extend([
                "--asset",
                "noise=noise.png",
                "--asset",
                "effect=effects/dissolve.shader.json",
            ]);
        }
        success(run(dir, &args));
    }
}

#[test]
fn motion_time_window_uses_ordinary_numeric_props() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let source = r##"export const composition = { width: 64, height: 64, fps: 30, duration: 3 };
export const controls = { props: { highlightStart: number({default:2,min:0}), highlightEnd: number({default:3,min:0}) } };
export default function T(ctx, props) {
  const active = ctx.seconds >= props.highlightStart && ctx.seconds < props.highlightEnd;
  return <Scene><View style={{width:64,height:64,backgroundColor:"#ffffff",opacity:active ? 1 : 0}}/></Scene>;
}"##;
    std::fs::write(dir.join("window.motion.tsx"), source).unwrap();
    success(run(dir, &["motion", "check", "window.motion.tsx"]));
    put(
        dir,
        "props.json",
        json!({"highlightStart":0,"highlightEnd":1}),
    );
    success(run(
        dir,
        &[
            "motion",
            "render",
            "window.motion.tsx",
            "--frame",
            "15",
            "--backend",
            "raster",
            "-o",
            "inactive.png",
        ],
    ));
    assert_eq!(pixel(dir, "inactive.png", 32, 32), vec![0, 0, 0]);
    success(run(
        dir,
        &[
            "motion",
            "render",
            "window.motion.tsx",
            "--frame",
            "15",
            "--props",
            "props.json",
            "--backend",
            "raster",
            "-o",
            "active.png",
        ],
    ));
    assert!(pixel(dir, "active.png", 32, 32)[0] > 80);
}

#[test]
fn repeated_motion_instances_bind_their_own_images_and_unused_files_are_not_loaded() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    png(dir, "red.png", [255, 0, 0]);
    png(dir, "blue.png", [0, 0, 255]);
    std::fs::write(dir.join("image.motion.tsx"),r#"export const composition = { width: 64, height: 64, fps: 10, duration: 9 };
export const controls=({assets:{hero:asset({kind:"image"})}});export default function T(){return <Scene style={{width:64,height:64}}><Image src="asset://hero" style={{width:64,height:64}}/></Scene>;}"#).unwrap();
    put(
        dir,
        "timeline.json",
        json!({"canvas":{"width":64,"height":64,"fps":10},"resources":{"component":"image.motion.tsx","red":"red.png","blue":"blue.png","unused":"missing.png"},"tracks":{"visual":[{"clips":[
          {"kind":"motion","component":"component","start":0,"duration":1,"resources":{"hero":"red"}},
          {"kind":"motion","component":"component","start":1,"duration":1,"resources":{"hero":"blue"}}
        ]}]}}),
    );
    success(run(dir, &["timeline", "check", "timeline.json"]));
    for (frame, file, color) in [
        ("0", "red-out.png", vec![255, 0, 0]),
        ("10", "blue-out.png", vec![0, 0, 255]),
    ] {
        success(run(
            dir,
            &[
                "timeline",
                "render",
                "timeline.json",
                "--frame",
                frame,
                "-o",
                file,
            ],
        ));
        assert_eq!(pixel(dir, file, 32, 32), color);
    }
    // Explicit inline images still render when the line contains no text glyphs.
    std::fs::write(dir.join("image.motion.tsx"),r#"export const composition = { width: 64, height: 64, fps: 10, duration: 9 };
export const controls=({assets:{hero:asset({kind:"image"})}});export default function T(){return <Scene style={{width:64,height:64}}><Image src="asset://hero" style={{display:"inline",width:32,height:32}}/></Scene>;}"#).unwrap();
    success(run(
        dir,
        &[
            "timeline",
            "render",
            "timeline.json",
            "--frame",
            "0",
            "-o",
            "inline.png",
        ],
    ));
    assert_eq!(pixel(dir, "inline.png", 16, 16), vec![255, 0, 0]);
    put(
        dir,
        "cover.json",
        json!({"canvas":{"width":64,"height":64,"fps":10},"resources":{"image":"red.png"},"tracks":{"visual":[{"clips":[{"kind":"image","src":"image","start":0,"duration":1,"fit":"cover"}]}]}}),
    );
    success(run(
        dir,
        &[
            "timeline",
            "render",
            "cover.json",
            "--frame",
            "0",
            "-o",
            "cover.png",
        ],
    ));
    assert_eq!(pixel(dir, "cover.png", 1, 1), vec![255, 0, 0]);
}

#[test]
fn video_original_audio_preserves_stereo_gain_trim_and_rate() {
    use valle_media::{
        codec::{FloatWavWriter, LibavAudioStream},
        frame::AudioBuffer,
    };
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let samples = (0..48_000)
        .flat_map(|i| {
            [440.0, 880.0].map(move |hz| {
                (0.15 * (std::f64::consts::TAU * hz * i as f64 / 48_000.0).sin()) as f32
            })
        })
        .collect();
    let mut writer = FloatWavWriter::create(&dir.join("stereo.wav"), 48_000, 2).unwrap();
    writer
        .write(&AudioBuffer {
            samples,
            sample_rate: 48_000,
            channels: 2,
        })
        .unwrap();
    writer.finish().unwrap();
    put(
        dir,
        "source.json",
        json!({"canvas":{"width":64,"height":64,"fps":10},"resources":{"sound":"stereo.wav"},"tracks":{"visual":[{"clips":[{"kind":"solid","color":"#123456","start":0,"duration":1}]}],"audio":[{"clips":[{"src":"sound","start":0,"duration":1}]}]}}),
    );
    success(run(
        dir,
        &["timeline", "render", "source.json", "-o", "source.mp4"],
    ));
    let amplitude = |file: &str, channel: usize, hz: f64| {
        let mut stream = LibavAudioStream::open(&dir.join(file), 48_000, 2).unwrap();
        let mut samples = Vec::new();
        loop {
            let block = stream.read(48_000).unwrap();
            if block.samples.is_empty() {
                break;
            }
            samples.extend(block.samples);
        }
        let start = 4800;
        let end = 14_400.min(samples.len() / 2);
        let (sin, cos) = (start..end).fold((0.0, 0.0), |(s, c), i| {
            let angle = std::f64::consts::TAU * hz * i as f64 / 48_000.0;
            let sample = f64::from(samples[i * 2 + channel]);
            (s + sample * angle.sin(), c + sample * angle.cos())
        });
        2.0 * sin.hypot(cos) / (end - start) as f64
    };
    assert!(amplitude("source.mp4", 0, 440.0) > 0.1);
    assert!(amplitude("source.mp4", 0, 880.0) < 0.01);
    assert!(amplitude("source.mp4", 1, 880.0) > 0.1);
    for (index, gain) in [1.0, 0.0, 2.0].into_iter().enumerate() {
        let mut clip =
            json!({"kind":"video","src":"v","start":0,"duration":0.4,"trimStart":0.2,"rate":2});
        if index > 0 {
            clip["gain"] = json!(gain);
        }
        put(
            dir,
            "video.json",
            json!({"canvas":{"width":64,"height":64,"fps":10},"resources":{"v":"source.mp4"},"tracks":{"visual":[{"clips":[clip]}]}}),
        );
        let file = format!("out-{index}.mp4");
        success(run(dir, &["timeline", "render", "video.json", "-o", &file]));
        let left = amplitude(&file, 0, 880.0);
        let right = amplitude(&file, 1, 1760.0);
        assert!(
            (left - 0.15 * gain).abs() < 0.025,
            "gain={gain}, left={left}"
        );
        assert!(
            (right - 0.15 * gain).abs() < 0.025,
            "gain={gain}, right={right}"
        );
    }
}

#[test]
fn motion_props_use_css_values_in_both_cli_and_timeline() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    std::fs::write(dir.join("props.motion.tsx"),r##"export const composition = { width: 64, height: 64, fps: 30, duration: 3 };
export const controls=({props:{ink:color({default:"#ff0000"}),extent:length({default:"8px"}),turn:angle({default:"0deg"})}});export default function T(ctx,props){return <Scene style={{width:64,height:64}}><View style={{width:props.extent,height:32,background:props.ink,rotate:props.turn}}/></Scene>;}"##).unwrap();
    let props = json!({"ink":"#00ff00","extent":"64px","turn":"0deg"});
    put(dir, "props.json", props.clone());
    success(run(
        dir,
        &[
            "motion",
            "check",
            "props.motion.tsx",
            "--props",
            "props.json",
        ],
    ));
    success(run(
        dir,
        &[
            "motion",
            "render",
            "props.motion.tsx",
            "--props",
            "props.json",
            "--frame",
            "0",
            "--backend",
            "raster",
            "-o",
            "props.png",
        ],
    ));
    assert!(
        pixel(dir, "props.png", 48, 16)
            .iter()
            .zip([0_u8, 255, 0])
            .all(|(a, b)| a.abs_diff(b) <= 2)
    );
    put(
        dir,
        "props.timeline.json",
        json!({"canvas":{"width":64,"height":64,"fps":10},"resources":{"c":"props.motion.tsx"},"tracks":{"visual":[{"clips":[{"kind":"motion","component":"c","start":0,"duration":1,"props":props}]}]}}),
    );
    success(run(dir, &["timeline", "check", "props.timeline.json"]));
    success(run(
        dir,
        &[
            "timeline",
            "render",
            "props.timeline.json",
            "--frame",
            "0",
            "-o",
            "timeline-props.png",
        ],
    ));
    assert!(
        pixel(dir, "timeline-props.png", 48, 16)
            .iter()
            .zip([0_u8, 255, 0])
            .all(|(a, b)| a.abs_diff(b) <= 2)
    );
}

#[test]
fn invalid_timeline_motion_emits_one_actionable_json_error() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    std::fs::write(
        dir.join("bad.motion.tsx"),
        r##"export default function T(){return <View style={{backgroundColour:"#fff"}}/>;}"##,
    )
    .unwrap();
    put(
        dir,
        "bad.json",
        json!({"canvas":{"width":64,"height":64,"fps":10},"resources":{"bad":"bad.motion.tsx"},"tracks":{"visual":[{"clips":[{"kind":"motion","component":"bad","start":0,"duration":1}]}]}}),
    );
    let output = run(dir, &["timeline", "check", "bad.json"]);
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let message = report["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("bad.motion.tsx") && message.contains("backgroundColour"),
        "{message}"
    );
}

#[test]
fn video_without_an_audio_stream_stays_silent_and_explicit_size_is_respected() {
    use valle_media::{
        codec::{Encoder, LibavAudioStream, encode::Mp4Encoder},
        frame::RgbaFrame,
    };
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let mut encoder = Mp4Encoder::new(&dir.join("silent.mp4"), 64, 64, 10, None).unwrap();
    let mut frame = RgbaFrame::new(64, 64);
    frame.data = [255, 0, 0, 255].repeat(64 * 64);
    for _ in 0..10 {
        encoder.encode_frame(&frame).unwrap();
    }
    encoder.finish().unwrap();
    assert!(
        valle_media::codec::audio::probe_audio_stream(&dir.join("silent.mp4"))
            .unwrap()
            .is_none()
    );
    put(
        dir,
        "silent.json",
        json!({"canvas":{"width":64,"height":64,"fps":10},"resources":{"v":"silent.mp4"},"tracks":{"visual":[{"clips":[{"kind":"video","src":"v","start":0,"duration":1,"size":[32,32]}]}]}}),
    );
    success(run(dir, &["timeline", "check", "silent.json"]));
    success(run(
        dir,
        &[
            "timeline",
            "render",
            "silent.json",
            "--frame",
            "0",
            "-o",
            "small.png",
        ],
    ));
    assert_eq!(pixel(dir, "small.png", 1, 1), vec![0, 0, 0]);
    assert!(pixel(dir, "small.png", 32, 32)[0] > 240);
    success(run(
        dir,
        &["timeline", "render", "silent.json", "-o", "out.mp4"],
    ));
    let mut stream = LibavAudioStream::open(&dir.join("out.mp4"), 48_000, 2).unwrap();
    loop {
        let block = stream.read(48_000).unwrap();
        if block.samples.is_empty() {
            break;
        }
        assert!(block.samples.iter().all(|v| v.abs() < 0.0001));
    }
}

#[test]
fn shader_file_assets_render_vectors_and_matrices_in_arbitrary_frame_order() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    put(
        dir,
        "effect.shader.json",
        json!({
            "name":"coordinate-field", "entry":"field.vsksl", "inputs":[],
            "uniforms":[
                {"name":"amount", "type":"float", "required":true,"min":0,"max":1},
                {"name":"offset", "type":"float3", "required":true,"min":0,"max":1},
                {"name":"basis", "type":"float2x2", "required":true,"min":-1,"max":1},
                {"name":"enabled", "type":"bool", "required":true}
            ],
            "output":{"colorSpace":"linear-srgb","alphaMode":"straight","allowTransparent":true},
            "budget":"local"
        }),
    );
    let shader = "float4 valle_main(float2 uv) { float2 p = basis * uv; if (enabled) { return float4(float3(amount + offset.z + p.x * 0.05), 1.0); } return float4(0.0); }";
    std::fs::write(dir.join("field.vsksl"), shader).unwrap();
    let source = r##"export const composition = { width: 64, height: 64, fps: 30, duration: 3 };
export const controls=({assets:{effect:asset({kind:"shader",required:true})}});
export default function T(ctx) { return <Scene style={{width:64,height:64}}><ShaderLayer source="asset://effect" uniforms={{amount:ctx.seconds*0.1,offset:[0,0,ctx.seconds*0.05],basis:[1,0,0,1],enabled:true}} style={{width:64,height:64}}><View style={{width:64,height:64,backgroundColor:"#fff"}}/></ShaderLayer></Scene>; }"##;
    std::fs::write(dir.join("effect.motion.tsx"), source).unwrap();
    put(
        dir,
        "effect.timeline.json",
        json!({
            "canvas":{"width":64,"height":64,"fps":30},
            "resources":{"effect":"effect.motion.tsx","shader":"effect.shader.json"},
            "tracks":{"visual":[{"clips":[{"kind":"motion","component":"effect","start":0,"duration":3,"resources":{"effect":"shader"}}]}]}
        }),
    );
    let render = |frame: u32, output: &str| {
        run(
            dir,
            &[
                "motion",
                "render",
                "effect.motion.tsx",
                "--asset",
                "effect=effect.shader.json",
                "--frame",
                &frame.to_string(),
                "--backend",
                "raster",
                "-o",
                output,
            ],
        )
    };
    let mut frames = std::collections::BTreeMap::new();
    for (request, frame) in [60, 0, 30, 60].into_iter().enumerate() {
        let motion = format!("motion-{request}.png");
        let timeline = format!("timeline-{request}.png");
        success(render(frame, &motion));
        success(run(
            dir,
            &[
                "timeline",
                "render",
                "effect.timeline.json",
                "--frame",
                &frame.to_string(),
                "-o",
                &timeline,
            ],
        ));
        let a = valle_media::codec::read_rgba_png(&dir.join(motion))
            .unwrap()
            .data;
        let b = valle_media::codec::read_rgba_png(&dir.join(timeline))
            .unwrap()
            .data;
        assert_eq!(a, b, "standalone and Timeline differ at frame {frame}");
        if let Some(previous) = frames.insert(frame, a.clone()) {
            assert_eq!(previous, a);
        }
    }
    assert_ne!(frames[&0], frames[&60]);
    std::thread::scope(|scope| {
        let a = scope.spawn(|| success(render(60, "parallel-a.png")));
        let b = scope.spawn(|| success(render(60, "parallel-b.png")));
        a.join().unwrap();
        b.join().unwrap();
    });
    for name in ["parallel-a.png", "parallel-b.png"] {
        assert_eq!(
            valle_media::codec::read_rgba_png(&dir.join(name))
                .unwrap()
                .data,
            frames[&60]
        );
    }
    std::fs::write(
        dir.join("field.vsksl"),
        shader.replace("amount + offset.z", "amount * 2.0 + offset.z"),
    )
    .unwrap();
    success(render(60, "changed.png"));
    assert_ne!(
        valle_media::codec::read_rgba_png(&dir.join("changed.png"))
            .unwrap()
            .data,
        frames[&60]
    );
    // Frame expressions go through the same finite/range validation as static values.
    std::fs::write(
        dir.join("effect.motion.tsx"),
        source.replace("ctx.seconds*0.05", "ctx.seconds"),
    )
    .unwrap();
    let invalid = render(60, "invalid.png");
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stdout).contains("outside its declared range"));
}

#[test]
fn shader_frame_budget_counts_nested_layers_and_separate_clips() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    put(
        dir,
        "effect.shader.json",
        json!({
            "name":"workload", "entry":"effect.vsksl", "inputs":[], "uniforms":[],
            "output":{"colorSpace":"linear-srgb","alphaMode":"straight","allowTransparent":true}, "budget":"local"
        }),
    );
    let sampler = "float4 valle_main(float2 uv) { float4 sum = float4(0.0); for (int i = 0; i < 64; ++i) { sum += sampleContent(uv) / 64.0; } return sum; }";
    let arithmetic = "float4 valle_main(float2 uv) { float4 value = float4(uv,0.0,1.0); for (int i = 0; i < 128; ++i) { value = value * 0.999 + float4(0.0001); } return value; }";
    let source = |body: &str| {
        format!(
            r#"export const controls=({{assets:{{effect:asset({{kind:"shader"}})}}}});export default function T(ctx){{return <Scene style={{{{width:ctx.viewport.width,height:ctx.viewport.height}}}}>{body}</Scene>;}}"#
        )
    };
    let layer = |body: &str| {
        format!(
            r#"<ShaderLayer source="asset://effect" style={{{{width:ctx.viewport.width,height:ctx.viewport.height}}}}>{body}</ShaderLayer>"#
        )
    };
    for (kind, shader, body, clips) in [
        ("arithmetic", arithmetic, layer(""), 1),
        ("nested", sampler, layer(&layer("")), 1),
        ("clips", sampler, layer(""), 2),
    ] {
        std::fs::write(dir.join("effect.vsksl"), shader).unwrap();
        for size in [64, 1920] {
            let (width, height) = if size == 64 { (64, 36) } else { (1920, 1080) };
            std::fs::write(
                dir.join("effect.motion.tsx"),
                format!(
                    "export const composition = {{ width: {width}, height: {height}, fps: 30, duration: 3 }};\n{}",
                    source(&body)
                ),
            )
            .unwrap();
            put(
                dir,
                "effect.timeline.json",
                json!({
                    "canvas":{"width":size,"height":if size == 64 { 36 } else { 1080 },"fps":30},
                    "resources":{"motion":"effect.motion.tsx","shader":"effect.shader.json"},
                    "tracks":{"visual":(0..clips).map(|_| json!({"clips":[{"kind":"motion","component":"motion","start":0,"duration":1,"resources":{"effect":"shader"}}]})).collect::<Vec<_>>()}
                }),
            );
            // Use actual device pixels: the same source is affordable at preview size.
            let output = run(
                dir,
                &[
                    "timeline",
                    "render",
                    "effect.timeline.json",
                    "--frame",
                    "0",
                    "-o",
                    &format!("{kind}-{size}.png"),
                ],
            );
            if size == 64 {
                success(output);
            } else {
                assert!(
                    !output.status.success(),
                    "{kind} bypassed the frame work budget"
                );
                let message = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(message.contains("frame Shader work"), "{kind}: {message}");
            }
        }
    }
}

#[test]
fn shader_sampling_uses_each_texture_size_filter_and_wrap() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let mut encoder =
        png::Encoder::new(std::fs::File::create(dir.join("steps.png")).unwrap(), 2, 1);
    encoder.set_color(png::ColorType::Rgb);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&[0, 0, 0, 255, 255, 255])
        .unwrap();
    std::fs::write(
        dir.join("sample.vsksl"),
        "float4 valle_main(float2 uv) { return sample_steps(float2(uv.x * 2.0, 0.5)); }",
    )
    .unwrap();
    std::fs::write(dir.join("sample.motion.tsx"), r##"export const composition = { width: 64, height: 64, fps: 30, duration: 3 };
export const controls=({assets:{effect:asset({kind:"shader"}),image:asset({kind:"image"})}});export default function T(){return <Scene style={{width:64,height:64}}><ShaderLayer source="asset://effect" inputs={{steps:"asset://image"}} style={{width:64,height:64}}><View style={{width:64,height:64,backgroundColor:"#fff"}}/></ShaderLayer></Scene>; }"##).unwrap();
    let render = |filter: &str, wrap: &str| {
        put(
            dir,
            "sample.shader.json",
            json!({
                "name":"sample-field","entry":"sample.vsksl",
                "inputs":[{"name":"steps","required":true,"sampling":filter,"wrap":wrap}],
                "uniforms":[],"output":{"colorSpace":"linear-srgb","alphaMode":"straight","allowTransparent":true},"budget":"local"
            }),
        );
        let output = format!("{filter}-{wrap}.png");
        success(run(
            dir,
            &[
                "motion",
                "render",
                "sample.motion.tsx",
                "--asset",
                "effect=sample.shader.json",
                "--asset",
                "image=steps.png",
                "--frame",
                "0",
                "--backend",
                "raster",
                "-o",
                &output,
            ],
        ));
        output
    };
    for (wrap, expected) in [
        ("clamp", [0, 255, 255, 255]),
        ("repeat", [0, 255, 0, 255]),
        ("mirror", [0, 255, 255, 0]),
    ] {
        let output = render("nearest", wrap);
        let actual = [0, 16, 32, 48].map(|x| pixel(dir, &output, x, 32)[0]);
        assert_eq!(actual, expected, "{wrap}");
    }
    let output = render("linear", "clamp");
    let value = pixel(dir, &output, 8, 32)[0];
    assert!(
        (48..=51).contains(&value),
        "linear-light interpolation expected about 49, got {value}"
    );
}

#[test]
fn shader_content_preserves_local_coordinates_and_complete_input() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    put(
        dir,
        "effect.shader.json",
        json!({
            "name":"content-map", "entry":"effect.vsksl", "inputs":[], "uniforms":[],
            "output":{"colorSpace":"linear-srgb","alphaMode":"straight","allowTransparent":true}, "budget":"local"
        }),
    );
    let render = |body: &str, shader: &str| {
        if dir.join("result.png").exists() {
            std::fs::remove_file(dir.join("result.png")).unwrap();
        }
        std::fs::write(dir.join("effect.vsksl"), shader).unwrap();
        std::fs::write(dir.join("effect.motion.tsx"), format!(r##"
            export const composition = {{ width: 64, height: 64, fps: 30, duration: 3 }};
            export const controls=({{assets:{{effect:asset({{kind:"shader"}})}}}});
            export default function T() {{ return <Scene style={{{{width:64,height:64,backgroundColor:"#000"}}}}>{body}</Scene>; }}
        "##)).unwrap();
        success(run(
            dir,
            &[
                "motion",
                "render",
                "effect.motion.tsx",
                "--asset",
                "effect=effect.shader.json",
                "--frame",
                "0",
                "--backend",
                "raster",
                "-o",
                "result.png",
            ],
        ));
    };
    let identity = "float4 valle_main(float2 uv) { return sampleContent(uv); }";
    render(
        r##"<ShaderLayer source="asset://effect" style={{position:"absolute",left:16,top:16,width:32,height:32,transform:"rotate(37deg) scale(1.25,0.75)"}}>
        <View style={{position:"absolute",width:16,height:32,backgroundColor:"#f00"}}/>
        <View style={{position:"absolute",left:16,width:16,height:32,backgroundColor:"#00f"}}/>
    </ShaderLayer>"##,
        identity,
    );
    for (x, y, expected) in [
        (24, 26, [255, 0, 0]),
        (40, 38, [0, 0, 255]),
        (8, 8, [0, 0, 0]),
    ] {
        let actual = pixel(dir, "result.png", x, y);
        assert!(
            actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2),
            "transformed ({x},{y}): {actual:?}"
        );
    }
    render(
        r##"<ShaderLayer source="asset://effect" style={{position:"absolute",left:-32,top:16,width:64,height:32}}>
        <View style={{position:"absolute",width:32,height:32,backgroundColor:"#f00"}}/>
        <View style={{position:"absolute",left:32,width:32,height:32,backgroundColor:"#00f"}}/>
    </ShaderLayer>"##,
        "float4 valle_main(float2 uv) { return sampleContent(uv - float2(0.5,0.0)); }",
    );
    let actual = pixel(dir, "result.png", 16, 32);
    assert!(
        actual[0] >= 253 && actual[2] <= 2,
        "offscreen input was lost: {actual:?}"
    );
    render(
        r##"<ShaderLayer source="asset://effect" style={{position:"absolute",left:8,top:8,width:48,height:48}}>
        <View style={{position:"absolute",left:8,top:8,width:8,height:8,backgroundColor:"#f00"}}/>
    </ShaderLayer>"##,
        identity,
    );
    assert!(pixel(dir, "result.png", 20, 20)[0] >= 253);
    assert_eq!(
        pixel(dir, "result.png", 32, 32),
        [0, 0, 0],
        "content outside its bounds is transparent"
    );
    render(
        r##"<ShaderLayer source="asset://effect" style={{position:"absolute",left:8,top:8,width:48,height:48}}/>"##,
        "float4 valle_main(float2 uv) { if (uv.x < 0.5) return float4(1.0); return float4(0.0); }",
    );
    assert_eq!(
        pixel(dir, "result.png", 16, 32),
        [255, 255, 255],
        "shader generates pixels from empty content"
    );
    assert_eq!(pixel(dir, "result.png", 48, 32), [0, 0, 0]);
    put(
        dir,
        "effect.shader.json",
        json!({
            "name":"content-map", "entry":"effect.vsksl", "inputs":[], "uniforms":[],
            "output":{"colorSpace":"linear-srgb","alphaMode":"straight","allowTransparent":true,"padding":[8,4,12,6]}, "budget":"local"
        }),
    );
    render(
        r##"<ShaderLayer source="asset://effect" style={{position:"absolute",left:24,top:24,width:16,height:16}}>
        <View style={{width:16,height:16,backgroundColor:"#fff"}}/>
    </ShaderLayer>"##,
        "float4 valle_main(float2 uv) { return sampleContent(uv + float2(0.5,0.0)); }",
    );
    assert_eq!(
        pixel(dir, "result.png", 20, 32),
        [255, 255, 255],
        "padding retains output beyond the border box"
    );
    assert_eq!(
        pixel(dir, "result.png", 28, 32),
        [255, 255, 255],
        "padding does not redefine content UVs"
    );
    assert_eq!(pixel(dir, "result.png", 40, 32), [0, 0, 0]);
    assert_eq!(pixel(dir, "result.png", 12, 32), [0, 0, 0]);
}

#[test]
fn timeline_accepts_a_component_whose_composition_differs_from_the_canvas() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    std::fs::write(
        dir.join("card.motion.tsx"),
        r##"export const composition = { width: 320, height: 180, fps: 30, duration: 1 };
export default function Card() {
  return <Scene className="h-full w-full" style={{ backgroundColor: "#102030" }} />;
}

"##,
    )
    .unwrap();
    put(
        dir,
        "card.timeline.json",
        json!({
            "canvas": {"width": 64, "height": 64, "fps": 30, "background": "#000000"},
            "resources": {"card": "card.motion.tsx"},
            "tracks": {"visual": [{"clips": [{"kind": "motion", "component": "card", "start": 0, "duration": 1}]}]}
        }),
    );
    success(run(dir, &["timeline", "check", "card.timeline.json"]));
}

#[test]
fn timeline_inline_data_prepares_distinct_instances_and_rejects_missing_data() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    std::fs::write(
        dir.join("data.motion.tsx"),
        r##"export const composition = { width: 32, height: 32, fps: 10, duration: 1 };
export const controls = { data: { rows: array(record({ id: string(), color: color() }), { minItems: 1, maxItems: 4, key: "id" }) } };
export default function DataScene(ctx, props, data) {
  return <Scene style={{ width: 32, height: 32 }}>
    {data.rows.map((row) => <View key={row.id} style={{ width: 32, height: 32, backgroundColor: row.color }} />)}
  </Scene>;
}"##,
    )
    .unwrap();
    let timeline = |first: &str| {
        json!({
            "canvas": {"width": 32, "height": 32, "fps": 10},
            "resources": {"scene": "data.motion.tsx"},
            "tracks": {"visual": [{"clips": [
                {"kind": "motion", "component": "scene", "start": 0, "duration": 1,
                 "data": {"rows": [{"id": "first", "color": first}]}},
                {"kind": "motion", "component": "scene", "start": 1, "duration": 1,
                 "data": {"rows": [{"id": "second", "color": "#0000ff"}]}}
            ]}]}
        })
    };
    put(dir, "data.timeline.json", timeline("#ff0000"));
    for (frame, file, color) in [
        ("0", "red.png", vec![255, 0, 0]),
        ("10", "blue.png", vec![0, 0, 255]),
    ] {
        success(run(
            dir,
            &[
                "timeline",
                "render",
                "data.timeline.json",
                "--frame",
                frame,
                "-o",
                file,
            ],
        ));
        assert_eq!(pixel(dir, file, 16, 16), color);
    }
    put(dir, "data.timeline.json", timeline("#00ff00"));
    success(run(
        dir,
        &[
            "timeline",
            "render",
            "data.timeline.json",
            "--frame",
            "0",
            "-o",
            "green.png",
        ],
    ));
    assert_eq!(pixel(dir, "green.png", 16, 16), vec![1, 255, 0]);
    let mut missing = timeline("#ff0000");
    missing["tracks"]["visual"][0]["clips"][0]
        .as_object_mut()
        .unwrap()
        .remove("data");
    put(dir, "data.timeline.json", missing);
    let output = run(dir, &["timeline", "check", "data.timeline.json"]);
    assert!(!output.status.success());

    let mut mismatched = timeline("#ff0000");
    mismatched["tracks"]["visual"][0]["clips"][0]["sourceDuration"] = json!(2);
    put(dir, "data.timeline.json", mismatched);
    let output = run(dir, &["timeline", "check", "data.timeline.json"]);
    assert!(
        !output.status.success(),
        "source duration must match the bound work"
    );
}

#[test]
fn timeline_data_fields_do_not_bind_or_reclassify_resources() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    std::fs::write(
        dir.join("metadata.motion.tsx"),
        r##"export const composition = { width: 8, height: 8, fps: 10, duration: 1 };
export const controls = { data: {
  item: record({ kind: string(), component: string(), font: string() }),
  decoy: record({ kind: string(), src: string() }),
} };
export default function Metadata(ctx, props, data) {
  return <Scene style={{ width: 8, height: 8, backgroundColor: "#ff0000" }} />;
}"##,
    )
    .unwrap();
    png(dir, "picture.png", [0, 0, 255]);
    put(
        dir,
        "metadata.timeline.json",
        json!({
            "canvas": {"width": 8, "height": 8, "fps": 10},
            "resources": {"scene": "metadata.motion.tsx", "picture": "picture.png"},
            "tracks": {"visual": [
                {"clips": [{"kind": "motion", "component": "scene", "start": 0, "duration": 1,
                    "data": {
                        "item": {"kind": "motion", "component": "scene", "font": "picture"},
                        "decoy": {"kind": "lottie", "src": "picture"}
                    }}]},
                {"clips": [{"kind": "image", "src": "picture", "start": 0, "duration": 1}]}
            ]}
        }),
    );
    success(run(dir, &["timeline", "check", "metadata.timeline.json"]));
}
