//! Real Chrome acceptance of the current Studio source editor and browser compiler.
//! Run these on the host machine, outside the sandbox, after rebuilding the web runtime.
use std::path::PathBuf;
use std::process::Command;

fn studio(mode: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime = std::env::var_os("VALLE_WEB_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("web/dist"));
    if !runtime
        .join(valle_cli::webruntime::BUILD_MANIFEST_FILE)
        .is_file()
    {
        assert!(
            std::env::var_os("VALLE_WEB_PLAYER_REQUIRE_BROWSER").is_none()
                && std::env::var_os("VALLE_WEB_PARITY_REQUIRE_BROWSER").is_none(),
            "build the web runtime before browser acceptance"
        );
        eprintln!("skip Studio browser acceptance: web runtime is not built");
        return;
    }
    let output = Command::new("bun")
        .arg(root.join("web/tests/studio-browser.ts"))
        .arg(mode)
        .env("VALLE_TEST_CLI", env!("CARGO_BIN_EXE_valle"))
        .output()
        .expect("run real Chrome Studio acceptance (requires Bun and Chrome)");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn motion_studio_source_drafts_update_preview_and_save() {
    studio("source");
}
#[test]
fn motion_studio_audio_analysis_survives_browser_recompilation() {
    studio("audio");
}
#[test]
fn motion_studio_invalid_source_recovers_the_same_pixels() {
    studio("hot");
}
#[test]
fn module_scenes_compile_in_the_real_studio_browser() {
    studio("modules");
}

#[test]
fn motion_studio_radial_and_film_shaders_survive_browser_recompilation() {
    studio("effects");
}
