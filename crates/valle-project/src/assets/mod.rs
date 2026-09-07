//! Asset library core. Full content hashes associate byte locations with machine analysis and human
//! knowledge. Files are authoritative; SQLite search is rebuildable. Human annotations are never
//! overwritten by analysis, and queries require no model execution. Probing and analyzers are
//! injected; resource-root contracts expose only published package and active lease digests.

pub mod add;
pub mod addr;
pub mod analysis;
pub mod analyze;
pub mod annotate;
pub mod asrseg;
pub mod cachefs;
pub mod clock;
pub mod crash;
pub mod ctx;
pub mod db;
pub mod entity;
pub mod execute;
pub mod fsutil;
pub mod fts;
pub mod home;
pub mod kind;
pub mod knowledge;
pub mod lock;
pub mod maintain;
pub mod meta;
pub mod read;
pub mod report;
pub mod resolve;
pub mod resource_roots;
pub mod search;
pub mod sqlgate;
pub mod transport;
pub mod units;
pub mod verb;
pub mod vlm;

pub use crate::resource_gc::{ActiveResourceLease, PublishedPackagePin, ResourceRootStore};
pub use ctx::{Ctx, NoProber, Probe, ProbeOutcome, Prober};
pub use db::Db;
pub use execute::{execute, execute_add_batch};
pub use home::{Home, fanout};
pub use kind::AssetKind;
pub use meta::{AssetMeta, Location, LocationKind};
pub use report::{AssetsError, ErrorCode, Report, Result};
pub use resource_roots::{CommittedResourceRootProvider, CommittedResourceRoots};
pub use verb::{AddMode, EntityOp, Verb};
