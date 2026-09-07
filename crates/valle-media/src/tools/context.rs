use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::models::ModelManager;

use super::{ToolError, ToolErrorCode};

/// A stable job phase suitable for terminal progress and future structured events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPhase {
    Resolving,
    Loading,
    Decoding,
    Preprocessing,
    Inferencing,
    Postprocessing,
    Encoding,
    Validating,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolEvent {
    pub phase: ToolPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ToolEvent {
    pub fn phase(phase: ToolPhase) -> Self {
        Self {
            phase,
            completed: None,
            total: None,
            message: None,
        }
    }

    pub fn count(phase: ToolPhase, completed: u64, total: u64) -> Self {
        Self {
            phase,
            completed: Some(completed),
            total: Some(total),
            message: None,
        }
    }
}

pub trait ProgressSink {
    fn event(&mut self, event: ToolEvent);
}

impl<F> ProgressSink for F
where
    F: FnMut(ToolEvent),
{
    fn event(&mut self, event: ToolEvent) {
        self(event);
    }
}

#[derive(Debug, Default)]
pub struct NoopProgress;

impl ProgressSink for NoopProgress {
    fn event(&mut self, _event: ToolEvent) {}
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Resource ceilings for one foreground media job.
#[derive(Debug, Clone)]
pub struct ResourcePolicy {
    /// Maximum CPU threads requested from software codecs and model kernels that expose a
    /// configurable per-job ceiling.
    pub cpu_threads: usize,
    /// Maximum number of logical model sessions that may be live concurrently for this job.
    ///
    /// The current file tools deliberately keep exactly one session live at a time. In
    /// particular, transcription drops its ASR session before opening the forced aligner.
    pub inference_sessions: usize,
    pub pipeline_capacity: usize,
    pub audio_memory_budget_bytes: u64,
    pub temporary_directory: PathBuf,
    pub temporary_disk_budget_bytes: u64,
}

impl Default for ResourcePolicy {
    fn default() -> Self {
        Self {
            cpu_threads: std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1),
            inference_sessions: 1,
            pipeline_capacity: 2,
            audio_memory_budget_bytes: 512 * 1024 * 1024,
            temporary_directory: std::env::temp_dir(),
            temporary_disk_budget_bytes: 10 * 1024 * 1024 * 1024,
        }
    }
}

impl ResourcePolicy {
    pub fn validate(&self) -> Result<(), ToolError> {
        for (name, value) in [
            ("cpu_threads", self.cpu_threads),
            ("inference_sessions", self.inference_sessions),
            ("pipeline_capacity", self.pipeline_capacity),
        ] {
            if value == 0 {
                return Err(ToolError::new(
                    ToolErrorCode::InvalidInput,
                    format!("resource policy {name} must be greater than zero"),
                ));
            }
        }
        if self.audio_memory_budget_bytes == 0 || self.temporary_disk_budget_bytes == 0 {
            return Err(ToolError::new(
                ToolErrorCode::InvalidInput,
                "resource policy memory and temporary-disk budgets must be greater than zero",
            ));
        }
        if !self.temporary_directory.is_dir() {
            return Err(ToolError::new(
                ToolErrorCode::InvalidInput,
                format!(
                    "resource policy temporary directory does not exist: {}",
                    self.temporary_directory.display()
                ),
            ));
        }
        Ok(())
    }
}

pub struct RunContext<'a> {
    pub models: &'a ModelManager,
    pub progress: &'a mut dyn ProgressSink,
    pub cancellation: CancellationToken,
    pub resources: ResourcePolicy,
}

impl<'a> RunContext<'a> {
    pub fn new(models: &'a ModelManager, progress: &'a mut dyn ProgressSink) -> Self {
        Self {
            models,
            progress,
            cancellation: CancellationToken::new(),
            resources: ResourcePolicy::default(),
        }
    }

    pub fn validate(&self) -> Result<(), ToolError> {
        self.resources.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_policy_rejects_unbounded_zero_capacity() {
        let policy = ResourcePolicy {
            pipeline_capacity: 0,
            ..ResourcePolicy::default()
        };
        assert_eq!(
            policy.validate().unwrap_err().code,
            ToolErrorCode::InvalidInput
        );
    }

    #[test]
    fn resource_policy_rejects_zero_cpu_threads_and_session_capacity() {
        for policy in [
            ResourcePolicy {
                cpu_threads: 0,
                ..ResourcePolicy::default()
            },
            ResourcePolicy {
                inference_sessions: 0,
                ..ResourcePolicy::default()
            },
        ] {
            assert_eq!(
                policy.validate().unwrap_err().code,
                ToolErrorCode::InvalidInput
            );
        }
    }

    #[test]
    fn resource_policy_requires_an_existing_spool_directory() {
        let policy = ResourcePolicy {
            temporary_directory: std::env::temp_dir()
                .join(format!("valle-media-missing-spool-{}", std::process::id())),
            ..ResourcePolicy::default()
        };
        assert_eq!(
            policy.validate().unwrap_err().code,
            ToolErrorCode::InvalidInput
        );
    }
}
