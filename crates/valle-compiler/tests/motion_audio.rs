#![cfg(feature = "motion")]

//! audio-analysis: decoded audio becomes a frozen frame table, with no runtime media reads.

use std::{collections::BTreeMap, sync::Arc};

use valle_compiler::motion::{
    AudioAnalysisEnv, AudioPcm, MotionModuleGraph,
    compile_motion_modules_with_full_env_and_data_and_audio,
    compile_motion_with_full_env_and_data_and_audio,
};
use valle_motion::{ContentDigest, Expr, ResourceRef};

const SOURCE: &str = r##"
export const composition = { width: 320, height: 180, fps: 30, duration: 2 };
export const controls = { assets: { beat: asset({ kind: "audio", required: true }) } };
const BEAT = audioAnalysis("asset://beat", { bands: 8, fps: 30 });
export default function AudioBars(ctx) {
  return <Scene style={{width:320,height:180}}>
    <View key="bar" style={{width:20,height:BEAT.level(ctx.seconds)*100,
      backgroundColor:"#ffffff"}} />
    <View key="onset" style={{width:20,height:BEAT.onset(ctx.seconds)*20,
      backgroundColor:"#ffffff"}} />
    <View key="frequency" style={{width:20,height:BEAT.band(3,ctx.seconds)*100,
      backgroundColor:"#ffffff"}} />
  </Scene>;
}
"##;

#[test]
fn uncertain_beat_phase_has_a_source_warning_and_exposes_confidence() {
    let hash = ContentDigest::of_bytes(b"silence");
    let mut audio = AudioAnalysisEnv::default();
    audio.sources.insert(
        "beat".into(),
        Arc::new(AudioPcm {
            content_hash: hash,
            sample_rate: 8000,
            samples: vec![0.0; 16000],
        }),
    );
    let source = SOURCE
        .replace("BEAT.level", "BEAT.beatPhase")
        .replace("BEAT.onset", "BEAT.beatConfidence");
    let compiled = compile_motion_with_full_env_and_data_and_audio(
        &source,
        &[ResourceRef {
            control: "beat".into(),
            content_hash: hash,
        }],
        None,
        None,
        None,
        Some(&audio),
        None,
    )
    .unwrap();
    let warning = compiled
        .warnings
        .iter()
        .find(|w| w.code == valle_motion::DiagCode::AudioBeatUncertain)
        .unwrap();
    assert!(warning.span.line > 1);
    assert!(warning.message.contains("confidence 0.000"));
    for expr in &compiled.artifact.exprs {
        if let Expr::AudioSample { samples, .. } = expr {
            assert!(samples.iter().all(|x| *x == 0.0));
        }
    }
}

#[test]
fn audio_analysis_freezes_level_band_and_onset_with_asset_digest() {
    let rate = 48_000_u32;
    let samples = (0..rate * 2)
        .map(|index| {
            if index < rate {
                0.0
            } else {
                (std::f64::consts::TAU * 440.0 * f64::from(index - rate) / f64::from(rate)).sin()
                    as f32
            }
        })
        .collect();
    let hash = ContentDigest::of_bytes(b"synthetic-audio");
    let mut audio = AudioAnalysisEnv::default();
    audio.sources.insert(
        "beat".into(),
        Arc::new(AudioPcm {
            content_hash: hash,
            sample_rate: rate,
            samples,
        }),
    );
    let graph = MotionModuleGraph::new(
        "main.motion.tsx",
        BTreeMap::from([("main.motion.tsx".into(), SOURCE.into())]),
    )
    .unwrap();
    let compiled = compile_motion_modules_with_full_env_and_data_and_audio(
        &graph,
        &[ResourceRef {
            control: "beat".into(),
            content_hash: hash,
        }],
        None,
        None,
        None,
        Some(&audio),
        None,
    )
    .unwrap_or_else(|diagnostics| panic!("{diagnostics:?}"));
    let tables = compiled
        .artifact
        .exprs
        .iter()
        .filter_map(|expr| match expr {
            Expr::AudioSample { samples, .. } => Some(samples),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(tables.len(), 3);
    let level = tables.iter().find(|values| values[45] > 0.6).unwrap();
    assert_eq!(level[15], 0.0);
    assert!((level[45] - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.01);
    let onset = tables
        .iter()
        .find(|values| values.iter().any(|value| *value == 1.0))
        .unwrap();
    assert!(onset[29..=31].iter().any(|value| *value == 1.0));
    assert_eq!(compiled.artifact.resource_refs[0].content_hash, hash);
}

#[test]
fn pulse_pcm_artifact_digest_is_shared_with_wasm() {
    let mut samples = vec![0.0; 8_000 * 2];
    for pulse in [2_000, 6_000, 10_000, 14_000] {
        samples[pulse..pulse + 128].fill(0.8);
    }
    let hash = ContentDigest::parse(&format!("sha256:{}", "a".repeat(64))).unwrap();
    let mut audio = AudioAnalysisEnv::default();
    audio.sources.insert(
        "beat".into(),
        Arc::new(AudioPcm {
            content_hash: hash,
            sample_rate: 8_000,
            samples,
        }),
    );
    let compiled = compile_motion_with_full_env_and_data_and_audio(
        include_str!("fixtures/motion/composition/audio-reactive-pulses.motion.tsx"),
        &[ResourceRef {
            control: "beat".into(),
            content_hash: hash,
        }],
        None,
        None,
        None,
        Some(&audio),
        None,
    )
    .unwrap_or_else(|diagnostics| panic!("{diagnostics:?}"));
    let bytes = valle_motion::canonical_bytes(&compiled.artifact).unwrap();
    assert_eq!(
        ContentDigest::of_bytes(&bytes).to_wire(),
        "sha256:ae43252a3fde55d8c7f9a540d55ac34483def1d6234635102bea454660883179"
    );
}

#[test]
fn audio_frame_lookup_does_not_drop_single_frame_pulses() {
    use valle_motion::{EvalInputs, MotionValue, eval_all, motion_context_at_frame, resolve_props};
    let mut samples = vec![0.0; 48_000 * 5];
    samples[123 * 1600..124 * 1600].fill(1.0);
    let hash = ContentDigest::of_bytes(b"single-frame-pulse");
    let mut audio = AudioAnalysisEnv::default();
    audio.sources.insert(
        "beat".into(),
        Arc::new(AudioPcm {
            content_hash: hash,
            sample_rate: 48_000,
            samples,
        }),
    );
    let mut artifact = compile_motion_with_full_env_and_data_and_audio(
        SOURCE,
        &[ResourceRef {
            control: "beat".into(),
            content_hash: hash,
        }],
        None,
        None,
        None,
        Some(&audio),
        None,
    )
    .unwrap()
    .artifact;
    // Independent ramp table reveals all wrong addresses, not just the selected pulse.
    for expr in &mut artifact.exprs {
        if let Expr::AudioSample { samples, .. } = expr {
            *samples = (0..10000).map(f64::from).collect();
        }
    }
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    for frame in 0..10000 {
        let ctx =
            motion_context_at_frame(frame, 10000, valle_timeline::FrameRate::new(30, 1).unwrap())
                .unwrap();
        let values = eval_all(
            &artifact,
            EvalInputs {
                ctx: &ctx,
                props: &props,
                unit: None,
                viewport: None,
            },
        )
        .unwrap();
        for (index, expr) in artifact.exprs.iter().enumerate() {
            if matches!(expr, Expr::AudioSample { .. }) {
                assert_eq!(
                    values[index],
                    MotionValue::Number(f64::from(frame)),
                    "frame {frame}"
                );
            }
        }
    }
}
