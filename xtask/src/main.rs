//! Repository tasks, run as `cargo xtask <command>`.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  fmt [--check]   format the code, or only check that it is formatted
  lint [--fix]    run clippy with warnings as errors, optionally applying fixes
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
    let mut args = vec!["fmt", "--all"];
    if switch(flags, "--check")? {
        args.push("--check");
    }
    cargo(&args)
}

fn lint(flags: &[String]) -> Result<(), String> {
    let mut args = vec!["clippy", "--locked", "--workspace", "--all-targets"];
    if switch(flags, "--fix")? {
        args.extend(["--fix", "--allow-dirty", "--allow-staged"]);
    }
    args.extend(["--", "-D", "warnings"]);
    cargo(&args)
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

/// Runs the cargo that started xtask, from the workspace root.
fn cargo(args: &[&str]) -> Result<(), String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root");
    let status = Command::new(&cargo)
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed ({status})", args.join(" ")))
    }
}
