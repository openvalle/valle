//! Content-addressed GC root boundary consumed by the asset library.
//!
//! This module keeps roots as typed content digests. Published fixed packages
//! and active render leases supply a guarded set via
//! [`CommittedResourceRootProvider`].

use std::collections::BTreeSet;

use crate::{ContentDigest, assets::report::Result, resource_gc::ResourceGcGuard};

/// A stable set of committed content digests.
///
/// The private guard keeps package/lease publication excluded for this value's
/// lifetime. A custom provider may construct a static set with
/// [`CommittedResourceRoots::from_digests`] when its backing store provides an
/// equivalent external lease.
#[derive(Debug)]
pub struct CommittedResourceRoots {
    digests: BTreeSet<ContentDigest>,
    _guard: Option<ResourceGcGuard>,
}

impl CommittedResourceRoots {
    /// Build roots supplied by an immutable or externally leased service.
    pub fn from_digests(digests: impl IntoIterator<Item = ContentDigest>) -> Self {
        Self {
            digests: digests.into_iter().collect(),
            _guard: None,
        }
    }

    pub fn contains(&self, digest: &ContentDigest) -> bool {
        self.digests.contains(digest)
    }

    pub fn len(&self) -> usize {
        self.digests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.digests.is_empty()
    }

    pub(crate) fn from_authoritative_store(
        digests: BTreeSet<ContentDigest>,
        guard: ResourceGcGuard,
    ) -> Self {
        Self {
            digests,
            _guard: Some(guard),
        }
    }
}

/// Supplies a root snapshot whose lifetime covers one destructive operation.
///
/// Implementations backed by mutable root records must keep root publication
/// excluded until the returned value is dropped.
pub trait CommittedResourceRootProvider: Send + Sync {
    fn acquire(&self) -> Result<CommittedResourceRoots>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_deduplicate_typed_content_digests() {
        let first = ContentDigest::of_bytes(b"first");
        let second = ContentDigest::of_bytes(b"second");
        let roots = CommittedResourceRoots::from_digests([first, first, second]);
        assert_eq!(roots.len(), 2);
        assert!(roots.contains(&first));
        assert!(roots.contains(&second));
    }
}
