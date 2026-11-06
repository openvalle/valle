//! Repository tasks, run as `cargo xtask <command>`.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitCode};

/// JavaScript and TypeScript sources, checked by both Prettier and Oxlint.
const SCRIPTS: [&str; 8] = [".js", ".mjs", ".cjs", ".jsx", ".ts", ".mts", ".cts", ".tsx"];

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  fmt [--check]   format the code with rustfmt and Prettier, or only check it
  lint [--fix]    run clippy and Oxlint with warnings as errors, optionally applying fixes
  test [--skip gpu,browser]
                  run Rust, Bun and browser tests; --skip leaves out test layers
  cases [path]    run dependency and scene cases, optionally under one path in tests/cases
  fonts           prepare the pinned test fonts from the download cache
  build-web       build the engine for the browser into web/packages/engine/dist, and
                  Studio into web/apps/studio/dist
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
        "cases" => cases(flags),
        "fonts" if flags.is_empty() => cargo(&[
            "run",
            "--locked",
            "-p",
            "valle-engine",
            "--example",
            "prepare-test-fonts",
        ]),
        "fonts" => Err(unexpected(flags)),
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
    let mut extensions = SCRIPTS.to_vec();
    extensions.extend([".json", ".html"]);
    bun(
        "prettier",
        &[mode, "--log-level=warn"],
        &files(&extensions)?,
    )
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
    bun("oxlint", &args, &files(&SCRIPTS)?)
}

/// Test layers past the first, which `--skip` can leave out: native GPU integration tests, and the
/// engine in Playwright's Chromium with WebGPU, compared with native by `web/tests/player.ts`,
/// with Studio driven end to end by `web/tests/studio.ts`.
const LAYERS: [&str; 2] = ["gpu", "browser"];

fn test(args: &[String]) -> Result<(), String> {
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
            arg => return Err(format!("unexpected argument {arg}")),
        }
    }
    let layers: Vec<&str> = LAYERS.into_iter().filter(|l| !skip.contains(l)).collect();
    run(
        Command::new("bun").args(["install", "--frozen-lockfile", "--silent"]),
        "bun install",
    )?;
    run(
        Command::new("bun").args(["test", "./web"]),
        "bun test ./web",
    )?;
    let command = ["test", "--locked", "--workspace"];
    run(
        Command::new(cargo_path())
            .args(command)
            // A direct case-runner filter must not silently narrow the workspace suite.
            .env_remove("VALLE_CASE")
            .env("VALLE_TEST_LAYERS", layers.join(",")),
        &format!("cargo {}", command.join(" ")),
    )?;
    if !layers.contains(&"browser") {
        return Ok(());
    }
    build_web()?;
    // The Playwright package pins the browser revision; install only the full Chromium build.
    run(
        Command::new("bun").args([
            "run",
            "--bun",
            "playwright",
            "install",
            "chromium",
            "--no-shell",
        ]),
        "playwright install chromium --no-shell",
    )?;
    cargo(&["build", "--locked", "-p", "valle-cli"])?;
    let valle = root().join("target/debug/valle");
    for test in ["web/tests/player.ts", "web/tests/studio.ts"] {
        run(
            Command::new("bun").arg(test).arg(&valle),
            &format!("bun {test}"),
        )?;
    }
    Ok(())
}

fn cases(args: &[String]) -> Result<(), String> {
    let filter = match args {
        [] => "",
        [path] if !path.starts_with('-') && !path.is_empty() => path,
        _ => return Err(unexpected(args)),
    };
    run(
        Command::new(cargo_path())
            .args([
                "test",
                "--locked",
                "-p",
                "valle-engine",
                "--test",
                "cases",
                "scene_cases",
                "--",
                "--exact",
                "--nocapture",
            ])
            .env("VALLE_CASE", filter)
            .env("VALLE_TEST_LAYERS", "gpu"),
        "cargo test -p valle-engine --test cases",
    )
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
    // The SDK bundles packages of package.json: mediabunny.
    let install = ["install", "--frozen-lockfile", "--silent"];
    run(Command::new("bun").args(install), "bun install")?;
    run(
        Command::new("bun")
            .args([
                "build",
                "--target",
                "browser",
                "--format",
                "esm",
                "--splitting",
                "--outdir",
            ])
            .arg(&dist)
            .args(
                ["index", "element", "export", "worker", "sound"]
                    .map(|entry| package.join(format!("src/{entry}.ts"))),
            ),
        "bun build",
    )?;
    let from = package.join("src/wasm/valle_web_bg.wasm");
    std::fs::copy(&from, dist.join("valle_web_bg.wasm"))
        .map_err(|e| format!("{}: {e}", from.display()))?;
    println!("built {}", dist.display());
    // Studio, which the local service serves beside the SDK's runtime.
    let studio = root().join("web/apps/studio");
    let dist = studio.join("dist");
    let _ = std::fs::remove_dir_all(&dist);
    run(
        Command::new("bun")
            .args([
                "build", "--target", "browser", "--format", "esm", "--outdir",
            ])
            .arg(&dist)
            .arg(studio.join("src/main.ts")),
        "bun build",
    )?;
    let from = studio.join("index.html");
    std::fs::copy(&from, dist.join("index.html"))
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
