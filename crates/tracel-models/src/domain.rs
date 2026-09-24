use std::fmt;
use std::str::FromStr;
use std::time::SystemTime;

use crate::ModelsError;

const LATEST: &str = "latest";

const MAX_ALIAS_LENGTH: usize = 64;

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
///
/// Parses from `latest` in any case, a version number with or without a leading `v`, or an alias
/// name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionSpec {
    /// This exact version, in whatever state it is.
    Exact(VersionId),
    /// Whichever ready version is newest when the call is made.
    Latest,
    /// Whichever version this alias points at when the call is made.
    Alias(String),
}

impl fmt::Display for VersionSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(id) => write!(formatter, "version {id}"),
            Self::Latest => formatter.write_str("latest version"),
            Self::Alias(alias) => write!(formatter, "version at alias '{alias}'"),
        }
    }
}

impl FromStr for VersionSpec {
    type Err = ModelsError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case(LATEST) {
            return Ok(Self::Latest);
        }
        if let Some(digits) = version_number(value) {
            let id = digits
                .parse::<u32>()
                .map_or_else(|_| digits.to_string(), |number| number.to_string());
            return Ok(Self::Exact(VersionId::new(id)));
        }
        check_alias_name(value)?;
        Ok(Self::Alias(value.to_string()))
    }
}

impl From<VersionId> for VersionSpec {
    fn from(id: VersionId) -> Self {
        Self::Exact(id)
    }
}

impl From<&VersionId> for VersionSpec {
    fn from(id: &VersionId) -> Self {
        Self::Exact(id.clone())
    }
}

pub fn check_alias_name(name: &str) -> Result<(), ModelsError> {
    let mut bytes = name.bytes();
    let well_formed = name.len() <= MAX_ALIAS_LENGTH
        && bytes
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        });
    if well_formed && name != LATEST && version_number(name).is_none() {
        Ok(())
    } else {
        Err(ModelsError::InvalidAlias(name.to_string()))
    }
}

fn version_number(value: &str) -> Option<&str> {
    let digits = value.strip_prefix('v').unwrap_or(value);
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())).then_some(digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_parses_the_way_a_registry_resolves_it() {
        let exact = |id: &str| VersionSpec::Exact(VersionId::new(id));
        let alias = |name: &str| VersionSpec::Alias(name.to_string());

        for (text, expected) in [
            ("12", exact("12")),
            ("v12", exact("12")),
            ("007", exact("7")),
            ("4294967295", exact("4294967295")),
            ("4294967296", exact("4294967296")),
            ("latest", VersionSpec::Latest),
            ("LATEST", VersionSpec::Latest),
            ("production", alias("production")),
            ("v1beta", alias("v1beta")),
            ("7up", alias("7up")),
            ("a.b-c_d", alias("a.b-c_d")),
        ] {
            assert_eq!(text.parse::<VersionSpec>().unwrap(), expected, "{text}");
        }
    }

    #[test]
    fn a_name_no_registry_can_hold_is_not_an_alias() {
        for text in ["Prod", "-x", "", "a/b", "../versions/3", &"a".repeat(65)] {
            let error = text.parse::<VersionSpec>().unwrap_err();

            assert!(
                matches!(&error, ModelsError::InvalidAlias(name) if name == text),
                "{text}: {error}"
            );
        }
    }

    #[test]
    fn latest_and_version_numbers_are_never_alias_names() {
        for name in ["latest", "7", "v7"] {
            assert!(matches!(
                check_alias_name(name),
                Err(ModelsError::InvalidAlias(_))
            ));
        }
        assert!(check_alias_name(&"a".repeat(64)).is_ok());
    }
}
