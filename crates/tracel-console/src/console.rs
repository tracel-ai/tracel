use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use chrono::{DateTime, NaiveDateTime, Utc};

use serde::Deserialize;
use tracel_artifact::TransferObserver;
use tracel_artifact::upload::{
    MultipartUploadFile, MultipartUploadPart, MultipartUploadSource,
    upload_bundle_multipart_with_client_and_observer,
};
use tracel_artifact::{FileTransferClient, ReqwestTransferClient};
use tracel_client::{
    console::{
        Client, Env, TracelCredentials,
        model::request::{
            CreateModelRequest, ModelFileSpecRequest, RequestModelVersionUploadRequest,
        },
        model::response::{
            ModelDownloadResponse, ModelListResponse, ModelResponse, ModelVersionListResponse,
            ModelVersionResponse,
        },
    },
    error::ClientError,
};
use tracel_models::{
    Model, ModelOps, ModelVersion, Models, ModelsError, VersionFile, VersionFileReader,
    VersionFileSource, VersionId, VersionManifest,
};
use url::Url;

use crate::{ConsoleError, Namespace, NamespaceKind, Organization, Project, User};

/// A blocking client rooted at one Tracel console URL.
#[derive(Clone)]
pub struct Console {
    inner: Arc<ConsoleInner>,
}

struct ConsoleInner {
    client: Client,
    transfer_client: ReqwestTransferClient,
    model_version_routes: ModelVersionRoutes,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ModelVersionRouteKey {
    owner: String,
    project: String,
    model: String,
    id: VersionId,
}

/// Remembers which numeric route each opaque version identity resolves to, so a version is
/// fetched with one request once its listing has been seen.
#[derive(Default)]
struct ModelVersionRoutes {
    routes: Mutex<HashMap<ModelVersionRouteKey, u32>>,
}

impl ModelVersionRoutes {
    /// Recovers a poisoned lock: the routes are independent, so the ones already learned
    /// stay usable.
    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<ModelVersionRouteKey, u32>> {
        self.routes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn get(&self, owner: &str, project: &str, model: &str, id: &VersionId) -> Option<u32> {
        let key = ModelVersionRouteKey {
            owner: owner.to_string(),
            project: project.to_string(),
            model: model.to_string(),
            id: id.clone(),
        };
        self.entries().get(&key).copied()
    }

    fn remember(&self, owner: &str, project: &str, model: &str, versions: &[ModelVersionResponse]) {
        let mut entries = self.entries();
        for version in versions {
            entries.insert(
                ModelVersionRouteKey {
                    owner: owner.to_string(),
                    project: project.to_string(),
                    model: model.to_string(),
                    id: VersionId::new(&version.id),
                },
                version.version,
            );
        }
    }
}

impl Console {
    /// Connects to a console and verifies the credentials.
    pub fn connect(env: Env, credentials: &TracelCredentials) -> Result<Self, ConsoleError> {
        let client = Client::connect(env, credentials)?;

        Ok(Self {
            inner: Arc::new(ConsoleInner {
                client,
                transfer_client: ReqwestTransferClient::new(),
                model_version_routes: ModelVersionRoutes::default(),
            }),
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
    pub fn project<O, P>(&self, owner: O, project: P) -> ProjectHandle
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
    /// Private and nonexistent projects both return [`ConsoleError::NotFound`] because the
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
            transfer_client: self.inner.transfer_client.clone(),
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
        if let Some(version) =
            self.inner
                .model_version_routes
                .get(&self.owner, &self.project, model, id)
        {
            return Ok(version);
        }

        let response = self
            .inner
            .client
            .list_model_versions(&self.owner, &self.project, model)
            .map_err(|error| map_model_error(error, model))?;
        self.inner.model_version_routes.remember(
            &self.owner,
            &self.project,
            model,
            &response.items,
        );
        find_route_version(model, id, &response.items)
    }
}

impl ModelOps for ConsoleModelOps {
    fn list_models(&self) -> Result<Vec<Model>, ModelsError> {
        let response = self
            .inner
            .client
            .list_models(&self.owner, &self.project)
            .map_err(console_failure)?;
        Ok(models_from_wire(response))
    }

    fn get_model(&self, name: &str) -> Result<Model, ModelsError> {
        self.inner
            .client
            .get_model(&self.owner, &self.project, name)
            .map(model_from_wire)
            .map_err(|error| map_model_error(error, name))
    }

    fn list_versions(&self, model: &str) -> Result<Vec<ModelVersion>, ModelsError> {
        let response = self
            .inner
            .client
            .list_model_versions(&self.owner, &self.project, model)
            .map_err(|error| map_model_error(error, model))?;
        self.inner.model_version_routes.remember(
            &self.owner,
            &self.project,
            model,
            &response.items,
        );
        model_versions_from_wire(response)
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
    ) -> Result<Vec<Box<dyn VersionFileSource>>, ModelsError> {
        let version = self.resolve_route_version(model, id)?;
        let response = self
            .inner
            .client
            .presign_model_download(&self.owner, &self.project, model, version)
            .map_err(|error| map_version_error(error, model, id))?;
        Ok(file_sources_from_wire(&self.transfer_client, response))
    }

    fn create_model(&self, name: &str, description: Option<&str>) -> Result<Model, ModelsError> {
        self.inner
            .client
            .create_model(
                &self.owner,
                &self.project,
                CreateModelRequest {
                    name: name.to_string(),
                    description: description.map(str::to_string),
                },
            )
            .map(model_from_wire)
            .map_err(console_failure)
    }

    fn publish_version(
        &self,
        model: &str,
        files: &[VersionFile],
        contents: &dyn MultipartUploadSource,
        metadata: Option<&serde_json::Value>,
        mut observer: &mut dyn TransferObserver,
    ) -> Result<ModelVersion, ModelsError> {
        let request = RequestModelVersionUploadRequest {
            files: files
                .iter()
                .map(|file| ModelFileSpecRequest {
                    rel_path: file.rel_path.clone(),
                    size_bytes: file.size_bytes,
                    checksum: file.checksum.clone(),
                })
                .collect(),
            metadata: metadata.cloned(),
        };
        let planned = self
            .inner
            .client
            .request_model_version_upload(&self.owner, &self.project, model, request)
            .map_err(|error| map_model_error(error, model))?;

        let uploads = planned
            .files
            .into_iter()
            .map(|file| MultipartUploadFile {
                rel_path: file.rel_path,
                parts: file
                    .urls
                    .parts
                    .into_iter()
                    .map(|part| MultipartUploadPart {
                        part: part.part,
                        url: part.url,
                        size_bytes: part.size_bytes,
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();

        upload_bundle_multipart_with_client_and_observer(
            &self.transfer_client,
            &contents,
            &uploads,
            &mut observer,
        )
        .map_err(ModelsError::other)?;

        self.inner
            .client
            .complete_model_version_upload(&self.owner, &self.project, model, planned.version)
            .map_err(|error| map_model_error(error, model))?;

        self.inner
            .client
            .get_model_version(&self.owner, &self.project, model, planned.version)
            .map_err(|error| map_model_error(error, model))
            .and_then(model_version_from_wire)
    }
}

fn models_from_wire(response: ModelListResponse) -> Vec<Model> {
    response.items.into_iter().map(model_from_wire).collect()
}

fn model_from_wire(value: ModelResponse) -> Model {
    Model {
        id: value.id,
        name: value.name,
        description: value.description,
        published_by: Some(value.created_by.username),
        created_at: console_timestamp(&value.created_at),
        version_count: value.version_count,
        latest_version: value.latest_version,
    }
}

fn model_versions_from_wire(
    response: ModelVersionListResponse,
) -> Result<Vec<ModelVersion>, ModelsError> {
    response
        .items
        .into_iter()
        .map(model_version_from_wire)
        .collect()
}

fn model_version_from_wire(value: ModelVersionResponse) -> Result<ModelVersion, ModelsError> {
    let manifest: WireManifest = serde_json::from_value(value.manifest)
        .map_err(|error| ModelsError::other(ConsoleError::InvalidResponse(error.to_string())))?;

    Ok(ModelVersion {
        id: VersionId::new(value.id),
        version: value.version,
        size_bytes: value.size,
        checksum: value.checksum,
        published_by: Some(value.created_by.username),
        created_at: console_timestamp(&value.created_at),
        manifest: manifest.into(),
        metadata: value.metadata,
    })
}

/// The manifest as this console writes it, so the model domain never has to name a field the
/// way one backend happens to spell it.
#[derive(Deserialize)]
struct WireManifest {
    files: Vec<WireManifestFile>,
}

#[derive(Deserialize)]
struct WireManifestFile {
    rel_path: String,
    size_bytes: u64,
    checksum: String,
}

impl From<WireManifest> for VersionManifest {
    fn from(value: WireManifest) -> Self {
        VersionManifest {
            files: value
                .files
                .into_iter()
                .map(|file| VersionFile {
                    rel_path: file.rel_path,
                    size_bytes: file.size_bytes,
                    checksum: file.checksum,
                })
                .collect(),
        }
    }
}

/// Reads the console's timestamps, which come either RFC 3339 or as naive UTC. An unreadable
/// one is left absent rather than failing a read over a display field.
fn console_timestamp(value: &str) -> Option<SystemTime> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
                .map(|naive| naive.and_utc())
        })
        .ok()
        .map(SystemTime::from)
}

fn find_route_version(
    model: &str,
    id: &VersionId,
    versions: &[ModelVersionResponse],
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
    response: ModelDownloadResponse,
) -> Vec<Box<dyn VersionFileSource>> {
    response
        .files
        .into_iter()
        .map(|file| {
            Box::new(ConsoleVersionFileSource {
                file: VersionFile {
                    rel_path: file.rel_path,
                    size_bytes: file.size_bytes,
                    checksum: file.checksum,
                },
                url: file.url,
                transfer_client: transfer_client.clone(),
            }) as Box<dyn VersionFileSource>
        })
        .collect()
}

struct ConsoleVersionFileSource {
    file: VersionFile,
    url: String,
    transfer_client: ReqwestTransferClient,
}

impl VersionFileSource for ConsoleVersionFileSource {
    fn file(&self) -> &VersionFile {
        &self.file
    }

    fn open(&self, _canonical_path: &str) -> Result<VersionFileReader, ModelsError> {
        self.transfer_client
            .get_reader(&self.url)
            .map_err(|error| ModelsError::other(ConsoleError::Transport(error.to_string())))
    }
}

fn map_model_error(error: ClientError, name: &str) -> ModelsError {
    if client_error_is_not_found(&error) {
        return ModelsError::ModelNotFound {
            name: name.to_string(),
        };
    }
    console_failure(error)
}

fn map_version_error(error: ClientError, model: &str, id: &VersionId) -> ModelsError {
    if client_error_is_not_found(&error) {
        return ModelsError::VersionNotFound {
            model: model.to_string(),
            id: id.clone(),
        };
    }
    console_failure(error)
}

fn client_error_is_not_found(error: &ClientError) -> bool {
    error.is_not_found()
        || matches!(
            error,
            ClientError::ApiError { status, .. } if status_is_not_found(*status)
        )
}

fn status_is_not_found(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::NOT_FOUND
}

/// Hands a client failure to the model domain as this console's own.
fn console_failure(error: ClientError) -> ModelsError {
    ModelsError::other(ConsoleError::from(error))
}
