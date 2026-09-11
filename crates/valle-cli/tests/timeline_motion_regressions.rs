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
fn motion_check_prepares_glass_and_native_shader_resources() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    png(dir, "noise.png", [128; 3]);
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../valle-compiler/tests/fixtures/motion/shader-packages/local-dissolve");
    let target = dir.join("shaders/local-dissolve");
    std::fs::create_dir_all(&target).unwrap();
    for file in ["manifest.json", "shader.vsksl"] {
        std::fs::copy(package.join(file), target.join(file)).unwrap();
    }
    let sources = [
        (
            "glass.motion.tsx",
            r##"export default function T(){return <Scene style={{width:64,height:64,backgroundColor:"#468aff"}}><Glass surfaceId="card" shape={{kind:"continuousRect",radius:4}} material={{clarity:0.8,depth:0.4,tint:"#b9d7ff18"}} style={{position:"absolute",left:8,top:8,width:40,height:30}}><Text style={{fontSize:12}}>Hi</Text></Glass></Scene>; }"##,
        ),
        (
            "shader.motion.tsx",
            r##"export const controls=defineControls({assets:{noise:asset({kind:"image"})}}); export default function T(ctx){return <Scene style={{width:64,height:64}}><ShaderLayer source="shader://local-dissolve@1" inputs={{noise:"asset://noise"}} uniforms={{progress:ctx.progress,edgeWidth:0.06,edgeColor:"#38bdf8"}} style={{width:64,height:64}}><View style={{width:64,height:64,background:"#ffffff"}}/></ShaderLayer></Scene>; }"##,
        ),
    ];
    for (name, source) in sources {
        std::fs::write(dir.join(name), source).unwrap();
        let mut args = vec!["motion", "check", name, "--size", "64x64"];
        if name.starts_with("shader") {
            args.extend(["--asset", "noise=noise.png"]);
        }
        success(run(dir, &args));
    }
}

#[test]
fn motion_cues_are_explicit_and_optional_cues_stay_inactive() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let source = r##"export const controls=defineControls({cues:{voice:spanCue({required:true})}});export default function T(ctx,props,signals){return <Scene><View style={{width:64,height:64,backgroundColor:"#ffffff",opacity:signals.voice.progress}}/></Scene>;}"##;
    std::fs::write(dir.join("cue.motion.tsx"), source).unwrap();
    let output = run(
        dir,
        &["motion", "check", "cue.motion.tsx", "--size", "64x64"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("voice"));
    put(
        dir,
        "cues.json",
        json!({"voice":{"type":"source-range","start":0,"end":1}}),
    );
    success(run(
        dir,
        &[
            "motion",
            "check",
            "cue.motion.tsx",
            "--size",
            "64x64",
            "--cues",
            "cues.json",
        ],
    ));
    std::fs::write(
        dir.join("optional.motion.tsx"),
        source.replace("required:true", "required:false"),
    )
    .unwrap();
    success(run(
        dir,
        &[
            "motion",
            "render",
            "optional.motion.tsx",
            "--size",
            "64x64",
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
            "optional.motion.tsx",
            "--size",
            "64x64",
            "--frame",
            "15",
            "--cues",
            "cues.json",
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
    std::fs::write(dir.join("image.motion.tsx"),r#"export const controls=defineControls({assets:{hero:asset({kind:"image"})}});export default function T(){return <Scene style={{width:64,height:64}}><Image src="asset://hero" style={{width:64,height:64}}/></Scene>;}"#).unwrap();
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
    std::fs::write(dir.join("image.motion.tsx"),r#"export const controls=defineControls({assets:{hero:asset({kind:"image"})}});export default function T(){return <Scene style={{width:64,height:64}}><Image src="asset://hero" style={{display:"inline",width:32,height:32}}/></Scene>;}"#).unwrap();
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
    std::fs::write(dir.join("props.motion.tsx"),r##"export const controls=defineControls({props:{ink:color({default:"#ff0000"}),extent:length({default:"8px"}),turn:angle({default:"0deg"})}});export default function T(ctx,props){return <Scene style={{width:64,height:64}}><View style={{width:props.extent,height:32,background:props.ink,rotate:props.turn}}/></Scene>;}"##).unwrap();
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
            "--size",
            "64x64",
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
            "--size",
            "64x64",
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
