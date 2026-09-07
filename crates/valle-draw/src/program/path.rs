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
