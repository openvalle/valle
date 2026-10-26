//! Attach authoring information at the CLI boundary without putting source maps in render artifacts.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use valle_compiler::motion::MotionSourceMap;
use valle_engine::render::{
    CompiledRender, EvaluatedCaption, EvaluatedSourceRef, EvaluatedVisualOperation, FrameKey,
};
use valle_render::host::{FrameRenderError, NativeRenderer, PipelineError};

#[derive(Default)]
pub(super) struct SourceMaps(BTreeMap<String, MotionSourceMap>);

impl SourceMaps {
    pub fn motion(map: MotionSourceMap) -> Self {
        Self(BTreeMap::from([(String::new(), map)]))
    }

    pub fn insert(&mut self, resource: String, map: MotionSourceMap) {
        self.0.insert(resource, map);
    }

    pub fn location(
        &self,
        render: &CompiledRender,
        frame: FrameKey,
        error: &FrameRenderError,
    ) -> Option<Value> {
        let map = if let Some(map) = self.0.get("") {
            map
        } else {
            let evaluated = render.evaluate(frame).ok()?;
            let path = error.path()?;
            let indexed = |prefix: &str| {
                let (index, suffix) = path.strip_prefix(prefix)?.split_once(']')?;
                Some((index.parse::<usize>().ok()?, suffix))
            };
            let source: &EvaluatedSourceRef = if let Some((index, suffix)) = indexed("visual[") {
                match evaluated.visual().get(index)? {
                    EvaluatedVisualOperation::Clip(clip) => clip.source(),
                    EvaluatedVisualOperation::Transition(transition) => {
                        if suffix.starts_with(".from") {
                            transition.from()
                        } else if suffix.starts_with(".to") {
                            transition.to()
                        } else {
                            transition.template()?
                        }
                    }
                }
            } else if let Some((index, _)) = indexed("captions[") {
                match evaluated.captions().get(index)? {
                    EvaluatedCaption::Motion(clip) => clip.source(),
                    EvaluatedCaption::Text(_) => return None,
                }
            } else if let Some((index, _)) = indexed("adjustments[") {
                evaluated.adjustments().get(index)?.template()?
            } else {
                return None;
            };
            self.0.get(source.resource()?.resource_id())?
        };
        source_location(&error.to_string(), map)
    }

    pub fn attach(&self, error: anyhow::Error, renderer: &NativeRenderer) -> anyhow::Error {
        let location = match crate::failed_frame(&error) {
            Some(PipelineError::AtFrame { frame, error, .. }) => {
                self.location(renderer.project().compiled(), FrameKey::new(*frame), error)
            }
            _ => None,
        };
        match location {
            Some(location) => anyhow::Error::new(SourceLocated {
                message: format!(
                    "{error:#} ({}:{}:{}, component {})",
                    location["path"].as_str().unwrap_or_default(),
                    location["line"],
                    location["column"],
                    location["component"].as_str().unwrap_or_default(),
                ),
                error,
                location,
            }),
            None => error,
        }
    }
}

fn source_location(message: &str, map: &MotionSourceMap) -> Option<Value> {
    let identifier = |prefix: &str| {
        message
            .split(prefix)
            .nth(1)?
            .split_once(':')?
            .0
            .parse::<usize>()
            .ok()
    };
    let expressions = match identifier("instance group ") {
        Some(group) => map.instance_exprs.get(group)?.as_slice(),
        None => &map.exprs,
    };
    let expression = identifier("expression ").and_then(|id| {
        expressions
            .iter()
            .find(|mapping| mapping.id.0 as usize == id)
    });
    let node = map.nodes.iter().find(|node| {
        message.contains(&format!("node `{}`", node.key))
            || message.contains(&format!("Transition '{}'", node.key))
    });
    let (path, span, stack) = match (expression, node) {
        (Some(expression), _) => (
            &expression.source_path,
            &expression.span,
            &expression.expansion_stack,
        ),
        (_, Some(node)) => (&node.source_path, &node.span, &node.expansion_stack),
        _ => return None,
    };
    Some(json!({"path":path,"line":span.line,"column":span.column,
        "component":map.component,"expansionStack":stack}))
}

/// Retain the original typed error so error codes and output-frame diagnostics survive annotation.
#[derive(Debug)]
pub(crate) struct SourceLocated {
    message: String,
    pub(crate) error: anyhow::Error,
    pub(crate) location: Value,
}

impl std::fmt::Display for SourceLocated {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SourceLocated {}
