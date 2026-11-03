//! Frames encoded into a video.

use std::path::Path;

use serde_json::Value;
use valle_engine::scene::Scene;

use crate::Failure;
use crate::args::Args;

pub fn render(_args: &Args, _scene: &mut Scene, out: &Path) -> Result<Value, Failure> {
    Err(Failure::new(
        "usage",
        format!("{}: video output is not supported yet", out.display()),
    )
    .hint("draw frames into a folder of PNG files"))
}
