#![cfg(feature = "motion")]

//! audio-analysis: decoded audio becomes frozen signals over the seconds of the sound, with no
//! runtime media reads.

use std::{collections::BTreeMap, sync::Arc};

use valle_compiler::motion::{
    AudioAnalysisEnv, AudioPcm, MotionModuleGraph,
    compile_motion_modules_with_full_env_and_data_and_audio,
    compile_motion_with_full_env_and_data_and_audio,
};
use valle_motion::{AudioSignal, ContentDigest, Expr, ResourceRef};

const SOURCE: &str = r##"
export const composition = { width: 320, height: 180, fps: 30, duration: 2 };
export const controls = { assets: { beat: asset({ kind: "audio", required: true }) } };
const BEAT = audioAnalysis("asset://beat", { bands: 8, rate: 30 });
export default function AudioBars() { const time = useCurrentTime();
  return <Scene style={{width:320,height:180}}>
    <View key="bar" style={{width:20,height:BEAT.level(time.seconds)*100,
      backgroundColor:"#ffffff"}} />
    <View key="onset" style={{width:20,height:BEAT.onset(time.seconds, 0.1)*20,
      backgroundColor:"#ffffff"}} />
    <View key="frequency" style={{width:20,height:BEAT.band(3,time.seconds)*100,
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
    let source = SOURCE.replace("BEAT.level", "BEAT.beatPhase").replace(
        "BEAT.onset(time.seconds, 0.1)",
        "BEAT.beatConfidence(time.seconds)",
    );
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
    // Without a beat grid the phase is 0, and silence has no level, band or confidence.
    let signals = signals(&compiled.artifact);
    assert!(signals.contains(&&AudioSignal::Constant { value: 0.0 }));
    assert!(signals.iter().all(|signal| signal.range() == (0.0, 0.0)));
}

fn signals(artifact: &valle_motion::SceneArtifact) -> Vec<&AudioSignal> {
    artifact
        .exprs
        .iter()
        .filter_map(|expr| match expr {
            Expr::AudioSample { signal, .. } => Some(signal),
            _ => None,
        })
        .collect()
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
    let signals = signals(&compiled.artifact);
    assert_eq!(signals.len(), 3);
    // The tone starts after a second of silence and lasts to the end of the sound, 2 s.
    let level = signals
        .iter()
        .find(|signal| matches!(signal, AudioSignal::Series { .. }) && signal.at(1.5) > 0.6)
        .unwrap();
    assert_eq!(level.at(0.5), 0.0);
    assert!((level.at(1.5) - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.01);
    assert!((level.at(2.0) - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.01);
    assert_eq!(level.at(2.1), 0.0);
    assert_eq!(level.at(-0.1), 0.0);
    // The onset of the tone is 1 at its time and falls to 0 over its 0.1 s.
    let Some(AudioSignal::Onsets { times, length }) = signals
        .iter()
        .find(|signal| matches!(signal, AudioSignal::Onsets { .. }))
    else {
        panic!("an onset signal")
    };
    assert_eq!(*length, 0.1);
    let onset = *times
        .iter()
        .find(|time| (**time - 1.0).abs() <= 1.0 / 30.0 + 1e-9)
        .unwrap_or_else(|| panic!("an onset at the tone, {times:?}"));
    let signal = AudioSignal::Onsets {
        times: times.clone(),
        length: *length,
    };
    assert_eq!(signal.at(onset), 1.0);
    assert!((signal.at(onset + 0.05) - 0.5).abs() < 1e-9);
    assert_eq!(signal.at(0.5), 0.0);
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
        "sha256:7429daa056b0be9052e51b669f63b614d37e2a56c36e84b6fec4f7e937d9afcb"
    );
}

#[test]
fn every_frame_reads_its_own_bin_at_the_bin_start() {
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
    // The pulse lasts one bin, and the frame at its start reads all of it.
    let level = signals(&artifact)
        .into_iter()
        .find(|signal| {
            matches!(signal, AudioSignal::Series { .. }) && signal.at(123.0 / 30.0) > 0.9
        })
        .expect("the pulse");
    assert!(level.at(122.0 / 30.0).abs() < 1e-9);
    // A ramp of the bin numbers reveals every wrong address, not just the pulse's.
    for expr in &mut artifact.exprs {
        if let Expr::AudioSample { signal, .. } = expr
            && matches!(signal, AudioSignal::Series { .. })
        {
            *signal = AudioSignal::Series {
                rate: 30,
                end: 10_000.0 / 30.0,
                values: (0..10_000).map(f64::from).collect(),
            };
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
            if matches!(
                expr,
                Expr::AudioSample {
                    signal: AudioSignal::Series { .. },
                    ..
                }
            ) {
                let MotionValue::Number(value) = values[index] else {
                    panic!("an audio sample is a number")
                };
                assert!((value - f64::from(frame)).abs() < 1e-6, "frame {frame}");
            }
        }
    }
}

#[test]
fn an_onset_takes_a_static_length_and_warns_under_one_frame() {
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
    let compile = |source: &str| {
        compile_motion_with_full_env_and_data_and_audio(
            source,
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
    };
    let messages = |source: &str| -> Vec<String> {
        compile(source)
            .unwrap_err()
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect()
    };
    let one =
        messages(&SOURCE.replace("BEAT.onset(time.seconds, 0.1)", "BEAT.onset(time.seconds)"));
    assert!(
        one.iter()
            .any(|message| message.contains("`onset(t, length)`")),
        "{one:?}"
    );
    let dynamic = messages(&SOURCE.replace("0.1)*20", "time.seconds)*20"));
    assert!(
        dynamic
            .iter()
            .any(|message| message.contains("onset length must be a static positive number")),
        "{dynamic:?}"
    );
    let under = |source: &str| {
        compile(source)
            .unwrap()
            .warnings
            .into_iter()
            .filter(|warning| warning.code == valle_motion::DiagCode::UnderOneFrame)
            .count()
    };
    // At 30 fps a frame lasts 0.033 s.
    assert_eq!(under(SOURCE), 0);
    assert_eq!(under(&SOURCE.replace("0.1)*20", "0.02)*20")), 1);
    // Inside a TimeScope of speed 2, an onset of 0.05 s lasts 0.025 s of output.
    let scoped = SOURCE
        .replace("0.1)*20", "0.05)*20")
        .replace(
            "export default function AudioBars() { const time = useCurrentTime();\n  return <Scene style={{width:320,height:180}}>",
            "export default function AudioBars() { return <Scene style={{width:320,height:180}}><TimeScope key=\"scope\" offset={0} speed={2}><Bars /></TimeScope></Scene>; }\nfunction Bars() { const time = useCurrentTime();\n  return <>",
        )
        .replace("  </Scene>;\n}", "  </>;\n}");
    assert!(scoped.contains("function Bars()"), "{scoped}");
    assert_eq!(under(&SOURCE.replace("0.1)*20", "0.05)*20")), 0);
    assert_eq!(under(&scoped), 1);
    // So does a time that runs twice as fast.
    let doubled = SOURCE.replace(
        "BEAT.onset(time.seconds, 0.1)",
        "BEAT.onset(time.seconds * 2, 0.05)",
    );
    assert_eq!(under(&doubled), 1);
    // A host meets the shortest onset with its own frames.
    let shortest = |source: &str| compile(source).unwrap().artifact.shortest_onset();
    assert_eq!(shortest(SOURCE), Some((0.1, 0.1)));
    assert_eq!(shortest(&doubled), Some((0.05, 0.025)));
    assert_eq!(shortest(&scoped), Some((0.05, 0.025)));
    // So does a row of an instance group, and a row whose time differs by row runs at the speed
    // of the clock it is computed from.
    const ROWS: &str = r##"
export const composition = { width: 320, height: 180, fps: 30, duration: 2 };
export const controls = { assets: { beat: asset({ kind: "audio", required: true }) } };
const BEAT = audioAnalysis("asset://beat", { bands: 8, rate: 30 });
const BARS = Array.from({ length: 4 }, (_, i) => ({ id: `bar${i}`, x: i * 24 }));
export default function AudioBars() { const time = useCurrentTime();
  return <Scene style={{width:320,height:180}}>
    {BARS.map((bar, i) => <View key={bar.id} className="absolute" style={{left:bar.x,top:40,
      width:20,height:20,opacity:BEAT.onset(time.seconds, 0.05),backgroundColor:"#ffffff"}} />)}
  </Scene>;
}
"##;
    let staggered = ROWS.replace(
        "BEAT.onset(time.seconds, 0.05)",
        "BEAT.onset(time.seconds - i * 0.1, 0.05)",
    );
    assert_ne!(staggered, ROWS);
    for rows in [ROWS, staggered.as_str()] {
        let artifact = compile(rows).unwrap().artifact;
        assert_eq!(artifact.instance_groups.len(), 1, "{rows}");
        assert_eq!(artifact.shortest_onset(), Some((0.05, 0.05)), "{rows}");
    }
}
