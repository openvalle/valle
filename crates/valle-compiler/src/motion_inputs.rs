//! The same immutable Motion preparation projection is used by Native and browser hosts.
use crate::{CompileTimelineError, motion_instance_key, timeline::prepare_caption_presenter_data};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use valle_timeline::{Timeline, wire::timeline::*};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionPreparationInput {
    pub clip_path: String,
    pub component: String,
    pub locator: String,
    pub resources: BTreeMap<String, String>,
    pub data: Value,
}
impl MotionPreparationInput {
    pub fn key(&self) -> String {
        motion_instance_key(
            &self.locator,
            &if self.resources.is_empty() {
                Value::Null
            } else {
                serde_json::to_value(&self.resources).expect("aliases serialize")
            },
            &self.data,
        )
    }
}

pub fn motion_preparation_inputs(
    timeline: &Timeline,
) -> Result<Vec<MotionPreparationInput>, CompileTimelineError> {
    motion_preparation_inputs_wire(&timeline.clone().into_wire())
}

pub(crate) fn motion_preparation_inputs_wire(
    timeline: &TimelineWire,
) -> Result<Vec<MotionPreparationInput>, CompileTimelineError> {
    let mut inputs = Vec::new();
    for (ti, track) in timeline.tracks.visual.iter().enumerate() {
        for (ci, clip) in track.clips.iter().enumerate() {
            if let TimelineVisualSourceWire::Motion {
                component,
                resources,
                data,
                ..
            } = &clip.source
            {
                let path = format!("/tracks/visual/{ti}/clips/{ci}");
                inputs.push(MotionPreparationInput {
                    locator: locator(&timeline.resources, component, &path)?,
                    component: component.clone(),
                    clip_path: path,
                    resources: resources.clone(),
                    data: if data.is_empty() {
                        Value::Null
                    } else {
                        serde_json::to_value(data).expect("data serializes")
                    },
                });
            }
        }
    }
    for (ti, track) in timeline.tracks.caption.iter().enumerate() {
        if let Some(presenter) = &track.presenter {
            for (ci, clip) in track.clips.iter().enumerate() {
                inputs.push(caption_motion_input(
                    &timeline.resources,
                    presenter,
                    &track.style,
                    track.layout.as_ref(),
                    clip,
                    [timeline.canvas.width, timeline.canvas.height],
                    &format!("/tracks/caption/{ti}/clips/{ci}"),
                )?);
            }
        }
    }
    Ok(inputs)
}

pub(crate) fn caption_motion_input(
    locators: &BTreeMap<String, String>,
    presenter: &TimelineCaptionPresenterWire,
    style: &TimelineCaptionStyleWire,
    layout: Option<&TimelineCaptionLayoutWire>,
    clip: &TimelineCaptionClipWire,
    canvas: [u32; 2],
    path: &str,
) -> Result<MotionPreparationInput, CompileTimelineError> {
    if presenter
        .resources
        .contains_key(valle_timeline::motion::CAPTION_FONT_CONTROL)
    {
        return Err(CompileTimelineError::MotionPreparation {
            reason: format!(
                "{path}: presenter resources cannot bind the reserved caption font slot"
            ),
        });
    }
    let data = prepare_caption_presenter_data(style, layout, clip, canvas, path)?;
    let mut resources = presenter.resources.clone();
    resources.insert(
        valle_timeline::motion::CAPTION_FONT_CONTROL.into(),
        style.font.clone(),
    );
    for alias in resources.values() {
        locator(locators, alias, path)?;
    }
    Ok(MotionPreparationInput {
        locator: locator(locators, &presenter.component, path)?,
        component: presenter.component.clone(),
        clip_path: path.into(),
        resources,
        data: serde_json::to_value(data).expect("caption input serializes"),
    })
}
fn locator(
    locators: &BTreeMap<String, String>,
    alias: &str,
    path: &str,
) -> Result<String, CompileTimelineError> {
    locators
        .get(alias)
        .cloned()
        .ok_or_else(|| CompileTimelineError::UnknownResourceAlias {
            alias: alias.into(),
            path: path.into(),
        })
}
