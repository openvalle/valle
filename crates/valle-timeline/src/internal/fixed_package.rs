//! Shared fixed-package storage shape. Engine verifies member bytes and render semantics;
//! Project GC consumes the same closed shape without depending on the renderer.

use super::ContentDigest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub const FIXED_PACKAGE_FORMAT: &str = "valle.fixed-render-package@1";

pub const CANONICAL_TIMELINE_MEMBER_PATH: &str = "canonical-timeline.json";
pub const RESOURCE_MANIFEST_MEMBER_PATH: &str = "resource-manifest.json";
pub const VERIFIED_BINDING_BUNDLE_MEMBER_PATH: &str = "verified-binding-bundle.json";
pub const EXECUTION_PROFILE_MEMBER_PATH: &str = "execution-profile.json";

/// The closed outer manifest for one immutable fixed render package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixedPackageManifest {
    pub format: String,
    pub members: Vec<FixedPackageMember>,
    pub resource_digests: Vec<ContentDigest>,
}

impl FixedPackageManifest {
    pub fn format(&self) -> &str {
        &self.format
    }

    /// Content digests of the actual resource blobs a package publisher must
    /// retain. Digest-valued descriptor facts stay bound by member digests and
    /// are intentionally absent unless they name independently stored bytes.
    pub fn resource_digests(&self) -> &[ContentDigest] {
        &self.resource_digests
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixedPackageMember {
    pub role: FixedPackageMemberRole,
    pub path: String,
    pub bytes: u64,
    pub digest: ContentDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FixedPackageMemberRole {
    CanonicalTimeline,
    ExecutionProfile,
    ResourceManifest,
    VerifiedBindingBundle,
}

impl FixedPackageMemberRole {
    pub const ALL: [Self; 4] = [
        Self::CanonicalTimeline,
        Self::ExecutionProfile,
        Self::ResourceManifest,
        Self::VerifiedBindingBundle,
    ];

    pub const fn path(self) -> &'static str {
        match self {
            Self::CanonicalTimeline => CANONICAL_TIMELINE_MEMBER_PATH,
            Self::ExecutionProfile => EXECUTION_PROFILE_MEMBER_PATH,
            Self::ResourceManifest => RESOURCE_MANIFEST_MEMBER_PATH,
            Self::VerifiedBindingBundle => VERIFIED_BINDING_BUNDLE_MEMBER_PATH,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CanonicalTimeline => "canonical-timeline",
            Self::ExecutionProfile => "execution-profile",
            Self::ResourceManifest => "resource-manifest",
            Self::VerifiedBindingBundle => "verified-binding-bundle",
        }
    }

    pub fn from_path(path: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.path() == path)
    }
}

pub fn validate_fixed_package_manifest(manifest: &FixedPackageManifest) -> Result<(), String> {
    if manifest.format != FIXED_PACKAGE_FORMAT {
        return Err(format!(
            "[fixed_package_manifest] unsupported format {:?}",
            manifest.format
        ));
    }
    if manifest.members.len() != FixedPackageMemberRole::ALL.len() {
        return Err(format!(
            "[fixed_package_manifest] expected {} closed members, found {}",
            FixedPackageMemberRole::ALL.len(),
            manifest.members.len()
        ));
    }

    let mut roles = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut previous_key: Option<(&str, &str)> = None;
    for member in &manifest.members {
        if member.bytes > MAX_SAFE_INTEGER {
            return Err(format!(
                "[fixed_package_manifest] byte length for {:?} exceeds the interoperable JSON range",
                member.path
            ));
        }
        if !is_normalized_package_relative_path(&member.path) {
            return Err(format!(
                "[fixed_package_manifest] member path {:?} is not a normalized package-relative path",
                member.path
            ));
        }
        if member.path != member.role.path() {
            return Err(format!(
                "[fixed_package_manifest] role {:?} must use path {:?}",
                member.role.as_str(),
                member.role.path()
            ));
        }
        if !roles.insert(member.role) {
            return Err(format!(
                "[fixed_package_manifest] duplicate member role {:?}",
                member.role.as_str()
            ));
        }
        if !paths.insert(member.path.as_str()) {
            return Err(format!(
                "[fixed_package_manifest] duplicate member path {:?}",
                member.path
            ));
        }
        let key = (member.role.as_str(), member.path.as_str());
        if previous_key.is_some_and(|previous| previous >= key) {
            return Err(
                "[fixed_package_manifest] members must be sorted by role and path".to_owned(),
            );
        }
        previous_key = Some(key);
    }
    for role in FixedPackageMemberRole::ALL {
        if !roles.contains(&role) {
            return Err(format!(
                "[fixed_package_manifest] missing member role {:?}",
                role.as_str()
            ));
        }
    }

    if manifest
        .resource_digests
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(
            "[fixed_package_manifest] resourceDigests must be sorted and duplicate-free".to_owned(),
        );
    }
    Ok(())
}

pub fn is_normalized_package_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && !path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
        && !path
            .split('/')
            .next()
            .is_some_and(|component| component.contains(':'))
}
