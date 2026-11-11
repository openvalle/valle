//! Repository tasks, run as `cargo xtask <command>`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
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
  package [--lavapipe <driver.br>] [--web-built]
                  build a release executable with Studio embedded into dist/valle (.exe
                  on Windows), for the current platform; on Linux and Windows, with the
                  pinned lavapipe driver embedded too, or the one given; --web-built
                  embeds the browser build already on disk instead of building it
  lavapipe        download the pinned lavapipe driver for this platform into
                  target/lavapipe, check it and print its path, for VALLE_LAVAPIPE
";

/// The valle-lavapipe release Valle draws on without a GPU, and the SHA-256 of each platform's
/// Brotli-compressed driver in it.
const LAVAPIPE: &str = "26.2.6";
const LAVAPIPE_DRIVERS: [(&str, &str); 4] = [
    (
        "linux-x86_64.so",
        "484cd1493f1db9d314dbc055daf1f3071e659ed67c55c9a06ce49831f52a4f8e",
    ),
    (
        "linux-aarch64.so",
        "a58595c2cbf9a55eaeb40a5b644e00579b0cd5fb1b82ef665ff8bbce65521851",
    ),
    (
        "windows-x86_64.dll",
        "545967a49134f1bdd3d7765683006eafe18948f0d9b8cb067ab4e986a6a60f7e",
    ),
    (
        "windows-aarch64.dll",
        "361f161c2c7bef0c6423abd72916eb6e9cc3e79e5288548bf9ad6a2ff0de0019",
    ),
];

/// The wasm-bindgen that makes the browser module's JavaScript: the version `valle-web` pins.
const WASM_BINDGEN: &str = "0.2.130";

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
        "package" => package(flags),
        "lavapipe" if flags.is_empty() => lavapipe().map(|path| println!("{}", path.display())),
        "lavapipe" => Err(unexpected(flags)),
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
    // The entries' declarations, from the same sources: their types for TypeScript.
    let tsconfig = package.join("tsconfig.json");
    run(
        Command::new("bun").args(["x", "tsc", "-p"]).arg(&tsconfig),
        "tsc",
    )?;
    let from = package.join("src/wasm/valle_web_bg.wasm");
    std::fs::copy(&from, dist.join("valle_web_bg.wasm"))
        .map_err(|e| format!("{}: {e}", from.display()))?;
    // npm ships a package's license only from its own directory.
    let license = root().join("LICENSE");
    std::fs::copy(&license, package.join("LICENSE"))
        .map_err(|e| format!("{}: {e}", license.display()))?;
    println!("built {}", dist.display());
    // Studio, which the local service serves beside the SDK's runtime.
    let studio = root().join("web/apps/studio");
    let dist = studio.join("dist");
    let _ = std::fs::remove_dir_all(&dist);
    run(
        Command::new("bun")
            // Packages' production builds: Lit's development one warns on every page.
            .args([
                "build",
                "--production",
                "--target",
                "browser",
                "--format",
                "esm",
            ])
            .arg("--outdir")
            .arg(&dist)
            .arg(studio.join("src/main.ts")),
        "bun build",
    )?;
    std::fs::create_dir_all(dist.join("brand")).map_err(|e| e.to_string())?;
    for file in [
        "index.html",
        "brand/valle-logo-dark.svg",
        "brand/valle-mark.svg",
        "brand/favicon.svg",
    ] {
        let from = studio.join(file);
        std::fs::copy(&from, dist.join(file)).map_err(|e| format!("{}: {e}", from.display()))?;
    }
    println!("built {}", dist.display());
    Ok(())
}

/// A standalone executable: the browser build must precede the native one that embeds it. On
/// Linux and Windows, the driver `--lavapipe` names, valle-lavapipe compressed for this
/// platform, is embedded too, or the pinned one.
fn package(flags: &[String]) -> Result<(), String> {
    let (mut lavapipe, mut web_built) = (None, false);
    let mut flags = flags.iter();
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--lavapipe" => {
                let driver = flags.next().ok_or("--lavapipe takes a driver's path")?;
                lavapipe = Some(Path::new(driver));
            }
            "--web-built" => web_built = true,
            _ => return Err(format!("unexpected argument {flag}")),
        }
    }
    let mut build = Command::new(cargo_path());
    build.args(["build", "--locked", "--release", "-p", "valle-cli"]);
    if cfg!(any(target_os = "linux", target_os = "windows")) {
        let driver = match lavapipe {
            Some(driver) => driver.to_path_buf(),
            None => self::lavapipe()?,
        };
        // `include_bytes!` reads it from the crate's folder unless the path is absolute; not
        // `canonicalize`, whose Windows paths take the `\\?\` prefix.
        let driver =
            (std::path::absolute(&driver)).map_err(|e| format!("{}: {e}", driver.display()))?;
        build.args(["--features", "embed-web,embed-lavapipe"]);
        build.env("VALLE_LAVAPIPE", driver);
    } else if lavapipe.is_some() {
        return Err("lavapipe is embedded on Linux and Windows only".into());
    } else {
        build.args(["--features", "embed-web"]);
    }
    // The browser build is the same on every platform: CI builds it once, where clang builds
    // QuickJS for wasm32, and packages each platform with it.
    if !web_built {
        build_web()?;
    }
    run(&mut build, "cargo build --release -p valle-cli")?;
    let name = format!("valle{}", std::env::consts::EXE_SUFFIX);
    let dist = root().join("dist");
    std::fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
    let output = dist.join(&name);
    let size = std::fs::copy(root().join("target/release").join(name), &output)
        .map_err(|e| format!("{}: {e}", output.display()))?;
    println!(
        "packaged {} ({:.2} MiB)",
        output.display(),
        size as f64 / 1048576.0
    );
    Ok(())
}

/// The pinned lavapipe driver for this platform, in `target/lavapipe`: downloaded unless a copy
/// with its SHA-256 is there.
fn lavapipe() -> Result<PathBuf, String> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "windows" => "windows",
        other => return Err(format!("lavapipe is for Linux and Windows, not {other}")),
    };
    let platform = format!("{os}-{}", std::env::consts::ARCH);
    let (kind, sha256) = (LAVAPIPE_DRIVERS.iter())
        .find(|(kind, _)| kind.rsplit_once('.').is_some_and(|(p, _)| p == platform))
        .ok_or_else(|| format!("valle-lavapipe has no driver for {platform}"))?;
    let name = format!("valle-lavapipe-{LAVAPIPE}-{kind}.br");
    let dir = root().join("target/lavapipe");
    let path = dir.join(&name);
    let digest = |bytes: &[u8]| -> String {
        let d = ring::digest::digest(&ring::digest::SHA256, bytes);
        d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    };
    if std::fs::read(&path).is_ok_and(|bytes| digest(&bytes) == *sha256) {
        return Ok(path);
    }
    let url =
        format!("https://github.com/openvalle/valle-lavapipe/releases/download/v{LAVAPIPE}/{name}");
    eprintln!("downloading {url}");
    let bytes = (ureq::get(&url).call())
        .and_then(|mut r| r.body_mut().with_config().limit(256 << 20).read_to_vec())
        .map_err(|e| format!("{url}: {e}"))?;
    let got = digest(&bytes);
    if got != *sha256 {
        return Err(format!("{url}: SHA-256 {got}, not the pinned {sha256}"));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = dir.join(format!("{name}.{}.part", std::process::id()));
    std::fs::write(&part, &bytes).map_err(|e| format!("{}: {e}", part.display()))?;
    std::fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
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
