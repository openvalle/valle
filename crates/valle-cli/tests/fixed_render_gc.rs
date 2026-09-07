use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};
use valle_engine::fixed_package::{
    canonical_fixed_execution_profile, canonical_fixed_package_manifest, fixed_package_files,
};
use valle_project::assets::{Ctx, Home, maintain::gc};
use valle_timeline::internal::{ContentDigest, encode_canonical};

struct Package {
    directory: tempfile::TempDir,
    home: Home,
    digest: ContentDigest,
    files: Vec<(&'static str, PathBuf)>,
    source: PathBuf,
}

impl Package {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let home = Home::at(directory.path().join("home"));
        let source = directory.path().join("resource.png");
        let mut encoder = png::Encoder::new(std::fs::File::create(&source).unwrap(), 8, 8);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[255, 0, 0, 255].repeat(64))
            .unwrap();
        writer.finish().unwrap();
        let digest = ContentDigest::of_bytes(&std::fs::read(&source).unwrap());
        let timeline = valle_timeline::decode_timeline(&json!({
            "canvas": {"width": 8, "height": 8, "fps": 30, "background": "#000000ff"},
            "resources": {"hero": "resource.png"},
            "tracks": {"visual": [{"clips": [{"start": 0, "duration": 1, "kind": "image", "src": "hero"}]}]}
        }).to_string()).unwrap();
        let timeline =
            encode_canonical(&valle_compiler::compile_timeline(timeline).unwrap()).unwrap();
        let descriptor = json!({
            "width": 8, "height": 8, "orientation": "identity",
            "color": {"primaries": "srgb", "transfer": "srgb", "matrix": "identity", "fullRange": true}
        });
        let manifest = json!({"entries": {
            "resource:hero": {"kind": "image", "digest": digest, "descriptor": descriptor}
        }})
        .to_string();
        let bindings = json!({"bindings": {
            "resource:hero": {"digest": digest, "handle": 1, "dependencies": [], "facts": {
                "kind": "image", "descriptor": descriptor, "temporalFootprint": {"pastFrames": 0, "futureFrames": 0}
            }}
        }, "capabilities": {"camera": false, "artifactAbis": [], "extensionKernels": {}}}).to_string();
        let profile = canonical_fixed_execution_profile("common").unwrap();
        let files = fixed_package_files(&timeline, &manifest, &bindings, &profile);
        let package_manifest = canonical_fixed_package_manifest(&files).unwrap();
        let mut paths = Vec::new();
        for (flag, filename, text) in [
            ("--package-manifest", "package.json", package_manifest),
            ("--canonical-timeline", "timeline.json", timeline),
            ("--resource-manifest", "resources.json", manifest),
            ("--verified-binding-bundle", "bindings.json", bindings),
            ("--execution-profile", "profile.json", profile),
        ] {
            let path = directory.path().join(filename);
            std::fs::write(&path, text).unwrap();
            paths.push((flag, path));
        }
        Self {
            directory,
            home,
            digest,
            files: paths,
            source,
        }
    }

    fn run(&self, action: &str, output: Option<&Path>) -> Output {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "fixed_package_test_helper", "--nocapture"])
            .env("VALLE_HOME", self.home.root())
            .env("VALLE_INTERNAL_PACKAGE_TEST", self.directory.path())
            .env("VALLE_INTERNAL_PACKAGE_ACTION", action);
        if let Some(output) = output {
            command.env("VALLE_INTERNAL_PACKAGE_OUTPUT", output);
        }
        command.output().unwrap()
    }

    fn blob(&self) -> PathBuf {
        self.home.object_path(&self.digest.as_hex(), None)
    }
    fn collect(&self) -> usize {
        gc(&Ctx::bare(self.home.clone())).unwrap().removed_blobs
    }
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn native_preview_releases_resources_and_delivers_the_same_pixels() {
    let package = Package::new();
    let output = package.directory.path().join("preview.png");
    success(package.run("preview", Some(&output)));
    let decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(output).unwrap(),
    ));
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!((info.width, info.height), (8, 8));
    let channels = info.color_type.samples();
    for pixel in bytes[..info.buffer_size()].chunks_exact(channels) {
        assert_eq!(&pixel[..3], &[255, 0, 0]);
    }
    assert!(package.blob().exists());
    assert_eq!(package.collect(), 1);
}

#[test]
fn verified_pin_retains_resources_until_explicit_unpin() {
    let package = Package::new();
    success(package.run("pin", None));
    assert_eq!(package.collect(), 0);
    assert!(package.blob().exists());
    std::fs::remove_file(&package.source).unwrap();
    success(package.run(
        "preview",
        Some(&package.directory.path().join("offline.png")),
    ));
    assert_eq!(package.collect(), 0);
    success(package.run("unpin", None));
    assert_eq!(package.collect(), 1);
}

#[test]
fn failed_render_releases_its_lease_without_overwriting_output() {
    let package = Package::new();
    let output = package.directory.path().join("keep.png");
    std::fs::write(&output, b"keep").unwrap();
    assert!(!package.run("preview", Some(&output)).status.success());
    assert_eq!(std::fs::read(output).unwrap(), b"keep");
    assert_eq!(package.collect(), 1);
}

#[test]
fn invalid_package_cannot_publish_a_pin() {
    let package = Package::new();
    std::fs::write(&package.files[1].1, b"{}").unwrap();
    assert!(!package.run("pin", None).status.success());
    assert!(!package.home.root().join("resource-roots/packages").exists());
}

#[test]
fn fixed_package_test_helper() {
    let Some(root) = std::env::var_os("VALLE_INTERNAL_PACKAGE_TEST") else {
        return;
    };
    let root = PathBuf::from(root);
    let output = std::env::var_os("VALLE_INTERNAL_PACKAGE_OUTPUT").map(PathBuf::from);
    let action = match std::env::var("VALLE_INTERNAL_PACKAGE_ACTION")
        .unwrap()
        .as_str()
    {
        "preview" => valle_cli::FixedRenderAction::Preview {
            frame: 0,
            output: output.unwrap(),
        },
        "pin" => valle_cli::FixedRenderAction::Pin,
        "unpin" => valle_cli::FixedRenderAction::Unpin,
        _ => panic!("unknown internal action"),
    };
    let source = root.join("resource.png");
    let package = valle_cli::FixedRenderPackageArgs {
        package_manifest: root.join("package.json"),
        canonical_timeline: root.join("timeline.json"),
        resource_manifest: root.join("resources.json"),
        verified_binding_bundle: root.join("bindings.json"),
        execution_profile: root.join("profile.json"),
        resources: if source.exists() {
            vec![format!("resource:hero={}", source.display())]
        } else {
            vec![]
        },
    };
    valle_cli::cmd::fixed_render::run(&package, action).unwrap();
}
