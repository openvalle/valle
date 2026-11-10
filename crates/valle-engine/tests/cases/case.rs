//! A case's specification, sample time and expected layout in canvas pixels.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use valle_engine::layout::Rect;
use valle_engine::time::Tick;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub spec: String,
    pub assertion: String,
    pub input: Option<String>,
    pub at: Vec<serde_json::Number>,
    pub browser: Option<Browser>,
    pub valle: Option<Valle>,
    #[serde(default)]
    pub layout: Vec<Layout>,
    #[serde(default)]
    pub lines: Vec<Lines>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Valle {
    pub reason: String,
    #[serde(default)]
    pub max_difference: u8,
    /// How many pixels may differ, each by up to `max_difference`: any number when absent.
    #[serde(default)]
    pub max_pixels: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Browser {
    pub selected: String,
    pub reason: String,
    #[serde(default)]
    pub pixels: Tolerance,
    #[serde(default, rename = "textEdges")]
    pub text_edges: Option<TextEdges>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextEdges {
    pub key: String,
    pub pixels: Tolerance,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Tolerance {
    pub max_difference: u8,
    pub max_pixels: usize,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Lines {
    pub key: String,
    pub text: Vec<String>,
    pub geometry: Vec<Line>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub x: f32,
    pub baseline: f32,
    pub height: f32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub key: String,
    pub border: Rectangle,
    pub padding: Rectangle,
    pub content: Rectangle,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rectangle {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Valle {
    pub fn accepts(&self, changed: usize, max: u8) -> bool {
        self.max_pixels.is_none_or(|n| changed <= n) && max <= self.max_difference
    }
}

impl Tolerance {
    pub fn accepts(&self, changed: usize, max: u8) -> bool {
        changed <= self.max_pixels && max <= self.max_difference
    }
}

impl Case {
    pub fn read(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let case: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if case.spec.trim().is_empty() || case.assertion.trim().is_empty() {
            return Err("spec and assertion must describe what the case verifies".into());
        }
        let times = case.times()?;
        if times.is_empty() || times[0] < Tick::ZERO || times.windows(2).any(|w| w[0] >= w[1]) {
            return Err("at must contain increasing, nonnegative sample times".into());
        }
        if case.browser.is_some() && times != [Tick::ZERO] {
            return Err("browser captures currently support only the static sample at [0]".into());
        }
        if let Some(valle) = &case.valle
            && valle.reason.trim().is_empty()
        {
            return Err("a Valle baseline needs a review reason".into());
        }
        if case.layout.is_empty() && case.lines.is_empty() && !case.visual() {
            return Err("a case needs numerical assertions or an image baseline".into());
        }
        if let Some(input) = &case.input
            && (input.is_empty()
                || !Path::new(input)
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
                || !(input.ends_with(".motion.tsx") || input.ends_with(".timeline.json")))
        {
            return Err(
                "input must name a .motion.tsx or .timeline.json file inside the case directory"
                    .into(),
            );
        }
        if let Some(browser) = &case.browser
            && (!matches!(browser.selected.as_str(), "chrome" | "firefox" | "webkit")
                || browser.reason.trim().is_empty())
        {
            return Err("browser needs a selected engine and a review reason".into());
        }
        let mut keys = HashSet::new();
        for layout in &case.layout {
            if layout.key.trim().is_empty() || !keys.insert(&layout.key) {
                return Err(format!("layout key {:?} is empty or repeated", layout.key));
            }
            for rect in [&layout.border, &layout.padding, &layout.content] {
                if ![rect.x, rect.y, rect.width, rect.height]
                    .into_iter()
                    .all(f32::is_finite)
                    || rect.width < 0.0
                    || rect.height < 0.0
                {
                    return Err(format!(
                        "{}: expected a finite, nonnegative size",
                        layout.key
                    ));
                }
            }
        }
        let mut keys = HashSet::new();
        for lines in &case.lines {
            if lines.key.is_empty()
                || lines.text.is_empty()
                || !keys.insert(&lines.key)
                || lines.geometry.len() != lines.text.len()
                || lines.geometry.iter().any(|g| {
                    ![g.x, g.baseline, g.height].into_iter().all(f32::is_finite) || g.height <= 0.0
                })
            {
                return Err(
                    "line assertions need unique keys, text and finite geometry for every line"
                        .into(),
                );
            }
        }
        if let Some(edges) = case.browser.as_ref().and_then(|b| b.text_edges.as_ref())
            && (!case.layout.iter().any(|l| l.key == edges.key)
                || !case.lines.iter().any(|l| l.key == edges.key))
        {
            return Err("textEdges must name a key with both layout and line assertions".into());
        }
        Ok(case)
    }

    pub fn times(&self) -> Result<Vec<Tick>, String> {
        self.at
            .iter()
            .map(|at| Tick::parse_seconds(&at.to_string()))
            .collect()
    }

    pub fn input(&self, dir: &Path) -> PathBuf {
        dir.join(self.input.as_deref().unwrap_or("scene.motion.tsx"))
    }

    pub fn visual(&self) -> bool {
        self.browser.is_some() || self.valle.is_some()
    }
}

impl From<Rect> for Rectangle {
    fn from(r: Rect) -> Self {
        Self {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

impl Rectangle {
    pub fn rect(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

impl Rectangle {
    pub fn check(&self, actual: Rect, name: &str, failures: &mut Vec<String>) {
        for (field, expected, actual) in [
            ("x", self.x, actual.x),
            ("y", self.y, actual.y),
            ("width", self.width, actual.width),
            ("height", self.height, actual.height),
        ] {
            if actual != expected {
                failures.push(format!(
                    "{name}.{field}: expected {expected}, actual {actual}"
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../../../tests/cases/css/box-model/content-box/case.json"
        ))
        .unwrap()
    }

    #[test]
    fn empty_and_misspelled_checks_are_errors() {
        for broken in [
            "empty",
            "field",
            "rectangle",
            "time",
            "empty_times",
            "order",
            "duplicate",
            "key",
            "input",
        ] {
            let mut value = input();
            match broken {
                "empty" => {
                    value["layout"] = serde_json::json!([]);
                    value["browser"] = serde_json::Value::Null;
                }
                "field" => value["layouts"] = value["layout"].take(),
                "rectangle" => value["layout"][0]["border"]["widht"] = 128.into(),
                "time" => value["at"] = serde_json::json!([-1]),
                "empty_times" => value["at"] = serde_json::json!([]),
                "order" => value["at"] = serde_json::json!([1, 0]),
                "duplicate" => value["at"] = serde_json::json!([0, 0]),
                "key" => value["layout"][0]["key"] = "".into(),
                "input" => value["input"] = "../outside.motion.tsx".into(),
                _ => unreachable!(),
            }
            assert!(
                Case::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{broken}"
            );
        }
    }

    #[test]
    fn rectangle_checks_report_offsets_and_nonfinite_values() {
        let expected = Rectangle {
            x: 20.0,
            y: 20.0,
            width: 128.0,
            height: 88.0,
        };
        let mut failures = Vec::new();
        expected.check(
            Rect {
                x: 21.0,
                y: 20.0,
                width: f32::NAN,
                height: 88.0,
            },
            "box.border",
            &mut failures,
        );
        assert_eq!(
            failures,
            [
                "box.border.x: expected 20, actual 21",
                "box.border.width: expected 128, actual NaN",
            ]
        );
    }
}
