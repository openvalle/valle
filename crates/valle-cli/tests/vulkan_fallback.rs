//! The distributed binary must still work on machines without a usable Vulkan driver.
#![cfg(any(target_os = "linux", target_os = "windows"))]

use std::process::Command;

#[test]
fn auto_falls_back_but_explicit_vulkan_fails_without_a_driver() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("scene.motion.tsx");
    std::fs::write(&input, r##"
export const composition = { width: 32, height: 32, fps: 30, duration: 1 };
export default function Frame() {
  return <Scene style={{ width: 32, height: 32, backgroundColor: "#22aacc" }} />;
}
"##).unwrap();
    let missing_driver = directory.path().join("missing-driver.json");
    for backend in ["auto", "vulkan"] {
        let output = directory.path().join(format!("{backend}.png"));
        let result = Command::new(env!("CARGO_BIN_EXE_valle"))
            .args(["motion", "render"])
            .arg(&input)
            .args(["--frame", "0", "--backend", backend, "--json", "-o"])
            .arg(&output)
            // Set both names for older loaders as well as current Vulkan installations.
            .env("VK_DRIVER_FILES", &missing_driver)
            .env("VK_ICD_FILENAMES", &missing_driver)
            .output().unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        if backend == "auto" {
            assert!(result.status.success(), "{stderr}");
            let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
            assert_eq!(report["delivery"]["backend"], "raster");
            assert!(output.is_file());
            assert!(stderr.is_empty(), "automatic fallback must preserve quiet JSON output: {stderr}");
        } else {
            assert!(!result.status.success());
            let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
            assert!(report.to_string().contains("Vulkan compositor unavailable"), "{report}");
            assert!(!output.exists(), "failed initialization must not publish output");
        }
    }
}
