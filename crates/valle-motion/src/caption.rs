//! Frozen caption input for captionPresenter. Placement and host clocks are runtime inputs.
use serde::{Deserialize, Serialize};
use valle_timeline::internal::wire::document::CaptionAlignWire;
use valle_timeline::{RationalTime, wire::timeline::TimelineTimeWire};

pub const CAPTION_FONT_CONTROL: &str = "caption";
pub const CAPTION_FONT_URI: &str = "asset://caption";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionPresenterData {
    pub text: String,
    pub runs: Vec<CaptionPresenterRun>,
    pub style: CaptionPresenterStyle,
    /// Canvas pixels: x, y, width, height.
    pub region: [f64; 4],
    pub align: CaptionAlignWire,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionPresenterRun {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionPresenterStyle {
    pub font: String,
    pub font_size: f64,
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<CaptionPresenterShadow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionPresenterShadow {
    pub color: String,
    pub offset: [f64; 2],
    pub blur: f64,
}

impl CaptionPresenterData {
    /// Compile checks the static shape; hosts additionally check timed runs against placement duration.
    pub fn validate(&self, host_duration: Option<RationalTime>) -> Result<(), String> {
        if self.runs.len() > crate::controls::MAX_PREPARE_DATA_ARRAY_ITEMS {
            return Err("caption runs exceed the prepare-data array budget".into());
        }
        // The closed shape has fixed depth. Count its serialized values against the existing budget.
        let total_items = 13
            + usize::from(self.style.shadow.is_some()) * 6
            + self
                .runs
                .iter()
                .map(|run| {
                    2 + usize::from(run.start.is_some())
                        + usize::from(run.end.is_some())
                        + usize::from(run.font_size.is_some())
                        + usize::from(run.color.is_some())
                })
                .sum::<usize>();
        if total_items > crate::controls::MAX_PREPARE_DATA_TOTAL_ITEMS {
            return Err("caption input exceeds the prepare-data value budget".into());
        }
        let bytes = crate::canonical_bytes(self)
            .map_err(|error| error.to_string())?
            .len();
        if bytes > crate::controls::MAX_PREPARE_DATA_BYTES {
            return Err("caption input exceeds the prepare-data byte budget".into());
        }
        if self.runs.is_empty()
            || self.text
                != self
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
        {
            return Err("caption text must equal the concatenation of its runs".into());
        }
        if self.style.font != CAPTION_FONT_URI {
            return Err("caption style.font must use the reserved asset://caption font".into());
        }
        positive_size(self.style.font_size, "style/fontSize")?;
        color(&self.style.color, "style/color")?;
        if self.region.iter().any(|v| !v.is_finite())
            || self.region[2] <= 0.0
            || self.region[3] <= 0.0
        {
            return Err(
                "caption region must contain finite pixel coordinates and positive dimensions"
                    .into(),
            );
        }
        if let Some(shadow) = &self.style.shadow {
            color(&shadow.color, "style/shadow/color")?;
            if shadow.offset.iter().any(|v| !v.is_finite())
                || !shadow.blur.is_finite()
                || shadow.blur < 0.0
            {
                return Err("caption shadow needs finite offsets and non-negative blur".into());
            }
        }
        let mut previous_end = RationalTime::ZERO;
        for (index, run) in self.runs.iter().enumerate() {
            let path = format!("runs/{index}");
            if let Some(size) = run.font_size {
                positive_size(size, &format!("{path}/fontSize"))?;
            }
            if let Some(value) = &run.color {
                color(value, &format!("{path}/color"))?;
            }
            match (run.start, run.end) {
                (None, None) => {}
                (Some(start), Some(end)) => {
                    let start = seconds(start, &format!("{path}/start"))?;
                    let end = seconds(end, &format!("{path}/end"))?;
                    if start < previous_end
                        || end <= start
                        || host_duration.is_some_and(|duration| end > duration)
                    {
                        return Err(format!(
                            "{path}: caption run times must be ordered, non-overlapping and within host duration"
                        ));
                    }
                    previous_end = end;
                }
                _ => return Err(format!("{path}: caption run needs both start and end")),
            }
        }
        Ok(())
    }
}

fn seconds(value: f64, path: &str) -> Result<RationalTime, String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{path}: expected finite non-negative seconds"));
    }
    let time = TimelineTimeWire::new(value.to_string())
        .map(|time| RationalTime::from_exact(time.to_exact()))
        .map_err(|error| format!("{path}: {error}"))?;
    if time.as_f64() != value {
        return Err(format!(
            "{path}: frozen caption seconds must already be normalized to six fractional digits"
        ));
    }
    Ok(time)
}
fn positive_size(value: f64, path: &str) -> Result<(), String> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(format!("{path}: expected a positive finite font size"))
    }
}
fn color(value: &str, path: &str) -> Result<(), String> {
    if valle_draw::Rgba::parse(value).is_some() {
        Ok(())
    } else {
        Err(format!("{path}: invalid caption color"))
    }
}
