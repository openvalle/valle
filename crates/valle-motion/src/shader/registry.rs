use std::collections::BTreeMap;

use super::{ShaderPackage, ShaderUri};

/// Backend-neutral, content-addressed shader package environment.
///
/// Hosts own filesystem/bundle resolution. Once bytes are admitted, compiler and render paths
/// share this exact registry instead of independently parsing manifests or trusting paths.
#[derive(Debug, Clone, Default)]
pub struct ShaderRegistry {
    packages: BTreeMap<String, ShaderPackage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderRegistryError {
    pub uri: String,
    pub message: String,
}

impl core::fmt::Display for ShaderRegistryError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "shader registry `{}`: {}",
            self.uri, self.message
        )
    }
}

impl std::error::Error for ShaderRegistryError {}

impl ShaderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a package after the caller has resolved and admitted its bytes.
    /// Identical registration is idempotent; one URI resolving to different bytes is rejected.
    pub fn register(&mut self, package: ShaderPackage) -> Result<(), ShaderRegistryError> {
        let uri = package.uri().to_string();
        if let Some(existing) = self.packages.get(&uri) {
            if existing.content_hash == package.content_hash
                && existing.manifest.abi_digest == package.manifest.abi_digest
            {
                return Ok(());
            }
            return Err(ShaderRegistryError {
                uri,
                message: format!(
                    "URI is already pinned to content {} / ABI {}, not content {} / ABI {}",
                    existing.content_hash,
                    existing.manifest.abi_digest,
                    package.content_hash,
                    package.manifest.abi_digest
                ),
            });
        }
        self.packages.insert(uri, package);
        Ok(())
    }

    pub fn resolve(&self, uri: &ShaderUri) -> Result<&ShaderPackage, ShaderRegistryError> {
        let key = uri.to_string();
        self.packages.get(&key).ok_or_else(|| ShaderRegistryError {
            uri: key,
            message: "package is not registered; resolve its locked manifest and source first"
                .into(),
        })
    }

    pub fn get(&self, uri: &str) -> Option<&ShaderPackage> {
        self.packages.get(uri)
    }

    pub fn len(&self) -> usize {
        self.packages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    pub fn packages(&self) -> impl Iterator<Item = &ShaderPackage> {
        self.packages.values()
    }
}
