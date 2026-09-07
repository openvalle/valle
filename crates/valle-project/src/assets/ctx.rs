//! Asset execution context with injected probing and analyzers. The project crate does not depend
//! on media codecs. Committed resource roots protect package pins and active leases during CAS
//! maintenance; the clock is supplied separately.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::assets::home::Home;
use crate::assets::kind::AssetKind;
use crate::assets::resource_roots::{CommittedResourceRootProvider, CommittedResourceRoots};
use crate::resource_gc::PackageResourceRootProvider;

/// Probe metadata with typed common fields and kind-specific extra fields.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Probe {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<u16>,
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// Probe outcome. Successful probes populate metadata; unavailable capabilities permit registration
/// with warnings; corrupt or unsupported media rejects the operation before publication.
#[derive(Debug, Clone, Default)]
pub struct ProbeOutcome {
    pub probe: Option<Probe>,
    pub warnings: Vec<String>,
}

/// Injected probing interface for media codecs, font parsers, or test implementations.
pub trait Prober {
    fn probe(&self, path: &Path, kind: AssetKind) -> crate::assets::report::Result<ProbeOutcome>;
}

/// Fallback prober that permits registration with empty metadata and a warning.
pub struct NoProber;

impl Prober for NoProber {
    fn probe(&self, _path: &Path, _kind: AssetKind) -> crate::assets::report::Result<ProbeOutcome> {
        Ok(ProbeOutcome {
            probe: None,
            warnings: vec![
                "probe skipped: no prober configured; duration and resolution are unavailable"
                    .to_owned(),
            ],
        })
    }
}

/// Context for one command execution.
pub struct Ctx {
    pub home: Home,
    pub prober: Box<dyn Prober>,
    /// Actor identity stored as the annotation author.
    pub actor: String,
    /// Registered analyzers, resolved by name during analysis.
    pub analyzers: Vec<Box<dyn crate::assets::analysis::Analyzer>>,
    /// Foreground progress callback; `None` is silent.
    pub progress: Option<Box<dyn Fn(&str)>>,
    committed_resource_roots: Box<dyn CommittedResourceRootProvider>,
}

impl Ctx {
    pub fn new(home: Home, prober: Box<dyn Prober>) -> Ctx {
        let project_store_root = home.root().to_path_buf();
        Ctx {
            home,
            prober,
            actor: "human-cli".to_owned(),
            analyzers: Vec::new(),
            progress: None,
            committed_resource_roots: Box::new(PackageResourceRootProvider::at(project_store_root)),
        }
    }

    /// Context without probing capabilities, useful for tests and lightweight callers.
    pub fn bare(home: Home) -> Ctx {
        Ctx::new(home, Box::new(NoProber))
    }

    pub fn with_actor(mut self, actor: impl Into<String>) -> Ctx {
        self.actor = actor.into();
        self
    }

    pub fn with_analyzers(
        mut self,
        analyzers: Vec<Box<dyn crate::assets::analysis::Analyzer>>,
    ) -> Ctx {
        self.analyzers = analyzers;
        self
    }

    /// Use an authoritative resource-root store separate from this asset CAS home.
    pub fn with_resource_root_store(mut self, root: impl Into<std::path::PathBuf>) -> Ctx {
        self.committed_resource_roots = Box::new(PackageResourceRootProvider::at(root));
        self
    }

    /// Inject a service-specific committed-root provider.
    pub fn with_committed_resource_root_provider(
        mut self,
        provider: impl CommittedResourceRootProvider + 'static,
    ) -> Ctx {
        self.committed_resource_roots = Box::new(provider);
        self
    }

    pub(crate) fn committed_resource_roots(
        &self,
    ) -> crate::assets::report::Result<CommittedResourceRoots> {
        self.committed_resource_roots.acquire()
    }

    pub(crate) fn report_progress(&self, msg: &str) {
        if let Some(p) = &self.progress {
            p(msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_extra_flattens() {
        let p = Probe {
            duration_ms: Some(754_000),
            width: Some(3840),
            ..Default::default()
        };
        let mut p = p;
        p.extra
            .insert("family".into(), serde_json::json!("Noto Sans CJK"));
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["duration_ms"], 754_000);
        assert_eq!(v["family"], "Noto Sans CJK");
        let back: Probe = serde_json::from_value(v).unwrap();
        assert_eq!(back, p);
    }
}
