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
#[test]
fn native_ffmpeg_imports_stay_inside_the_version_adapters() {
    fn visit(dir: &Path, adapter: &Path, hits: &mut Vec<String>) {
        if dir == adapter {
            return;
        }
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, adapter, hits);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                if source.contains("valle_ffmpeg")
                    || source.contains("ffmpeg_next")
                    || source.contains("ffmpeg_sys_next")
                {
                    hits.push(path.display().to_string());
                }
            }
        }
    }
    let media = Path::new(env!("CARGO_MANIFEST_DIR"));
    let adapter = media.join("src/codec/backend");
    let mut hits = Vec::new();
    for entry in std::fs::read_dir(media.parent().unwrap()).unwrap() {
        let source = entry.unwrap().path().join("src");
        if source.is_dir() {
            visit(&source, &adapter, &mut hits);
        }
    }
    assert!(
        hits.is_empty(),
        "native FFmpeg imports escaped the ABI boundary: {hits:#?}"
    );
}
