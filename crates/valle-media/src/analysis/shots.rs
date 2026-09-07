//! Model-neutral shot-boundary and shot-list contracts.

use serde::{Deserialize, Serialize};

const TIME_EPSILON_SECONDS: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShotBoundary {
    /// Zero-based source frame index when the detector has a frame-grid identity.
    pub frame_index: u64,
    /// Source-local presentation timestamp in seconds.
    pub pts_seconds: f64,
    /// Calibrated detector confidence normalized to `[0, 1]`, when the detector exposes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Shot {
    pub index: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShotList {
    pub duration_seconds: f64,
    pub boundaries: Vec<ShotBoundary>,
    pub shots: Vec<Shot>,
}

impl ShotList {
    /// Build contiguous half-open shots from sorted model boundaries.
    ///
    /// A boundary at zero or at the exact source duration is redundant and is omitted. Duplicate
    /// timestamps are coalesced by keeping the highest available confidence while preserving the
    /// first frame-grid identity. Absence is preserved instead of inventing a probability.
    pub fn from_boundaries(
        duration_seconds: f64,
        mut boundaries: Vec<ShotBoundary>,
    ) -> Result<Self, String> {
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return Err("shot-list duration must be finite and greater than zero".to_owned());
        }
        for boundary in &boundaries {
            if !boundary.pts_seconds.is_finite()
                || boundary.pts_seconds < 0.0
                || boundary.pts_seconds > duration_seconds
            {
                return Err(format!(
                    "shot boundary {:.6}s is outside source duration {:.6}s",
                    boundary.pts_seconds, duration_seconds
                ));
            }
            if let Some(confidence) = boundary.confidence
                && (!confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
            {
                return Err(format!(
                    "shot boundary confidence {confidence} is outside [0, 1]"
                ));
            }
        }
        boundaries.sort_by(|left, right| {
            left.pts_seconds
                .total_cmp(&right.pts_seconds)
                .then(left.frame_index.cmp(&right.frame_index))
        });
        boundaries.retain(|boundary| {
            boundary.pts_seconds > TIME_EPSILON_SECONDS
                && boundary.pts_seconds < duration_seconds - TIME_EPSILON_SECONDS
        });

        let mut normalized: Vec<ShotBoundary> = Vec::with_capacity(boundaries.len());
        for boundary in boundaries {
            if let Some(previous) = normalized.last_mut()
                && (previous.pts_seconds - boundary.pts_seconds).abs() <= TIME_EPSILON_SECONDS
            {
                previous.confidence = match (previous.confidence, boundary.confidence) {
                    (Some(previous), Some(current)) => Some(previous.max(current)),
                    (None, available @ Some(_)) => available,
                    (available, None) => available,
                };
                continue;
            }
            normalized.push(boundary);
        }

        let mut points = Vec::with_capacity(normalized.len() + 2);
        points.push(0.0);
        points.extend(normalized.iter().map(|boundary| boundary.pts_seconds));
        points.push(duration_seconds);
        let shots = points
            .windows(2)
            .enumerate()
            .map(|(index, window)| Shot {
                index: index as u64,
                start_seconds: window[0],
                end_seconds: window[1],
            })
            .collect();

        Ok(Self {
            duration_seconds,
            boundaries: normalized,
            shots,
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        let rebuilt = Self::from_boundaries(self.duration_seconds, self.boundaries.clone())?;
        if rebuilt.boundaries != self.boundaries || rebuilt.shots != self.shots {
            return Err("shot list is not in canonical contiguous form".to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_form_contiguous_half_open_shots() {
        let list = ShotList::from_boundaries(
            10.0,
            vec![
                ShotBoundary {
                    frame_index: 150,
                    pts_seconds: 5.0,
                    confidence: Some(0.8),
                },
                ShotBoundary {
                    frame_index: 60,
                    pts_seconds: 2.0,
                    confidence: Some(0.7),
                },
            ],
        )
        .unwrap();
        assert_eq!(list.shots.len(), 3);
        assert_eq!(
            (list.shots[0].start_seconds, list.shots[0].end_seconds),
            (0.0, 2.0)
        );
        assert_eq!(
            (list.shots[2].start_seconds, list.shots[2].end_seconds),
            (5.0, 10.0)
        );
        list.validate().unwrap();
    }

    #[test]
    fn redundant_edges_and_duplicate_boundaries_are_canonicalized() {
        let list = ShotList::from_boundaries(
            4.0,
            vec![
                ShotBoundary {
                    frame_index: 0,
                    pts_seconds: 0.0,
                    confidence: None,
                },
                ShotBoundary {
                    frame_index: 30,
                    pts_seconds: 1.0,
                    confidence: Some(0.4),
                },
                ShotBoundary {
                    frame_index: 31,
                    pts_seconds: 1.0,
                    confidence: Some(0.9),
                },
                ShotBoundary {
                    frame_index: 120,
                    pts_seconds: 4.0,
                    confidence: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(list.boundaries.len(), 1);
        assert_eq!(list.boundaries[0].frame_index, 30);
        assert_eq!(list.boundaries[0].confidence, Some(0.9));
    }

    #[test]
    fn invalid_time_or_confidence_is_rejected() {
        assert!(
            ShotList::from_boundaries(
                1.0,
                vec![ShotBoundary {
                    frame_index: 1,
                    pts_seconds: 2.0,
                    confidence: Some(0.5),
                }]
            )
            .is_err()
        );
        assert!(
            ShotList::from_boundaries(
                1.0,
                vec![ShotBoundary {
                    frame_index: 1,
                    pts_seconds: 0.5,
                    confidence: Some(f32::NAN),
                }]
            )
            .is_err()
        );
    }

    #[test]
    fn unavailable_confidence_is_omitted_instead_of_fabricated() {
        let list = ShotList::from_boundaries(
            2.0,
            vec![ShotBoundary {
                frame_index: 10,
                pts_seconds: 1.0,
                confidence: None,
            }],
        )
        .unwrap();
        let value = serde_json::to_value(list).unwrap();
        assert!(value["boundaries"][0].get("confidence").is_none());
    }
}
