use valle_timeline::internal::{DiscontinuityIndex, MotionInstanceId, TimeMapSegment, TrackEpoch};

#[test]
fn track_epochs_preserve_instance_and_reset_boundaries_on_the_wire() {
    let instance = MotionInstanceId::new("clip:motion").unwrap();
    let initial = TrackEpoch::new(
        17,
        instance.clone(),
        TimeMapSegment::new(2),
        0,
        DiscontinuityIndex::NONE,
    );
    assert_eq!(initial.instance().as_str(), "clip:motion");
    assert_eq!(initial.time_map_segment().index(), 2);
    assert_eq!(initial.loop_iteration(), 0);
    assert_eq!(initial.discontinuity().index(), 0);
    let cut = initial
        .clone()
        .with_loop_iteration(3)
        .with_discontinuity(DiscontinuityIndex::new(4));
    assert_eq!(cut.instance(), &instance);
    assert_eq!(cut.render_seed(), 17);
    assert_eq!(cut.loop_iteration(), 3);
    assert_eq!(cut.discontinuity().index(), 4);
    assert_ne!(cut, initial);
    assert_eq!(
        serde_json::from_value::<TrackEpoch>(serde_json::to_value(&cut).unwrap()).unwrap(),
        cut
    );
    assert!(MotionInstanceId::new("x".repeat(129)).is_err());
}
