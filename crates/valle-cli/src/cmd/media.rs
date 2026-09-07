//! Presentation shared by `valle media` commands.

use std::{path::Path, process::ExitCode, sync::OnceLock};

use anyhow::Result;

use serde::Serialize;
use valle_media::tools::{
    MediaRunEnvelope, ProgressSink, ToolError, ToolErrorCode, ToolEvent, ToolRun, ToolWarning,
};

/// Human progress goes to stderr so stdout remains a stable command result channel.
pub(crate) struct TerminalProgress {
    enabled: bool,
}

impl TerminalProgress {
    pub(crate) fn new(enabled: bool) -> Self {
        Self { enabled }
    }
}

impl ProgressSink for TerminalProgress {
    fn event(&mut self, event: ToolEvent) {
        if crate::output::events() {
            crate::events::emit(crate::events::EventKind::AnalyzeProgress {
                message: format!("{:?}: {:?}/{:?}", event.phase, event.completed, event.total),
            });
            return;
        }
        if !self.enabled {
            return;
        }
        let phase = serde_json::to_value(event.phase)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| "working".to_owned());
        match (event.completed, event.total, event.message) {
            (Some(completed), Some(total), Some(message)) => {
                eprintln!("{phase} {completed}/{total}: {message}");
            }
            (Some(completed), Some(total), None) => eprintln!("{phase} {completed}/{total}"),
            (_, _, Some(message)) => eprintln!("{phase}: {message}"),
            _ => eprintln!("{phase}"),
        }
    }
}

pub(crate) fn print_json_success<T: Serialize>(run: &ToolRun<T>) -> Result<()> {
    crate::output::emit(serde_json::to_value(MediaRunEnvelope::success(run))?);
    Ok(())
}

pub(crate) fn print_warnings(warnings: &[ToolWarning]) {
    for warning in warnings {
        eprintln!("warning [{}]: {}", warning.code, warning.message);
    }
}

pub(crate) fn enable_cancellation(
    context: &mut valle_media::tools::RunContext<'_>,
) -> Result<(), ToolError> {
    use valle_media::tools::CancellationToken;

    static TOKEN: OnceLock<Result<CancellationToken, String>> = OnceLock::new();
    let token = TOKEN.get_or_init(|| {
        let token = CancellationToken::new();
        let signal_token = token.clone();
        ctrlc::set_handler(cancellation_callback(signal_token))
            .map(|()| token)
            .map_err(|error| error.to_string())
    });
    context.cancellation = token.clone().map_err(|error| {
        ToolError::new(
            ToolErrorCode::RuntimeUnavailable,
            format!("install Ctrl-C cancellation handler: {error}"),
        )
    })?;
    Ok(())
}

fn cancellation_callback(
    token: valle_media::tools::CancellationToken,
) -> impl FnMut() + Send + 'static {
    move || token.cancel()
}

pub(crate) fn render_error(error: &ToolError, json: bool) -> Result<ExitCode> {
    if json {
        crate::output::emit(serde_json::to_value(MediaRunEnvelope::failure(error, &[]))?);
    } else {
        eprintln!("error [{:?}]: {}", error.code, error.message);
        if let Some(hint) = &error.hint {
            eprintln!("  hint: {hint}");
        }
    }
    Ok(exit_code(error.code))
}

pub(crate) fn validate_distinct_paths(
    input: &Path,
    output: Option<&Path>,
    report: Option<&Path>,
) -> Result<(), ToolError> {
    if output.is_some_and(|output| paths_refer_to_same_entry(input, output)) {
        return Err(ToolError::invalid_input(
            "input and output must be different files",
        ));
    }
    if report.is_some_and(|report| paths_refer_to_same_entry(input, report)) {
        return Err(ToolError::invalid_input(
            "input and report must be different files",
        ));
    }
    if let (Some(output), Some(report)) = (output, report)
        && paths_refer_to_same_entry(output, report)
    {
        return Err(ToolError::invalid_input(
            "media output and report must be different files",
        ));
    }
    Ok(())
}

fn paths_refer_to_same_entry(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => {
            let resolve_parent = |path: &Path| {
                let parent = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                Some((
                    std::fs::canonicalize(parent).ok()?,
                    path.file_name()?.to_owned(),
                ))
            };
            match (resolve_parent(left), resolve_parent(right)) {
                (Some(left), Some(right)) => left == right,
                _ => false,
            }
        }
    }
}

fn exit_code(code: ToolErrorCode) -> ExitCode {
    let code = match code {
        ToolErrorCode::InvalidInput => 2,
        ToolErrorCode::ModelNotInstalled
        | ToolErrorCode::CatalogUnavailable
        | ToolErrorCode::AuthRequired
        | ToolErrorCode::ArtifactIntegrity
        | ToolErrorCode::NoCompatibleRoute
        | ToolErrorCode::UnsupportedAdapter
        | ToolErrorCode::RuntimeUnavailable => 3,
        ToolErrorCode::ResourceBusy
        | ToolErrorCode::ModelLoadFailed
        | ToolErrorCode::InferenceFailed
        | ToolErrorCode::OutputValidationFailed => 4,
        ToolErrorCode::Cancelled => 130,
        ToolErrorCode::Internal => 1,
    };
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_errors_map_to_stable_process_exit_codes() {
        assert_eq!(exit_code(ToolErrorCode::InvalidInput), ExitCode::from(2));
        assert_eq!(
            exit_code(ToolErrorCode::ModelNotInstalled),
            ExitCode::from(3)
        );
        assert_eq!(exit_code(ToolErrorCode::InferenceFailed), ExitCode::from(4));
        assert_eq!(exit_code(ToolErrorCode::Cancelled), ExitCode::from(130));
        assert_eq!(exit_code(ToolErrorCode::Internal), ExitCode::from(1));
    }

    #[test]

    fn conflicting_media_and_report_destinations_are_rejected() {
        let output = Path::new("out.mov");
        assert!(validate_distinct_paths(Path::new("in.mov"), Some(output), Some(output)).is_err());
        assert!(
            validate_distinct_paths(
                Path::new("in.mov"),
                Some(output),
                Some(Path::new("run.json"))
            )
            .is_ok()
        );
    }

    #[test]

    fn cancellation_callback_trips_the_job_token() {
        let token = valle_media::tools::CancellationToken::new();
        let mut callback = cancellation_callback(token.clone());
        assert!(!token.is_cancelled());
        callback();
        assert!(token.is_cancelled());
    }
}
