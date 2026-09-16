use std::sync::Arc;

use bytes::Bytes;
use futures::{TryStreamExt, stream};
use serde::Deserialize;
use tracel_artifact::upload::{
    MultipartUploadFile, MultipartUploadPart, MultipartUploadSource, UploadError, upload_multipart,
};
use tracel_artifact::{TransferClient, TransferError, TransferObserver};
use tracel_client::{
    console::model::request::{
        CreateModelRequest, ModelFileSpecRequest, RequestModelVersionUploadRequest,
    },
    console::model::response::{
        ModelDownloadResponse, ModelListResponse, ModelResponse, ModelVersionListResponse,
        ModelVersionResponse,
    },
    error::ClientError,
};
use tracel_models::{
    Model, ModelOps, ModelVersion, ModelsError, VersionFile, VersionFileSource, VersionId,
    VersionManifest, VersionSpec,
};
use tracel_task::{Job, Streaming};

use crate::ConsoleError;
use crate::console::{ConsoleInner, ProjectScope};
use crate::error::client_error_is_not_found;
use crate::wire::console_timestamp;

#[derive(Clone)]
pub struct ConsoleModelOps {
    pub scope: Arc<ProjectScope>,
}

impl ConsoleModelOps {
    fn route_version(&self, model: &str, id: &VersionId) -> Result<u32, ModelsError> {
        id.as_str()
            .parse()
            .map_err(|_| ModelsError::VersionNotFound {
                model: model.to_string(),
                version: VersionSpec::Exact(id.clone()),
            })
    }
}

impl ModelOps for ConsoleModelOps {
    fn list_models(&self) -> Job<Vec<Model>, ModelsError> {
        let this = self.clone();
        self.scope.console.attach(async move {
            let scope = &this.scope;
            scope
                .console
                .client
                .list_models(&scope.owner, &scope.project)
                .await
                .map(models_from_wire)
                .map_err(console_failure)
        })
    }

    fn get_model(&self, name: String) -> Job<Model, ModelsError> {
        let this = self.clone();
        self.scope.console.attach(async move {
            let scope = &this.scope;
            scope
                .console
                .client
                .get_model(&scope.owner, &scope.project, &name)
                .await
                .map(model_from_wire)
                .map_err(|error| map_model_error(error, &name))
        })
    }

    fn list_versions(&self, model: String) -> Job<Vec<ModelVersion>, ModelsError> {
        let this = self.clone();
        self.scope.console.attach(async move {
            let scope = &this.scope;
            scope
                .console
                .client
                .list_model_versions(&scope.owner, &scope.project, &model)
                .await
                .map_err(|error| map_model_error(error, &model))
                .and_then(model_versions_from_wire)
        })
    }

    fn get_version(&self, model: String, spec: VersionSpec) -> Job<ModelVersion, ModelsError> {
        let this = self.clone();
        self.scope.console.attach(async move {
            let id = match &spec {
                VersionSpec::Exact(id) => id.clone(),
                VersionSpec::Latest => this
                    .list_versions(model.clone())
                    .await?
                    .into_iter()
                    .max_by_key(|version| version.version)
                    .map(|version| version.id)
                    .ok_or_else(|| ModelsError::VersionNotFound {
                        model: model.clone(),
                        version: spec.clone(),
                    })?,
            };

            let route = this.route_version(&model, &id)?;
            let scope = &this.scope;
            scope
                .console
                .client
                .get_model_version(&scope.owner, &scope.project, &model, route)
                .await
                .map_err(|error| map_version_error(error, &model, &id))
                .and_then(model_version_from_wire)
        })
    }

    fn fetch_version_files(
        &self,
        model: String,
        id: VersionId,
    ) -> Job<Vec<Box<dyn VersionFileSource>>, ModelsError> {
        let route = match self.route_version(&model, &id) {
            Ok(route) => route,
            Err(error) => return Job::failed(error),
        };
        let this = self.clone();
        self.scope.console.attach(async move {
            let scope = &this.scope;
            let console = &scope.console;
            console
                .client
                .presign_model_download(&scope.owner, &scope.project, &model, route)
                .await
                .map_err(|error| map_version_error(error, &model, &id))
                .map(|response| file_sources_from_wire(console, response))
        })
    }

    fn create_model(&self, name: String, description: Option<String>) -> Job<Model, ModelsError> {
        let this = self.clone();
        self.scope.console.attach(async move {
            let scope = &this.scope;
            scope
                .console
                .client
                .create_model(
                    &scope.owner,
                    &scope.project,
                    CreateModelRequest { name, description },
                )
                .await
                .map(model_from_wire)
                .map_err(console_failure)
        })
    }

    fn publish_version(
        &self,
        model: String,
        files: Vec<VersionFile>,
        contents: Arc<dyn MultipartUploadSource>,
        metadata: Option<serde_json::Value>,
        mut observer: Box<dyn TransferObserver>,
    ) -> Job<ModelVersion, ModelsError> {
        let this = self.clone();
        self.scope.console.attach(async move {
            let scope = &this.scope;
            let (owner, project) = (scope.owner.as_str(), scope.project.as_str());
            let client = &scope.console.client;
            let request = RequestModelVersionUploadRequest {
                files: files
                    .iter()
                    .map(|file| ModelFileSpecRequest {
                        rel_path: file.rel_path.clone(),
                        size_bytes: file.size_bytes,
                        checksum: file.checksum.clone(),
                    })
                    .collect(),
                metadata,
            };
            let planned = client
                .request_model_version_upload(owner, project, &model, request)
                .await
                .map_err(|error| map_model_error(error, &model))?;

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

            upload_multipart(
                &scope.console.transfer,
                &*contents,
                &uploads,
                &mut *observer,
            )
            .await
            .map_err(model_upload_failure)?;

            let version = planned.version;
            client
                .complete_model_version_upload(owner, project, &model, version)
                .await
                .map_err(|error| map_model_error(error, &model))?;

            client
                .get_model_version(owner, project, &model, version)
                .await
                .map_err(|error| map_model_error(error, &model))
                .and_then(model_version_from_wire)
        })
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
        id: VersionId::new(value.version.to_string()),
        version: Some(value.version),
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

fn file_sources_from_wire(
    console: &Arc<ConsoleInner>,
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
                console: Arc::clone(console),
            }) as Box<dyn VersionFileSource>
        })
        .collect()
}

struct ConsoleVersionFileSource {
    file: VersionFile,
    url: String,
    console: Arc<ConsoleInner>,
}

impl VersionFileSource for ConsoleVersionFileSource {
    fn file(&self) -> &VersionFile {
        &self.file
    }

    fn open(&self, _canonical_path: String) -> Streaming<Bytes, TransferError> {
        let transfer = self.console.transfer.clone();
        let url = self.url.clone();
        let size = self.file.size_bytes;
        self.console.attach_stream(
            stream::once(async move { transfer.get(&url, Some(size)).await }).try_flatten(),
        )
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
            version: VersionSpec::Exact(id.clone()),
        };
    }
    console_failure(error)
}

fn console_failure(error: ClientError) -> ModelsError {
    match ConsoleError::from(error) {
        ConsoleError::Transport(reason) => ModelsError::Transport(reason),
        error => ModelsError::other(error),
    }
}

fn model_upload_failure(error: UploadError) -> ModelsError {
    match error {
        UploadError::Cancelled { .. } => ModelsError::Cancelled,
        error @ UploadError::Transfer { .. } => ModelsError::Transport(error.to_string()),
        error => ModelsError::other(error),
    }
}
