//! Scene cases with explicit expectations, run with `cargo xtask cases [path]`.

mod browser;
mod case;
mod d3;
mod fonts;
mod pixels;
mod result;
mod scene;
mod seek;
mod tailwind;

use std::path::{Path, PathBuf};
use valle_engine::gpu::Gpu;

#[test]
fn scene_cases() {
    with_stack(run).unwrap_or_else(|error| panic!("{error}"));
}

fn with_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    // QuickJS reserves a 12 MiB stack; use the same allowance as the scene unit tests.
    std::thread::Builder::new()
        .name("scene cases".into())
        .stack_size(32 << 20)
        .spawn(f)
        .expect("start scene cases")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

fn run() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cases = root.join("tests/cases");
    let filter = std::env::var("VALLE_CASE").unwrap_or_default();
    let filter = filter.trim_end_matches('/');
    let selected =
        |name: &str| filter.is_empty() || name == filter || name.starts_with(&format!("{filter}/"));
    let mut paths = Vec::new();
    discover(&cases, &mut paths)?;
    paths.sort();
    let mut count = 0;
    let mut checks = 0;
    let mut failures = Vec::new();
    let mut dependencies = 0;
    for (name, check) in [
        ("d3/cases", d3::check as fn(&Path) -> Result<usize, String>),
        ("tailwind/classes", tailwind::check),
    ] {
        if !selected(name) {
            continue;
        }
        dependencies += 1;
        let result = check(&root);
        let out = root.join("target/test-results").join(name);
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        let report = serde_json::json!({
            "case": name, "status": if result.is_ok() { "PASS" } else { "FAIL" },
            "checks": result.as_ref().ok(), "error": result.as_ref().err(),
        });
        std::fs::write(
            out.join("result.json"),
            serde_json::to_string_pretty(&report).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        match result {
            Ok(n) => println!("PASS {name}: {n} dependency checks"),
            Err(error) => failures.push(format!("FAIL {name}\n{error}")),
        }
    }
    let gpu_enabled = std::env::var("VALLE_TEST_LAYERS")
        .unwrap_or_default()
        .split(',')
        .any(|layer| layer == "gpu");
    let mut gpu = None;
    for path in paths {
        let name = path
            .strip_prefix(&cases)
            .expect("a case under tests/cases")
            .to_string_lossy()
            .replace('\\', "/");
        if !selected(&name) {
            continue;
        }
        count += 1;
        let result: Result<usize, String> = (|| {
            let out = root.join("target/test-results").join(&name);
            if out.exists() {
                std::fs::remove_dir_all(&out).map_err(|e| e.to_string())?;
            }
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            let case = case::Case::read(&path.join("case.json"))?;
            let n = run_case(&root, &path, &name, &case, gpu_enabled, &mut gpu)?;
            let mut modes = Vec::new();
            if !case.layout.is_empty() {
                modes.push("boxes");
            }
            if !case.lines.is_empty() {
                modes.push("lines");
            }
            if case.visual() {
                modes.push(if gpu_enabled {
                    "pixels"
                } else {
                    "pixels skipped"
                });
            }
            if case.at.len() > 1 {
                modes.push("seek");
            }
            let status = if case.visual() && !gpu_enabled {
                "PARTIAL"
            } else {
                "PASS"
            };
            println!(
                "{status} {name}: {}; {n} numerical assertions, {} sampled frames",
                modes.join(", "),
                case.at.len()
            );
            Ok(n)
        })();
        match result {
            Ok(n) => {
                checks += n;
            }
            Err(error) => failures.push(format!("FAIL {name}\n{error}")),
        }
    }
    if count + dependencies == 0 {
        return Err(format!("no scene cases match {filter:?}"));
    }
    if !failures.is_empty() {
        return Err(format!(
            "{}\n{} cases failed",
            failures.join("\n\n"),
            failures.len()
        ));
    }
    println!(
        "{count} scene cases and {dependencies} dependency suites checked; {checks} layout/line assertions passed{}",
        if gpu_enabled {
            ""
        } else {
            "; image checks skipped"
        }
    );
    Ok(())
}

fn run_case(
    root: &Path,
    dir: &Path,
    name: &str,
    case: &case::Case,
    gpu_enabled: bool,
    gpu: &mut Option<Gpu>,
) -> Result<usize, String> {
    let out = root.join("target/test-results").join(name);
    let times = case.times()?;
    let mut snapshots = Vec::new();
    let mut failures = Vec::new();
    let mut checks = 0;
    let mut records = Vec::new();
    for at in times {
        let sample = scene::load(root, &case.input(dir), at)?;
        let number = sample.number();
        let sample_out = if case.at.len() == 1 {
            out.clone()
        } else {
            out.join(number.to_string())
        };
        if case.at.len() > 1 {
            // Multiple samples in one frame would overwrite the same output and baseline.
            std::fs::create_dir(&sample_out)
                .map_err(|e| format!("frame {number}: cannot create sample directory: {e}"))?;
        }
        let checked = run_sample(root, dir, case, &sample, gpu_enabled, gpu, &sample_out);
        records
            .push(serde_json::json!({ "frame": number, "at": at.seconds_text(), "error": checked.as_ref().err() }));
        match checked {
            Ok(n) => checks += n,
            Err(error) => failures.push(format!(
                "frame {number}, at={}s: {error}",
                at.seconds_text()
            )),
        }
        snapshots.push(sample.frame);
    }
    let seek = if case.at.len() > 1 {
        Some(seek::check(
            root,
            dir,
            case,
            &snapshots,
            gpu.as_ref().filter(|_| gpu_enabled && case.visual()),
            &out,
        ))
    } else {
        None
    };
    if let Some(Err(error)) = &seek {
        failures.push(error.clone());
    }
    if case.at.len() > 1 {
        let seek_report = match &seek {
            Some(Ok(visits)) => {
                serde_json::json!({ "status": "passed", "visits": visits, "pixels": gpu_enabled && case.visual() })
            }
            Some(Err(error)) => serde_json::json!({ "status": "failed", "error": error }),
            None => serde_json::Value::Null,
        };
        let report = serde_json::json!({
            "case": name, "status": if !failures.is_empty() { "FAIL" } else if !gpu_enabled && case.visual() { "PARTIAL" } else { "PASS" },
            "samples": records, "seek": seek_report,
        });
        std::fs::write(
            out.join("result.json"),
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    println!("results: {}", out.display());
    if failures.is_empty() {
        Ok(checks)
    } else {
        Err(failures.join("\n"))
    }
}

fn run_sample(
    root: &Path,
    dir: &Path,
    case: &case::Case,
    sample: &scene::Sample,
    gpu_enabled: bool,
    gpu: &mut Option<Gpu>,
    out: &Path,
) -> Result<usize, String> {
    let layout = scene::layout(&sample.frame, case);
    let pixels = (|| {
        if !case.visual() || !gpu_enabled {
            return Ok(None);
        }
        if gpu.is_none() {
            *gpu = Some(valle_native::gpu::adapter(None).and_then(|c| Gpu::with(c.adapter))?);
        }
        pixels::run(root, dir, case, sample, gpu.as_ref().unwrap(), out).map(Some)
    })();
    result::write(
        out,
        &dir.strip_prefix(root.join("tests/cases"))
            .map_err(|e| e.to_string())?
            .to_string_lossy(),
        case,
        sample,
        &layout,
        &pixels,
    )?;
    let mut errors = Vec::new();
    if let Err(error) = &layout {
        errors.push(error.clone());
    }
    match pixels {
        Ok(Some(p)) => {
            for check in &p.checks {
                if check.required && !check.passed {
                    errors.push(format!("{}: {} image pixels (max {}), {} text-edge pixels (max {}); measurements: {}", check.source, check.pixels.changed, check.pixels.max, check.text_edges.changed, check.text_edges.max, check.measurement_error.as_deref().unwrap_or("passed")));
                }
            }
        }
        Ok(None) => {}
        Err(error) => errors.push(error),
    }
    if errors.is_empty() {
        layout
    } else {
        Err(errors.join("\n"))
    }
}

fn discover(dir: &Path, cases: &mut Vec<PathBuf>) -> Result<(), String> {
    if dir.join("case.json").is_file() {
        cases.push(dir.to_path_buf());
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            discover(&entry.path(), cases)?;
        }
    }
    Ok(())
}

fn version(root: &Path, name: &str) -> Result<String, String> {
    let bytes = std::fs::read(root.join("package.json")).map_err(|e| e.to_string())?;
    let package: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    package["devDependencies"][name]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("package.json does not pin {name}"))
}

#[test]
fn incorrect_expectations_fail_against_a_real_scene() {
    with_stack(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = root.join("tests/cases/css/box-model/content-box");
        let mut case = case::Case::read(&dir.join("case.json")).unwrap();

        case.layout[0].border.width = 129.0;
        let error = scene::check(&root, &dir, &case).unwrap_err();
        assert!(
            error.contains("box.border.width: expected 129, actual 128"),
            "{error}"
        );

        case.layout[0].border.width = 128.0;
        case.layout[0].key = "missing".into();
        let error = scene::check(&root, &dir, &case).unwrap_err();
        assert!(error.contains("key \"missing\": no node"), "{error}");

        case.layout[0].key = "box".into();
        case.at = vec![1.into()];
        let error = scene::check(&root, &dir, &case).unwrap_err();
        assert!(
            error.contains("at=1s is outside the scene duration"),
            "{error}"
        );
    });
}

#[test]
fn text_checks_detect_changed_spacing_and_line_geometry() {
    with_stack(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = root.join("tests/cases/css/text/preserved-spaces");
        let case = case::Case::read(&dir.join("case.json")).unwrap();
        let sample = scene::load(&root, &dir.join("scene.tsx"), case.times().unwrap()[0]).unwrap();
        for broken in ["text", "x", "baseline", "height"] {
            let mut m = scene::measure(&sample.frame);
            match broken {
                "text" => m.lines[0].text[0] = "Valle motion".into(),
                "x" => m.lines[0].geometry[0].x += 1.0,
                "baseline" => m.lines[0].geometry[1].baseline += 1.0,
                "height" => m.lines[0].geometry[0].height += 1.0,
                _ => unreachable!(),
            }
            assert!(m.check(&case).is_err(), "{broken}");
        }
    });
}
