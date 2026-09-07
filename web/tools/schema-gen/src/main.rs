use std::env;
use std::fs;
use std::path::PathBuf;

fn output_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/engine/src/generated/protocol.ts")
}

fn main() {
    let mode = env::args().nth(1).unwrap_or_else(|| "--check".to_owned());
    let generated = valle_web_schema_gen::render_types(mode == "--drift-probe");
    match mode.as_str() {
        "--stdout" | "--drift-probe" => print!("{generated}"),
        "--write" => {
            let path = output_path();
            fs::create_dir_all(path.parent().expect("generated directory"))
                .expect("create generated directory");
            fs::write(&path, generated).expect("write generated protocol");
        }
        "--check" => {
            let path = output_path();
            let checked = fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!(
                    "read {}: {error}; run bun run generate:protocol",
                    path.display()
                )
            });
            assert_eq!(
                checked,
                generated,
                "{} drifted; run bun run generate:protocol",
                path.display()
            );
        }
        other => panic!("unknown mode {other}; use --write, --check, --stdout, or --drift-probe"),
    }
}
