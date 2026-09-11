//! Opt-in process tests for one executable against independently installed FFmpeg ABIs.
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    time::Duration,
};

fn installation(major: u8) -> PathBuf {
    let name = format!("VALLE_TEST_FFMPEG{major}_DIR");
    std::env::var_os(&name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {name} to a complete shared-library installation"))
}
fn run(dir: &Path, libraries: &Path, args: &[&str]) -> Output {
    // Packaging CI exercises the exact Developer ID-signed distribution executable.
    let binary =
        std::env::var_os("VALLE_TEST_BINARY").unwrap_or_else(|| env!("CARGO_BIN_EXE_valle").into());
    Command::new(binary)
        .current_dir(dir)
        .env("VALLE_FFMPEG_DIR", libraries)
        .env("VALLE_HOME", dir.join("home"))
        .env("VALLE_CACHE_DIR", dir.join("cache"))
        .args(args)
        .output()
        .unwrap()
}
fn success(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn scene(dir: &Path) {
    std::fs::write(dir.join("scene.motion.tsx"), "export default function Test(){return <Scene style={{backgroundColor:'#123456'}}><Text style={{fontSize:24}}>ABI</Text></Scene>;}").unwrap();
}
fn render(dir: &Path, libraries: &Path, hardware: bool) {
    let mut args = vec![
        "--json",
        "motion",
        "render",
        "scene.motion.tsx",
        "--duration",
        "0.2",
        "--size",
        "160x90",
        "--fps",
        "30",
        "--backend",
        if hardware { "metal" } else { "raster" },
        "-o",
        "video.mp4",
    ];
    if hardware {
        args.push("--hardware-encode");
    } else {
        args.extend(["--workers", "2", "--encode-threads", "2"]);
    }
    let value = success(&run(dir, libraries, &args));
    assert_eq!(value["frameCount"], 6, "{value}");
    assert!(std::fs::metadata(dir.join("video.mp4")).unwrap().len() > 100);
    success(&run(
        dir,
        libraries,
        &["--json", "assets", "add", "video.mp4", "--mode", "copy"],
    ));
}

#[test]
#[ignore = "requires VALLE_TEST_FFMPEG7_DIR, VALLE_TEST_FFMPEG8_DIR and VALLE_TEST_FFMPEG9_DIR with libx264 and AAC"]
fn one_binary_renders_with_all_ffmpeg_abis() {
    for major in [7, 8, 9] {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        scene(dir);
        let libraries = installation(major);
        let report = success(&run(dir, &libraries, &["--json", "media", "capabilities"]));
        assert_eq!(report["ffmpegMajor"], major);
        assert_eq!(report["libraries"].as_array().unwrap().len(), 5);
        render(dir, &libraries, false);
        std::thread::sleep(Duration::from_secs(2));
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires FFmpeg 7/8/9 installations, Metal and a real VideoToolbox encoder"]
fn all_ffmpeg_abis_support_metal_hardware_export() {
    for major in [7, 8, 9] {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        scene(dir);
        let libraries = installation(major);
        let report = success(&run(dir, &libraries, &["--json", "media", "capabilities"]));
        assert_eq!(report["ffmpegMajor"], major);
        assert_eq!(report["hardware"]["h264VideoToolboxAvailable"], true);
        render(dir, &libraries, true);
        std::thread::sleep(Duration::from_secs(2));
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires both FFmpeg installations; validates real transitive library bindings"]
fn rejects_mixed_installations_before_struct_access() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let old = success(&run(
        dir,
        &installation(8),
        &["--json", "media", "capabilities"],
    ));
    let new = success(&run(
        dir,
        &installation(9),
        &["--json", "media", "capabilities"],
    ));
    let newer_swr = new["libraries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|lib| lib["name"] == "swresample")
        .unwrap();
    for lib in old["libraries"].as_array().unwrap() {
        let name = lib["name"].as_str().unwrap();
        let major = lib["version"].as_str().unwrap().split('.').next().unwrap();
        if name == "swresample" {
            // A deliberately inconsistent forwarding component advertises the expected ABI,
            // but re-exports another installation's dependency chain. Only version functions
            // may execute before the loader rejects it.
            let version = lib["version"]
                .as_str()
                .unwrap()
                .split('.')
                .map(|n| n.parse::<u32>().unwrap())
                .fold(0, |v, n| (v << 8) | n);
            let source = dir.join("mixed.c");
            std::fs::write(
                &source,
                format!("unsigned swresample_version(void) {{ return {version}u; }}\n"),
            )
            .unwrap();
            let status = Command::new("cc")
                .arg("-dynamiclib")
                .arg(&source)
                .args(["-Xlinker", "-reexport_library", "-Xlinker"])
                .arg(newer_swr["path"].as_str().unwrap())
                .arg("-o")
                .arg(dir.join(format!("lib{name}.{major}.dylib")))
                .status()
                .unwrap();
            assert!(status.success());
        } else {
            symlink(
                lib["path"].as_str().unwrap(),
                dir.join(format!("lib{name}.{major}.dylib")),
            )
            .unwrap();
        }
    }
    let out = run(dir, dir, &["--json", "media", "capabilities"]);
    assert!(!out.status.success());
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("mixed FFmpeg library set"),
        "{value}"
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
}
