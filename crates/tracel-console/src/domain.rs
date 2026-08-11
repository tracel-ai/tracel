use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ConsoleError;

/// A console namespace and the kind of owner it represents.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Namespace {
    /// Namespace slug used in console routes.
    pub name: String,
    /// Kind of account that owns the namespace.
    pub kind: NamespaceKind,
}

impl Namespace {
    /// Creates a user namespace.
    pub fn user(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: NamespaceKind::User,
        }
    }

    /// Creates an organization namespace.
    pub fn organization(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: NamespaceKind::Organization,
        }
    }
}

impl AsRef<Namespace> for Namespace {
    fn as_ref(&self) -> &Namespace {
        self
    }
}

/// Kind of account that owns a console namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamespaceKind {
    /// A person's namespace.
    User,
    /// An organization's namespace.
    Organization,
}

/// The user associated with a live console session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// Console-local user identifier.
    pub id: i32,
    /// User's display and login name.
    pub username: String,
    /// User's email address.
    pub email: String,
    /// Namespace owned by the user.
    pub namespace: Namespace,
}

/// An organization visible to the current session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organization {
    /// Organization display name.
    pub name: String,
    /// Organization namespace used in project routes.
    pub namespace: Namespace,
}

/// Visibility applied to a console project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// Only explicitly authorized users can see the project.
    #[default]
    Private,
    /// Anonymous callers can see the project.
    Public,
}

/// A project in a user or organization namespace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Project name within its namespace.
    pub name: String,
    /// Namespace that owns the project.
    pub namespace: Namespace,
    /// Project description.
    pub description: String,
    /// Username that created the project.
    pub created_by: String,
    /// Project access level.
    pub visibility: Visibility,
}

/// A page returned by a console listing endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    /// Values in this page.
    pub items: Vec<T>,
    /// Total number of matching values reported by the console.
    pub total: usize,
}

/// A compact user identity attached to model records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSummary {
    /// Console-local user identifier.
    pub id: i32,
    /// User's display and login name.
    pub username: String,
    /// Namespace owned by the user.
    pub namespace: String,
}

/// A model registered inside a project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    /// Opaque model identifier.
    pub id: String,
    /// Console-local project identifier.
    pub project_id: i32,
    /// Model name within the project.
    pub name: String,
    /// Optional model description.
    pub description: Option<String>,
    /// User that created the model.
    pub created_by: UserSummary,
    /// Creation timestamp as returned by the console.
    pub created_at: String,
    /// Number of published versions.
    pub version_count: u64,
    /// Highest version number for display and ordering, when a version exists.
    pub latest_version: Option<u32>,
}

/// An opaque identifier used to address a model version.
#[derive(Clone)]
pub struct VersionId {
    value: String,
    route_version: Option<u32>,
}

impl VersionId {
    /// Restores an opaque identifier previously obtained from the console.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            route_version: None,
        }
    }

    /// Returns the stable value suitable for persistence or equality checks.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub(crate) fn from_wire(value: String, route_version: u32) -> Self {
        Self {
            value,
            route_version: Some(route_version),
        }
    }

    pub(crate) fn route_version(&self) -> Option<u32> {
        self.route_version
    }
}

impl fmt::Debug for VersionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("VersionId")
            .field(&self.value)
            .finish()
    }
}

impl fmt::Display for VersionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(formatter)
    }
}

impl PartialEq for VersionId {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Eq for VersionId {}

impl Hash for VersionId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl Serialize for VersionId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.value)
    }
}

impl<'de> Deserialize<'de> for VersionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self::new)
    }
}

/// Experiment provenance attached to a model version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentSource {
    /// Console-local experiment identifier.
    pub id: i32,
    /// Experiment number shown in the console.
    pub experiment_num: i32,
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

/// A published version of a registered model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelVersion {
    /// Opaque identity used to create a [`crate::VersionHandle`].
    pub id: VersionId,
    /// Optional experiment that produced this version.
    pub experiment: Option<ExperimentSource>,
    /// Version number intended only for display and ordering.
    pub version: u32,
    /// Aggregate version size in bytes.
    pub size_bytes: u64,
    /// Aggregate version checksum.
    pub checksum: String,
    /// User that created the version.
    pub created_by: UserSummary,
    /// Creation timestamp as returned by the console.
    pub created_at: String,
    /// Files published in this version.
    pub manifest: VersionManifest,
    /// Opaque application metadata, with absent metadata represented by JSON `null`.
    pub metadata: serde_json::Value,
}

impl TryFrom<tracel_client::response::ProjectResponse> for Project {
    type Error = ConsoleError;

    fn try_from(value: tracel_client::response::ProjectResponse) -> Result<Self, Self::Error> {
        let kind = match value.namespace_type.as_str() {
            "user" => NamespaceKind::User,
            "organization" => NamespaceKind::Organization,
            other => {
                return Err(ConsoleError::InvalidResponse(format!(
                    "unknown namespace type `{other}`"
                )));
            }
        };

        Ok(Self {
            name: value.project_name,
            namespace: Namespace {
                name: value.namespace_name,
                kind,
            },
            description: value.description,
            created_by: value.created_by,
            visibility: match value.visibility {
                tracel_client::request::Visibility::Private => Visibility::Private,
                tracel_client::request::Visibility::Public => Visibility::Public,
            },
        })
    }
}

impl From<tracel_client::response::CreatedByUserResponse> for UserSummary {
    fn from(value: tracel_client::response::CreatedByUserResponse) -> Self {
        Self {
            id: value.id,
            username: value.username,
            namespace: value.namespace,
        }
    }
}

impl From<tracel_client::response::ModelResponse> for Model {
    fn from(value: tracel_client::response::ModelResponse) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            name: value.name,
            description: value.description,
            created_by: value.created_by.into(),
            created_at: value.created_at,
            version_count: value.version_count,
            latest_version: value.latest_version,
        }
    }
}

impl TryFrom<tracel_client::response::ModelVersionResponse> for ModelVersion {
    type Error = ConsoleError;

    fn try_from(value: tracel_client::response::ModelVersionResponse) -> Result<Self, Self::Error> {
        let manifest = serde_json::from_value(value.manifest)
            .map_err(|error| ConsoleError::InvalidResponse(error.to_string()))?;

        Ok(Self {
            id: VersionId::from_wire(value.id, value.version),
            experiment: value.experiment.map(|source| ExperimentSource {
                id: source.id,
                experiment_num: source.experiment_num,
            }),
            version: value.version,
            size_bytes: value.size,
            checksum: value.checksum,
            created_by: value.created_by.into(),
            created_at: value.created_at,
            manifest,
            metadata: value.metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_project_fixture_defaults_to_private_visibility() {
        let wire: tracel_client::response::ProjectResponse = serde_json::from_str(
            r#"{
                "project_name": "vision",
                "namespace_name": "ada",
                "namespace_type": "user",
                "description": "",
                "created_by": "ada"
            }"#,
        )
        .unwrap();
        let project = Project::try_from(wire).unwrap();

        assert_eq!(project.visibility, Visibility::Private);
    }

    #[test]
    fn version_fixture_keeps_null_metadata_and_typed_manifest_files() {
        let wire: tracel_client::response::ModelVersionResponse = serde_json::from_str(
            r#"{
                "id": "0198f0a1-0000-7000-8000-000000000001",
                "experiment": null,
                "version": 2,
                "size": 2048,
                "checksum": "sha256:abc",
                "created_by": {"id": 7, "username": "ada", "namespace": "ada"},
                "created_at": "2026-03-05 18:45:43.397",
                "manifest": {"files": [{
                    "rel_path": "weights.bpk",
                    "size_bytes": 2048,
                    "checksum": "sha256:abc"
                }]}
            }"#,
        )
        .unwrap();
        let version = ModelVersion::try_from(wire).unwrap();

        assert!(version.metadata.is_null());
        assert_eq!(version.manifest.files.len(), 1);
        assert_eq!(version.manifest.files[0].rel_path, "weights.bpk");
        assert_eq!(version.id.as_str(), "0198f0a1-0000-7000-8000-000000000001");
        assert_eq!(version.id.route_version(), Some(2));
    }

    #[test]
    fn version_fixture_preserves_explicit_null_metadata() {
        let wire: tracel_client::response::ModelVersionResponse =
            serde_json::from_value(serde_json::json!({
                "id": "0198f0a1-0000-7000-8000-000000000001",
                "experiment": null,
                "version": 2,
                "size": 0,
                "checksum": "sha256:empty",
                "created_by": {"id": 7, "username": "ada", "namespace": "ada"},
                "created_at": "2026-03-05 18:45:43.397",
                "manifest": {"files": []},
                "metadata": null
            }))
            .unwrap();

        let version = ModelVersion::try_from(wire).unwrap();

        assert!(version.metadata.is_null());
    }
}
