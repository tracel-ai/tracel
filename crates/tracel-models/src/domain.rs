use std::fmt;
use std::time::SystemTime;

/// A model available from a model capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    /// Opaque model identifier.
    pub id: String,
    /// Model name within the capability's backend-defined scope.
    pub name: String,
    /// Optional model description.
    pub description: Option<String>,
    /// Display name or handle of whoever published the model, when known.
    pub published_by: Option<String>,
    /// When the model was created, when the backend supplies an instant.
    pub created_at: Option<SystemTime>,
    /// Number of published versions.
    pub version_count: u64,
    /// Highest version number for display and ordering, when supplied by the backend.
    pub latest_version: Option<u32>,
}

/// An opaque identifier used to address a model version.
#[derive(Clone, PartialEq, Eq, Hash)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionFile {
    /// Relative path inside the version bundle.
    pub rel_path: String,
    /// Expected file size in bytes.
    pub size_bytes: u64,
    /// Expected SHA-256 checksum.
    pub checksum: String,
}

/// The verified file listing published with a model version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionManifest {
    /// Files contained in the version.
    pub files: Vec<VersionFile>,
}

/// A published version of a model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelVersion {
    /// Opaque version identity.
    pub id: VersionId,
    /// Version number intended only for display and ordering.
    pub version: u32,
    /// Aggregate version size in bytes.
    pub size_bytes: u64,
    /// Aggregate version checksum.
    pub checksum: String,
    /// Display name or handle of whoever published the version, when known.
    pub published_by: Option<String>,
    /// When the version was published, when the backend supplies an instant.
    pub created_at: Option<SystemTime>,
    /// Files published in this version.
    pub manifest: VersionManifest,
    /// Opaque application metadata, with absent metadata represented by JSON `null`.
    pub metadata: serde_json::Value,
}
