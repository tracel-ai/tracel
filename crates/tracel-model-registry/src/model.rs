use serde_json::Value;

/// A model in the registry.
#[derive(Clone, Debug)]
pub struct Model {
    pub name: String,
    pub description: Option<String>,
    pub latest_version: Option<u32>,
    pub version_count: u32,
}

/// An immutable, published version of a model.
///
/// The version number is the caller-facing handle; the manifest digest is the version's
/// content identity, used for deduplication and sync.
#[derive(Clone, Debug)]
pub struct Version {
    pub number: u32,
    pub manifest: Manifest,
    /// App-defined metadata, for example a model family or precision. Stored and synced
    /// verbatim; never read by the registry.
    pub metadata: Value,
}

/// The content-addressed list of files that make up a version.
#[derive(Clone, Debug)]
pub struct Manifest {
    pub files: Vec<FileEntry>,
}

/// One file in a version, addressed by the digest of its contents.
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub digest: String,
}

/// Whether a version's artifact bytes are locally present.
///
/// Availability is **computed local state**, probed against the local store, never stored or
/// synced (two replicas legitimately disagree about the same version). Pre-sync every existing
/// version is `Present`; sync introduces `Absent` ("metadata known, artifacts not pulled").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// Every file of the version's manifest is present in the local store; `fetch` will succeed.
    Present,
    /// At least one file is missing locally; `fetch` fails with
    /// [`ArtifactsUnavailable`](crate::ModelRegistryError::ArtifactsUnavailable) until the
    /// version is pulled.
    Absent,
}

/// How a caller points at a version, in the spirit of a git revision.
///
/// Today this is a version number; the identity it resolves to is the version's content
/// digest. It is the seam for later addressing by digest or by a named tag without changing
/// the API.
#[derive(Clone, Debug)]
pub struct Revision(RevisionKind);

#[derive(Clone, Debug)]
enum RevisionKind {
    Number(u32),
    Latest,
}

impl Revision {
    /// Point at a version by its number.
    pub fn number(n: u32) -> Self {
        Self(RevisionKind::Number(n))
    }

    /// Point at the highest-numbered version.
    pub fn latest() -> Self {
        Self(RevisionKind::Latest)
    }

    /// The version number this revision resolves to, if it is a number revision.
    pub fn as_number(&self) -> Option<u32> {
        match self.0 {
            RevisionKind::Number(n) => Some(n),
            RevisionKind::Latest => None,
        }
    }

    /// Whether this revision points at the latest version.
    pub fn is_latest(&self) -> bool {
        matches!(self.0, RevisionKind::Latest)
    }
}

impl From<u32> for Revision {
    fn from(n: u32) -> Self {
        Self::number(n)
    }
}
