//! `~/.valle/settings.json`: what a person sets once for every command they run. It holds only
//! what is read from it below; a field it does not know is an error that names the field.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Settings {
    /// The folder of FFmpeg's shared libraries, as `VALLE_FFMPEG_DIR` and `--ffmpeg-dir` give it;
    /// they come first.
    pub ffmpeg_dir: Option<PathBuf>,
}

/// The settings file: `settings.json` in `~/.valle`, which `$VALLE_HOME` names instead.
pub fn path() -> Option<PathBuf> {
    let home = match std::env::var_os("VALLE_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::home_dir()?.join(".valle"),
    };
    Some(home.join("settings.json"))
}

impl Settings {
    /// The settings, all unset where there is no file.
    pub fn load() -> Result<Settings, String> {
        match path() {
            Some(path) => Settings::read(&path),
            None => Ok(Settings::default()),
        }
    }

    pub fn read(path: &Path) -> Result<Settings, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads `text` as a settings file of its own, in a folder per test.
    fn read(name: &str, text: &str) -> Result<Settings, String> {
        let dir =
            std::env::temp_dir().join(format!("valle-settings-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, text).unwrap();
        let settings = Settings::read(&path);
        let _ = std::fs::remove_dir_all(&dir);
        settings
    }

    #[test]
    fn a_missing_file_sets_nothing() {
        let path = std::env::temp_dir().join("valle-settings-none/settings.json");
        assert_eq!(Settings::read(&path), Ok(Settings::default()));
    }

    #[test]
    fn reads_the_ffmpeg_directory() {
        let settings = read("directory", r#"{ "ffmpegDir": "/opt/ffmpeg/lib" }"#).unwrap();
        assert_eq!(settings.ffmpeg_dir, Some(PathBuf::from("/opt/ffmpeg/lib")));
        assert_eq!(read("empty", "{}"), Ok(Settings::default()));
    }

    #[test]
    fn a_field_it_does_not_know_is_an_error_naming_it() {
        let error = read("unknown", r#"{ "ffmpegDirr": "/x" }"#).unwrap_err();
        assert!(error.contains("ffmpegDirr"), "{error}");
        assert!(read("broken", "{").is_err());
    }
}
