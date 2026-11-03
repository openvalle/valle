//! Repository tasks, run as `cargo xtask <command>`.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitCode};

/// What Prettier formats and Oxlint checks, by extension.
const SCRIPTS: [&str; 9] = [
    ".js", ".mjs", ".cjs", ".jsx", ".ts", ".mts", ".cts", ".tsx", ".json",
];

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  fmt [--check]   format the code with rustfmt and Prettier, or only check it
  lint [--fix]    run clippy and Oxlint with warnings as errors, optionally applying fixes
  test [case...] [--skip gpu,browser]
                  run the tests, or only the cases under tests/cases at these paths;
                  --skip leaves out test layers
  accept case...  take the frames the last test run drew for these cases as their baselines
  build-web       build the engine for the browser into web/packages/engine/dist
";

/// The wasm-bindgen that makes the browser module's JavaScript: the version `valle-web` pins.
const WASM_BINDGEN: &str = "0.2.128";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, flags)) = args.split_first() else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let result = match command.as_str() {
        "fmt" => fmt(flags),
        "lint" => lint(flags),
        "test" => test(flags),
        "accept" => accept(flags),
        "build-web" if flags.is_empty() => build_web(),
        "build-web" => Err(unexpected(flags)),
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(())
        }
        _ => Err(format!("unknown command `{command}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn fmt(flags: &[String]) -> Result<(), String> {
    let check = switch(flags, "--check")?;
    let mut args = vec!["fmt", "--all"];
    if check {
        args.push("--check");
    }
    cargo(&args)?;
    let mode = if check { "--check" } else { "--write" };
    bun("prettier", &[mode, "--log-level=warn"], &files(&SCRIPTS)?)
}

fn lint(flags: &[String]) -> Result<(), String> {
    let fix = switch(flags, "--fix")?;
    let mut args = vec!["clippy", "--locked", "--workspace", "--all-targets"];
    if fix {
        args.extend(["--fix", "--allow-dirty", "--allow-staged"]);
    }
    args.extend(["--", "-D", "warnings"]);
    cargo(&args)?;
    // The browser's engine, which the workspace's own target leaves empty.
    run(
        wasm(&["clippy", "--locked"]).args(["--", "-D", "warnings"]),
        "cargo clippy -p valle-web --target wasm32-unknown-unknown",
    )?;
    let mut args = vec!["--deny-warnings", "--report-unused-disable-directives"];
    if fix {
        args.push("--fix");
    }
    let scripts: Vec<String> = files(&SCRIPTS)?
        .into_iter()
        .filter(|f| !f.ends_with(".json"))
        .collect();
    bun("oxlint", &args, &scripts)
}

/// Test layers past the first, which `--skip` can leave out: the cases the GPU draws, and the
/// engine in Chrome with WebGPU, which `web/tests/player.ts` compares with the native engine.
const LAYERS: [&str; 2] = ["gpu", "browser"];

fn test(args: &[String]) -> Result<(), String> {
    let mut cases = Vec::new();
    let mut skip = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--skip" => {
                let list = args
                    .next()
                    .ok_or("--skip takes layers, as in --skip gpu,browser")?;
                for layer in list.split(',') {
                    if !LAYERS.contains(&layer) {
                        return Err(format!("no test layer is called {layer}"));
                    }
                    skip.push(layer);
                }
            }
            flag if flag.starts_with('-') => return Err(format!("unexpected flag {flag}")),
            case => cases.push(case),
        }
    }
    let layers: Vec<&str> = LAYERS.into_iter().filter(|l| !skip.contains(l)).collect();
    let mut command = vec!["test", "--locked"];
    if cases.is_empty() {
        command.push("--workspace");
    } else {
        command.extend(["-p", "valle-engine", "--test", "cases", "--"]);
        command.extend(&cases);
    }
    run(
        Command::new(cargo_path())
            .args(&command)
            .env("VALLE_TEST_LAYERS", layers.join(",")),
        &format!("cargo {}", command.join(" ")),
    )?;
    if !cases.is_empty() || !layers.contains(&"browser") {
        return Ok(());
    }
    build_web()?;
    cargo(&["build", "--locked", "-p", "valle-cli"])?;
    let valle = root().join("target/debug/valle");
    run(
        Command::new("bun").arg("web/tests/player.ts").arg(valle),
        "bun web/tests/player.ts",
    )
}

/// Takes the frames the last test run drew for `cases` as their Valle baselines.
fn accept(cases: &[String]) -> Result<(), String> {
    if cases.is_empty() {
        return Err("name the cases to accept, as shown on target/review/index.html".into());
    }
    if let Some(flag) = cases.iter().find(|c| c.starts_with('-')) {
        return Err(format!("unexpected flag {flag}"));
    }
    let review = root().join("target/review");
    let mut drawn = Vec::new();
    frames(&review, "", &mut drawn)?;
    for case in cases {
        let case = case.trim_end_matches('/');
        let case = case.strip_suffix(".tsx").unwrap_or(case);
        let standard = ["css", "svg", "tailwind"]
            .iter()
            .any(|d| case == *d || case.starts_with(&format!("{d}/")));
        if standard {
            return Err(format!(
                "{case}: Chrome draws the css, svg and tailwind baselines; a case that differs \
                 is the engine's to fix"
            ));
        }
        let mine: Vec<&String> = drawn
            .iter()
            .filter(|f| {
                let stem = f.rsplitn(3, '.').nth(2).unwrap_or_default();
                stem == case || stem.starts_with(&format!("{case}/"))
            })
            .collect();
        if mine.is_empty() {
            return Err(format!(
                "the last test run drew nothing to review for {case}"
            ));
        }
        for frame in mine {
            let to = root().join("tests/baselines/valle").join(frame);
            if let Some(dir) = to.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::copy(review.join(frame), &to).map_err(|e| format!("{frame}: {e}"))?;
            println!("accepted {frame}");
        }
    }
    Ok(())
}

/// The drawn frames under `dir`, `<case>.<frame>.png`, as paths from it.
fn frames(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) if prefix.is_empty() => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() {
            frames(&entry.path(), &format!("{prefix}{name}/"), out)?;
            continue;
        }
        let frame = name.strip_suffix(".png").and_then(|n| n.rsplit_once('.'));
        if frame.is_some_and(|(_, i)| !i.is_empty() && i.bytes().all(|b| b.is_ascii_digit())) {
            out.push(format!("{prefix}{name}"));
        }
    }
    Ok(())
}

/// Cargo's `command` for `valle-web` in the browser.
fn wasm(command: &[&str]) -> Command {
    let mut cargo = Command::new(cargo_path());
    cargo.args(command);
    cargo.args(["-p", "valle-web", "--target", "wasm32-unknown-unknown"]);
    // QuickJS is C, and Apple's clang builds no wasm32: Homebrew's LLVM does.
    let llvm = Path::new("/opt/homebrew/opt/llvm/bin");
    if std::env::var_os("CC_wasm32_unknown_unknown").is_none() && llvm.join("clang").exists() {
        cargo.env("CC_wasm32_unknown_unknown", llvm.join("clang"));
        cargo.env("AR_wasm32_unknown_unknown", llvm.join("llvm-ar"));
    }
    cargo
}

/// Builds `valle-web` for wasm32, makes its JavaScript with the pinned wasm-bindgen, installed
/// into `target/tools` the first time, and bundles the SDK with its worker beside them.
fn build_web() -> Result<(), String> {
    run(
        &mut wasm(&["build", "--locked", "--release"]),
        "cargo build -p valle-web",
    )?;
    let tools = root().join("target/tools");
    let bindgen = tools.join("bin/wasm-bindgen");
    let version = Command::new(&bindgen).arg("--version").output();
    let installed =
        version.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(WASM_BINDGEN));
    if !installed {
        let tools = tools.to_string_lossy();
        let args = [
            "install",
            "--locked",
            "--force",
            "wasm-bindgen-cli",
            "--version",
            WASM_BINDGEN,
            "--root",
            &tools,
        ];
        cargo(&args)?;
    }
    let package = root().join("web/packages/engine");
    let wasm = root().join("target/wasm32-unknown-unknown/release/valle_web.wasm");
    run(
        Command::new(&bindgen)
            .args(["--target", "web", "--remove-name-section", "--out-dir"])
            .arg(package.join("src/wasm"))
            .arg(&wasm),
        "wasm-bindgen",
    )?;
    let dist = package.join("dist");
    let _ = std::fs::remove_dir_all(&dist);
    run(
        Command::new("bun")
            .args([
                "build", "--target", "browser", "--format", "esm", "--outdir",
            ])
            .arg(&dist)
            .arg(package.join("src/index.ts"))
            .arg(package.join("src/worker.ts"))
            .arg(package.join("src/sound.ts")),
        "bun build",
    )?;
    let from = package.join("src/wasm/valle_web_bg.wasm");
    std::fs::copy(&from, dist.join("valle_web_bg.wasm"))
        .map_err(|e| format!("{}: {e}", from.display()))?;
    println!("built {}", dist.display());
    Ok(())
}

/// Accepts either no flags or exactly `name`, and reports which.
fn switch(flags: &[String], name: &str) -> Result<bool, String> {
    match flags {
        [] => Ok(false),
        [flag] if flag == name => Ok(true),
        _ => Err(unexpected(flags)),
    }
}

fn unexpected(flags: &[String]) -> String {
    format!("unexpected arguments: {}", flags.join(" "))
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root")
}

/// Runs the cargo that started xtask, from the workspace root.
fn cargo(args: &[&str]) -> Result<(), String> {
    run(
        Command::new(cargo_path()).args(args),
        &format!("cargo {}", args.join(" ")),
    )
}

fn cargo_path() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

/// Runs a tool of `package.json` on `files` through bun, after installing the locked packages.
fn bun(tool: &str, args: &[&str], files: &[String]) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }
    let install = ["install", "--frozen-lockfile", "--silent"];
    run(Command::new("bun").args(install), "bun install")?;
    run(
        Command::new("bun")
            .args(["run", "--bun", tool])
            .args(args)
            .args(files),
        &format!("{tool} {}", args.join(" ")),
    )
}

/// The repository's files with one of these extensions: tracked, or new and not ignored.
fn files(extensions: &[&str]) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root())
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if !output.status.success() {
        return Err(format!("`git ls-files` failed ({})", output.status));
    }
    let list = String::from_utf8(output.stdout).map_err(|_| "git listed a non-UTF-8 path")?;
    Ok(list
        .split('\0')
        .filter(|f| extensions.iter().any(|e| f.ends_with(e)))
        // Tracked files deleted from the working tree are listed too.
        .filter(|f| root().join(f).exists())
        .map(String::from)
        .collect())
}

/// Runs `command` from the workspace root; `what` names it in errors.
fn run(command: &mut Command, what: &str) -> Result<(), String> {
    let status = command
        .current_dir(root())
        .status()
        .map_err(|error| format!("cannot run `{what}`: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{what}` failed ({status})"))
    }
}
