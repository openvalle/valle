use serde::{Deserialize, Serialize};

/// Stable Timeline visual-item identity that qualifies a Motion Artifact instance.
///
/// Engine assigns this from the visual item's durable id, never from traversal order or
/// a heap address. Two clips sharing one Artifact keep separate Glass tracks.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MotionInstanceId(String);

impl MotionInstanceId {
    pub fn new(id: impl Into<String>) -> Result<Self, super::TimeError> {
        let id = id.into();
        if id.is_empty() || id.len() > 128 {
            return Err(super::TimeError::InvalidIdentity);
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One continuous segment of a Timeline time map. Cuts and non-differentiable ramps
/// open a new segment; that change is part of [`TrackEpoch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TimeMapSegment(u32);

impl TimeMapSegment {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Discontinuity counter derived from compile-time continuity boundaries, not a
/// runtime incrementing clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DiscontinuityIndex(u32);

impl DiscontinuityIndex {
    pub const NONE: Self = Self(0);

    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Identity of one Glass surface track across time.
///
/// Epoch is derived from the admitted render seed, motion instance, time-map segment,
/// loop iteration, and compile-time discontinuity index. Crossing an epoch forbids
/// finite differences; the response kernel resets.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackEpoch {
    render_seed: u32,
    instance: MotionInstanceId,
    time_map_segment: TimeMapSegment,
    loop_iteration: u32,
    discontinuity: DiscontinuityIndex,
}

impl TrackEpoch {
    pub fn new(
        render_seed: u32,
        instance: MotionInstanceId,
        time_map_segment: TimeMapSegment,
        loop_iteration: u32,
        discontinuity: DiscontinuityIndex,
    ) -> Self {
        Self {
            render_seed,
            instance,
            time_map_segment,
            loop_iteration,
            discontinuity,
        }
    }

    pub const fn render_seed(&self) -> u32 {
        self.render_seed
    }

    pub fn instance(&self) -> &MotionInstanceId {
        &self.instance
    }

    pub fn time_map_segment(&self) -> TimeMapSegment {
        self.time_map_segment
    }

    pub fn loop_iteration(&self) -> u32 {
        self.loop_iteration
    }

    pub fn discontinuity(&self) -> DiscontinuityIndex {
        self.discontinuity
    }

    pub fn with_discontinuity(self, discontinuity: DiscontinuityIndex) -> Self {
        Self {
            discontinuity,
            ..self
        }
    }

    pub fn with_loop_iteration(self, loop_iteration: u32) -> Self {
        Self {
            loop_iteration,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::FrameRate;
    use crate::internal::time::{FrameKey, SampleTime};

    #[test]
    fn sample_time_from_frame_is_canonical_and_hash_stable() {
        let fps = FrameRate::new(30, 1).unwrap();
        let a = SampleTime::from_frame(FrameKey::new(15), fps).unwrap();
        let b = SampleTime::from_frame(FrameKey::new(15), fps).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            a.composition(),
            crate::internal::RationalTime::new(1, 2).unwrap()
        );
    }

    #[test]
    fn equivalent_anchor_offsets_share_one_sample_key() {
        let fps = FrameRate::new(60, 1).unwrap();
        let t0 = SampleTime::from_frame(FrameKey::new(10), fps).unwrap();
        let offset = crate::internal::RationalTime::new(1, 60).unwrap();
        let shifted = t0.checked_offset(offset).unwrap();
        let direct = SampleTime::from_frame(FrameKey::new(11), fps).unwrap();
        assert_eq!(shifted, direct);
    }

    #[test]
    fn empty_motion_instance_id_fails_closed() {
        assert!(MotionInstanceId::new("").is_err());
    }

    #[test]
    fn track_epoch_names_the_render_seed_without_revision_terminology() {
        let epoch = TrackEpoch::new(
            7,
            MotionInstanceId::new("clip-a").unwrap(),
            TimeMapSegment::new(0),
            0,
            DiscontinuityIndex::NONE,
        );
        assert_eq!(epoch.render_seed(), 7);

        let wire = serde_json::to_value(epoch).unwrap();
        assert_eq!(wire["render_seed"], 7);
        assert!(wire.get("revision").is_none());

        let mut retired = wire;
        let render_seed = retired
            .as_object_mut()
            .unwrap()
            .remove("render_seed")
            .unwrap();
        retired
            .as_object_mut()
            .unwrap()
            .insert("revision".into(), render_seed);
        assert!(serde_json::from_value::<TrackEpoch>(retired).is_err());
    }
}
