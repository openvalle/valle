//! Public CLI surface gates for the Timeline greenfield contract.

use std::{path::Path, process::Command};

use serde_json::Value;

fn valle() -> Command {
    Command::new(env!("CARGO_BIN_EXE_valle"))
}

#[test]
fn timeline_studio_accepts_files_and_project_studio_discovers_a_folder() {
    let help = valle()
        .args(["timeline", "studio", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let text = String::from_utf8(help.stdout).unwrap();
    assert!(text.contains("<INPUT>"));
    assert!(text.contains("Save writes back"));
    let project = valle()
        .args(["project", "studio", "--help"])
        .output()
        .unwrap();
    assert!(project.status.success());
    let help = String::from_utf8(project.stdout).unwrap();
    assert!(help.contains("--directory"));
    assert!(!help.contains("PROJECT_ID"));
}

fn listed_help_commands(help: &str) -> Vec<&str> {
    help.lines()
        .skip_while(|line| line.trim() != "Commands:")
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let row = line.strip_prefix("  ")?;
            if row.chars().next().is_some_and(char::is_whitespace) {
                return None;
            }
            row.split_whitespace().next()
        })
        .collect()
}

#[test]
fn root_surface_exposes_only_timeline_authoring_and_non_timeline_tools() {
    let output = valle().arg("--help").output().expect("run valle --help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");

    for command in [
        "docs", "timeline", "motion", "project", "assets", "models", "media",
    ] {
        assert!(help.contains(command), "missing `{command}` in:\n{help}");
    }
    for compatibility_alias in ["transcribe", "matte"] {
        assert!(
            !help
                .lines()
                .any(|line| line.trim_start().starts_with(compatibility_alias)),
            "compatibility alias `{compatibility_alias}` leaked into the public surface:\n{help}"
        );
    }
    for retired in [
        "probe",
        "normalize",
        "preview",
        "preview-frame",
        "preview-source",
        "preview-web",
        "storyboard",
        "inspect-frame",
        "console",
    ] {
        assert!(
            !help
                .lines()
                .any(|line| line.trim_start().starts_with(retired)),
            "retired `{retired}` leaked into:\n{help}"
        );
    }
}

#[test]
fn media_surface_owns_file_level_model_tools() {
    let output = valle()
        .args(["media", "--help"])
        .output()
        .expect("run valle media --help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");
    assert_eq!(
        listed_help_commands(&help),
        [
            "capabilities",
            "transcribe",
            "synthesize",
            "matte",
            "enhance",
            "separate",
            "shots",
            "segment",
            "inpaint",
            "upscale",
            "interpolate",
            "help",
        ],
        "the public media surface contains runtime diagnostics and the ten designed tools"
    );

    let transcribe = valle()
        .args(["media", "transcribe", "--help"])
        .output()
        .expect("run valle media transcribe --help");
    assert!(transcribe.status.success());
    let transcribe_help = String::from_utf8(transcribe.stdout).expect("UTF-8 help");
    for flag in [
        "<INPUT>",
        "--output",
        "--model",
        "--model-version",
        "--backend",
        "--overwrite",
        "--report",
        "--json",
    ] {
        assert!(
            transcribe_help.contains(flag),
            "missing `{flag}` in:\n{transcribe_help}"
        );
    }
    assert!(transcribe_help.contains("qwen3-asr-0.6b"));
    assert!(transcribe_help.contains("accepts `auto` only"));
    assert!(transcribe_help.contains("standalone word-level transcript"));
    assert!(transcribe_help.contains("macOS, Linux and Windows"));
    assert!(transcribe_help.contains(".sentences.json"));
    assert!(transcribe_help.contains("valle models install"));
    assert!(transcribe_help.contains("before offline use"));

    let matte = valle()
        .args(["media", "matte", "--help"])
        .output()
        .expect("run valle media matte --help");
    assert!(matte.status.success());
    let matte_help = String::from_utf8(matte.stdout).expect("UTF-8 help");
    for flag in [
        "<INPUT>",
        "--output",
        "--model",
        "--model-version",
        "--backend",
        "--overwrite",
        "--report",
        "--json",
    ] {
        assert!(
            matte_help.contains(flag),
            "missing `{flag}` in:\n{matte_help}"
        );
    }
    assert!(matte_help.contains("ONNX is available on every supported platform"));
    assert!(matte_help.contains("CoreML is also available on macOS"));

    let enhance = valle()
        .args(["media", "enhance", "--help"])
        .output()
        .expect("run valle media enhance --help");
    assert!(enhance.status.success());
    let enhance_help = String::from_utf8(enhance.stdout).expect("UTF-8 help");
    assert!(
        enhance_help.contains(".flac") && enhance_help.contains("PCM24"),
        "enhance help must disclose FLAC output precision:\n{enhance_help}"
    );
    for flag in [
        "<INPUT>",
        "--output",
        "--model",
        "--model-version",
        "--backend",
        "--overwrite",
        "--report",
        "--json",
    ] {
        assert!(
            enhance_help.contains(flag),
            "missing `{flag}` in:\n{enhance_help}"
        );
    }
    assert!(enhance_help.contains("DPDFNet adapter implements ONNX only"));

    let separate = valle()
        .args(["media", "separate", "--help"])
        .output()
        .expect("run valle media separate --help");
    assert!(separate.status.success());
    let separate_help = String::from_utf8(separate.stdout).expect("UTF-8 help");
    for flag in [
        "<INPUT>",
        "--output",
        "--model",
        "--model-version",
        "--backend",
        "--report",
        "--json",
    ] {
        assert!(
            separate_help.contains(flag),
            "missing `{flag}` in:\n{separate_help}"
        );
    }
    assert!(separate_help.contains("Demucs adapter implements ONNX only"));

    let segment = valle()
        .args(["media", "segment", "--help"])
        .output()
        .expect("run valle media segment --help");
    assert!(segment.status.success());
    let segment_help = String::from_utf8(segment.stdout).expect("UTF-8 help");
    for flag in [
        "<INPUT>",
        "--output",
        "--foreground-output",
        "--prompt",
        "--range",
        "--threshold",
        "--model",
        "--model-version",
        "--backend",
        "--overwrite",
        "--report",
        "--json",
    ] {
        assert!(
            segment_help.contains(flag),
            "missing `{flag}` in:\n{segment_help}"
        );
    }
    assert!(segment_help.contains("EdgeTAM adapter implements ONNX only"));

    let shots = valle()
        .args(["media", "shots", "--help"])
        .output()
        .expect("run valle media shots --help");
    assert!(shots.status.success());
    let shots_help = String::from_utf8(shots.stdout).expect("UTF-8 help");
    for flag in [
        "<INPUT>",
        "--output",
        "--model",
        "--model-version",
        "--backend",
        "--overwrite",
        "--report",
        "--json",
    ] {
        assert!(
            shots_help.contains(flag),
            "missing `{flag}` in:\n{shots_help}"
        );
    }
    assert!(shots_help.contains("shot-detector adapters implement ONNX only"));

    let inpaint = valle()
        .args(["media", "inpaint", "--help"])
        .output()
        .expect("run valle media inpaint --help");
    assert!(inpaint.status.success());
    let inpaint_help = String::from_utf8(inpaint.stdout).expect("UTF-8 help");
    for flag in [
        "<INPUT>",
        "--mask",
        "--output",
        "--range",
        "--model",
        "--model-version",
        "--backend",
        "--overwrite",
        "--report",
        "--json",
    ] {
        assert!(
            inpaint_help.contains(flag),
            "missing `{flag}` in:\n{inpaint_help}"
        );
    }
    assert!(inpaint_help.contains("LaMa adapter implements ONNX only"));

    let upscale = valle()
        .args(["media", "upscale", "--help"])
        .output()
        .expect("run valle media upscale --help");
    assert!(upscale.status.success());
    let upscale_help = String::from_utf8(upscale.stdout).expect("UTF-8 help");
    for flag in [
        "<INPUT>",
        "--output",
        "--range",
        "--scale",
        "--model",
        "--model-version",
        "--backend",
        "--overwrite",
        "--report",
        "--json",
    ] {
        assert!(
            upscale_help.contains(flag),
            "missing `{flag}` in:\n{upscale_help}"
        );
    }
    assert!(upscale_help.contains("Real-ESRGAN adapter implements ONNX only"));

    let interpolate = valle()
        .args(["media", "interpolate", "--help"])
        .output()
        .expect("run valle media interpolate --help");
    assert!(interpolate.status.success());
    let interpolate_help = String::from_utf8(interpolate.stdout).expect("UTF-8 help");
    for flag in [
        "<INPUT>",
        "--output",
        "--fps",
        "--shots",
        "--model",
        "--model-version",
        "--backend",
        "--overwrite",
        "--report",
        "--json",
    ] {
        assert!(
            interpolate_help.contains(flag),
            "missing `{flag}` in:\n{interpolate_help}"
        );
    }
    assert!(interpolate_help.contains("RIFE adapter implements ONNX only"));
}

#[test]
fn media_argument_errors_honor_the_json_channel_contract() {
    let json = valle()
        .args(["media", "upscale", "unused.png", "--scale", "3", "--json"])
        .output()
        .expect("reject invalid media argument in JSON mode");
    assert_eq!(json.status.code(), Some(2));
    assert!(json.stderr.is_empty(), "JSON parse failure leaked stderr");
    let envelope: Value = serde_json::from_slice(&json.stdout).expect("media error envelope");
    assert_eq!(envelope["status"], "error");
    assert_eq!(envelope["error"]["code"], "invalid_arguments");

    let human = valle()
        .args(["media", "upscale", "unused.png", "--scale", "3"])
        .output()
        .expect("reject invalid media argument in human mode");
    assert_eq!(human.status.code(), Some(2));
    assert!(human.stdout.is_empty(), "human parse failure leaked stdout");
    assert!(
        !human.stderr.is_empty(),
        "human parse failure omitted stderr"
    );
}

#[test]
fn segment_multi_output_rejects_overwrite_before_model_resolution() {
    let temporary = tempfile::tempdir().unwrap();
    let input = temporary.path().join("input.png");
    let prompt = temporary.path().join("prompt.json");
    let mask = temporary.path().join("mask.png");
    let foreground = temporary.path().join("foreground.png");
    std::fs::write(&input, b"validation stops before PNG decode").unwrap();
    std::fs::write(&prompt, b"validation stops before prompt decode").unwrap();
    let output = valle()
        .args(["media", "segment"])
        .arg(&input)
        .arg("--prompt")
        .arg(&prompt)
        .arg("--output")
        .arg(&mask)
        .arg("--foreground-output")
        .arg(&foreground)
        .args(["--overwrite", "--json"])
        .output()
        .expect("reject multi-output overwrite");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], "invalid_input");
    assert!(!mask.exists());
    assert!(!foreground.exists());
}

#[test]
fn enhance_help_discloses_flac_precision() {
    let output = valle()
        .args(["media", "enhance", "--help"])
        .output()
        .expect("run valle media enhance --help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");
    assert!(
        help.contains(".wav") && help.contains("float32"),
        "enhance help must disclose WAV precision:\n{help}"
    );
    assert!(
        help.contains(".flac") && help.contains("PCM24"),
        "enhance help must disclose FLAC precision:\n{help}"
    );
}

#[test]
fn models_surface_exposes_explicit_offline_management_contract() {
    let output = valle()
        .args(["models", "--help"])
        .output()
        .expect("run valle models --help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");
    assert_eq!(
        listed_help_commands(&help),
        ["list", "install", "verify", "help"]
    );

    let install = valle()
        .args(["models", "install", "--help"])
        .output()
        .expect("run valle models install --help");
    assert!(install.status.success());
    let install_help = String::from_utf8(install.stdout).expect("UTF-8 help");
    for flag in ["--version", "--backend", "--refresh-catalog", "--json"] {
        assert!(
            install_help.contains(flag),
            "missing `{flag}` in:\n{install_help}"
        );
    }

    let verify = valle()
        .args(["models", "verify", "--help"])
        .output()
        .expect("run valle models verify --help");
    assert!(verify.status.success());
    let verify_help = String::from_utf8(verify.stdout).expect("UTF-8 help");
    for flag in ["--version", "--backend", "--artifact", "--json"] {
        assert!(
            verify_help.contains(flag),
            "missing `{flag}` in:\n{verify_help}"
        );
    }
}

#[test]
fn model_listing_is_machine_readable_and_offline() {
    let temporary = tempfile::tempdir().unwrap();
    let output = valle()
        .env("VALLE_MODEL_CACHE", temporary.path())
        .env(
            "VALLE_MODEL_CATALOG",
            "https://invalid.example/list-must-not-contact-network.json",
        )
        .args(["models", "list", "--json"])
        .output()
        .expect("list embedded models");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "JSON mode leaked stderr");
    let models: Value = serde_json::from_slice(&output.stdout).expect("model status JSON");
    let models = models.as_array().expect("model status array");
    assert!(models.iter().any(|model| model["id"] == "birefnet"));
    assert!(models.iter().any(|model| model["id"] == "demucs"));
    assert!(models.iter().all(|model| model["artifacts"].is_array()));
}

#[test]
fn model_verify_reports_missing_artifacts_without_network() {
    let temporary = tempfile::tempdir().unwrap();
    let output = valle()
        .env("VALLE_MODEL_CACHE", temporary.path())
        .env(
            "VALLE_MODEL_CATALOG",
            "https://invalid.example/verify-must-not-contact-network.json",
        )
        .args(["models", "verify", "birefnet", "--json"])
        .output()
        .expect("verify missing model");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty(), "JSON mode leaked stderr");
    let report: Value = serde_json::from_slice(&output.stdout).expect("verification JSON");
    assert_eq!(report["id"], "birefnet");
    assert!(
        report["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|artifact| artifact["state"] == "missing")
    );
}

#[test]
fn media_invalid_input_uses_exit_two_and_keeps_json_stderr_empty() {
    let output = valle()
        .args(["media", "enhance", "definitely-missing.wav", "--json"])
        .output()
        .expect("reject missing media input");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty(), "JSON mode leaked stderr");
    let envelope: Value = serde_json::from_slice(&output.stdout).expect("media error envelope");
    assert_eq!(envelope["error"]["code"], "invalid_input");
}

#[test]
fn synthesize_requires_one_text_source_and_exposes_voice_cloning_options() {
    let help = valle()
        .args(["media", "synthesize", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    for flag in [
        "--text",
        "--text-file",
        "--reference",
        "--ref-text",
        "--ref-text-file",
        "--output",
        "--lang",
        "--model",
        "--seed",
        "--temperature",
        "--max-tokens",
        "--max-chunk-chars",
        "--report",
        "--json",
    ] {
        assert!(help.contains(flag), "missing {flag}: {help}");
    }
    for text_args in [vec![], vec!["--text", "Hello", "--text-file", "speech.txt"]] {
        let output = valle()
            .args([
                "media",
                "synthesize",
                "--reference",
                "voice.wav",
                "-o",
                "speech.wav",
            ])
            .args(text_args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
}

#[test]
fn synthesize_reports_text_file_errors_and_never_overwrites_a_text_source() {
    let root = tempfile::tempdir().unwrap();
    let text = root.path().join("speech.wav");
    let reference = root.path().join("voice.wav");
    std::fs::write(&text, "你好，Valle。\n").unwrap();
    std::fs::write(&reference, b"not decoded during validation").unwrap();
    for (source, output_path) in [
        (
            root.path().join("missing.txt"),
            root.path().join("output.wav"),
        ),
        (text.clone(), text.clone()),
    ] {
        let output = valle()
            .args(["media", "synthesize", "--text-file"])
            .arg(source)
            .arg("--reference")
            .arg(&reference)
            .arg("-o")
            .arg(output_path)
            .args(["--overwrite", "--json"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stderr.is_empty());
        let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["error"]["code"], "invalid_input");
        assert_eq!(std::fs::read_to_string(&text).unwrap(), "你好，Valle。\n");
    }
}

#[test]
fn synthesize_uses_offline_model_installation_and_keeps_existing_output() {
    let root = tempfile::tempdir().unwrap();
    let reference = root.path().join("voice.wav");
    let output_path = root.path().join("speech.wav");
    std::fs::write(&reference, b"resolution precedes decode").unwrap();
    std::fs::write(&output_path, b"existing audio").unwrap();
    let output = valle()
        .args(["media", "synthesize", "--text", "Hello", "--reference"])
        .arg(reference)
        .arg("-o")
        .arg(&output_path)
        .args(["--overwrite", "--json"])
        .env("VALLE_MODEL_CACHE", root.path().join("empty-model-cache"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty());
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        envelope["error"]["code"],
        if cfg!(all(
            any(
                target_os = "macos",
                target_os = "linux",
                target_os = "windows"
            ),
            any(target_arch = "aarch64", target_arch = "x86_64")
        )) {
            "model_not_installed"
        } else {
            "no_compatible_route"
        }
    );
    assert_eq!(std::fs::read(output_path).unwrap(), b"existing audio");
}

#[test]
fn transcribe_rejects_explicit_non_native_backend_without_resolving_a_model() {
    let temporary = tempfile::tempdir().unwrap();
    let input = temporary.path().join("input.wav");
    std::fs::write(
        &input,
        b"backend validation happens before model resolution",
    )
    .unwrap();
    let output = valle()
        .args(["media", "transcribe"])
        .arg(&input)
        .args(["--backend", "onnx", "--json"])
        .output()
        .expect("reject unsupported explicit transcription backend");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty(), "JSON mode leaked stderr");
    let envelope: Value = serde_json::from_slice(&output.stdout).expect("media error envelope");
    assert_eq!(envelope["error"]["code"], "no_compatible_route");
    assert_eq!(envelope["error"]["hint"], "use --backend auto");
}

#[test]
fn transcribe_requires_a_verified_installed_route_even_if_a_legacy_directory_exists() {
    let temporary = tempfile::tempdir().unwrap();
    let input = temporary.path().join("input.wav");
    let formal = temporary.path().join("formal-models");
    let legacy = temporary.path().join("legacy-models");
    std::fs::write(&input, b"model resolution happens before media decoding").unwrap();
    std::fs::create_dir_all(legacy.join("qwen3-asr-0.6b")).unwrap();

    let output = valle()
        .args(["media", "transcribe"])
        .arg(&input)
        .arg("--json")
        .env("VALLE_MODEL_CACHE", &formal)
        .env("VALLE_LEGACY_MODEL_CACHE", &legacy)
        .output()
        .expect("require installation of the pinned official Qwen artifact");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty(), "JSON mode leaked stderr");
    let envelope: Value = serde_json::from_slice(&output.stdout).expect("media error envelope");
    if cfg!(all(
        any(
            target_os = "macos",
            target_os = "linux",
            target_os = "windows"
        ),
        any(target_arch = "aarch64", target_arch = "x86_64")
    )) {
        assert_eq!(envelope["error"]["code"], "model_not_installed");
        assert!(
            envelope["error"]["hint"]
                .as_str()
                .is_some_and(|hint| hint.contains("valle models install qwen3-asr-0.6b"))
        );
    } else {
        assert_eq!(envelope["error"]["code"], "no_compatible_route");
    }
}

#[test]
fn media_report_cannot_replace_the_primary_output() {
    let temporary = tempfile::tempdir().unwrap();
    let input = temporary.path().join("input.wav");
    let output = temporary.path().join("result.json");
    std::fs::write(&input, b"not decoded because destinations fail first").unwrap();
    let result = valle()
        .args(["media", "transcribe"])
        .arg(&input)
        .arg("--output")
        .arg(&output)
        .arg("--report")
        .arg(&output)
        .arg("--json")
        .output()
        .expect("reject overlapping output and report paths");
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stderr.is_empty(), "JSON mode leaked stderr");
    let envelope: Value = serde_json::from_slice(&result.stdout).expect("media error envelope");
    assert_eq!(envelope["error"]["code"], "invalid_input");
    assert!(!output.exists());
}

#[test]
fn enhance_report_cannot_replace_the_primary_output() {
    let temporary = tempfile::tempdir().unwrap();
    let input = temporary.path().join("input.wav");
    let output = temporary.path().join("enhanced.wav");
    std::fs::write(&input, b"destinations fail before decoding").unwrap();
    let result = valle()
        .args(["media", "enhance"])
        .arg(&input)
        .arg("--output")
        .arg(&output)
        .arg("--report")
        .arg(&output)
        .arg("--json")
        .output()
        .expect("reject overlapping enhance output and report paths");
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stderr.is_empty(), "JSON mode leaked stderr");
    let envelope: Value = serde_json::from_slice(&result.stdout).expect("media error envelope");
    assert_eq!(envelope["error"]["code"], "invalid_input");
    assert!(!output.exists());
}

#[test]
fn assets_exposes_only_folder_and_catalog_operations() {
    let output = valle().args(["assets", "--help"]).output().unwrap();
    let help = String::from_utf8(output.stdout).unwrap();
    for command in [
        "add", "list", "search", "show", "edit", "tag", "annotate", "remove", "recover", "sql",
    ] {
        assert!(help.contains(command), "{help}");
    }
    for command in [
        "analyze",
        "entity",
        "maintenance",
        "resolve",
        "transcript",
        "stats",
    ] {
        assert!(
            !valle()
                .args(["assets", command, "--help"])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

#[test]
fn internal_render_commands_are_not_public() {
    for command in [
        "render",
        "compile",
        "web-runtime",
        "package",
        "matte",
        "transcribe",
    ] {
        assert!(
            !valle()
                .args([command, "--help"])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    assert!(
        !valle()
            .args(["motion", "build", "--help"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn project_surface_uses_folders_and_source_snapshots() {
    let output = valle()
        .args(["project", "--help"])
        .output()
        .expect("run valle project --help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");

    for command in [
        "create", "show", "add", "check", "save", "history", "render", "studio", "restore",
    ] {
        assert!(help.contains(command), "missing `{command}` in:\n{help}");
    }
    for retired in [
        "add-track",
        "add-clip",
        "set-canvas",
        "commit",
        "undo",
        "update-resource-manifest",
        "apply",
    ] {
        assert!(
            !help.contains(retired),
            "retired `{retired}` leaked into:\n{help}"
        );
    }
}

#[test]
fn assets_add_outputs_paths_that_compose_into_show() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("sample.bin");
    std::fs::write(&source, b"asset path contract").unwrap();
    let human = valle()
        .env("VALLE_HOME", temporary.path())
        .args(["assets", "add"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    assert!(
        String::from_utf8(human.stdout)
            .unwrap()
            .starts_with("other/sample.bin")
    );
    let output = valle()
        .env("VALLE_HOME", temporary.path())
        .args(["assets", "--json", "add"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "ok");
    let path = report["items"][0]["path"].as_str().unwrap();
    assert!(report["items"][0].get("digest").is_none());
    let show = valle()
        .env("VALLE_HOME", temporary.path())
        .args(["assets", "--json", "show", path])
        .output()
        .unwrap();
    assert!(show.status.success());
}

#[test]
fn motion_surface_exposes_direct_export() {
    let output = valle()
        .args(["motion", "--help"])
        .output()
        .expect("run valle motion --help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");
    for command in ["check", "review", "render", "studio"] {
        assert!(help.contains(command), "missing `{command}` in:\n{help}");
    }
    let commands: Vec<_> = help
        .lines()
        .filter(|line| line.starts_with("  "))
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    for retired in ["preview"] {
        assert!(
            !commands.contains(&retired),
            "retired `{retired}` leaked into:\n{help}"
        );
    }
}

#[test]
fn motion_render_exports_mp4_and_preserves_existing_output() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("demo.motion.tsx");
    let output = dir.path().join("demo.mp4");
    std::fs::write(
        &source,
        r##"
export const composition = { width: 160, height: 90, fps: 10, duration: 0.3 };
export default function Demo() { const time = useCurrentTime();
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#123456" }}>
    <Text style={{ color: "#ffffff", fontSize: 24 }}>你好 Noto</Text>
    <View style={{ width: 20, height: 20, backgroundColor: "#ff5500", opacity: time.progress }} />
  </Scene>;
}

"##,
    )
    .unwrap();
    let invoke = || {
        valle()
            .args(["motion", "render"])
            .arg(&source)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap()
    };
    let result = invoke();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(report["delivery"]["frames"], 3);
    assert_eq!(report["delivery"]["width"], 160);
    assert_eq!(report["delivery"]["height"], 90);
    let backend = report["delivery"]["backend"].as_str().unwrap();
    if cfg!(target_os = "macos") {
        assert!(["metal", "raster"].contains(&backend));
    } else {
        assert_eq!(backend, "raster");
    }
    let bytes = std::fs::read(&output).unwrap();
    assert_eq!(&bytes[4..8], b"ftyp");
    let repeated = invoke();
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("overwrite"));
    assert_eq!(std::fs::read(&output).unwrap(), bytes);

    let raster = valle()
        .args(["motion", "render"])
        .arg(&source)
        .args([
            "--ffmpeg-log-level",
            "info",
            "--backend",
            "raster",
            "--threads",
            "4",
            "--encode-threads",
            "2",
            "--output-size",
            "320x180",
        ])
        .arg("-o")
        .arg(dir.path().join("raster.mp4"))
        .output()
        .unwrap();
    assert!(
        raster.status.success(),
        "{}",
        String::from_utf8_lossy(&raster.stderr)
    );
    let raster_report: Value = serde_json::from_slice(&raster.stdout).unwrap();
    let stderr = String::from_utf8_lossy(&raster.stderr);
    assert!(stderr.contains("[libx264"), "{stderr}");
    assert!(stderr.contains("Qavg:"), "{stderr}");
    assert_eq!(raster_report["delivery"]["backend"], "raster");
    assert_eq!(raster_report["delivery"]["width"], 320);
    assert_eq!(raster_report["delivery"]["height"], 180);
    assert!(stderr.contains("threads=2 "), "{stderr}");
    assert_eq!(raster_report["renderId"], report["renderId"]);
}

#[test]
fn motion_oklch_interpolation_matches_native_pixel() {
    let dir = tempfile::tempdir().unwrap();
    for fixture in ["oklch-color-interpolation", "float-color-gradient"] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../valle-compiler/tests/fixtures/motion/composition/{fixture}.motion.tsx"
        ));
        let output = dir.path().join(format!("{fixture}.png"));
        let result = valle()
            .args(["motion", "render"])
            .arg(&source)
            .args(["--backend", "raster", "--frame", "30", "-o"])
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{fixture}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let pixels = valle_media::codec::read_rgba_png(&output).unwrap();
        let center = &pixels.data[(180 * 640 + 320) * 4..(180 * 640 + 320) * 4 + 4];
        for (actual, expected) in center.iter().zip([0_u8, 196, 159, 255]) {
            assert!(actual.abs_diff(expected) <= 6, "{fixture} pixel {center:?}");
        }
    }
}

#[test]
fn motion_render_rejects_invalid_source_and_removed_flags_without_output() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("broken.motion.tsx");
    let output = dir.path().join("broken.mp4");
    std::fs::write(&source, "export default function Broken( {").unwrap();
    let result = valle()
        .args(["motion", "render"])
        .arg(&source)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!output.exists());

    for extra in [vec!["--duration", "3"], vec!["--size", "160x90"]] {
        let moved = valle()
            .args(["motion", "render"])
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .args(extra)
            .output()
            .unwrap();
        assert!(!moved.status.success(), "{moved:?}");
        let message = format!(
            "{}{}",
            String::from_utf8_lossy(&moved.stdout),
            String::from_utf8_lossy(&moved.stderr)
        );
        assert!(message.contains("unexpected argument"), "{message}");
        assert!(!output.exists());
    }
}

#[test]
fn timeline_and_project_render_without_internal_package_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("timeline.json");
    let image = dir.path().join("image.png");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../valle-compiler/tests/fixtures/motion/modules/assets/dot.png"),
        &image,
    )
    .unwrap();
    std::fs::write(
        &input,
        serde_json::json!({
            "canvas":{"width":160,"height":90,"fps":10},
            "resources":{"hero":"image.png"},
            "tracks":{"visual":[{"clips":[{"kind":"image","src":"hero","start":0,"duration":0.3}]}]}
        })
        .to_string(),
    )
    .unwrap();
    let invoke = |args: &[&str]| {
        let out = valle()
            .env("VALLE_HOME", dir.path().join("home"))
            .current_dir(std::env::temp_dir())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    };
    invoke(&["timeline", "check", input.to_str().unwrap(), "--json"]);
    let video = dir.path().join("video.mp4");
    invoke(&[
        "timeline",
        "render",
        input.to_str().unwrap(),
        "-o",
        video.to_str().unwrap(),
        "--json",
    ]);
    let frame = dir.path().join("frame.png");
    invoke(&[
        "timeline",
        "render",
        input.to_str().unwrap(),
        "--frame",
        "1",
        "-o",
        frame.to_str().unwrap(),
    ]);
    assert_eq!(&std::fs::read(&frame).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
    std::fs::write(
        dir.path().join("project.json"),
        r#"{"name":"demo","timeline":"timeline.json"}"#,
    )
    .unwrap();
    let project_frame = dir.path().join("project.png");
    invoke(&[
        "project",
        "render",
        "-C",
        dir.path().to_str().unwrap(),
        "--frame",
        "1",
        "-o",
        project_frame.to_str().unwrap(),
    ]);
    let timeline_bytes = std::fs::read(frame).unwrap();
    let project_bytes = std::fs::read(project_frame).unwrap();
    assert!(
        timeline_bytes == project_bytes,
        "Timeline and Project PNG differ ({} vs {} bytes), first differing byte {:?}",
        timeline_bytes.len(),
        project_bytes.len(),
        timeline_bytes
            .iter()
            .zip(&project_bytes)
            .position(|(a, b)| a != b),
    );
    let mut timeline: Value = serde_json::from_slice(&std::fs::read(&input).unwrap()).unwrap();
    timeline["resources"]["hero"] = serde_json::json!(video);
    timeline["tracks"]["visual"][0]["clips"][0]["kind"] = serde_json::json!("video");
    std::fs::write(&input, timeline.to_string()).unwrap();
    let video_frame = dir.path().join("video-frame.png");
    invoke(&[
        "timeline",
        "render",
        input.to_str().unwrap(),
        "--frame",
        "1",
        "-o",
        video_frame.to_str().unwrap(),
    ]);
}

#[test]
fn timeline_renders_motion_source_with_internal_fonts() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("title.motion.tsx"), r##"export const composition = { width: 160, height: 90, fps: 10, duration: 1 };
export default function Title() { return <Scene><Text style={{ fontSize: 24, color: "#ffffff" }}>Hello</Text></Scene>; }"##).unwrap();
    let timeline = dir.path().join("timeline.json");
    std::fs::write(&timeline, serde_json::json!({
        "canvas":{"width":160,"height":90,"fps":10},
        "resources":{"title":"title.motion.tsx"},
        "tracks":{"visual":[{"clips":[{"kind":"motion","component":"title","start":0,"duration":1}]}]}
    }).to_string()).unwrap();
    let frame = dir.path().join("frame.png");
    let out = valle()
        .args(["timeline", "render"])
        .arg(&timeline)
        .args(["--frame", "0", "-o"])
        .arg(&frame)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(frame.exists());
}

#[test]
fn motion_render_rejects_invalid_delivery_options() {
    for extra in [
        vec!["--workers", "0"],
        vec!["--workers", "9"],
        vec!["--encode-threads", "0"],
        vec!["--hardware-encode", "--frame", "0"],
        vec!["--hardware-encode", "--encode-threads", "2"],
        vec!["--hardware-encode", "--bitrate", "0"],
        vec!["--output-size", "0x1080"],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.mp4");
        let result = valle()
            .args(["motion", "render", "missing.motion.tsx", "-o"])
            .arg(&output)
            .args(&extra)
            .output()
            .unwrap();
        assert_eq!(
            result.status.code(),
            Some(2),
            "{extra:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!output.exists());
    }
}

#[test]
fn machine_parse_errors_are_consistent_across_domains() {
    for domain in ["motion", "timeline", "project", "assets", "media", "models"] {
        for mode in ["--json", "--events"] {
            // Global flags work before the domain and after the subcommand.
            for args in [vec![mode, domain, "unknown"], vec![domain, "unknown", mode]] {
                let out = valle().args(&args).output().unwrap();
                assert_eq!(out.status.code(), Some(2), "{args:?}");
                assert!(out.stderr.is_empty(), "{args:?}: {:?}", out.stderr);
                let value: Value = serde_json::from_slice(&out.stdout).unwrap();
                let report = if mode == "--events" {
                    assert_eq!(value["type"], "report");
                    assert_eq!(value["level"], "error");
                    &value["data"]
                } else {
                    &value
                };
                assert_eq!(report["error"]["code"], "invalid_arguments");
            }
        }
    }
}

#[test]
fn positional_output_flag_names_do_not_change_output_mode() {
    for flag in ["--json", "--events"] {
        let home = tempfile::tempdir().unwrap();
        let out = valle()
            .args(["assets", "search", "--", flag])
            .env("VALLE_HOME", home.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "{:?}", out.stderr);
        assert_eq!(
            String::from_utf8(out.stdout).unwrap().trim(),
            "(no matches)"
        );
    }
}

#[test]
fn motion_compile_errors_include_source_diagnostics_in_machine_output() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("broken.motion.tsx");
    std::fs::write(
        &input,
        "export default function Broken() { return <Scene><Unknown /></Scene>; }",
    )
    .unwrap();
    for mode in ["--json", "--events"] {
        for action in ["check", "render"] {
            let mut cmd = valle();
            cmd.args(["motion", action]).arg(&input).arg(mode);
            if action == "render" {
                cmd.arg("-o").arg(dir.path().join("out.mp4"));
            }
            let out = cmd.output().unwrap();
            assert_eq!(out.status.code(), Some(1));
            assert!(
                out.stderr.is_empty(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let report: Value = if mode == "--events" {
                let events = String::from_utf8(out.stdout)
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str::<Value>(line).unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(
                    events
                        .iter()
                        .filter(|event| event["type"] == "motion.compilation")
                        .count(),
                    1
                );
                assert_eq!(events.last().unwrap()["type"], "report");
                events.last().unwrap()["data"].clone()
            } else {
                serde_json::from_slice(&out.stdout).unwrap()
            };
            assert_eq!(report["error"]["code"], "motion_compile_failed", "{report}");
            let diagnostic = &report["error"]["diagnostics"][0];
            assert!(diagnostic["message"].is_string(), "{report}");
            assert!(diagnostic["span"]["line"].is_number(), "{report}");
        }
    }
}

#[test]
fn timeline_and_project_delivery_preserve_authored_background() {
    let dir = tempfile::tempdir().unwrap();
    for (index, (background, expected)) in [
        ("#102030ff", [16_u8, 32, 48, 255]),
        ("#00000000", [0_u8, 0, 0, 0]),
    ]
    .into_iter()
    .enumerate()
    {
        let input = dir.path().join(format!("{index}.json"));
        std::fs::write(
            &input,
            serde_json::json!({
                "canvas": {"width":160,"height":90,"fps":10,"background":background},
                "tracks":{"visual":[{"clips":[
                    {"kind":"solid","color":"#ffffffff","start":0.2,"duration":0.1}
                ]}]}
            })
            .to_string(),
        )
        .unwrap();
        let invoke = |args: Vec<String>, success: bool| {
            let out = valle()
                .args(args)
                .arg("--json")
                .env("VALLE_HOME", dir.path().join("home"))
                .output()
                .unwrap();
            assert_eq!(
                out.status.success(),
                success,
                "{} {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            serde_json::from_slice::<Value>(&out.stdout).unwrap()
        };
        let preview = dir.path().join(format!("{index}.png"));
        invoke(
            vec![
                "timeline".into(),
                "render".into(),
                input.display().to_string(),
                "--frame".into(),
                "0".into(),
                "-o".into(),
                preview.display().to_string(),
            ],
            true,
        );
        let pixels = valle_media::codec::read_rgba_png(&preview).unwrap();
        assert!(pixels.data.chunks_exact(4).all(|p| {
            p.iter()
                .zip(expected)
                .all(|(actual, expected)| actual.abs_diff(expected) <= 1)
        }));
        std::fs::write(
            dir.path().join("project.json"),
            serde_json::json!({"name":format!("background-{index}"),"timeline":format!("{index}.json")}).to_string(),
        ).unwrap();
        let project_preview = dir.path().join(format!("project-{index}.png"));
        invoke(
            vec![
                "project".into(),
                "render".into(),
                "-C".into(),
                dir.path().display().to_string(),
                "--frame".into(),
                "0".into(),
                "-o".into(),
                project_preview.display().to_string(),
            ],
            true,
        );
        assert_eq!(
            std::fs::read(&preview).unwrap(),
            std::fs::read(project_preview).unwrap()
        );
        let video = dir.path().join(format!("{index}.mp4"));
        let result = invoke(
            vec![
                "timeline".into(),
                "render".into(),
                input.display().to_string(),
                "-o".into(),
                video.display().to_string(),
            ],
            index == 0,
        );
        if index == 1 {
            assert!(
                result["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("opaque background")
            );
            assert!(!video.exists());
        }
    }
}
