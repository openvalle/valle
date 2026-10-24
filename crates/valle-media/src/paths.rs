//! Home and non-model cache paths shared by project, library, and render hosts.
use anyhow::{Context, Result};
use std::path::PathBuf;

fn variable(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

pub fn app_home() -> Result<PathBuf> {
    if let Some(root) = variable("VALLE_HOME") {
        return Ok(root.into());
    }
    let home = variable("HOME")
        .or_else(|| variable("USERPROFILE"))
        .context("set VALLE_HOME because no user home directory is available")?;
    Ok(PathBuf::from(home).join(".valle"))
}

pub fn cache_dir() -> PathBuf {
    variable("VALLE_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            app_home()
                .unwrap_or_else(|_| PathBuf::from(".valle"))
                .join("cache")
        })
}
