//! `valle motion check` warns where the scene's structure follows values that change, so that
//! more than a tenth of its frames trace it again.

use std::process::Command;

use serde_json::Value;

const VALLE: &str = env!("CARGO_BIN_EXE_valle");

/// `valle motion check` of `scene`, as JSON.
fn check(name: &str, scene: &str) -> Value {
    let dir = std::env::temp_dir().join(format!("valle-check-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a folder");
    let file = dir.join("scene.motion.tsx");
    std::fs::write(&file, scene).expect("written");
    let out = Command::new(VALLE)
        .args(["motion", "check"])
        .arg(&file)
        .arg("--json")
        .output()
        .expect("valle runs");
    let _ = std::fs::remove_dir_all(&dir);
    serde_json::from_slice(&out.stdout).expect("JSON")
}

#[test]
fn a_structure_that_follows_time_is_warned_of() {
    let scene = "export const composition = { width: 64, height: 36, fps: 30, duration: 1 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  const bars = Array.from({ length: 1 + Math.floor(seconds * 20) }, (_, i) => i);
  return (
    <View className=\"h-full w-full flex\">
      {bars.map((i) => <View key={i} style={{ width: 2, height: 10, backgroundColor: \"#fff\" }} />)}
    </View>
  );
}
";
    let out = check("bars", scene);
    assert_eq!(out["status"], "ok", "{out}");
    let message = out["warnings"][0]["message"].as_str().unwrap_or_default();
    assert!(message.contains("trace the scene again"), "{out}");
    assert!(message.contains("scene.motion.tsx:"), "{out}");
}

#[test]
fn a_structure_that_holds_is_not() {
    let scene = "export const composition = { width: 64, height: 36, fps: 30, duration: 1 };
export default function Scene() {
  const { progress } = useCurrentTime();
  return <View className=\"h-full w-full\" style={{ opacity: progress }} />;
}
";
    let out = check("still", scene);
    assert_eq!(out["status"], "ok", "{out}");
    assert_eq!(out["warnings"], serde_json::json!([]), "{out}");
}
