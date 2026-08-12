use std::fmt;

use serde::{Deserialize, Serialize};

/// One page returned by a model listing operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    /// Values in this page.
    pub items: Vec<T>,
    /// Total number of matching values reported by the backend.
    pub total: usize,
}

/// A model available from a model capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    /// Opaque model identifier.
    pub id: String,
    /// Model name within the capability's backend-defined scope.
    pub name: String,
    /// Optional model description.
    pub description: Option<String>,
    /// Backend-supplied display name or handle for the model's publisher.
    #[serde(default)]
    pub published_by: Option<String>,
    /// Creation timestamp as returned by the backend.
    pub created_at: String,
    /// Number of published versions.
    pub version_count: u64,
    /// Highest version number for display and ordering, when supplied by the backend.
    pub latest_version: Option<u32>,
}

/// An opaque identifier used to address a model version.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VersionId(String);

impl VersionId {
    /// Creates an identity from a value supplied by a model backend.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the stable value suitable for persistence and equality checks.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for VersionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("VersionId").field(&self.0).finish()
    }
}

impl fmt::Display for VersionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A file declared by a model version manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionFile {
    /// Relative path inside the version bundle.
    pub rel_path: String,
    /// Expected file size in bytes.
    pub size_bytes: u64,
    /// Expected SHA-256 checksum.
    pub checksum: String,
}

/// The verified file listing published with a model version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionManifest {
    /// Files contained in the version.
    pub files: Vec<VersionFile>,
}

/// A published version of a model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelVersion {
    /// Opaque version identity.
    pub id: VersionId,
    /// Version number intended only for display and ordering.
    pub version: u32,
    /// Aggregate version size in bytes.
    pub size_bytes: u64,
    /// Aggregate version checksum.
    pub checksum: String,
    /// Backend-supplied display name or handle for the version's publisher.
    #[serde(default)]
    pub published_by: Option<String>,
    /// Creation timestamp as returned by the backend.
    pub created_at: String,
    /// Files published in this version.
    pub manifest: VersionManifest,
    /// Opaque application metadata, with absent metadata represented by JSON `null`.
    pub metadata: serde_json::Value,
}
