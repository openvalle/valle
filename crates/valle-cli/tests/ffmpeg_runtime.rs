// Process isolation here tests real loader/startup behavior, not a mocked initializer.
use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
};

fn run(directory: &Path, libraries: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_valle"))
        .current_dir(directory)
        .env("VALLE_FFMPEG_DIR", libraries)
        .env("VALLE_CACHE_DIR", directory.join("cache"))
        .args(args)
        .output()
        .unwrap()
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn scene(directory: &Path) {
    // The delivery contract lives in the file: 160x90 at 30 FPS for six frames.
    std::fs::write(directory.join("scene.motion.tsx"), "export const composition = { width: 160, height: 90, fps: 30, duration: 0.2 }; export default function Test() { return <Scene style={{backgroundColor:'#123456'}}><Text style={{fontSize:24}}>Valle</Text></Scene>; }\n").unwrap();
}
#[test]
fn non_media_commands_and_png_work_without_ffmpeg() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let missing = dir.join("missing");
    scene(dir);
    for args in [
        vec!["--version"],
        vec!["--help"],
        vec!["--json", "motion", "check", "scene.motion.tsx"],
        vec![
            "--json",
            "motion",
            "render",
            "scene.motion.tsx",
            "--frame",
            "0",
            "--backend",
            "raster",
            "-o",
            "frame.png",
        ],
        vec![
            "--json",
            "motion",
            "render",
            "scene.motion.tsx",
            "--backend",
            "raster",
            "--frames",
            "0,3,5",
            "-o",
            "frames/%05d.png",
            "--storyboard",
            "sheet.png",
        ],
    ] {
        success(&run(dir, &missing, &args));
    }
    let licenses = run(dir, &missing, &["--json", "licenses"]);
    success(&licenses);
    let licenses: Value = serde_json::from_slice(&licenses.stdout).unwrap();
    assert_eq!(licenses["status"], "ok");
    assert_eq!(licenses["complete"], cfg!(feature = "embedded-runtime"));
    assert!(
        licenses["text"]
            .as_str()
            .unwrap()
            .contains("Apache License")
    );
    assert!(std::fs::metadata(dir.join("frame.png")).unwrap().len() > 100);
    assert!(std::fs::metadata(dir.join("sheet.png")).unwrap().len() > 100);
    assert_eq!(std::fs::read_dir(dir.join("frames")).unwrap().count(), 3);
    std::fs::write(dir.join("image.motion.tsx"), r#"export const composition = { width: 160, height: 90, fps: 30, duration: 0.2 };
export const controls = ({assets:{poster:asset({kind:"image"})}});
export default function Test(){return <Scene style={{width:160,height:90,backgroundColor:"rgb(18,52,86)"}}><Image src="asset://poster" style={{position:"absolute",left:0,top:0,width:160,height:90}} /></Scene>;}"#).unwrap();
    success(&run(
        dir,
        &missing,
        &[
            "--json",
            "motion",
            "render",
            "image.motion.tsx",
            "--asset",
            "poster=frame.png",
            "--frame",
            "0",
            "--backend",
            "raster",
            "-o",
            "image.png",
        ],
    ));
    assert!(std::fs::metadata(dir.join("image.png")).unwrap().len() > 100);
}
#[cfg(not(target_os = "macos"))]
#[test]
fn non_macos_transcription_is_rejected_before_loading_or_writing() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let missing = dir.join("missing-ffmpeg");
    let output_path = dir.join("words.json");
    std::fs::write(&output_path, b"preserve existing output").unwrap();
    for args in [
        vec![
            "--json",
            "media",
            "transcribe",
            "missing.wav",
            "-o",
            "words.json",
            "--overwrite",
        ],
        vec![
            "media",
            "transcribe",
            "missing.wav",
            "--text-only",
            "--json",
        ],
    ] {
        let output = run(dir, &missing, &args);
        assert_eq!(output.status.code(), Some(3));
        assert!(output.stderr.is_empty(), "JSON mode leaked stderr");
        let diagnostic: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(diagnostic["error"]["code"], "unsupported_adapter");
        assert!(
            diagnostic["error"]["message"]
                .as_str()
                .unwrap()
                .contains("supported only on macOS")
        );
    }
    assert_eq!(
        std::fs::read(&output_path).unwrap(),
        b"preserve existing output"
    );
}

#[test]
fn missing_libraries_fail_only_at_media_use_with_json_diagnostic() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    scene(dir);
    for args in [
        vec!["--json", "media", "capabilities"],
        vec![
            "--json",
            "motion",
            "render",
            "scene.motion.tsx",
            "--backend",
            "raster",
            "-o",
            "video.mp4",
        ],
        vec![
            "--json",
            "motion",
            "render",
            "scene.motion.tsx",
            "--backend",
            "raster",
            "-o",
            "video.mov",
        ],
    ] {
        let out = run(dir, &dir.join("not-installed"), &args);
        assert!(!out.status.success());
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["error"]["code"], "ffmpeg_unavailable", "{value}");
        let error = value["error"]["message"].as_str().unwrap();
        assert!(error.contains("VALLE_FFMPEG_DIR"), "{error}");
        assert!(error.contains("\nhint: install"), "{error}");
        assert!(
            !String::from_utf8_lossy(&out.stderr).contains("panicked"),
            "{out:?}"
        );
    }
    assert!(!dir.join("video.mov").exists());
    assert!(!std::fs::read_dir(dir).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".valle-part-")
    }));
}

#[test]
fn audio_analysis_reports_missing_ffmpeg_before_compilation_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    std::fs::write(
        dir.join("audio.motion.tsx"),
        r#"export const composition = { width: 160, height: 90, fps: 30, duration: 0.2 };
export const controls = { assets: { beat: asset({ kind: "audio", required: true }) } };
const BEAT = audioAnalysis("asset://beat", { bands: 8, fps: 30 });
export default function AudioBars(ctx) {
    return <Scene><View style={{width:30,height:BEAT.level(ctx.seconds)*90}} /></Scene>;
}"#,
    )
    .unwrap();
    // A valid mono PCM WAV keeps the failure specific to the missing shared libraries.
    let samples = 9_600_u32;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&48_000_u32.to_le_bytes());
    wav.extend_from_slice(&96_000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples * 2).to_le_bytes());
    wav.resize(wav.len() + samples as usize * 2, 0);
    std::fs::write(dir.join("beat.wav"), wav).unwrap();
    std::fs::write(
        dir.join("timeline.json"),
        serde_json::json!({
            "canvas": {"width":160,"height":90,"fps":30},
            "resources": {"bars":"audio.motion.tsx","beat":"beat.wav"},
            "tracks": {"visual":[{"clips":[{
                "start":0,"duration":0.2,"kind":"motion","component":"bars",
                "resources":{"beat":"beat"}
            }]}]}
        })
        .to_string(),
    )
    .unwrap();

    for domain in ["motion", "timeline"] {
        for action in ["check", "render"] {
            for mode in ["--json", "--events"] {
                let mut args = vec![mode, domain, action];
                if domain == "motion" {
                    args.extend(["audio.motion.tsx", "--asset", "beat=beat.wav"]);
                } else {
                    args.push("timeline.json");
                }
                if action == "render" {
                    args.extend(["--frame", "0", "-o", "audio.png"]);
                }
                let out = run(dir, &dir.join("not-installed"), &args);
                assert!(!out.status.success(), "{args:?}: {out:?}");
                let value: Value = if mode == "--events" {
                    let events = String::from_utf8(out.stdout)
                        .unwrap()
                        .lines()
                        .map(|line| serde_json::from_str::<Value>(line).unwrap())
                        .collect::<Vec<_>>();
                    assert_eq!(events.last().unwrap()["type"], "report", "{events:?}");
                    assert_eq!(events.last().unwrap()["level"], "error", "{events:?}");
                    events.last().unwrap()["data"].clone()
                } else {
                    serde_json::from_slice(&out.stdout).unwrap()
                };
                assert_eq!(
                    value["error"]["code"], "ffmpeg_unavailable",
                    "{args:?}: {value}"
                );
                let message = value["error"]["message"].as_str().unwrap();
                assert!(message.contains("audioAnalysis asset `beat`"), "{message}");
                assert!(message.contains("VALLE_FFMPEG_DIR"), "{message}");
                assert!(message.contains("\nhint: install"), "{message}");
            }
        }
    }
    assert!(!dir.join("audio.png").exists());
}

#[test]
fn mentioning_audio_analysis_does_not_load_ffmpeg() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("comment.motion.tsx"),
        "// audioAnalysis is optional.\nexport const composition = {width:160,height:90,fps:30,duration:0.2}; export default function Test(){return <Scene />;}",
    )
    .unwrap();
    success(&run(
        temp.path(),
        &temp.path().join("not-installed"),
        &["--json", "motion", "check", "comment.motion.tsx"],
    ));
}

#[test]
fn cli_directory_overrides_environment_without_eager_loading() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let out = run(
        dir,
        &dir.join("from-env"),
        &[
            "--ffmpeg-dir",
            "from-cli",
            "--json",
            "media",
            "capabilities",
        ],
    );
    assert!(!out.status.success());
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    let error = value["error"]["message"].as_str().unwrap();
    assert!(
        error.contains("from-cli") && !error.contains("from-env"),
        "{error}"
    );
}
#[cfg(target_os = "macos")]
#[test]
fn rejects_wrong_architecture_before_loading() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let opposite = if cfg!(target_arch = "aarch64") {
        0x01000007u32
    } else {
        0x0100000cu32
    };
    let mut macho = [0u8; 64];
    macho[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
    macho[4..8].copy_from_slice(&opposite.to_le_bytes());
    std::fs::write(dir.join("libavutil.61.dylib"), macho).unwrap();
    let out = run(dir, dir, &["--json", "media", "capabilities"]);
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("architecture mismatch")
    );
}
#[cfg(target_os = "macos")]
#[test]
fn rejects_wrong_abi_before_using_structs_or_codecs() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    std::fs::write(
        dir.join("old.c"),
        "unsigned avutil_version(void) { return 60u << 16; }\n",
    )
    .unwrap();
    let status = Command::new("cc")
        .current_dir(dir)
        .args(["-dynamiclib", "old.c", "-o", "libavutil.61.dylib"])
        .status()
        .unwrap();
    assert!(status.success());
    let out = run(dir, dir, &["--json", "media", "capabilities"]);
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ABI/version mismatch"),
        "{value}"
    );
}
#[cfg(target_os = "macos")]
#[test]
fn executable_has_no_ffmpeg_startup_dependency() {
    let out = Command::new("/usr/bin/otool")
        .args(["-L", env!("CARGO_BIN_EXE_valle")])
        .output()
        .unwrap();
    success(&out);
    let libraries = String::from_utf8(out.stdout).unwrap();
    for name in [
        "libavcodec",
        "libavformat",
        "libavutil",
        "libswscale",
        "libswresample",
        "libx264",
        "libx265",
    ] {
        assert!(!libraries.contains(name), "{libraries}");
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires installed FFmpeg 9 shared libraries and a C linker"]
fn missing_named_software_encoder_is_reported_without_selecting_another_h264_encoder() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    scene(dir);
    let report = Command::new(env!("CARGO_BIN_EXE_valle"))
        .args(["--json", "media", "capabilities"])
        .output()
        .unwrap();
    success(&report);
    let report: Value = serde_json::from_slice(&report.stdout).unwrap();
    let mut real_codec = None;
    for library in report["libraries"].as_array().unwrap() {
        let name = library["name"].as_str().unwrap();
        let major = library["version"]
            .as_str()
            .unwrap()
            .split('.')
            .next()
            .unwrap();
        let path = library["path"].as_str().unwrap();
        if name == "avcodec" {
            real_codec = Some(path.to_owned());
        } else {
            symlink(path, dir.join(format!("lib{name}.{major}.dylib"))).unwrap();
        }
    }
    // Re-export the real ABI but hide named encoder lookup. ID-based lookup still sees the real
    // H.264 encoder, so this test catches an unintended fallback to a different implementation.
    std::fs::write(dir.join("missing.c"),
        "struct AVCodec; const struct AVCodec *avcodec_find_encoder_by_name(const char *name) { (void)name; return 0; }\n").unwrap();
    let output = Command::new("cc")
        .current_dir(dir)
        .args([
            "-dynamiclib",
            "missing.c",
            "-Xlinker",
            "-reexport_library",
            "-Xlinker",
        ])
        .arg(real_codec.unwrap())
        .args(["-o", "libavcodec.63.dylib"])
        .output()
        .unwrap();
    success(&output);
    let output = run(
        dir,
        dir,
        &[
            "--json",
            "motion",
            "render",
            "scene.motion.tsx",
            "--backend",
            "raster",
            "-o",
            "missing.mp4",
        ],
    );
    assert!(!output.status.success());
    let diagnostic: Value = serde_json::from_slice(&output.stdout).unwrap();
    let message = diagnostic["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("libx264") && message.contains("software H.264"),
        "{diagnostic}"
    );
    assert!(
        !dir.join("missing.mp4").exists(),
        "codec failure must precede output creation"
    );
}
