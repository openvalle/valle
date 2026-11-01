//! The case runner: every scene under `tests/cases` is a case, and cases run in parallel. A case
//! in `errors/` names the error it must fail with on its first line, as `// error: …`.
//!
//! `cargo test -p valle-engine --test cases [-- <case>…]` runs the cases, or those under the
//! given paths (`errors`, `errors/style.tsx`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use valle_engine::scene::Scene;

/// The largest canvas, px a side, and the most frames a case may have.
const MAX_SIZE: u32 = 512;
const MAX_FRAMES: i64 = 10;
/// Scene code needs a stack of more than 12 MiB.
const STACK: usize = 32 << 20;

fn main() -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cases");
    let filters: Vec<String> = std::env::args().skip(1).collect();
    match select(&root, &filters) {
        Ok(cases) => run_all(&root, &cases),
        Err(e) => {
            eprintln!("cases: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The cases under `root`, as `/`-separated paths from it, sorted; only those the filters name
/// when there are any.
fn select(root: &Path, filters: &[String]) -> Result<Vec<String>, String> {
    if let Some(flag) = filters.iter().find(|f| f.starts_with('-')) {
        return Err(format!("unknown flag {flag}"));
    }
    let mut cases = Vec::new();
    walk(root, "", &mut cases)?;
    cases.sort();
    let named = |case: &str, filter: &str| {
        let f = filter.trim_end_matches('/');
        case == f || case.strip_suffix(".tsx") == Some(f) || case.starts_with(&format!("{f}/"))
    };
    for f in filters {
        if !cases.iter().any(|c| named(c, f)) {
            return Err(format!("no case is at {f}"));
        }
    }
    if !filters.is_empty() {
        cases.retain(|c| filters.iter().any(|f| named(c, f)));
    }
    Ok(cases)
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path: PathBuf = entry.path();
        if path.is_dir() {
            walk(&path, &format!("{prefix}{name}/"), out)?;
        } else if name.ends_with(".tsx") {
            out.push(format!("{prefix}{name}"));
        }
    }
    Ok(())
}

fn run_all(root: &Path, cases: &[String]) -> ExitCode {
    println!("\nrunning {} cases", cases.len());
    let start = Instant::now();
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<(usize, Duration, Option<String>)>> = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..workers.min(cases.len()) {
            std::thread::Builder::new()
                .stack_size(STACK)
                .spawn_scoped(s, || {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(case) = cases.get(i) else { break };
                        let began = Instant::now();
                        let failure = run(root, case).err();
                        let took = began.elapsed();
                        let status = if failure.is_some() { "FAILED" } else { "ok" };
                        let mut done = done.lock().unwrap_or_else(|e| e.into_inner());
                        println!("case {case} ... {status}");
                        done.push((i, took, failure));
                    }
                })
                .expect("a worker thread");
        }
    });
    let mut done = done.into_inner().unwrap_or_else(|e| e.into_inner());
    done.sort_by_key(|&(i, ..)| i);
    let failed = done.iter().filter(|(.., f)| f.is_some()).count();
    if failed > 0 {
        println!("\nfailures:");
        for (i, _, failure) in &done {
            if let Some(failure) = failure {
                println!("\n---- {}\n{failure}", cases[*i]);
            }
        }
    }
    done.sort_by_key(|&(_, took, _)| std::cmp::Reverse(took));
    println!("\nslowest:");
    for (i, took, _) in done.iter().take(10) {
        println!("  {:>8.3} s  {}", took.as_secs_f64(), cases[*i]);
    }
    let result = if failed == 0 { "ok" } else { "FAILED" };
    println!(
        "\ncase result: {result}. {} passed; {} failed; finished in {:.2}s\n",
        cases.len() - failed,
        failed,
        start.elapsed().as_secs_f64()
    );
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn run(root: &Path, case: &str) -> Result<(), String> {
    let source = std::fs::read_to_string(root.join(case)).map_err(|e| e.to_string())?;
    let expected = source
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("// error: "));
    match case.split('/').next() {
        Some("errors") => {
            let expected =
                expected.ok_or("an errors/ case names its error on its first line: // error: …")?;
            match frames(root, case)? {
                Ok(()) => Err(format!("no error; expected: {expected}")),
                Err(e) if e == expected => Ok(()),
                Err(e) => Err(format!("error:    {e}\nexpected: {expected}")),
            }
        }
        _ => Err("no layer runs cases here yet".into()),
    }
}

/// Every frame of the case's scene: `Ok(Err)` is the scene's error, `Err` the case breaking the
/// runner's rules.
fn frames(root: &Path, case: &str) -> Result<Result<(), String>, String> {
    let read = |path: &str| std::fs::read_to_string(root.join(path)).ok();
    let mut scene = match Scene::load(case, &read) {
        Ok(scene) => scene,
        Err(e) => return Ok(Err(e)),
    };
    let c = scene.composition();
    if c.width > MAX_SIZE || c.height > MAX_SIZE {
        return Err(format!("cases are at most {MAX_SIZE} × {MAX_SIZE} px"));
    }
    let n = c.fps.frames_in(c.duration);
    if n > MAX_FRAMES {
        return Err(format!("cases have at most {MAX_FRAMES} frames, not {n}"));
    }
    for i in 0..n {
        let at = c.fps.frame_start(i)?;
        if let Err(e) = scene.frame(at) {
            return Ok(Err(e));
        }
    }
    Ok(Ok(()))
}
