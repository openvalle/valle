#![cfg(feature = "motion")]

//! Checks of how a scene reads its clock: a comparison that flips on the last frame alone, a
//! loop that does not last a whole number of frames, and a time that is likely not in seconds.

use valle_compiler::motion::{CompilerDiagnostic, compile_motion};
use valle_motion::DiagCode;

const SCENE: &str = r##"
export const composition = { width: 320, height: 180, fps: 30, duration: 2 };
export default function Ends() {
  const { seconds, progress } = useCurrentTime();
  const { duration } = useComposition();
  return <Scene style={{width:320,height:180}}>
    <View key="box" visible={CONDITION}
      style={{width:40,height:40,backgroundColor:"#ffffff"}} />
  </Scene>;
}
"##;

fn errors(source: &str) -> Vec<CompilerDiagnostic> {
    compile_motion(source).err().unwrap_or_default()
}

fn warnings(source: &str, code: DiagCode) -> Vec<CompilerDiagnostic> {
    compile_motion(source)
        .unwrap()
        .warnings
        .into_iter()
        .filter(|warning| warning.code == code)
        .collect()
}

#[test]
fn a_comparison_with_the_end_of_a_one_shot_scene_is_rejected() {
    for condition in [
        "seconds >= duration",
        "seconds < 2",
        "duration <= seconds",
        "progress < 1",
        "1 > progress",
    ] {
        let found = errors(&SCENE.replace("CONDITION", condition));
        assert!(
            found
                .iter()
                .any(|error| error.message.contains("flips on the last frame alone")),
            "{condition}: {found:?}"
        );
    }
    for condition in [
        "seconds >= duration - 0.5",
        "progress >= 0.9",
        "seconds <= duration",
    ] {
        let found = errors(&SCENE.replace("CONDITION", condition));
        assert!(found.is_empty(), "{condition}: {found:?}");
    }
    // A loop never shows its end.
    let looping = SCENE
        .replace("duration: 2 }", "duration: 2, loop: true }")
        .replace("CONDITION", "seconds >= duration");
    assert!(errors(&looping).is_empty());
}

#[test]
fn a_loop_of_a_fractional_number_of_frames_warns() {
    let source = SCENE.replace("CONDITION", "true");
    let found = warnings(
        &source.replace("duration: 2 }", "duration: 1.01, loop: true }"),
        DiagCode::LoopSeam,
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].message.contains("30.3 frames") && found[0].message.contains("`duration: 1`"),
        "{}",
        found[0].message
    );
    assert!(
        warnings(
            &source.replace("duration: 2 }", "duration: 2, loop: true }"),
            DiagCode::LoopSeam
        )
        .is_empty()
    );
    // A one-shot scene ends on its own last frame.
    assert!(
        warnings(
            &source.replace("duration: 2 }", "duration: 1.01 }"),
            DiagCode::LoopSeam
        )
        .is_empty()
    );
}

#[test]
fn a_time_past_the_end_of_the_scene_warns_of_its_unit() {
    let found = warnings(
        &SCENE.replace("CONDITION", "seconds >= 300"),
        DiagCode::TimeUnit,
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].message.contains("300 milliseconds are `0.3`")
            && found[0].message.contains("300 frames at 30 fps are `10`"),
        "{}",
        found[0].message
    );
    assert!(
        warnings(
            &SCENE.replace("CONDITION", "seconds >= 1.5"),
            DiagCode::TimeUnit
        )
        .is_empty()
    );
}
