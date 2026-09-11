//! Shared filesystem helpers for the narrow Timeline CLI surface.

pub mod assets;
pub mod assets_analyzers;
pub mod enhance;

pub mod fixed_render;
pub mod inpaint;
pub mod interpolate;
pub mod matte;
pub mod media;
pub mod models;

pub mod motion;

mod motion_package;
pub mod project;
pub mod segment;
pub mod separate;
pub mod shots;
pub mod timeline;
#[cfg(not(target_os = "windows"))]
pub mod transcribe;
pub mod upscale;

use std::path::Path;

use anyhow::{Context, Result};

pub(crate) fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}
