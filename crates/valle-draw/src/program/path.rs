use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathVerb {
    MoveTo,
    LineTo,
    QuadTo,
    CubicTo,
    Close,
}

impl PathVerb {
    pub(crate) const fn point_count(self) -> usize {
        match self {
            Self::MoveTo | Self::LineTo => 1,
            Self::QuadTo => 2,
            Self::CubicTo => 3,
            Self::Close => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathData {
    pub verbs: Vec<PathVerb>,
    pub points: Vec<[f64; 2]>,
}

impl PathData {
    pub const fn empty() -> Self {
        Self {
            verbs: Vec::new(),
            points: Vec::new(),
        }
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn empty_path_wire_and_runtime_point_counts_preserve_the_closed_verb_contract() {
        let constructor = std::hint::black_box(PathData::empty as fn() -> PathData);
        let empty = constructor();
        assert_eq!(
            serde_json::to_value(empty).unwrap(),
            serde_json::json!({"verbs":[],"points":[]})
        );
        let count = std::hint::black_box(PathVerb::point_count as fn(PathVerb) -> usize);
        for (verb, expected) in [
            (PathVerb::MoveTo, 1),
            (PathVerb::LineTo, 1),
            (PathVerb::QuadTo, 2),
            (PathVerb::CubicTo, 3),
            (PathVerb::Close, 0),
        ] {
            assert_eq!(count(verb), expected);
        }
    }
}
