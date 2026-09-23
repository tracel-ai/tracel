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
    /// Number of ready versions.
    pub version_count: u64,
    /// Highest ready version number, when supplied by the backend.
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

/// Where a model version is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionState {
    /// Numbered, and still waiting for its files.
    Pending,
    /// Complete: only a ready version is listed, downloaded, `latest` or an alias target.
    Ready,
    /// Never completed; the version's failure reason says why.
    Failed,
    /// Its files are gone, but the version stays readable by number.
    Deleted,
}

/// A version of a model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelVersion {
    /// Opaque version identity.
    pub id: VersionId,
    /// Version number for display and ordering, when the backend numbers versions.
    pub version: Option<u32>,
    /// Where the version is in its life.
    pub state: VersionState,
    /// Why a failed version failed, as the backend names the reason.
    pub failure_reason: Option<String>,
    /// Aggregate version size in bytes.
    pub size_bytes: u64,
    /// The version digest: a SHA-256 over the manifest's sorted `rel_path:checksum` lines, which
    /// metadata never enters.
    pub checksum: String,
    /// Aliases pointing at this version.
    pub aliases: Vec<String>,
    /// Display name or handle of whoever published the version, when known.
    pub published_by: Option<String>,
    /// When the version was published, when the backend supplies an instant.
    pub created_at: Option<SystemTime>,
    /// Files published in this version.
    pub manifest: VersionManifest,
    /// Opaque application metadata, with absent metadata represented by JSON `null`.
    pub metadata: serde_json::Value,
    /// When the version was deleted, when the backend supplies an instant.
    pub deleted_at: Option<SystemTime>,
}

/// Selects which version of a model to use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionSpec {
    /// This exact version.
    Exact(VersionId),
    /// Whichever version is newest when the call is made.
    Latest,
}

impl fmt::Display for VersionSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(id) => write!(formatter, "version {id}"),
            Self::Latest => formatter.write_str("latest version"),
        }
    }
}

impl From<VersionId> for VersionSpec {
    fn from(id: VersionId) -> Self {
        Self::Exact(id)
    }
}
