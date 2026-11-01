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
  test            run the tests
";

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

fn test(flags: &[String]) -> Result<(), String> {
    if !flags.is_empty() {
        return Err(unexpected(flags));
    }
    cargo(&["test", "--locked", "--workspace"])
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
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    run(
        Command::new(cargo).args(args),
        &format!("cargo {}", args.join(" ")),
    )
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
