//! Media analysis contracts are domain-owned values. Timeline conversion happens only in an
//! assembly root such as CLI or Project, so this module never imports authoring types.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub id: String,
    pub text: String,
    pub start: f64,
    pub end: f64,
}

impl Transcript {
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = HashSet::new();
        let mut previous_start = f64::NEG_INFINITY;
        for (index, word) in self.words.iter().enumerate() {
            if word.id.is_empty() {
                return Err(format!("words[{index}]: empty id"));
            }
            if !seen.insert(word.id.as_str()) {
                return Err(format!("words[{index}]: duplicate id {:?}", word.id));
            }
            if !(word.start.is_finite() && word.end.is_finite()) || word.start < 0.0 {
                return Err(format!(
                    "words[{index}] {:?}: non-finite/negative time",
                    word.id
                ));
            }
            if word.end < word.start {
                return Err(format!(
                    "words[{index}] {:?}: end {} < start {}",
                    word.id, word.end, word.start,
                ));
            }
            if word.start < previous_start {
                return Err(format!(
                    "words[{index}] {:?}: start {} out of order (prev {})",
                    word.id, word.start, previous_start,
                ));
            }
            previous_start = word.start;
        }
        Ok(())
    }
}

#[cfg(feature = "beat")]
pub mod beat;

#[cfg(feature = "beat")]
pub use beat::{Beat, Onset};

mod sentences;
mod shots;
pub use sentences::{Sentence, sentences};
pub use shots::{Shot, ShotBoundary, ShotList};
