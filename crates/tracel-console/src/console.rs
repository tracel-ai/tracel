use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracel_artifact::{FileTransferClient, ReqwestTransferClient};
use tracel_client::ClientError;
use tracel_models::{
    ExperimentSource, Model, ModelOps, ModelVersion, Models, ModelsError, Page, VersionFile,
    VersionFileSource, VersionId, VersionManifest,
};
use url::Url;

use crate::{
    ConsoleError, Namespace, NamespaceKind, Organization, Project, User, normalize_base_url,
};

/// A session identifier issued by a Tracel console.
///
/// Debug formatting is redacted so ordinary diagnostic output does not disclose credentials.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionToken(String);

impl SessionToken {
    /// Wraps a token obtained from authentication or persistent storage.
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// Exposes the token when a caller explicitly needs to persist it.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    /// Consumes the wrapper and returns the token.
    pub fn into_secret(self) -> String {
        self.0
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionToken([REDACTED])")
    }
}

/// Authentication state used by a [`Console`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    /// Make only requests available to anonymous callers.
    Anonymous,
    /// Send the supplied session token as the console's `id` cookie.
    Session(SessionToken),
}

/// A blocking client rooted at one Tracel console URL.
#[derive(Clone)]
pub struct Console {
    inner: Arc<ConsoleInner>,
}

struct ConsoleInner {
    client: tracel_client::Client,
}

impl Console {
    /// Creates a console client without making a network request.
    ///
    /// The exact path prefix is preserved and normalized with a trailing slash so versioned API
    /// paths are joined underneath it.
    pub fn new(url: impl AsRef<str>, auth: Auth) -> Result<Self, ConsoleError> {
        let url = normalize_base_url(url.as_ref())?;
        let client = match auth {
            Auth::Anonymous => tracel_client::Client::anonymous(url),
            Auth::Session(token) => {
                tracel_client::Client::from_url_with_session_token(url, token.expose_secret())
            }
        };

        Ok(Self {
            inner: Arc::new(ConsoleInner { client }),
        })
    }

    /// Returns the normalized console API base URL.
    pub fn base_url(&self) -> &Url {
        self.inner.client.base_url()
    }

    /// Returns the current user, or `None` when the session is absent or dead.
    ///
    /// A dead session is represented by the console as a successful `null` response and remains a
    /// value rather than [`ConsoleError::SessionExpired`].
    pub fn me(&self) -> Result<Option<User>, ConsoleError> {
        self.inner
            .client
            .get_current_user()
            .map(|user| {
                user.map(|user| User {
                    id: user._id,
                    username: user.username,
                    email: user.email,
                    namespace: Namespace::user(user.namespace),
                })
            })
            .map_err(Into::into)
    }

    /// Destroys the current session on the console.
    pub fn logout(&self) -> Result<(), ConsoleError> {
        self.inner.client.logout().map_err(Into::into)
    }

    /// Lists organizations available to the current session.
    pub fn organizations(&self) -> Result<Vec<Organization>, ConsoleError> {
        self.inner
            .client
            .get_user_organizations()
            .map(|response| {
                response
                    .organizations
                    .into_iter()
                    .map(|organization| Organization {
                        name: organization.name,
                        namespace: Namespace::organization(organization.namespace),
                    })
                    .collect()
            })
            .map_err(Into::into)
    }

    /// Lists visible projects owned by a user or organization namespace.
    pub fn projects_of(
        &self,
        namespace: impl AsRef<Namespace>,
    ) -> Result<Vec<Project>, ConsoleError> {
        let namespace = namespace.as_ref();
        let projects = match namespace.kind {
            NamespaceKind::User => self.inner.client.list_user_projects(&namespace.name),
            NamespaceKind::Organization => self
                .inner
                .client
                .list_organization_projects(&namespace.name),
        }?;

        projects.into_iter().map(Project::try_from).collect()
    }

    /// Creates a project handle without performing I/O.
    pub fn project<O, P>(&self, (owner, project): (O, P)) -> ProjectHandle
    where
        O: Into<String>,
        P: Into<String>,
    {
        ProjectHandle {
            inner: Arc::clone(&self.inner),
            owner: owner.into(),
            project: project.into(),
        }
    }
}

impl fmt::Debug for Console {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Console")
            .field("base_url", &self.base_url())
            .finish_non_exhaustive()
    }
}

/// A cheap view of one project that shares its console client's session.
#[derive(Clone)]
pub struct ProjectHandle {
    inner: Arc<ConsoleInner>,
    owner: String,
    project: String,
}

impl ProjectHandle {
    /// Returns the project's owner namespace.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Returns the project name.
    pub fn name(&self) -> &str {
        &self.project
    }

    /// Fetches project details.
    ///
    /// Private and nonexistent projects both return [`ConsoleError::NotVisible`] because the
    /// console intentionally does not reveal which case applies.
    pub fn get(&self) -> Result<Project, ConsoleError> {
        self.inner
            .client
            .get_project(&self.owner, &self.project)
            .map_err(ConsoleError::from)
            .and_then(Project::try_from)
    }

    /// Returns model operations already scoped to this project without performing I/O.
    pub fn models(&self) -> Models {
        Models::new(Arc::new(ConsoleModelOps {
            inner: Arc::clone(&self.inner),
            owner: self.owner.clone(),
            project: self.project.clone(),
            transfer_client: ReqwestTransferClient::new(),
        }))
    }
}

impl fmt::Debug for ProjectHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectHandle")
            .field("owner", &self.owner)
            .field("project", &self.project)
            .finish()
    }
}

#[derive(Clone)]
struct ConsoleModelOps {
    inner: Arc<ConsoleInner>,
    owner: String,
    project: String,
    transfer_client: ReqwestTransferClient,
}

impl ConsoleModelOps {
    fn resolve_route_version(&self, model: &str, id: &VersionId) -> Result<u32, ModelsError> {
        let response = self
            .inner
            .client
            .list_model_versions(&self.owner, &self.project, model)
            .map_err(|error| map_model_error(error, model))?;
        find_route_version(model, id, &response.items)
    }
}

impl ModelOps for ConsoleModelOps {
    fn list_models(&self) -> Result<Page<Model>, ModelsError> {
        let response = self
            .inner
            .client
            .list_models(&self.owner, &self.project)
            .map_err(map_scope_error)?;
        Ok(model_page_from_wire(response))
    }

    fn get_model(&self, name: &str) -> Result<Model, ModelsError> {
        self.inner
            .client
            .get_model(&self.owner, &self.project, name)
            .map(model_from_wire)
            .map_err(|error| map_model_error(error, name))
    }

    fn list_versions(&self, model: &str) -> Result<Page<ModelVersion>, ModelsError> {
        let response = self
            .inner
            .client
            .list_model_versions(&self.owner, &self.project, model)
            .map_err(|error| map_model_error(error, model))?;
        model_version_page_from_wire(response)
    }

    fn get_version(&self, model: &str, id: &VersionId) -> Result<ModelVersion, ModelsError> {
        let version = self.resolve_route_version(model, id)?;
        self.inner
            .client
            .get_model_version(&self.owner, &self.project, model, version)
            .map_err(|error| map_version_error(error, model, id))
            .and_then(model_version_from_wire)
    }

    fn fetch_version_files(
        &self,
        model: &str,
        id: &VersionId,
    ) -> Result<Vec<VersionFileSource>, ModelsError> {
        let version = self.resolve_route_version(model, id)?;
        let response = self
            .inner
            .client
            .presign_model_download(&self.owner, &self.project, model, version)
            .map_err(|error| map_version_error(error, model, id))?;
        Ok(file_sources_from_wire(&self.transfer_client, response))
    }
}

fn model_page_from_wire(response: tracel_client::response::ModelListResponse) -> Page<Model> {
    Page {
        items: response.items.into_iter().map(model_from_wire).collect(),
        total: response.total,
    }
}

fn model_from_wire(value: tracel_client::response::ModelResponse) -> Model {
    Model {
        id: value.id,
        name: value.name,
        description: value.description,
        created_at: value.created_at,
        version_count: value.version_count,
        latest_version: value.latest_version,
    }
}

fn model_version_page_from_wire(
    response: tracel_client::response::ModelVersionListResponse,
) -> Result<Page<ModelVersion>, ModelsError> {
    let items = response
        .items
        .into_iter()
        .map(model_version_from_wire)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Page {
        items,
        total: response.total,
    })
}

fn model_version_from_wire(
    value: tracel_client::response::ModelVersionResponse,
) -> Result<ModelVersion, ModelsError> {
    let manifest: VersionManifest = serde_json::from_value(value.manifest)
        .map_err(|error| ModelsError::InvalidResponse(error.to_string()))?;

    Ok(ModelVersion {
        id: VersionId::new(value.id),
        experiment: value.experiment.map(|source| ExperimentSource {
            id: source.id,
            experiment_num: source.experiment_num,
        }),
        version: value.version,
        size_bytes: value.size,
        checksum: value.checksum,
        created_at: value.created_at,
        manifest,
        metadata: value.metadata,
    })
}

fn find_route_version(
    model: &str,
    id: &VersionId,
    versions: &[tracel_client::response::ModelVersionResponse],
) -> Result<u32, ModelsError> {
    versions
        .iter()
        .find_map(|version| (version.id == id.as_str()).then_some(version.version))
        .ok_or_else(|| ModelsError::VersionNotFound {
            model: model.to_string(),
            id: id.clone(),
        })
}

fn file_sources_from_wire(
    transfer_client: &ReqwestTransferClient,
    response: tracel_client::response::ModelDownloadResponse,
) -> Vec<VersionFileSource> {
    response
        .files
        .into_iter()
        .map(|file| {
            let transfer_client = transfer_client.clone();
            let url = file.url;
            VersionFileSource::new(
                VersionFile {
                    rel_path: file.rel_path,
                    size_bytes: file.size_bytes,
                    checksum: file.checksum,
                },
                move || {
                    transfer_client
                        .get_reader(&url)
                        .map_err(|error| ModelsError::Transport(error.to_string()))
                },
            )
        })
        .collect()
}

fn map_scope_error(error: ClientError) -> ModelsError {
    if client_error_is_not_found(&error) {
        return ModelsError::ScopeNotFound;
    }
    map_model_client_error(error)
}

fn map_model_error(error: ClientError, name: &str) -> ModelsError {
    if client_error_is_not_found(&error) {
        return ModelsError::ModelNotFound {
            name: name.to_string(),
        };
    }
    map_model_client_error(error)
}

fn map_version_error(error: ClientError, model: &str, id: &VersionId) -> ModelsError {
    if client_error_is_not_found(&error) {
        return ModelsError::VersionNotFound {
            model: model.to_string(),
            id: id.clone(),
        };
    }
    map_model_client_error(error)
}

fn client_error_is_not_found(error: &ClientError) -> bool {
    error.is_not_found()
        || matches!(
            error,
            ClientError::ApiError { status, .. } if *status == reqwest::StatusCode::NOT_FOUND
        )
}

fn map_model_client_error(error: ClientError) -> ModelsError {
    match error {
        ClientError::Unauthorized | ClientError::BadSessionId => {
            ModelsError::Authentication(error.to_string())
        }
        ClientError::ApiError { status, body }
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN =>
        {
            ModelsError::Authentication(format!("HTTP {status}: {body}"))
        }
        ClientError::Serialization(error) => ModelsError::InvalidResponse(error.to_string()),
        error => ModelsError::Transport(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_normalizes_root_and_path_prefix_urls() {
        let root = Console::new("http://localhost:9001", Auth::Anonymous).unwrap();
        let api = Console::new("https://console.example/api", Auth::Anonymous).unwrap();
        let already_normalized =
            Console::new("https://console.example/api/", Auth::Anonymous).unwrap();

        assert_eq!(root.base_url().as_str(), "http://localhost:9001/");
        assert_eq!(api.base_url().as_str(), "https://console.example/api/");
        assert_eq!(
            already_normalized.base_url().as_str(),
            "https://console.example/api/"
        );
        assert_eq!(
            api.base_url().join("v1/user").unwrap().as_str(),
            "https://console.example/api/v1/user"
        );
    }

    #[test]
    fn console_rejects_non_http_and_qualified_base_urls() {
        assert!(Console::new("file:///tmp/console", Auth::Anonymous).is_err());
        assert!(Console::new("https://console.example/api?q=1", Auth::Anonymous).is_err());
    }

    #[test]
    fn model_list_fixture_maps_to_tenancy_free_domain() {
        let wire: tracel_client::response::ModelListResponse = serde_json::from_str(
            r#"{
                "items": [{
                    "id": "0198f0a1-0000-7000-8000-000000000000",
                    "project_id": 3,
                    "name": "resnet",
                    "description": null,
                    "created_by": {"id": 7, "username": "ada", "namespace": "ada"},
                    "created_at": "2026-03-05 18:45:43.397",
                    "version_count": 2,
                    "latest_version": 2
                }],
                "total": 1
            }"#,
        )
        .unwrap();

        let page = model_page_from_wire(wire);

        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "resnet");
        assert_eq!(page.items[0].latest_version, Some(2));
    }

    #[test]
    fn version_fixture_maps_manifest_metadata_and_opaque_id() {
        let wire: tracel_client::response::ModelVersionResponse = serde_json::from_str(
            r#"{
                "id": "0198f0a1-0000-7000-8000-000000000001",
                "experiment": {"id": 12, "experiment_num": 4},
                "version": 2,
                "size": 2048,
                "checksum": "sha256:abc",
                "created_by": {"id": 7, "username": "ada", "namespace": "ada"},
                "created_at": "2026-03-05 18:45:43.397",
                "manifest": {"files": [{
                    "rel_path": "weights.bpk",
                    "size_bytes": 2048,
                    "checksum": "sha256:abc"
                }]},
                "metadata": {"burnpack": {"schema": 1}}
            }"#,
        )
        .unwrap();

        let version = model_version_from_wire(wire).unwrap();

        assert_eq!(version.id.as_str(), "0198f0a1-0000-7000-8000-000000000001");
        assert_eq!(version.experiment.unwrap().experiment_num, 4);
        assert_eq!(version.manifest.files[0].rel_path, "weights.bpk");
        assert_eq!(version.metadata["burnpack"]["schema"], 1);
    }

    #[test]
    fn legacy_version_fixture_defaults_metadata_to_null() {
        let wire: tracel_client::response::ModelVersionResponse = serde_json::from_str(
            r#"{
                "id": "opaque-id",
                "experiment": null,
                "version": 2,
                "size": 0,
                "checksum": "sha256:empty",
                "created_by": {"id": 7, "username": "ada", "namespace": "ada"},
                "created_at": "2026-03-05 18:45:43.397",
                "manifest": {"files": []}
            }"#,
        )
        .unwrap();

        assert!(model_version_from_wire(wire).unwrap().metadata.is_null());
    }

    #[test]
    fn route_version_resolution_uses_only_the_opaque_identity() {
        let wire: tracel_client::response::ModelVersionListResponse = serde_json::from_str(
            r#"{
                "items": [{
                    "id": "opaque-id",
                    "experiment": null,
                    "version": 42,
                    "size": 0,
                    "checksum": "sha256:empty",
                    "created_by": {"id": 7, "username": "ada", "namespace": "ada"},
                    "created_at": "2026-03-05 18:45:43.397",
                    "manifest": {"files": []}
                }],
                "total": 1
            }"#,
        )
        .unwrap();

        assert_eq!(
            find_route_version("resnet", &VersionId::new("opaque-id"), &wire.items).unwrap(),
            42
        );
        assert!(matches!(
            find_route_version("resnet", &VersionId::new("missing"), &wire.items),
            Err(ModelsError::VersionNotFound { model, .. }) if model == "resnet"
        ));
    }

    #[test]
    fn download_plan_fixture_maps_verified_file_descriptors() {
        let wire: tracel_client::response::ModelDownloadResponse = serde_json::from_str(
            r#"{
                "files": [{
                    "rel_path": "model.bpk",
                    "url": "https://blobs.example.com/model.bpk?signature=x",
                    "size_bytes": 1048576,
                    "checksum": "9f86d0818"
                }]
            }"#,
        )
        .unwrap();

        let files = file_sources_from_wire(&ReqwestTransferClient::new(), wire);

        assert_eq!(files[0].file().rel_path, "model.bpk");
        assert_eq!(files[0].file().size_bytes, 1048576);
        assert_eq!(files[0].file().checksum, "9f86d0818");
    }

    #[test]
    fn session_tokens_are_redacted_in_debug_output() {
        let token = SessionToken::new("do-not-log-me");

        assert_eq!(format!("{token:?}"), "SessionToken([REDACTED])");
        assert!(!format!("{:?}", Auth::Session(token)).contains("do-not-log-me"));
    }
}
