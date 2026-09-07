//! MotionContext JSON snapshots cover phase boundaries, clamping, hold cycles and strict roundtrip decoding.

use serde::{Deserialize, Serialize};
use valle_motion::{MotionContext, PhaseLayout, PhaseSpec, motion_context_at, phase_windows};
use valle_timeline::FrameRate;

const GOLDEN: &str = include_str!("golden/motion-context-sweep.json");

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Sweep {
    spec: PhaseSpec,
    layout: PhaseLayout,
    contexts: Vec<MotionContext>,
}

/// A 24-frame clip at 30fps with 6-frame enter/exit windows and three 4-frame hold cycles.
fn sweep() -> Sweep {
    let spec = PhaseSpec {
        enter_frames: 6,
        exit_frames: 6,
        hold_cycle_frames: Some(4),
    };
    let layout = phase_windows(&spec, 24);
    let fps = FrameRate::new(30, 1).unwrap();
    let contexts: Vec<MotionContext> = (0..layout.duration_frames)
        .map(|f| motion_context_at(f, &layout, fps).expect("frame in range"))
        .collect();
    Sweep {
        spec,
        layout,
        contexts,
    }
}

#[test]
fn sweep_matches_golden_json() {
    let actual = serde_json::to_string_pretty(&sweep()).unwrap();
    if std::env::var_os("VALLE_UPDATE_MOTION_CONTEXT_GOLDEN").is_some() {
        std::fs::write(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/golden/motion-context-sweep.json"
            ),
            format!("{actual}\n"),
        )
        .expect("update MotionContext golden");
        return;
    }
    assert_eq!(
        actual.trim(),
        GOLDEN.trim(),
        "MotionContext wire format changed; review intentional contract changes and update the golden fixture"
    );
}

#[test]
fn golden_json_roundtrips() {
    let parsed: Sweep = serde_json::from_str(GOLDEN).expect("golden must parse");
    assert_eq!(
        parsed,
        sweep(),
        "decoded golden fixture differs from evaluation"
    );
    let reserialized = serde_json::to_string_pretty(&parsed).unwrap();
    assert_eq!(reserialized.trim(), GOLDEN.trim(), "roundtrip is unstable");
}

/// Reject unknown fields rather than silently accepting misspelled input.
#[test]
fn unknown_fields_are_rejected() {
    let cases = [
        r#"{"localFrame":0,"sample":{"composition":"0/1"},"progress":0.0,
            "durationFrames":1,"fps":"30/1","currentPhase":"enter",
            "enter":{"active":true,"frame":0,"elapsedFrames":0,"durationFrames":1,"progress":0.0},
            "hold":{"active":false,"frame":0,"elapsedFrames":0,"durationFrames":0,"progress":0.0,
                    "iteration":0,"cycleFrame":0,"cycleProgress":0.0},
            "exit":{"active":false,"frame":0,"elapsedFrames":0,"durationFrames":0,"progress":0.0},
            "absoluteFrame":7}"#,
        r#"{"localFrame":0,"sample":{"composition":"0/1"},"progress":0.0,
            "durationFrames":1,"fps":"30/1","currentPhase":"enter",
            "enter":{"active":true,"frame":0,"durationFrames":1,"progress":0.0,"kind":"enter"},
            "hold":{"active":false,"frame":0,"elapsedFrames":0,"durationFrames":0,"progress":0.0,
                    "iteration":0,"cycleFrame":0,"cycleProgress":0.0},
            "exit":{"active":false,"frame":0,"elapsedFrames":0,"durationFrames":0,"progress":0.0}}"#,
    ];
    for case in cases {
        assert!(
            serde_json::from_str::<MotionContext>(case).is_err(),
            "unknown fields must be rejected"
        );
    }
    // Removing the unknown field must make the same document valid.
    let ok = r#"{"localFrame":0,"sample":{"composition":"0/1"},"progress":0.0,
        "durationFrames":1,"fps":"30/1","currentPhase":"enter",
        "enter":{"active":true,"frame":0,"elapsedFrames":0,"durationFrames":1,"progress":0.0},
        "hold":{"active":false,"frame":0,"elapsedFrames":0,"durationFrames":0,"progress":0.0,
                "iteration":0,"cycleFrame":0,"cycleProgress":0.0},
        "exit":{"active":false,"frame":0,"elapsedFrames":0,"durationFrames":0,"progress":0.0}}"#;
    let ctx: MotionContext = serde_json::from_str(ok).expect("well-formed context must parse");
    assert_eq!(
        ctx.fps,
        FrameRate::new(30, 1).unwrap(),
        "fps uses Timeline's canonical exact frame-rate kernel"
    );
    assert!(serde_json::from_str::<MotionContext>(&ok.replace("\"30/1\"", "30")).is_err());
    assert!(serde_json::from_str::<PhaseSpec>(r#"{"enterFrames":1,"nope":2}"#).is_err());
    assert!(
        serde_json::from_str::<PhaseLayout>(
            r#"{"durationFrames":1,"enterFrames":1,"holdFrames":0,"exitFrames":0,
                "holdCycleFrames":null,"nope":2}"#
        )
        .is_err()
    );
}
