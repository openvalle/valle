//! Offline failure contracts for model tools. No model weights or network are needed.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::Value;

const TOOLS: [&str; 4] = ["inpaint", "upscale", "interpolate", "separate"];
const INPUT: &[u8] = b"input must survive every failed run";
const PREVIOUS: &[u8] = b"previous successful output";

struct Fixture {
    root: tempfile::TempDir,
    tool: &'static str,
    input: PathBuf,
    output: PathBuf,
    report: PathBuf,
}

impl Fixture {
    fn new(tool: &'static str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let extension = match tool {
            "inpaint" | "upscale" => "png",
            "separate" => "wav",
            _ => "mp4",
        };
        let input = root.path().join(format!("input.{extension}"));
        std::fs::write(&input, INPUT).unwrap();
        std::fs::write(root.path().join("mask.png"), b"mask fixture").unwrap();
        let output = root.path().join(if tool == "separate" {
            "stems".into()
        } else {
            format!("output.{extension}")
        });
        let report = root.path().join("report.json");
        Self {
            root,
            tool,
            input,
            output,
            report,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_valle"));
        command.env("VALLE_MODEL_CACHE", self.root.path().join("models"));
        command.env("VALLE_LEGACY_MODEL_CACHE", self.root.path().join("legacy"));
        command.env("VALLE_HOME", self.root.path().join("home"));
        command
            .args(["media", self.tool])
            .arg(&self.input)
            .arg("--output")
            .arg(&self.output);
        if self.tool == "inpaint" {
            command.arg("--mask").arg(self.root.path().join("mask.png"));
        }
        command
    }

    fn assert_inputs_and_staging(&self) {
        assert_eq!(std::fs::read(&self.input).unwrap(), INPUT);
        assert_eq!(
            std::fs::read(self.root.path().join("mask.png")).unwrap(),
            b"mask fixture"
        );
        assert_no_staging(self.root.path());
    }
}

fn assert_no_staging(root: &Path) {
    for entry in std::fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        assert!(
            !entry
                .file_name()
                .to_string_lossy()
                .contains(".valle-staging-"),
            "left staging behind: {}",
            entry.path().display()
        );
        if entry.file_type().unwrap().is_dir() {
            assert_no_staging(&entry.path());
        }
    }
}

fn error(result: &Output, exit: i32, code: &str) -> Value {
    assert_eq!(
        result.status.code(),
        Some(exit),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        result.stderr.is_empty(),
        "JSON mode leaked stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let envelope: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], code, "{envelope}");
    envelope
}

#[test]
fn inputs_outputs_and_reports_cannot_alias_even_with_overwrite() {
    for tool in TOOLS {
        for alias in ["input-output", "input-report", "output-report"] {
            let mut fixture = Fixture::new(tool);
            let report = match alias {
                "input-output" => {
                    fixture.output = fixture.input.clone();
                    fixture.report.clone()
                }
                "input-report" => fixture.input.clone(),
                _ => fixture.output.clone(),
            };
            let result = fixture
                .command()
                .arg("--report")
                .arg(report)
                .args(["--overwrite", "--json"])
                .output()
                .unwrap();
            error(&result, 2, "invalid_input");
            fixture.assert_inputs_and_staging();
            assert!(!fixture.report.exists());
        }
    }
}

#[cfg(unix)]
#[test]
fn interpolation_rejects_symlink_aliases_without_touching_the_source() {
    let mut fixture = Fixture::new("interpolate");
    let alias = fixture.root.path().join("alias.mp4");
    std::os::unix::fs::symlink(&fixture.input, &alias).unwrap();
    fixture.output = alias.clone();
    let result = fixture
        .command()
        .args(["--overwrite", "--json"])
        .output()
        .unwrap();
    error(&result, 2, "invalid_input");
    fixture.assert_inputs_and_staging();
    assert!(
        std::fs::symlink_metadata(alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn interpolation_shots_cannot_alias_input_output_or_report() {
    for alias in ["input", "output", "report"] {
        let fixture = Fixture::new("interpolate");
        let shots = match alias {
            "input" => &fixture.input,
            "output" => &fixture.output,
            _ => &fixture.report,
        };
        if alias != "input" {
            std::fs::write(shots, b"protected shots").unwrap();
        }
        let result = fixture
            .command()
            .arg("--shots")
            .arg(shots)
            .arg("--report")
            .arg(&fixture.report)
            .args(["--overwrite", "--json"])
            .output()
            .unwrap();
        error(&result, 2, "invalid_input");
        fixture.assert_inputs_and_staging();
        if alias != "input" {
            assert_eq!(std::fs::read(shots).unwrap(), b"protected shots");
        }
    }
}

#[test]
fn single_file_outputs_require_explicit_overwrite_before_model_resolution() {
    for tool in ["inpaint", "upscale", "interpolate"] {
        let fixture = Fixture::new(tool);
        std::fs::write(&fixture.output, PREVIOUS).unwrap();
        let result = fixture
            .command()
            .arg("--report")
            .arg(&fixture.report)
            .arg("--json")
            .output()
            .unwrap();
        let envelope = error(&result, 2, "invalid_input");
        assert!(
            envelope["error"]["message"]
                .as_str()
                .unwrap()
                .contains("already exists")
        );
        assert_eq!(std::fs::read(&fixture.output).unwrap(), PREVIOUS);
        assert!(!fixture.report.exists());
        fixture.assert_inputs_and_staging();
    }
}

#[test]
fn separate_never_replaces_an_existing_directory_even_with_overwrite() {
    let fixture = Fixture::new("separate");
    std::fs::create_dir(&fixture.output).unwrap();
    let vocals = fixture.output.join("vocals.wav");
    std::fs::write(&vocals, PREVIOUS).unwrap();
    let result = fixture
        .command()
        .arg("--report")
        .arg(&fixture.report)
        .args(["--overwrite", "--json"])
        .output()
        .unwrap();
    let envelope = error(&result, 2, "invalid_input");
    assert!(
        envelope["error"]["message"]
            .as_str()
            .unwrap()
            .contains("output directory already exists")
    );
    assert_eq!(std::fs::read(vocals).unwrap(), PREVIOUS);
    assert!(!fixture.report.exists());
    fixture.assert_inputs_and_staging();
}

#[test]
fn existing_reports_require_overwrite_before_model_resolution() {
    for tool in TOOLS {
        let fixture = Fixture::new(tool);
        std::fs::write(&fixture.report, PREVIOUS).unwrap();
        let result = fixture
            .command()
            .arg("--report")
            .arg(&fixture.report)
            .arg("--json")
            .output()
            .unwrap();
        error(&result, 2, "invalid_input");
        assert_eq!(std::fs::read(&fixture.report).unwrap(), PREVIOUS);
        assert!(!fixture.output.exists());
        fixture.assert_inputs_and_staging();
    }
}

#[test]
fn missing_models_keep_existing_media_and_reports_and_discard_report_staging() {
    for tool in TOOLS {
        let fixture = Fixture::new(tool);
        std::fs::write(&fixture.report, PREVIOUS).unwrap();
        if tool != "separate" {
            std::fs::write(&fixture.output, PREVIOUS).unwrap();
        }
        let result = fixture
            .command()
            .arg("--report")
            .arg(&fixture.report)
            .args(["--overwrite", "--json"])
            .output()
            .unwrap();
        let envelope = error(&result, 3, "model_not_installed");
        assert!(
            envelope["error"]["hint"]
                .as_str()
                .unwrap()
                .contains("valle models install")
        );
        assert_eq!(std::fs::read(&fixture.report).unwrap(), PREVIOUS);
        if tool != "separate" {
            assert_eq!(std::fs::read(&fixture.output).unwrap(), PREVIOUS);
        } else {
            assert!(!fixture.output.exists());
        }
        fixture.assert_inputs_and_staging();
    }
}

#[test]
fn invalid_interpolation_fps_and_output_format_return_input_errors() {
    for fps in ["0", "1001"] {
        let fixture = Fixture::new("interpolate");
        let result = fixture
            .command()
            .args(["--fps", fps, "--json"])
            .output()
            .unwrap();
        error(&result, 2, "invalid_input");
        assert!(!fixture.output.exists());
    }
    let mut fixture = Fixture::new("interpolate");
    fixture.output = fixture.root.path().join("output.png");
    let result = fixture.command().arg("--json").output().unwrap();
    error(&result, 2, "invalid_input");
    fixture.assert_inputs_and_staging();
}

#[test]
fn missing_or_malformed_shots_fail_before_model_resolution_and_publication() {
    for contents in [None, Some(b"{invalid".as_slice())] {
        let fixture = Fixture::new("interpolate");
        let shots = fixture.root.path().join("shots.json");
        if let Some(contents) = contents {
            std::fs::write(&shots, contents).unwrap();
        }
        let result = fixture
            .command()
            .arg("--shots")
            .arg(shots)
            .arg("--report")
            .arg(&fixture.report)
            .arg("--json")
            .output()
            .unwrap();
        error(&result, 2, "invalid_input");
        assert!(!fixture.output.exists());
        assert!(!fixture.report.exists());
        fixture.assert_inputs_and_staging();
    }
}

#[test]
fn human_errors_keep_stdout_empty_and_explain_the_failure() {
    for tool in ["interpolate", "separate"] {
        let fixture = Fixture::new(tool);
        let result = fixture.command().output().unwrap();
        assert_eq!(result.status.code(), Some(3));
        assert!(result.stdout.is_empty());
        let stderr = String::from_utf8(result.stderr).unwrap();
        assert!(stderr.contains("ModelNotInstalled"), "{stderr}");
        assert!(stderr.contains("valle models install"), "{stderr}");
        fixture.assert_inputs_and_staging();
    }
}
