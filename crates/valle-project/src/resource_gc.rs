//! Authoritative content-GC roots for published fixed packages and active jobs.
//!
//! The root store deliberately has no database or secondary index. Immutable
//! canonical fixed-package manifests under `packages/` are persistent roots;
//! canonical lease records whose file lock is held are transient roots.

use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use valle_timeline::internal::fixed_package::{
    FixedPackageManifest, validate_fixed_package_manifest,
};

use crate::{
    ContentDigest,
    assets::{
        CommittedResourceRootProvider, CommittedResourceRoots,
        home::Home,
        report::{AssetsError, Result as AssetsResult},
    },
};

const LOCK_FILE: &str = ".resource-gc.lock";
const ROOTS_DIR: &str = "resource-roots";
const FORMAT_FILE: &str = "FORMAT";
const PACKAGES_DIR: &str = "packages";
const LEASES_DIR: &str = "leases";
const ROOTS_FORMAT: &[u8] = b"valle.resource-roots@1\n";
const LEASE_FORMAT: &str = "valle.resource-lease@1";

#[derive(Debug)]
pub(crate) struct ResourceGcGuard {
    _file: File,
}

pub(crate) fn acquire(store_root: &Path) -> std::io::Result<ResourceGcGuard> {
    std::fs::create_dir_all(store_root)?;
    let root_metadata = std::fs::symlink_metadata(store_root)?;
    if !root_metadata.file_type().is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "resource-GC lock parent is not a real directory: {}",
                store_root.display()
            ),
        ));
    }
    let lock_path = store_root.join(LOCK_FILE);
    match std::fs::symlink_metadata(&lock_path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "resource-GC lock is not a regular file: {}",
                    lock_path.display()
                ),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    if !std::fs::symlink_metadata(&lock_path)?.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "resource-GC lock is not a regular file: {}",
                lock_path.display()
            ),
        ));
    }
    file.lock_exclusive()?;
    Ok(ResourceGcGuard { _file: file })
}

/// Resource-root store associated with one Valle home/CAS root.
#[derive(Debug, Clone)]
pub struct ResourceRootStore {
    store_root: PathBuf,
}

impl ResourceRootStore {
    pub fn at(store_root: impl Into<PathBuf>) -> Self {
        Self {
            store_root: store_root.into(),
        }
    }

    pub fn roots_dir(&self) -> PathBuf {
        self.store_root.join(ROOTS_DIR)
    }

    /// Low-level host adapter for publishing one already Engine-verified
    /// canonical `FixedPackageManifest` as an immutable persistent root.
    ///
    /// This crate cannot construct or name Engine's opaque
    /// `VerifiedFixedPackage` without reversing the dependency direction. A
    /// host that depends on both crates must expose the safe entry point and
    /// pass only `VerifiedFixedPackage::canonical_manifest_bytes()` here. This
    /// method validates storage shape, but deliberately cannot re-prove member
    /// bytes or the complete external blob set from the manifest alone.
    #[doc(hidden)]
    pub fn publish_package_pin_unchecked(
        &self,
        manifest_json: &str,
    ) -> AssetsResult<PublishedPackagePin> {
        let manifest = decode_package_manifest(manifest_json.as_bytes())?;
        let package_digest = digest_bytes(manifest_json.as_bytes());
        let package_hex = package_digest.as_hex();

        // Global order is always assets write lock -> resource-GC lock.
        let home = Home::at(&self.store_root);
        let _assets = crate::assets::lock::acquire(&home)?;
        process_test_barrier("resource-root-package-pin-after-assets-lock")?;
        let _gc = acquire(&self.store_root).map_err(AssetsError::from)?;
        self.initialize_or_validate()?;
        ensure_cas_blobs_exist(&home, &manifest.resource_digests)?;

        let packages = self.roots_dir().join(PACKAGES_DIR);
        let path = packages.join(format!("{package_hex}.json"));
        publish_immutable_file(
            &packages,
            &path,
            manifest_json.as_bytes(),
            "resource-root-package-pin",
        )?;
        Ok(PublishedPackagePin {
            package_digest,
            path,
        })
    }

    /// Acquire transient roots for an in-flight render. The returned value
    /// owns the OS advisory lock; dropping it releases the roots and leaves a
    /// stale record that the next root scan removes safely.
    pub fn acquire_active_lease(
        &self,
        resource_digests: impl IntoIterator<Item = String>,
    ) -> AssetsResult<ActiveResourceLease> {
        let resource_digests = canonical_digest_set(resource_digests)?;
        let home = Home::at(&self.store_root);
        let _assets = crate::assets::lock::acquire(&home)?;
        let _gc = acquire(&self.store_root).map_err(AssetsError::from)?;
        self.initialize_or_validate()?;
        ensure_cas_blobs_exist(&home, &resource_digests)?;

        self.create_active_lease(resource_digests)
    }

    /// Copy explicit host resources into CAS and acquire their lease in one
    /// assets-write / GC critical section. No unprotected gap exists between
    /// publication and retention, and readers use the copied immutable files.
    pub fn import_and_lease(
        &self,
        resources: impl IntoIterator<Item = (ContentDigest, PathBuf)>,
    ) -> AssetsResult<ActiveResourceLease> {
        let home = Home::at(&self.store_root);
        let _assets = crate::assets::lock::acquire(&home)?;
        let _gc = acquire(&self.store_root).map_err(AssetsError::from)?;
        self.initialize_or_validate()?;
        let mut digests = BTreeSet::new();
        for (digest, source) in resources {
            let destination = home.object_path(&digest.as_hex(), None);
            std::fs::create_dir_all(home.objects_dir())?;
            require_real_directory(&home.objects_dir(), "asset CAS objects root")?;
            let shard = destination.parent().expect("CAS path has a shard");
            std::fs::create_dir_all(shard)?;
            require_real_directory(shard, "asset CAS shard")?;
            match std::fs::symlink_metadata(&destination) {
                Ok(metadata) => {
                    if !metadata.file_type().is_file() || digest_file(&destination)? != digest {
                        return Err(AssetsError::io(format!("invalid CAS blob {digest}")));
                    }
                    // An explicit source must match even when an identical CAS entry exists.
                    if digest_file(&source)? != digest {
                        return Err(AssetsError::io(format!(
                            "resource digest mismatch: {}",
                            source.display()
                        )));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    crate::assets::fsutil::stage_atomic(
                        &destination,
                        "render-resource",
                        |staged| {
                            let mut input = File::open(&source)?;
                            let mut output = OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .open(staged)?;
                            std::io::copy(&mut input, &mut output)?;
                            drop(output);
                            if digest_file(staged)? != digest {
                                return Err(AssetsError::io(format!(
                                    "resource digest mismatch: {}",
                                    source.display()
                                )));
                            }
                            Ok(())
                        },
                    )?;
                }
                Err(error) => return Err(error.into()),
            }
            digests.insert(digest);
        }
        self.create_active_lease(digests.into_iter().collect())
    }

    /// Remove a persistent package root under the same lock used by GC scans.
    /// Active render leases continue to protect their resources.
    pub fn remove_package_pin(&self, package_digest: ContentDigest) -> AssetsResult<()> {
        let _gc = acquire(&self.store_root).map_err(AssetsError::from)?;
        self.initialize_or_validate()?;
        let packages = self.roots_dir().join(PACKAGES_DIR);
        let path = packages.join(format!("{}.json", package_digest.as_hex()));
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                let bytes = std::fs::read(&path)?;
                if digest_bytes(&bytes) != package_digest {
                    return Err(AssetsError::io("package pin digest mismatch"));
                }
                decode_package_manifest(&bytes)?;
                std::fs::remove_file(path)?;
                sync_directory(&packages)?;
            }
            Ok(_) => return Err(AssetsError::io("package pin is not a regular file")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    // Caller holds assets-write and GC locks and has verified/copied every blob.
    fn create_active_lease(
        &self,
        resource_digests: Vec<ContentDigest>,
    ) -> AssetsResult<ActiveResourceLease> {
        let leases = self.roots_dir().join(LEASES_DIR);
        let mut temporary = tempfile::Builder::new()
            .prefix(".lease-")
            .tempfile_in(&leases)
            .map_err(AssetsError::from)?;
        let random = temporary
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(".lease-"))
            .filter(|name| !name.is_empty())
            .ok_or_else(|| AssetsError::io("temporary lease has no safe identity"))?;
        let lease_id = format!("{}-{random}", std::process::id());
        let record = ActiveLeaseRecord {
            format: LEASE_FORMAT.to_owned(),
            lease_id: lease_id.clone(),
            resource_digests: resource_digests.clone(),
        };
        let bytes = canonical_json(&record)?;
        temporary
            .as_file_mut()
            .write_all(&bytes)
            .map_err(AssetsError::from)?;
        crate::assets::crash::maybe_crash("resource-root-active-lease-after-write");
        temporary.as_file_mut().flush().map_err(AssetsError::from)?;
        temporary.as_file().sync_all().map_err(AssetsError::from)?;
        crate::assets::crash::maybe_crash("resource-root-active-lease-after-file-fsync");
        temporary
            .as_file()
            .lock_exclusive()
            .map_err(AssetsError::from)?;
        let path = leases.join(format!("{lease_id}.json"));
        let file = temporary.persist_noclobber(&path).map_err(|error| {
            AssetsError::io(format!("publishing active lease: {}", error.error))
        })?;
        crate::assets::crash::maybe_crash("resource-root-active-lease-after-rename");
        sync_directory(&leases)?;
        crate::assets::crash::maybe_crash("resource-root-active-lease-after-directory-fsync");
        Ok(ActiveResourceLease {
            lease_id,
            resource_digests,
            path,
            store_root: self.store_root.clone(),
            file,
        })
    }

    fn initialize_or_validate(&self) -> AssetsResult<()> {
        let root = self.roots_dir();
        match std::fs::symlink_metadata(&root) {
            Ok(metadata) if !metadata.file_type().is_dir() => {
                return Err(AssetsError::io(format!(
                    "resource root store is not a directory: {}",
                    root.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&root)?;
                sync_directory(&self.store_root)?;
            }
            Err(error) => return Err(AssetsError::from(error)),
        }
        let format_path = root.join(FORMAT_FILE);
        let format_exists = match std::fs::symlink_metadata(&format_path) {
            Ok(metadata) if metadata.file_type().is_file() => true,
            Ok(_) => {
                return Err(AssetsError::io(
                    "resource root FORMAT is not a regular file",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(AssetsError::from(error)),
        };

        if !format_exists {
            sweep_staging_files(&root)?;
            // FORMAT is the initialization commit point. A crash before it may
            // leave only empty required directories, which are safe to finish.
            // Any authoritative record without FORMAT is ambiguous and must
            // fail closed instead of being reclassified as an empty root set.
            for entry in std::fs::read_dir(&root)? {
                let entry = entry?;
                let name = entry.file_name();
                if name != PACKAGES_DIR && name != LEASES_DIR {
                    return Err(AssetsError::io(
                        "non-empty resource root store is missing exact FORMAT",
                    ));
                }
                reject_non_directory(&entry, "resource root member")?;
                if std::fs::read_dir(entry.path())?.next().is_some() {
                    return Err(AssetsError::io(
                        "non-empty resource root store is missing exact FORMAT",
                    ));
                }
            }

            for directory in [root.join(PACKAGES_DIR), root.join(LEASES_DIR)] {
                match std::fs::create_dir(&directory) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        require_real_directory(&directory, "resource root member")?;
                    }
                    Err(error) => return Err(AssetsError::from(error)),
                }
                sync_directory(&directory)?;
            }
            sync_directory(&root)?;
            publish_immutable_file(&root, &format_path, ROOTS_FORMAT, "resource-root-format")?;
        }
        let format = std::fs::read(&format_path)?;
        if format != ROOTS_FORMAT {
            return Err(AssetsError::io("unsupported resource root store FORMAT"));
        }
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            let name = entry.file_name();
            if name != FORMAT_FILE && name != PACKAGES_DIR && name != LEASES_DIR {
                return Err(AssetsError::io(format!(
                    "unknown resource root store entry {}",
                    entry.path().display()
                )));
            }
        }
        for directory in [root.join(PACKAGES_DIR), root.join(LEASES_DIR)] {
            match std::fs::symlink_metadata(&directory) {
                Ok(_) => require_real_directory(&directory, "resource root member")?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(AssetsError::io(format!(
                        "initialized resource root store is missing required directory: {}",
                        directory.display()
                    )));
                }
                Err(error) => return Err(AssetsError::from(error)),
            }
        }
        sync_directory(&root)
    }

    fn scan(&self) -> AssetsResult<BTreeSet<ContentDigest>> {
        self.initialize_or_validate()?;
        let mut roots = BTreeSet::new();
        self.scan_packages(&mut roots)?;
        self.scan_leases(&mut roots)?;
        Ok(roots)
    }

    fn scan_packages(&self, roots: &mut BTreeSet<ContentDigest>) -> AssetsResult<()> {
        let packages = self.roots_dir().join(PACKAGES_DIR);
        for entry in std::fs::read_dir(&packages)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with(".root-") {
                reject_non_regular(&entry, "package staging file")?;
                std::fs::remove_file(path)?;
                continue;
            }
            reject_non_regular(&entry, "package pin")?;
            let file_name = entry
                .file_name()
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| AssetsError::io("package pin name is not UTF-8"))?;
            let expected_digest = file_name
                .strip_suffix(".json")
                .and_then(|hex| ContentDigest::from_hex(hex).ok())
                .ok_or_else(|| AssetsError::io(format!("invalid package pin name {file_name}")))?;
            let bytes = std::fs::read(&path)?;
            let actual_digest = digest_bytes(&bytes);
            if actual_digest != expected_digest {
                return Err(AssetsError::io(format!(
                    "package pin digest mismatch: {}",
                    path.display()
                )));
            }
            let manifest = decode_package_manifest(&bytes)?;
            extend_roots(roots, &manifest.resource_digests);
        }
        Ok(())
    }

    fn scan_leases(&self, roots: &mut BTreeSet<ContentDigest>) -> AssetsResult<()> {
        let leases = self.roots_dir().join(LEASES_DIR);
        for entry in std::fs::read_dir(&leases)? {
            let entry = entry?;
            let path = entry.path();
            let file_name = entry
                .file_name()
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| AssetsError::io("lease name is not UTF-8"))?;
            if file_name.starts_with(".lease-") {
                reject_non_regular(&entry, "lease staging file")?;
                std::fs::remove_file(&path)?;
                continue;
            }
            reject_non_regular(&entry, "active lease")?;
            let lease_id = file_name
                .strip_suffix(".json")
                .filter(|value| valid_lease_id(value))
                .ok_or_else(|| AssetsError::io(format!("invalid active lease name {file_name}")))?;
            let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
            match file.try_lock_exclusive() {
                Ok(()) => {
                    // No live process owns this inode. It cannot protect a job
                    // and is removed while the resource-GC lock excludes creators.
                    drop(file);
                    std::fs::remove_file(&path)?;
                }
                Err(error) if lock_is_contended(&error) => {
                    file.seek(SeekFrom::Start(0))?;
                    let mut bytes = Vec::new();
                    file.read_to_end(&mut bytes)?;
                    let record = decode_active_lease(&bytes)?;
                    if record.lease_id != lease_id {
                        return Err(AssetsError::io(format!(
                            "active lease identity mismatch: {}",
                            path.display()
                        )));
                    }
                    extend_roots(roots, &record.resource_digests);
                }
                Err(error) => return Err(AssetsError::from(error)),
            }
        }
        sync_directory(&leases)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedPackagePin {
    package_digest: ContentDigest,
    path: PathBuf,
}

impl PublishedPackagePin {
    pub const fn package_digest(&self) -> &ContentDigest {
        &self.package_digest
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Held active-lease inode. Drop releases the OS lock; stale-file cleanup is
/// intentionally delegated to the next authoritative scan.
#[derive(Debug)]
pub struct ActiveResourceLease {
    lease_id: String,
    resource_digests: Vec<ContentDigest>,
    path: PathBuf,
    store_root: PathBuf,
    file: File,
}

impl ActiveResourceLease {
    pub fn lease_id(&self) -> &str {
        &self.lease_id
    }

    pub fn resource_digests(&self) -> &[ContentDigest] {
        &self.resource_digests
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Explicitly remove the lease record. Dropping without release remains
    /// crash-safe: the next scan observes the unlocked inode as stale.
    pub fn release(self) -> AssetsResult<()> {
        let ActiveResourceLease {
            path,
            store_root,
            file,
            ..
        } = self;
        let _gc = acquire(&store_root).map_err(AssetsError::from)?;
        FileExt::unlock(&file).map_err(AssetsError::from)?;
        std::fs::remove_file(&path)?;
        let parent = path
            .parent()
            .ok_or_else(|| AssetsError::io("active lease has no parent"))?;
        sync_directory(parent)
    }
}

/// Default root provider: only published packages and live leases count.
#[derive(Debug, Clone)]
pub(crate) struct PackageResourceRootProvider {
    store: ResourceRootStore,
}

impl PackageResourceRootProvider {
    pub(crate) fn at(store_root: impl Into<PathBuf>) -> Self {
        Self {
            store: ResourceRootStore::at(store_root),
        }
    }
}

impl CommittedResourceRootProvider for PackageResourceRootProvider {
    fn acquire(&self) -> AssetsResult<CommittedResourceRoots> {
        let guard = acquire(&self.store.store_root).map_err(AssetsError::from)?;
        let digests = self.store.scan()?;
        Ok(CommittedResourceRoots::from_authoritative_store(
            digests, guard,
        ))
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActiveLeaseRecord {
    format: String,
    lease_id: String,
    resource_digests: Vec<ContentDigest>,
}

fn decode_package_manifest(bytes: &[u8]) -> AssetsResult<FixedPackageManifest> {
    let manifest: FixedPackageManifest = serde_json::from_slice(bytes)
        .map_err(|error| AssetsError::io(format!("invalid fixed package pin: {error}")))?;
    validate_fixed_package_manifest(&manifest).map_err(AssetsError::io)?;
    require_canonical_json(bytes, &manifest, "fixed package pin")?;
    Ok(manifest)
}

fn decode_active_lease(bytes: &[u8]) -> AssetsResult<ActiveLeaseRecord> {
    let record: ActiveLeaseRecord = serde_json::from_slice(bytes)
        .map_err(|error| AssetsError::io(format!("invalid active resource lease: {error}")))?;
    if record.format != LEASE_FORMAT || !valid_lease_id(&record.lease_id) {
        return Err(AssetsError::io(
            "active resource lease has invalid identity",
        ));
    }
    require_canonical_json(bytes, &record, "active resource lease")?;
    require_sorted_unique_digests(&record.resource_digests, "active resource lease")?;
    Ok(record)
}

fn canonical_digest_set(
    values: impl IntoIterator<Item = String>,
) -> AssetsResult<Vec<ContentDigest>> {
    let mut values = values
        .into_iter()
        .map(|value| {
            ContentDigest::parse(&value)
                .map_err(|_| AssetsError::io(format!("invalid resource digest {value:?}")))
        })
        .collect::<AssetsResult<Vec<_>>>()?;
    values.sort();
    values.dedup();
    Ok(values)
}

fn require_sorted_unique_digests(values: &[ContentDigest], subject: &str) -> AssetsResult<()> {
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(AssetsError::io(format!(
            "{subject} resourceDigests must be strictly sorted and unique"
        )));
    }
    Ok(())
}

fn require_canonical_json<T: Serialize>(
    actual: &[u8],
    value: &T,
    subject: &str,
) -> AssetsResult<()> {
    if canonical_json(value)? != actual {
        return Err(AssetsError::io(format!(
            "{subject} must use canonical JCS bytes"
        )));
    }
    Ok(())
}

fn canonical_json<T: Serialize>(value: &T) -> AssetsResult<Vec<u8>> {
    serde_jcs::to_vec(value)
        .map_err(|error| AssetsError::io(format!("canonical JSON encoding failed: {error}")))
}

fn digest_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::of_bytes(bytes)
}

fn valid_lease_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn extend_roots(roots: &mut BTreeSet<ContentDigest>, digests: &[ContentDigest]) {
    roots.extend(digests.iter().copied());
}

fn ensure_cas_blobs_exist(home: &Home, digests: &[ContentDigest]) -> AssetsResult<()> {
    if digests.is_empty() {
        return Ok(());
    }
    require_real_directory(&home.objects_dir(), "asset CAS objects root")?;
    for digest in digests {
        let hex = digest.as_hex();
        let (shard, tail) = crate::assets::fanout(&hex);
        let directory = home.objects_dir().join(shard);
        let found = match std::fs::symlink_metadata(&directory) {
            Ok(_) => {
                require_real_directory(&directory, "asset CAS shard")?;
                let mut found = false;
                for entry in std::fs::read_dir(&directory)? {
                    let entry = entry?;
                    let name = entry.file_name();
                    let Some(name) = name.to_str() else {
                        continue;
                    };
                    if name != tail
                        && !name
                            .strip_prefix(tail)
                            .is_some_and(|suffix| suffix.starts_with('.'))
                    {
                        continue;
                    }
                    reject_non_regular(&entry, "asset CAS digest candidate")?;
                    if digest_file(&entry.path())? == *digest {
                        found = true;
                        break;
                    }
                }
                found
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(AssetsError::from(error)),
        };
        if !found {
            return Err(AssetsError::io(format!(
                "cannot publish resource root before CAS blob {digest} exists"
            )));
        }
    }
    Ok(())
}

fn publish_immutable_file(
    directory: &Path,
    path: &Path,
    bytes: &[u8],
    crash_label: &str,
) -> AssetsResult<()> {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if !metadata.file_type().is_file() {
            return Err(AssetsError::io(format!(
                "immutable resource root entry is not a regular file: {}",
                path.display()
            )));
        }
        let existing = std::fs::read(path)?;
        if existing == bytes {
            return sync_directory(directory);
        }
        return Err(AssetsError::io(format!(
            "immutable resource root entry already differs: {}",
            path.display()
        )));
    }
    let mut temporary = tempfile::Builder::new()
        .prefix(".root-")
        .tempfile_in(directory)
        .map_err(AssetsError::from)?;
    temporary.write_all(bytes).map_err(AssetsError::from)?;
    crate::assets::crash::maybe_crash(&format!("{crash_label}-after-write"));
    temporary.flush().map_err(AssetsError::from)?;
    temporary.as_file().sync_all().map_err(AssetsError::from)?;
    crate::assets::crash::maybe_crash(&format!("{crash_label}-after-file-fsync"));
    match temporary.persist_noclobber(path) {
        Ok(_) => {
            crate::assets::crash::maybe_crash(&format!("{crash_label}-after-rename"));
        }
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(path)?;
            if !metadata.file_type().is_file() {
                return Err(AssetsError::io(format!(
                    "immutable resource root entry raced with a non-regular file: {}",
                    path.display()
                )));
            }
            if std::fs::read(path)? != bytes {
                return Err(AssetsError::io(format!(
                    "immutable resource root entry raced with different bytes: {}",
                    path.display()
                )));
            }
        }
        Err(error) => return Err(AssetsError::from(error.error)),
    }
    sync_directory(directory)?;
    crate::assets::crash::maybe_crash(&format!("{crash_label}-after-directory-fsync"));
    Ok(())
}

/// Deterministic process barrier used only by crash/locking integration tests.
///
/// Like the existing crash injector, this is inert unless its exact opt-in
/// environment variable is present. Keeping the barrier immediately after the
/// assets lock lets a parent process prove the cross-process lock order without
/// timing guesses or sleeps.
fn process_test_barrier(label: &str) -> AssetsResult<()> {
    if std::env::var_os("VALLE_RESOURCE_ROOT_BARRIER_AT")
        .is_none_or(|value| value.to_string_lossy() != label)
    {
        return Ok(());
    }

    println!("RESOURCE_ROOT_BARRIER_READY {label}");
    std::io::stdout().flush().map_err(AssetsError::from)?;
    let mut release = [0_u8; 1];
    std::io::stdin()
        .read_exact(&mut release)
        .map_err(AssetsError::from)
}

fn sweep_staging_files(directory: &Path) -> AssetsResult<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with(".root-") {
            continue;
        }
        reject_non_regular(&entry, "resource root staging file")?;
        std::fs::remove_file(entry.path())?;
    }
    sync_directory(directory)
}

fn reject_non_regular(entry: &std::fs::DirEntry, subject: &str) -> AssetsResult<()> {
    let metadata = std::fs::symlink_metadata(entry.path())?;
    if !metadata.file_type().is_file() {
        return Err(AssetsError::io(format!(
            "{subject} must be a regular file: {}",
            entry.path().display()
        )));
    }
    Ok(())
}

fn reject_non_directory(entry: &std::fs::DirEntry, subject: &str) -> AssetsResult<()> {
    require_real_directory(&entry.path(), subject)
}

fn digest_file(path: &Path) -> AssetsResult<ContentDigest> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(ContentDigest::from_bytes(hasher.finalize().into()))
}

fn lock_is_contended(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

fn require_real_directory(path: &Path, subject: &str) -> AssetsResult<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() {
        return Err(AssetsError::io(format!(
            "{subject} must be a real directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn sync_directory(path: &Path) -> AssetsResult<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        // Required by CreateFileW when opening a directory handle. Flushing
        // that handle makes directory-entry publication durable just as
        // fsync(2) does on Unix.
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?
            .sync_all()?;
    }
    #[cfg(not(any(unix, windows)))]
    return Err(AssetsError::io(
        "durable resource-root publication is unsupported on this platform",
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::lock_is_contended;

    #[test]
    fn platform_lock_contention_is_recognized() {
        assert!(lock_is_contended(&fs2::lock_contended_error()));
    }
}
