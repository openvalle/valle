//! Production crate sources must use in-process libav APIs rather than launching ffmpeg or ffprobe.

use std::path::Path;

fn scan(dir: &Path, hits: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            // Scan src directories only; tests legitimately contain forbidden-pattern literals.
            if p.file_name().is_some_and(|n| n == "target" || n == "tests") {
                continue;
            }
            scan(&p, hits);
        } else if p.extension().is_some_and(|x| x == "rs")
            && let Ok(src) = std::fs::read_to_string(&p)
        {
            for forbidden in ["Command::new(\"ffmpeg\"", "Command::new(\"ffprobe\""] {
                if src.contains(forbidden) {
                    hits.push(format!("{} contains `{forbidden}`", p.display()));
                }
            }
        }
    }
}

#[test]
fn no_ffmpeg_cli_shellout_anywhere() {
    // Scan every crate in the workspace.
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut hits = Vec::new();
    scan(&crates, &mut hits);
    assert!(
        hits.is_empty(),
        "forbidden ffmpeg/ffprobe CLI shell-out:\n{}",
        hits.join("\n")
    );
}
