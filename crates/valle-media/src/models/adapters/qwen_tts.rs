//! Product bridge for offline, verified reference voice cloning releases.

use anyhow::Result;

use crate::models::{
    ResolvedModel,
    inference::qwen_tts::{OpenRequest, QwenTtsSession},
};

pub(crate) fn open(model: &ResolvedModel) -> Result<QwenTtsSession> {
    QwenTtsSession::open(OpenRequest {
        manifest: &model.manifest,
        route: &model.route,
        artifact: &model.artifact,
        artifact_root: &model.resolved.root,
    })
}
