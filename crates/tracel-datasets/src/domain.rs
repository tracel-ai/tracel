use std::fmt;
use std::time::SystemTime;

/// A dataset in a backend's scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dataset {
    /// Name within that scope.
    pub name: String,
    /// Description, when set.
    pub description: Option<String>,
    /// Application-defined metadata.
    pub metadata: Option<serde_json::Value>,
}

/// Opaque identity of a dataset version.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VersionId(String);

impl VersionId {
    /// Wraps a value supplied by a backend.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The underlying value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VersionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// One published version of a dataset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatasetVersion {
    /// Dataset the version belongs to.
    pub dataset: String,
    /// Opaque identity used to address the version.
    pub id: VersionId,
    /// Version number for display and ordering, when the backend numbers versions.
    pub version: Option<u32>,
    /// How many items the version holds.
    pub item_count: u64,
    /// Application-defined metadata.
    pub metadata: Option<serde_json::Value>,
    /// When the version was published, when the backend supplies it.
    pub created_at: Option<SystemTime>,
}

/// One item of a dataset version.
///
/// The example's byte format and the annotation's schema are the publisher's contract.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The example payload, as raw bytes.
    pub example: Vec<u8>,
    /// The annotation, when the item carries one.
    pub annotation: Option<serde_json::Value>,
    /// Identity of the source item, when the backend tracks it.
    pub source_item_id: Option<String>,
    /// Application-defined metadata.
    pub metadata: Option<serde_json::Value>,
}

/// Selects which version of a dataset to read.
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

/// One item offered for publication.
#[derive(Clone, Debug, PartialEq)]
pub struct NewItem {
    /// Identity of this item within the dataset.
    ///
    /// Supplying one is what lets an interrupted publication resume without duplicating the
    /// item; an item offered without one is published again if the draft is replayed.
    pub source_item_id: Option<String>,
    /// The example payload, as raw bytes.
    pub example: Vec<u8>,
    /// The annotation, when the item carries one.
    pub annotation: Option<serde_json::Value>,
    /// Application-defined metadata.
    pub metadata: Option<serde_json::Value>,
}
