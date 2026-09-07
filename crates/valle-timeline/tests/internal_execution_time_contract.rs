use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use valle_timeline::internal::{
    FrameKey, SampleTime,
    quantize::{
        quantize_frame_boundary, quantize_frame_interval, quantize_sample_boundary,
        quantize_sample_interval,
    },
};
use valle_timeline::{FrameRate, RationalTime, TimeError};

fn hash(value: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn exact_frame_and_sample_quantization_obeys_ties_and_ntsc() {
    let fps_30 = FrameRate::new(30, 1).unwrap();
    assert_eq!(
        quantize_frame_boundary(RationalTime::new(1, 60).unwrap(), fps_30),
        Ok(1)
    );
    assert_eq!(
        quantize_frame_boundary(RationalTime::new(-1, 60).unwrap(), fps_30),
        Ok(-1)
    );
    assert_eq!(
        quantize_sample_boundary(RationalTime::new(1, 96_000).unwrap(), 48_000),
        Ok(1)
    );
    assert_eq!(
        quantize_sample_boundary(RationalTime::new(-1, 96_000).unwrap(), 48_000),
        Ok(-1)
    );
    assert_eq!(
        quantize_frame_boundary(RationalTime::ONE, FrameRate::new(30_000, 1_001).unwrap()),
        Ok(30)
    );
    assert_eq!(
        quantize_sample_boundary(RationalTime::ONE, 0),
        Err(TimeError::InvalidSampleRate)
    );
}

#[test]
fn exact_intervals_quantize_boundaries_not_duration() {
    let frame = quantize_frame_interval(
        RationalTime::new(49, 3_000).unwrap(),
        RationalTime::new(1, 1_500).unwrap(),
        FrameRate::new(30, 1).unwrap(),
    )
    .unwrap();
    assert_eq!(
        (frame.start_frame, frame.end_frame, frame.duration_frames),
        (0, 1, 1)
    );

    let sample = quantize_sample_interval(
        RationalTime::new(1, 96_000).unwrap(),
        RationalTime::new(1, 48_000).unwrap(),
        48_000,
    )
    .unwrap();
    assert_eq!(
        (
            sample.start_sample,
            sample.end_sample,
            sample.duration_samples
        ),
        (1, 2, 1)
    );
}

#[test]
fn quantization_overflow_is_fallible_not_saturating() {
    assert_eq!(
        quantize_frame_boundary(
            RationalTime::new(i64::MAX, 1).unwrap(),
            FrameRate::new(i64::MAX, 1).unwrap(),
        ),
        Err(TimeError::Overflow)
    );
    assert_eq!(
        quantize_sample_boundary(RationalTime::new(i64::MAX, 1).unwrap(), u32::MAX),
        Err(TimeError::Overflow)
    );
}

#[test]
fn frame_anchor_and_offset_collapse_to_one_absolute_sample_time() {
    let fps = FrameRate::new(30, 1).unwrap();
    let anchor = SampleTime::from_frame(FrameKey::new(30), fps).unwrap();
    let via_offset = anchor
        .checked_offset(RationalTime::new(-1, 2).unwrap())
        .unwrap();
    let direct = SampleTime::from_frame(FrameKey::new(15), fps).unwrap();
    assert_eq!(via_offset, direct);
    assert_eq!(via_offset.composition(), RationalTime::new(1, 2).unwrap());
    assert_eq!(hash(via_offset), hash(direct));
    assert_eq!(
        serde_json::to_vec(&via_offset).unwrap(),
        serde_json::to_vec(&direct).unwrap()
    );
}

#[test]
fn ntsc_and_subframe_samples_stay_exact() {
    let ntsc = FrameRate::new(30_000, 1_001).unwrap();
    let frame = SampleTime::from_frame(FrameKey::new(30_000), ntsc).unwrap();
    assert_eq!(frame.composition(), RationalTime::new(1_001, 1).unwrap());
    let subframe = frame
        .checked_offset(RationalTime::new(1, 120_000).unwrap())
        .unwrap();
    assert_eq!(
        subframe.composition(),
        RationalTime::new(120_120_001, 120_000).unwrap()
    );
}
