//! Export/check the build-only Timeline JSON Schema bundle.

use std::{env, fs, path::PathBuf, process::ExitCode};

use valle_timeline::{
    internal::schema::generated_artifacts as generated_internal_artifacts,
    schema::generated_artifacts,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut check = false;
    let mut output_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema");
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--check" => check = true,
            "--out-dir" => {
                output_dir = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--out-dir requires one path".to_owned())?;
            }
            _ => return Err(format!("unknown argument `{argument}`")),
        }
    }

    let mut artifacts = generated_artifacts();
    artifacts.extend(generated_internal_artifacts());

    if check {
        for (name, generated) in artifacts {
            let path = output_dir.join(name);
            let checked = fs::read_to_string(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            if checked != generated {
                return Err(format!(
                    "{} is stale; run `cargo run -p valle-timeline --features schema --bin valle-schema-gen`",
                    path.display()
                ));
            }
        }
    } else {
        fs::create_dir_all(&output_dir)
            .map_err(|error| format!("cannot create {}: {error}", output_dir.display()))?;
        for (name, generated) in artifacts {
            let path = output_dir.join(name);
            fs::write(&path, generated)
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        }
    }
    Ok(())
}
