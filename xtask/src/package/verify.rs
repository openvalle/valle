use super::*;
use serde_json::Value;
use std::{
    process::Child,
    thread,
    time::{Duration, Instant},
};

pub fn verify(binary: &Path) -> Result<()> {
    let platform = host_platform()?;
    let binary = binary.canonicalize()?;
    if cfg!(target_os = "macos") {
        verify_macos(&binary)?;
    }
    // Copy only the executable, outside the checkout and into a path containing spaces.
    let temp = tempfile::Builder::new()
        .prefix("valle-single-binary-")
        .tempdir()?;
    let moved = temp.path().join("relocated Valle");
    fs::create_dir(&moved)?;
    let executable = moved.join(executable_name());
    fs::copy(&binary, &executable)?;
    verify_runtime(&binary, &executable, &temp, platform)
}

fn verify_macos(binary: &Path) -> Result<()> {
    let info = inspect(binary)?;
    ensure!(info.rpaths.is_empty(), "binary has unexpected rpaths");
    for dependency in info.dependencies {
        ensure!(
            system_library(&dependency),
            "unexpected startup dependency: {dependency}"
        );
    }
    super::super::run(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict"])
            .arg(binary),
    )?;
    Ok(())
}

fn verify_runtime(
    binary: &Path,
    executable: &Path,
    temp: &tempfile::TempDir,
    platform: &str,
) -> Result<()> {
    // A regular file cannot host an extracted Web cache. Studio must serve embedded bytes.
    fs::write(temp.path().join("cache"), "no runtime extraction")?;
    let version = capture(clean_command(executable, temp.path()).arg("--version"))?;
    ensure!(
        version.starts_with("valle "),
        "unexpected CLI version: {version}"
    );
    let licenses = capture(clean_command(executable, temp.path()).args(["--json", "licenses"]))?;
    let licenses: Value = serde_json::from_str(&licenses)?;
    ensure!(
        licenses["status"] == "ok" && licenses["complete"] == true,
        "embedded notices are missing"
    );
    let notices = licenses["text"].as_str().context("license text missing")?;
    for required in [
        "Apache License",
        "cssparser",
        "https://static.crates.io/",
        "GL-Transitions-MIT.txt",
        "OFL",
        "CanvasKit",
    ] {
        ensure!(
            notices.contains(required),
            "embedded notices missing {required}"
        );
    }
    let motion = temp.path().join("check.motion.tsx");
    fs::write(
        &motion,
        "export default function Check(ctx) { return <Scene style={{backgroundColor: '#102030'}}><Text style={{fontSize: 24, color: '#ffffff'}}>Valle</Text></Scene>; }\n",
    )?;
    let check = capture(
        clean_command(executable, temp.path())
            .args(["--json", "motion", "check"])
            .arg(&motion)
            .args(["--size", "320x180"]),
    )?;
    let report: Value = serde_json::from_str(&check)?;
    ensure!(
        report["status"] == "ok",
        "Motion check requires FFmpeg unexpectedly: {report}"
    );
    let image = temp.path().join("check.png");
    let mut render = clean_command(executable, temp.path());
    render
        .args(["--json", "motion", "render"])
        .arg(&motion)
        .args([
            "--frame",
            "0",
            "--size",
            "320x180",
            "--backend",
            "raster",
            "-o",
        ])
        .arg(&image);
    let report: Value = serde_json::from_str(&run_bounded(&mut render, temp.path(), "frame", 90)?)?;
    ensure!(
        report["status"] == "ok" && fs::metadata(&image)?.len() > 100,
        "PNG render without FFmpeg failed: {report}"
    );
    let missing = clean_command(executable, temp.path())
        .args(["--json", "media", "capabilities"])
        .output()?;
    ensure!(
        !missing.status.success(),
        "missing FFmpeg unexpectedly loaded"
    );
    let diagnostic: Value = serde_json::from_slice(&missing.stdout)?;
    ensure!(
        diagnostic["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("VALLE_FFMPEG_DIR")),
        "missing-runtime diagnostic: {diagnostic}"
    );
    let ready = temp.path().join("studio.stdout");
    let errors = temp.path().join("studio.stderr");
    let mut command = clean_command(executable, temp.path());
    let child = command
        .args(["--json", "motion", "studio"])
        .arg(&motion)
        .args(["--port", "0", "--size", "320x180"])
        .stdout(fs::File::create(&ready)?)
        .stderr(fs::File::create(&errors)?)
        .spawn()?;
    let mut child = Guard(child);
    let deadline = Instant::now() + Duration::from_secs(30);
    let boot = loop {
        if let Ok(value) = serde_json::from_slice::<Value>(&fs::read(&ready)?) {
            break value;
        }
        ensure!(
            child.0.try_wait()?.is_none() && Instant::now() < deadline,
            "Studio did not start: {}",
            fs::read_to_string(&errors)?
        );
        thread::sleep(Duration::from_millis(100));
    };
    ensure!(
        boot["status"] == "ready" && boot["runtime_source"] == "embedded",
        "Studio did not use embedded resources: {boot}"
    );
    let port = boot["port"].as_u64().context("Studio port missing")?;
    let base = format!("http://127.0.0.1:{port}");
    fetch(&format!("{base}/studio"), &temp.path().join("studio.html"))?;
    let manifest_path = temp.path().join("web-manifest.json");
    fetch(&format!("{base}/runtime/manifest.json"), &manifest_path)?;
    let runtime: Value = serde_json::from_slice(&fs::read(manifest_path)?)?;
    ensure!(
        runtime["schemaVersion"] == 2
            && runtime["runtimeAssets"]["canvasKit"].get("base").is_none(),
        "unexpected CanvasKit base binding"
    );
    for asset in runtime["assets"]
        .as_array()
        .context("runtime asset list missing")?
    {
        let relative = asset["path"].as_str().unwrap();
        {
            let downloaded = temp.path().join("downloaded-asset");
            fetch(&format!("{base}/{relative}"), &downloaded)?;
            ensure!(
                hash_file(&downloaded)? == asset["sha256"].as_str().unwrap(),
                "served runtime asset mismatch: {relative}"
            );
        }
    }
    // Motion mounts its complete native font pack in memory, beyond the base Web manifest font.
    let font_specs: Vec<Value> = serde_json::from_str(include_str!(
        "../../../crates/valle-motion/src/runtime-fonts.json"
    ))?;
    for font in font_specs {
        let name = font["name"].as_str().context("font name missing")?;
        let route = if name.starts_with("KaTeX_") {
            format!("runtime/fonts/katex/{name}")
        } else {
            format!("runtime/fonts/{name}")
        };
        let downloaded = temp.path().join("downloaded-font");
        fetch(&format!("{base}/{route}"), &downloaded)?;
        ensure!(
            hash_file(&downloaded)? == font["sha256"].as_str().unwrap(),
            "served built-in font mismatch: {name}"
        );
    }
    println!(
        "Verified {}: {platform}, single binary, embedded licenses/Web assets, no resource extraction, Motion/PNG/Studio without FFmpeg",
        binary.display()
    );
    Ok(())
}
fn clean_command(binary: &Path, directory: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .current_dir(directory)
        .env("VALLE_CACHE_DIR", directory.join("cache"))
        .env("VALLE_FFMPEG_DIR", directory.join("no-ffmpeg-installed"));
    #[cfg(unix)]
    command.env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
    #[cfg(windows)]
    command.env("PATH", system_tool("curl").parent().unwrap());
    for key in std::env::vars_os().map(|(key, _)| key) {
        if key.to_string_lossy().starts_with("DYLD_") || key.to_string_lossy().starts_with("LD_") {
            command.env_remove(key);
        }
    }
    command
}
fn fetch(url: &str, output: &Path) -> Result<()> {
    super::super::run(
        Command::new(system_tool("curl"))
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--max-time",
                "15",
                "--noproxy",
                "*",
            ])
            .arg(url)
            .arg("--output")
            .arg(output),
    )
}
struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn run_bounded(command: &mut Command, root: &Path, label: &str, seconds: u64) -> Result<String> {
    let output = root.join(format!("{label}.stdout"));
    let errors = root.join(format!("{label}.stderr"));
    let mut child = Guard(
        command
            .stdout(fs::File::create(&output)?)
            .stderr(fs::File::create(&errors)?)
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if let Some(status) = child.0.try_wait()? {
            ensure!(
                status.success(),
                "{label} failed: {}",
                fs::read_to_string(errors)?
            );
            return Ok(fs::read_to_string(output)?);
        }
        ensure!(
            Instant::now() < deadline,
            "{label} timed out: {}",
            fs::read_to_string(&errors)?
        );
        thread::sleep(Duration::from_millis(100));
    }
}
