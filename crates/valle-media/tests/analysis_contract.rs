use valle_media::analysis::{Transcript, Word};

fn transcript() -> Transcript {
    Transcript {
        audio: Some("speech.wav".into()),
        lang: Some("en".into()),
        words: vec![
            Word {
                id: "w0".into(),
                text: "hello".into(),
                start: 0.0,
                end: 0.4,
            },
            Word {
                id: "w1".into(),
                text: "world".into(),
                start: 0.3,
                end: 0.8,
            },
        ],
    }
}

#[test]
fn transcript_allows_overlapping_words_but_rejects_unusable_ids_and_clocks() {
    let valid = transcript();
    valid.validate().unwrap();
    assert_eq!(
        serde_json::from_slice::<Transcript>(&serde_json::to_vec(&valid).unwrap()).unwrap(),
        valid
    );
    for (id, start, end, message) in [
        ("", 0.3, 0.8, "empty id"),
        ("w0", 0.3, 0.8, "duplicate id"),
        ("w1", f64::NAN, 0.8, "non-finite"),
        ("w1", 0.3, f64::INFINITY, "non-finite"),
        ("w1", -0.1, 0.8, "negative"),
        ("w1", 0.3, 0.2, "end"),
    ] {
        let mut invalid = valid.clone();
        invalid.words[1].id = id.into();
        invalid.words[1].start = start;
        invalid.words[1].end = end;
        assert!(invalid.validate().unwrap_err().contains(message));
    }
    let mut reversed = valid.clone();
    reversed.words.reverse();
    assert!(reversed.validate().unwrap_err().contains("out of order"));
    Transcript {
        words: vec![],
        ..valid
    }
    .validate()
    .unwrap();
}

#[cfg(feature = "media-tools")]
#[test]
fn time_ranges_reject_nonfinite_and_reversed_intervals() {
    use valle_media::tools::TimeRange;
    for start in [f64::NAN, f64::INFINITY, -0.1] {
        assert!(TimeRange::new(start, None).is_err());
    }
    for end in [f64::NAN, f64::INFINITY, 0.0, 1.0] {
        assert!(TimeRange::new(1.0, Some(end)).is_err());
    }
    assert_eq!(TimeRange::new(1.0, None).unwrap().end, None);
}

#[test]
fn model_errors_keep_machine_codes_and_actionable_hints() {
    use valle_media::models::{ModelError, ModelErrorCode};
    let bare = ModelError::new(ModelErrorCode::NotInstalled, "missing graph");
    assert_eq!(bare.to_string(), "missing graph");
    let error = bare.with_hint("install the selected artifact");
    assert_eq!(
        error.to_string(),
        "missing graph\ninstall the selected artifact"
    );
    let wire = serde_json::to_value(&error).unwrap();
    assert_eq!(wire["hint"], "install the selected artifact");
}

#[test]
fn codec_timing_preserves_results_and_resets_all_counters() {
    use valle_media::codec::perf;
    // This integration-test binary owns the environment and runs no codecs in parallel.
    unsafe {
        std::env::set_var("VALLE_PERF", "false");
    }
    perf::init_from_env_and_reset();
    assert_eq!(perf::time_src_convert(|| 7), 7);
    assert_eq!(perf::snapshot_ns(), (0, 0, 0));
    unsafe {
        std::env::set_var("VALLE_PERF", "1");
    }
    perf::init_from_env_and_reset();
    let work = || std::hint::black_box((0..10_000_u64).sum::<u64>());
    assert_eq!(perf::time_src_convert(work), 49_995_000);
    assert_eq!(perf::time_enc_convert(work), 49_995_000);
    assert_eq!(perf::time_enc_submit(work), 49_995_000);
    let (source, convert, submit) = perf::snapshot_ns();
    assert!(source > 0 && convert > 0 && submit > 0);
    perf::init_from_env_and_reset();
    assert_eq!(perf::snapshot_ns(), (0, 0, 0));
    unsafe {
        std::env::remove_var("VALLE_PERF");
    }
    perf::init_from_env_and_reset();
}
